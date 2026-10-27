//! Thin wrapper around the `git` binary.
//!
//! All output is handled as bytes. Nothing is stripped or re-encoded, so diffs
//! containing `\r`, form feeds or non-UTF-8 content survive the round trip.
//!
//! Editors and git clients re-scan a repository after every commit, and a
//! `git status` that refreshes the index briefly holds `index.lock`. Commands
//! that lose that race are retried for up to [`DEFAULT_LOCK_WAIT`] instead of
//! failing the commit.

use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long a command waits for another process to release a git lock file.
pub const DEFAULT_LOCK_WAIT: Duration = Duration::from_secs(10);

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("could not run git: {0}")]
    Spawn(#[from] std::io::Error),
    #[error("git {args} failed with exit code {code}: {stderr}")]
    Failed {
        args: String,
        code: i32,
        stderr: String,
    },
}

pub type GitResult<T> = Result<T, GitError>;

#[derive(Debug, Clone)]
pub struct GitOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub code: i32,
}

impl GitOutput {
    pub fn stdout_str(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    pub fn stderr_str(&self) -> String {
        String::from_utf8_lossy(&self.stderr).trim().to_string()
    }

    pub fn success(&self) -> bool {
        self.code == 0
    }
}

/// How the child's standard streams are connected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stdio3 {
    /// Capture stdout and stderr, feed stdin from a buffer (or nothing).
    Captured,
    /// Inherit the terminal. Used outside the TUI so hooks and editors can talk to the user.
    Inherit,
}

#[derive(Debug, Clone)]
pub struct GitRunner {
    cwd: PathBuf,
    extra_env: Vec<(OsString, OsString)>,
    lock_wait: Duration,
}

impl GitRunner {
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        Self {
            cwd: cwd.into(),
            extra_env: Vec::new(),
            lock_wait: DEFAULT_LOCK_WAIT,
        }
    }

    /// Adds an environment variable for every git invocation (used by tests to isolate config).
    pub fn with_env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.extra_env.push((key.into(), value.into()));
        self
    }

    /// Changes how long commands wait for another process to release a lock file.
    pub fn with_lock_wait(mut self, wait: Duration) -> Self {
        self.lock_wait = wait;
        self
    }

    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    pub fn with_cwd(&self, cwd: impl Into<PathBuf>) -> Self {
        Self {
            cwd: cwd.into(),
            extra_env: self.extra_env.clone(),
            lock_wait: self.lock_wait,
        }
    }

    fn command(&self, args: &[&OsStr]) -> Command {
        let mut cmd = Command::new("git");
        cmd.current_dir(&self.cwd);
        // Raw UTF-8 paths in diff headers; git still quotes paths with control characters.
        cmd.arg("-c").arg("core.quotePath=false");
        cmd.args(args);
        // Never take `index.lock` just to refresh stat data in `git status`; the explicit
        // `update-index --refresh` in the preflight does that when it matters.
        cmd.env("GIT_OPTIONAL_LOCKS", "0");
        for (k, v) in &self.extra_env {
            cmd.env(k, v);
        }
        cmd
    }

    fn run_once(&self, args: &[&OsStr], stdin: Option<&[u8]>) -> GitResult<GitOutput> {
        let mut cmd = self.command(args);
        cmd.stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = cmd.spawn()?;
        if let Some(data) = stdin {
            let mut handle = child.stdin.take().expect("stdin piped");
            // A write error here means git exited early; the exit status carries the reason.
            let _ = handle.write_all(data);
            drop(handle);
        }
        let out = child.wait_with_output()?;
        Ok(GitOutput {
            stdout: out.stdout,
            stderr: out.stderr,
            code: out.status.code().unwrap_or(-1),
        })
    }

    /// Runs git with captured output and returns it regardless of the exit code.
    ///
    /// A command that fails because another process holds a lock file is retried
    /// with backoff until the lock clears or the lock wait is exhausted; nothing
    /// has happened in the repository when git fails that early.
    pub fn run_raw<S: AsRef<OsStr>>(
        &self,
        args: &[S],
        stdin: Option<&[u8]>,
    ) -> GitResult<GitOutput> {
        let args: Vec<&OsStr> = args.iter().map(|a| a.as_ref()).collect();
        let started = Instant::now();
        let mut delay = Duration::from_millis(25);
        loop {
            let mut out = self.run_once(&args, stdin)?;
            if out.success() || !is_lock_contention(&args, &out) {
                return Ok(out);
            }
            let remaining = self.lock_wait.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                out.stderr.extend_from_slice(
                    format!(
                        "\n\nVibeGit waited {:?} for the lock. Another program (an editor or \
                         git client) keeps locking the repository, or the lock file is stale.",
                        self.lock_wait
                    )
                    .as_bytes(),
                );
                return Ok(out);
            }
            std::thread::sleep(delay.min(remaining));
            delay = (delay * 2).min(Duration::from_millis(500));
        }
    }

    /// Runs git with captured output and fails on a non-zero exit code.
    pub fn run<S: AsRef<OsStr>>(&self, args: &[S], stdin: Option<&[u8]>) -> GitResult<GitOutput> {
        let out = self.run_raw(args, stdin)?;
        if out.success() {
            Ok(out)
        } else {
            Err(failed(args, &out))
        }
    }

    /// Runs git on the terminal (for commits/pushes outside the TUI). Returns the exit code.
    ///
    /// Stdin and stdout are inherited so editors and hooks work as usual. Stderr is
    /// streamed to the terminal through a pipe and kept, so a lost race for a lock
    /// file is detected and retried exactly like in [`run_raw`](Self::run_raw).
    pub fn run_inherit<S: AsRef<OsStr>>(&self, args: &[S]) -> GitResult<i32> {
        let args: Vec<&OsStr> = args.iter().map(|a| a.as_ref()).collect();
        let started = Instant::now();
        let mut delay = Duration::from_millis(25);
        let mut announced = false;
        loop {
            let mut cmd = self.command(&args);
            cmd.stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::piped());
            let mut child = cmd.spawn()?;
            let mut pipe = child.stderr.take().expect("stderr piped");
            let reader = std::thread::spawn(move || {
                let mut kept = Vec::new();
                let mut chunk = [0u8; 4096];
                let mut terminal = std::io::stderr();
                loop {
                    match pipe.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let _ = terminal.write_all(&chunk[..n]);
                            let _ = terminal.flush();
                            kept.extend_from_slice(&chunk[..n]);
                        }
                    }
                }
                kept
            });
            let status = child.wait()?;
            let out = GitOutput {
                stdout: Vec::new(),
                stderr: reader.join().unwrap_or_default(),
                code: status.code().unwrap_or(-1),
            };
            if out.success() || !is_lock_contention(&args, &out) {
                return Ok(out.code);
            }
            let remaining = self.lock_wait.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                eprintln!(
                    "vibegit: waited {:?} for the lock. Another program (an editor or git \
                     client) keeps locking the repository, or the lock file is stale.",
                    self.lock_wait
                );
                return Ok(out.code);
            }
            if !announced {
                eprintln!("vibegit: another git process holds a lock file; retrying...");
                announced = true;
            }
            std::thread::sleep(delay.min(remaining));
            delay = (delay * 2).min(Duration::from_millis(500));
        }
    }
}

/// Whether a failed command lost the race for a git lock file.
///
/// Git reports `fatal: Unable to create '<dir>/index.lock': File exists.` (the
/// wording of the second half is locale dependent). `update-index -q` exits
/// 128 without any message in the same situation.
fn is_lock_contention(args: &[&OsStr], out: &GitOutput) -> bool {
    let stderr = String::from_utf8_lossy(&out.stderr);
    if stderr.contains(".lock'") && stderr.to_lowercase().contains("exist") {
        return true;
    }
    out.code == 128
        && stderr.trim().is_empty()
        && args.first().is_some_and(|a| *a == "update-index")
}

pub fn failed<S: AsRef<OsStr>>(args: &[S], out: &GitOutput) -> GitError {
    let shown: Vec<String> = args
        .iter()
        .map(|a| a.as_ref().to_string_lossy().into_owned())
        .collect();
    let mut stderr = out.stderr_str();
    if stderr.is_empty() {
        stderr = out.stdout_str().trim().to_string();
    }
    GitError::Failed {
        args: shown.join(" "),
        code: out.code,
        stderr,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_repo() -> (tempfile::TempDir, GitRunner) {
        let dir = tempfile::tempdir().unwrap();
        let runner = GitRunner::new(dir.path())
            .with_env("HOME", dir.path())
            .with_env("GIT_CONFIG_NOSYSTEM", "1");
        runner.run(&["init", "-q"], None).unwrap();
        fs::write(dir.path().join("a.txt"), "a\n").unwrap();
        (dir, runner)
    }

    fn output(code: i32, stderr: &str) -> GitOutput {
        GitOutput {
            stdout: Vec::new(),
            stderr: stderr.as_bytes().to_vec(),
            code,
        }
    }

    #[test]
    fn recognises_lock_contention() {
        let add: Vec<&OsStr> = vec![OsStr::new("add")];
        let refresh: Vec<&OsStr> = vec![OsStr::new("update-index"), OsStr::new("-q")];
        assert!(is_lock_contention(
            &add,
            &output(
                128,
                "fatal: Unable to create '/r/.git/index.lock': File exists.\n\nAnother git process seems to be running"
            )
        ));
        assert!(is_lock_contention(
            &add,
            &output(
                128,
                "fatal: Impossible de créer '/r/.git/index.lock' : Le fichier existe."
            )
        ));
        assert!(is_lock_contention(&refresh, &output(128, "")));
        assert!(!is_lock_contention(&add, &output(128, "")));
        assert!(!is_lock_contention(
            &add,
            &output(1, "error: patch failed: a.txt:1")
        ));
        assert!(!is_lock_contention(
            &add,
            &output(128, "fatal: not a git repository")
        ));
    }

    #[test]
    fn waits_for_a_briefly_held_index_lock() {
        let (dir, runner) = temp_repo();
        let lock = dir.path().join(".git/index.lock");
        fs::write(&lock, "").unwrap();
        let holder = {
            let lock = lock.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(300));
                fs::remove_file(lock).unwrap();
            })
        };
        let started = Instant::now();
        runner.run(&["add", "a.txt"], None).unwrap();
        assert!(
            started.elapsed() >= Duration::from_millis(250),
            "did not wait"
        );
        holder.join().unwrap();
        let out = runner.run(&["ls-files"], None).unwrap();
        assert_eq!(out.stdout_str().trim(), "a.txt");
    }

    #[test]
    fn gives_up_on_a_stale_index_lock() {
        let (dir, runner) = temp_repo();
        let runner = runner.with_lock_wait(Duration::from_millis(200));
        fs::write(dir.path().join(".git/index.lock"), "").unwrap();
        let started = Instant::now();
        let err = runner.run(&["add", "a.txt"], None).unwrap_err();
        assert!(started.elapsed() >= Duration::from_millis(200));
        let text = err.to_string();
        assert!(
            text.contains("index.lock") && text.contains("waited 200ms"),
            "{text}"
        );
    }

    #[test]
    fn terminal_mode_waits_for_a_briefly_held_index_lock() {
        let (dir, runner) = temp_repo();
        let lock = dir.path().join(".git/index.lock");
        fs::write(&lock, "").unwrap();
        let holder = {
            let lock = lock.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(300));
                fs::remove_file(lock).unwrap();
            })
        };
        let started = Instant::now();
        let code = runner.run_inherit(&["add", "a.txt"]).unwrap();
        assert_eq!(code, 0);
        assert!(
            started.elapsed() >= Duration::from_millis(250),
            "did not wait"
        );
        holder.join().unwrap();
        let stale = runner.with_lock_wait(Duration::from_millis(100));
        fs::write(&lock, "").unwrap();
        assert_eq!(stale.run_inherit(&["add", "a.txt"]).unwrap(), 128);
    }

    #[test]
    fn status_does_not_take_optional_locks() {
        let (dir, runner) = temp_repo();
        fs::write(dir.path().join(".git/index.lock"), "").unwrap();
        let out = runner
            .run(&["status", "--porcelain=v2", "-z"], None)
            .unwrap();
        assert!(out.stdout_str().contains("a.txt"));
    }
}

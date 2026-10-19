//! Thin wrapper around the `git` binary.
//!
//! All output is handled as bytes. Nothing is stripped or re-encoded, so diffs
//! containing `\r`, form feeds or non-UTF-8 content survive the round trip.

use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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
}

impl GitRunner {
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        Self {
            cwd: cwd.into(),
            extra_env: Vec::new(),
        }
    }

    /// Adds an environment variable for every git invocation (used by tests to isolate config).
    pub fn with_env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.extra_env.push((key.into(), value.into()));
        self
    }

    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    pub fn with_cwd(&self, cwd: impl Into<PathBuf>) -> Self {
        Self {
            cwd: cwd.into(),
            extra_env: self.extra_env.clone(),
        }
    }

    fn command(&self, args: &[&OsStr]) -> Command {
        let mut cmd = Command::new("git");
        cmd.current_dir(&self.cwd);
        // Raw UTF-8 paths in diff headers; git still quotes paths with control characters.
        cmd.arg("-c").arg("core.quotePath=false");
        cmd.args(args);
        for (k, v) in &self.extra_env {
            cmd.env(k, v);
        }
        cmd
    }

    /// Runs git with captured output and returns it regardless of the exit code.
    pub fn run_raw<S: AsRef<OsStr>>(
        &self,
        args: &[S],
        stdin: Option<&[u8]>,
    ) -> GitResult<GitOutput> {
        let args: Vec<&OsStr> = args.iter().map(|a| a.as_ref()).collect();
        let mut cmd = self.command(&args);
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

    /// Runs git with captured output and fails on a non-zero exit code.
    pub fn run<S: AsRef<OsStr>>(&self, args: &[S], stdin: Option<&[u8]>) -> GitResult<GitOutput> {
        let out = self.run_raw(args, stdin)?;
        if out.success() {
            Ok(out)
        } else {
            Err(failed(args, &out))
        }
    }

    /// Runs git with inherited stdio (for commits/pushes outside the TUI). Returns the exit code.
    pub fn run_inherit<S: AsRef<OsStr>>(&self, args: &[S]) -> GitResult<i32> {
        let args: Vec<&OsStr> = args.iter().map(|a| a.as_ref()).collect();
        let mut cmd = self.command(&args);
        cmd.stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        let status = cmd.status()?;
        Ok(status.code().unwrap_or(-1))
    }
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

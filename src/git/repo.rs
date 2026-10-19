//! Repository operations built on git plumbing.
//!
//! Every command here is chosen so that its output does not depend on the
//! user's `diff.*`, `color.*`, pager or external-diff configuration, and so
//! that the working tree is never written. The only index writes are
//! `apply --cached`, `reset -- <paths>` and the stat refresh.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use super::runner::{failed, GitError, GitResult, GitRunner};
use super::status::{parse_porcelain_v2, StatusEntry};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InProgress {
    Merge,
    Rebase,
    CherryPick,
    Revert,
    Bisect,
}

impl InProgress {
    pub fn name(&self) -> &'static str {
        match self {
            InProgress::Merge => "merge",
            InProgress::Rebase => "rebase",
            InProgress::CherryPick => "cherry-pick",
            InProgress::Revert => "revert",
            InProgress::Bisect => "bisect",
        }
    }

    pub fn resolve_hint(&self) -> &'static str {
        match self {
            InProgress::Merge => "finish it with `git commit` or abort with `git merge --abort`",
            InProgress::Rebase => {
                "continue with `git rebase --continue` or abort with `git rebase --abort`"
            }
            InProgress::CherryPick => {
                "continue with `git cherry-pick --continue` or abort with `git cherry-pick --abort`"
            }
            InProgress::Revert => {
                "continue with `git revert --continue` or abort with `git revert --abort`"
            }
            InProgress::Bisect => "finish with `git bisect reset`",
        }
    }
}

#[derive(Debug, Clone)]
pub struct RepoState {
    pub root: PathBuf,
    pub git_dir: PathBuf,
    /// `None` when HEAD is detached.
    pub branch: Option<String>,
    /// `None` when HEAD is unborn (no commits yet).
    pub head: Option<String>,
    pub in_progress: Option<InProgress>,
}

impl RepoState {
    pub fn describe_head(&self) -> String {
        match (&self.branch, &self.head) {
            (Some(b), _) => b.clone(),
            (None, Some(sha)) => format!("detached HEAD at {}", &sha[..sha.len().min(10)]),
            (None, None) => "detached HEAD".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitMode {
    /// Capture hook output; stdin is closed. Used inside the TUI.
    Captured,
    /// Inherit the terminal so hooks and gpg can interact. Used by the non-interactive CLI.
    Inherit,
    /// Like `Inherit`, but opens the configured editor on the message first.
    Editor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitInfo {
    pub sha: String,
    pub short_sha: String,
    pub subject: String,
}

#[derive(Debug, Clone)]
pub struct GitRepo {
    runner: GitRunner,
    root: PathBuf,
    git_dir: PathBuf,
}

const EMPTY_TREE_FALLBACK: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

impl GitRepo {
    /// Finds the repository containing `cwd`.
    pub fn discover(cwd: &Path) -> GitResult<Self> {
        Self::discover_with(GitRunner::new(cwd))
    }

    /// Like `discover`, with a pre-configured runner (tests use this to isolate git config).
    pub fn discover_with(runner: GitRunner) -> GitResult<Self> {
        let out = runner.run(
            &["rev-parse", "--show-toplevel", "--absolute-git-dir"],
            None,
        )?;
        let text = out.stdout_str();
        let mut lines = text.lines();
        let root = lines.next().unwrap_or("").trim();
        let git_dir = lines.next().unwrap_or("").trim();
        if root.is_empty() {
            return Err(GitError::Failed {
                args: "rev-parse --show-toplevel".into(),
                code: 128,
                stderr: "this looks like a bare repository; VibeGit needs a working tree".into(),
            });
        }
        let root = PathBuf::from(root);
        Ok(Self {
            runner: runner.with_cwd(&root),
            root,
            git_dir: PathBuf::from(git_dir),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn git_dir(&self) -> &Path {
        &self.git_dir
    }

    pub fn runner(&self) -> &GitRunner {
        &self.runner
    }

    pub fn state(&self) -> GitResult<RepoState> {
        let head = self
            .runner
            .run_raw(&["rev-parse", "-q", "--verify", "HEAD"], None)?;
        let head = if head.success() {
            Some(head.stdout_str().trim().to_string())
        } else {
            None
        };
        let branch = self
            .runner
            .run_raw(&["symbolic-ref", "-q", "--short", "HEAD"], None)?;
        let branch = if branch.success() {
            Some(branch.stdout_str().trim().to_string())
        } else {
            None
        };
        let markers = [
            ("MERGE_HEAD", InProgress::Merge),
            ("rebase-merge", InProgress::Rebase),
            ("rebase-apply", InProgress::Rebase),
            ("CHERRY_PICK_HEAD", InProgress::CherryPick),
            ("REVERT_HEAD", InProgress::Revert),
            ("BISECT_LOG", InProgress::Bisect),
        ];
        let mut args: Vec<&str> = vec!["rev-parse"];
        for (name, _) in &markers {
            args.push("--git-path");
            args.push(name);
        }
        let out = self.runner.run(&args, None)?;
        let text = out.stdout_str();
        let mut in_progress = None;
        for (line, (_, kind)) in text.lines().zip(markers.iter()) {
            let path = self.root.join(line.trim());
            if path.exists() {
                in_progress = Some(*kind);
                break;
            }
        }
        Ok(RepoState {
            root: self.root.clone(),
            git_dir: self.git_dir.clone(),
            branch,
            head,
            in_progress,
        })
    }

    /// `HEAD` when it exists, otherwise the id of the empty tree.
    pub fn base_tree(&self) -> GitResult<String> {
        let head = self
            .runner
            .run_raw(&["rev-parse", "-q", "--verify", "HEAD"], None)?;
        if head.success() {
            return Ok("HEAD".to_string());
        }
        let out = self.runner.run_raw(&["mktree"], Some(b""))?;
        if out.success() {
            Ok(out.stdout_str().trim().to_string())
        } else {
            Ok(EMPTY_TREE_FALLBACK.to_string())
        }
    }

    pub fn status(&self) -> GitResult<Vec<StatusEntry>> {
        let out = self.runner.run(
            &["status", "--porcelain=v2", "-z", "--untracked-files=all"],
            None,
        )?;
        parse_porcelain_v2(&out.stdout).map_err(|e| GitError::Failed {
            args: "status --porcelain=v2".into(),
            code: 0,
            stderr: e.to_string(),
        })
    }

    /// Diff of tracked changes (index and worktree) against the base tree, as one patch.
    pub fn diff_tracked(&self) -> GitResult<Vec<u8>> {
        // Refresh stat information so diff-index does not report stat-dirty files.
        let _ = self.runner.run_raw(
            &[
                "update-index",
                "-q",
                "--refresh",
                "--unmerged",
                "--ignore-submodules",
            ],
            None,
        )?;
        let base = self.base_tree()?;
        let out = self.runner.run(
            &[
                "diff-index",
                "-p",
                "--binary",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
                "--no-renames",
                "--ignore-submodules=dirty",
                &base,
                "--",
            ],
            None,
        )?;
        Ok(out.stdout)
    }

    /// Diff that adds one untracked file.
    pub fn diff_untracked(&self, path: &str) -> GitResult<Vec<u8>> {
        let args = [
            "diff",
            "--no-index",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "--binary",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            "--",
            "/dev/null",
            path,
        ];
        let out = self.runner.run_raw(&args, None)?;
        // 1 means "differences" and is the normal outcome; a missing file also exits 1
        // but prints nothing.
        if (out.code == 0 || out.code == 1) && !out.stdout.is_empty() {
            Ok(out.stdout)
        } else {
            Err(failed(&args, &out))
        }
    }

    /// True when the index has no changes relative to the base tree.
    pub fn index_matches_base(&self) -> GitResult<bool> {
        let base = self.base_tree()?;
        let out = self
            .runner
            .run_raw(&["diff-index", "--cached", "--quiet", &base, "--"], None)?;
        Ok(out.code == 0)
    }

    pub fn apply_cached(&self, patch: &[u8]) -> GitResult<()> {
        self.runner.run(
            &["apply", "--cached", "--whitespace=nowarn", "-"],
            Some(patch),
        )?;
        Ok(())
    }

    /// Removes the given paths from the index (pathspec-limited, keeps merge state).
    pub fn unstage_paths(&self, paths: &[String]) -> GitResult<()> {
        if paths.is_empty() {
            return Ok(());
        }
        let mut args: Vec<&OsStr> = vec![
            OsStr::new("--literal-pathspecs"),
            OsStr::new("reset"),
            OsStr::new("-q"),
            OsStr::new("--"),
        ];
        args.extend(paths.iter().map(|p| OsStr::new(p.as_str())));
        self.runner.run(&args, None)?;
        Ok(())
    }

    /// Unstages everything without touching merge/rebase state (`git reset -- .` from the root).
    pub fn unstage_all(&self) -> GitResult<()> {
        self.runner.run(&["reset", "-q", "--", "."], None)?;
        Ok(())
    }

    pub fn recent_commit_messages(&self, n: usize) -> GitResult<Vec<String>> {
        let count = n.to_string();
        let out = self
            .runner
            .run_raw(&["log", "-n", &count, "-z", "--format=%B"], None)?;
        if !out.success() {
            return Ok(Vec::new());
        }
        Ok(out
            .stdout
            .split(|b| *b == 0)
            .map(|m| String::from_utf8_lossy(m).trim().to_string())
            .filter(|m| !m.is_empty())
            .collect())
    }

    /// One-line subjects of the submodule commits between `old` and `new` (best effort).
    pub fn submodule_log(&self, path: &str, old: &str, new: &str) -> Vec<String> {
        let runner = self.runner.with_cwd(self.root.join(path));
        let range = format!("{old}..{new}");
        match runner.run_raw(
            &["log", "--oneline", "--no-decorate", "-n", "20", &range],
            None,
        ) {
            Ok(out) if out.success() => out.stdout_str().lines().map(|l| l.to_string()).collect(),
            _ => Vec::new(),
        }
    }

    pub fn commit(&self, message: &str, mode: CommitMode) -> GitResult<CommitInfo> {
        let mut message = message.to_string();
        if !message.ends_with('\n') {
            message.push('\n');
        }
        match mode {
            CommitMode::Captured => {
                self.runner.run(
                    &["commit", "-q", "--cleanup=whitespace", "-F", "-"],
                    Some(message.as_bytes()),
                )?;
            }
            CommitMode::Inherit | CommitMode::Editor => {
                let msg_path = self.git_dir.join("VIBEGIT_MSG");
                std::fs::write(&msg_path, &message)?;
                let path_arg = msg_path.to_string_lossy().into_owned();
                let args: Vec<&str> = if mode == CommitMode::Editor {
                    vec!["commit", "-e", "-F", &path_arg]
                } else {
                    vec!["commit", "-q", "--cleanup=whitespace", "-F", &path_arg]
                };
                let code = self.runner.run_inherit(&args)?;
                let _ = std::fs::remove_file(&msg_path);
                if code != 0 {
                    return Err(GitError::Failed {
                        args: args.join(" "),
                        code,
                        stderr: "git commit did not complete".into(),
                    });
                }
            }
        }
        self.head_info()
    }

    pub fn head_info(&self) -> GitResult<CommitInfo> {
        let out = self
            .runner
            .run(&["log", "-1", "--format=%H%x00%h%x00%s"], None)?;
        let text = out.stdout_str();
        let mut parts = text.trim_end_matches('\n').split('\0');
        Ok(CommitInfo {
            sha: parts.next().unwrap_or("").to_string(),
            short_sha: parts.next().unwrap_or("").to_string(),
            subject: parts.next().unwrap_or("").to_string(),
        })
    }

    pub fn has_upstream(&self) -> bool {
        self.runner
            .run_raw(
                &["rev-parse", "-q", "--verify", "--abbrev-ref", "@{upstream}"],
                None,
            )
            .map(|o| o.success())
            .unwrap_or(false)
    }

    /// Pushes the current branch, setting an upstream on `origin` when there is none.
    pub fn push(&self, inherit: bool) -> GitResult<String> {
        let state = self.state()?;
        let branch = state.branch.ok_or_else(|| GitError::Failed {
            args: "push".into(),
            code: 1,
            stderr: "HEAD is detached; check out a branch before pushing".into(),
        })?;
        let args: Vec<String> = if self.has_upstream() {
            vec!["push".into()]
        } else {
            vec!["push".into(), "-u".into(), "origin".into(), branch]
        };
        if inherit {
            let code = self.runner.run_inherit(&args)?;
            if code == 0 {
                Ok(String::new())
            } else {
                Err(GitError::Failed {
                    args: args.join(" "),
                    code,
                    stderr: "git push failed".into(),
                })
            }
        } else {
            let out = self.runner.run(&args, None)?;
            let mut text = out.stderr_str();
            if text.is_empty() {
                text = out.stdout_str().trim().to_string();
            }
            Ok(text)
        }
    }
}

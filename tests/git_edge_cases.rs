//! End-to-end tests of the git layer against throwaway repositories.
//!
//! Each scenario builds a repository state, runs the real workflow with a fake
//! proposer that puts every change unit into its own commit (forwards and
//! backwards), and asserts that the working tree is clean afterwards, i.e. that
//! every committed byte equals the working tree.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;
use vibegit::config::Config;
use vibegit::git::{CommitMode, GitRepo, GitRunner, InProgress};
use vibegit::llm::{LlmError, ProposeRequest, Proposer, RulesRequest};
use vibegit::proposal::{CommitProposal, ProposalResult};
use vibegit::rules::DerivedRules;
use vibegit::workflow::{Workflow, WorkflowError};

struct EachOwn {
    reverse: bool,
}

impl Proposer for EachOwn {
    fn propose(&self, req: ProposeRequest<'_>) -> Result<ProposalResult, LlmError> {
        let mut ids: Vec<u32> = req
            .user
            .lines()
            .filter_map(|l| l.strip_prefix("# Change ID: "))
            .filter_map(|s| s.trim().parse().ok())
            .collect();
        if self.reverse {
            ids.reverse();
        }
        let result = ProposalResult {
            commit_proposals: ids
                .iter()
                .map(|id| CommitProposal {
                    explanation: "test".into(),
                    commit_message: format!("commit change {id}"),
                    change_ids: vec![*id],
                })
                .collect(),
            excluded_groups: vec![],
        };
        (req.validate)(&result).map_err(|reason| LlmError::Invalid {
            attempts: 1,
            reason,
            raw: String::new(),
        })?;
        Ok(result)
    }

    fn derive_rules(&self, req: RulesRequest<'_>) -> Result<DerivedRules, LlmError> {
        let commits = req
            .user
            .lines()
            .filter(|l| l.starts_with("## Commit "))
            .count();
        Ok(DerivedRules {
            observations: vec![format!("{commits} commits analysed")],
            rules: format!("Test rules from {commits} commits."),
        })
    }
}

struct Sandbox {
    tmp: TempDir,
    home: PathBuf,
    root: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        fs::write(
            home.join(".gitconfig"),
            "[user]\n\tname = Test\n\temail = test@example.com\n[commit]\n\tgpgsign = false\n[protocol \"file\"]\n\tallow = always\n[init]\n\tdefaultBranch = main\n",
        )
        .unwrap();
        let root = tmp.path().join("repo");
        fs::create_dir_all(&root).unwrap();
        let sb = Self { tmp, home, root };
        sb.git(&["init", "-q", "-b", "main"]);
        sb
    }

    fn with_initial_commit() -> Self {
        let sb = Self::new();
        sb.write("base.txt", lines(1..=40).as_bytes());
        let mut bin: Vec<u8> = Vec::new();
        for _ in 0..4 {
            bin.extend(0u8..=255);
        }
        sb.write("bin.dat", &bin);
        sb.git(&["add", "-A"]);
        sb.git(&["commit", "-q", "-m", "initial"]);
        sb
    }

    fn cmd(&self, cwd: &Path) -> Command {
        let mut c = Command::new("git");
        c.current_dir(cwd)
            .env("HOME", &self.home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("GIT_DIR")
            .env_remove("GIT_CONFIG_GLOBAL");
        c
    }

    fn git_in(&self, cwd: &Path, args: &[&str]) -> String {
        let out = self.cmd(cwd).args(args).output().unwrap();
        assert!(
            out.status.success(),
            "git {:?} failed: {}{}",
            args,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn git(&self, args: &[&str]) -> String {
        self.git_in(&self.root, args)
    }

    fn git_may_fail(&self, args: &[&str]) {
        let _ = self.cmd(&self.root).args(args).output().unwrap();
    }

    fn write(&self, rel: &str, content: &[u8]) {
        let p = self.root.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(p, content).unwrap();
    }

    fn edit_line(&self, rel: &str, lineno: usize, text: &str) {
        let p = self.root.join(rel);
        let content = fs::read_to_string(&p).unwrap();
        let mut out: Vec<String> = content.lines().map(|l| l.to_string()).collect();
        out[lineno - 1] = text.to_string();
        fs::write(p, out.join("\n") + "\n").unwrap();
    }

    fn runner_for(&self, cwd: &Path) -> GitRunner {
        GitRunner::new(cwd)
            .with_env("HOME", &self.home)
            .with_env("GIT_CONFIG_NOSYSTEM", "1")
    }

    fn repo_at(&self, cwd: &Path) -> GitRepo {
        GitRepo::discover_with(self.runner_for(cwd)).unwrap()
    }

    fn workflow_at(&self, cwd: &Path, reverse: bool) -> Workflow {
        let config = Config {
            watermark: false,
            ..Config::default()
        };
        Workflow::new(self.repo_at(cwd), config, Box::new(EachOwn { reverse }))
    }

    fn workflow(&self, reverse: bool) -> Workflow {
        self.workflow_at(&self.root, reverse)
    }

    fn status(&self) -> String {
        self.git(&["status", "--porcelain", "--untracked-files=all"])
    }

    fn add_submodule(&self) -> PathBuf {
        let src = self.tmp.path().join("sub-src");
        fs::create_dir_all(&src).unwrap();
        self.git_in(&src, &["init", "-q", "-b", "main"]);
        fs::write(src.join("inner.txt"), "inner\n").unwrap();
        self.git_in(&src, &["add", "inner.txt"]);
        self.git_in(&src, &["commit", "-q", "-m", "inner"]);
        self.git(&["submodule", "add", "-q", src.to_str().unwrap(), "sub"]);
        self.git(&["commit", "-q", "-m", "add submodule"]);
        self.root.join("sub")
    }
}

fn lines(range: std::ops::RangeInclusive<usize>) -> String {
    range.map(|i| format!("line {i}\n")).collect()
}

fn run_pipeline(wf: &Workflow) -> Result<usize, WorkflowError> {
    let pre = wf.preflight()?;
    if !pre.staged.is_empty() {
        wf.unstage_all()?;
    }
    let mut changes = wf.collect()?;
    if changes.is_empty() {
        return Ok(0);
    }
    let result = wf.propose(&changes, None)?;
    let mut count = 0;
    for p in &result.commit_proposals {
        wf.apply(
            &mut changes,
            &p.change_ids,
            &p.commit_message,
            CommitMode::Captured,
        )?;
        count += 1;
    }
    Ok(count)
}

/// Runs the scenario in both orders and asserts a clean tree; returns the commit count.
fn clean_in_both_orders(setup: impl Fn(&Sandbox)) -> usize {
    let mut counts = Vec::new();
    for reverse in [false, true] {
        let sb = Sandbox::with_initial_commit();
        setup(&sb);
        let wf = sb.workflow(reverse);
        let count = run_pipeline(&wf).unwrap_or_else(|e| panic!("reverse={reverse}: {e:#}"));
        assert_eq!(
            sb.status().trim(),
            "",
            "reverse={reverse}: leftover changes"
        );
        // The committed tree must equal the working tree.
        sb.git(&["diff", "HEAD", "--exit-code", "--quiet"]);
        counts.push(count);
    }
    assert_eq!(counts[0], counts[1]);
    counts[0]
}

// --- text content -----------------------------------------------------------

#[test]
fn modify_one_line() {
    assert_eq!(
        clean_in_both_orders(|sb| sb.edit_line("base.txt", 5, "changed 5")),
        1
    );
}

#[test]
fn hunks_split_across_commits_in_any_order() {
    let count = clean_in_both_orders(|sb| {
        sb.edit_line("base.txt", 3, "changed 3");
        let p = sb.root.join("base.txt");
        let mut v: Vec<String> = fs::read_to_string(&p)
            .unwrap()
            .lines()
            .map(String::from)
            .collect();
        v.splice(
            19..19,
            [
                "inserted a".into(),
                "inserted b".into(),
                "inserted c".into(),
            ],
        );
        fs::write(&p, v.join("\n") + "\n").unwrap();
        sb.edit_line("base.txt", 43, "changed near end");
    });
    assert_eq!(count, 3);
}

#[test]
fn repeated_identical_blocks_pick_the_right_one() {
    clean_in_both_orders(|sb| {
        let block = "def f():\n    x = 1\n    y = 2\n    return x + y\n\n\n";
        let mut text = String::new();
        for i in 0..8 {
            text.push_str(&format!("# pad {i}\n"));
        }
        text.push_str(block);
        for i in 0..8 {
            text.push_str(&format!("# mid {i}\n"));
        }
        text.push_str(block);
        sb.write("rep.py", text.as_bytes());
        sb.git(&["add", "rep.py"]);
        sb.git(&["commit", "-q", "-m", "rep"]);
        let mut edited = text.replacen(
            "# pad 7\n",
            "# pad 7\n# n0\n# n1\n# n2\n# n3\n# n4\n# n5\n",
            1,
        );
        let idx = edited.rfind("    y = 2\n").unwrap();
        edited.replace_range(idx..idx + "    y = 2\n".len(), "    y = 3\n");
        sb.write("rep.py", edited.as_bytes());
    });
}

#[test]
fn no_newline_at_eof_variants() {
    clean_in_both_orders(|sb| {
        sb.write("nonl.txt", b"a\nb");
        sb.git(&["add", "nonl.txt"]);
        sb.git(&["commit", "-q", "-m", "nonl"]);
        sb.write("nonl.txt", b"a\nc");
    });
    clean_in_both_orders(|sb| {
        sb.write("nonl.txt", b"a\nb");
        sb.git(&["add", "nonl.txt"]);
        sb.git(&["commit", "-q", "-m", "nonl"]);
        sb.write("nonl.txt", b"a\nb\n");
    });
    let count = clean_in_both_orders(|sb| {
        let body: String = (1..30).map(|i| format!("l{i}\n")).collect();
        sb.write("nonl.txt", format!("{body}end").as_bytes());
        sb.git(&["add", "nonl.txt"]);
        sb.git(&["commit", "-q", "-m", "nonl"]);
        let body2: String = (2..30).map(|i| format!("l{i}\n")).collect();
        sb.write("nonl.txt", format!("L1\n{body2}END").as_bytes());
    });
    assert_eq!(count, 2);
}

#[test]
fn crlf_form_feed_and_u2028_content() {
    clean_in_both_orders(|sb| {
        sb.write("crlf.txt", b"a\r\nb\r\nc\r\n");
        sb.git(&["add", "crlf.txt"]);
        sb.git(&["commit", "-q", "-m", "crlf"]);
        sb.write("crlf.txt", b"a\r\nB\r\nc\r\n");
    });
    clean_in_both_orders(|sb| {
        sb.write(
            "ff.py",
            b"import os\n\x0c\ndef a():\n    pass\n\x0c\ndef b():\n    pass\n",
        );
        sb.git(&["add", "ff.py"]);
        sb.git(&["commit", "-q", "-m", "ff"]);
        sb.write(
            "ff.py",
            b"import os\n\x0c\ndef a():\n    return 1\n\x0c\ndef b():\n    pass\n",
        );
    });
    clean_in_both_orders(|sb| {
        sb.write("ls.js", "var a = 'x\u{2028}y';\nvar b = 1;\n".as_bytes());
        sb.git(&["add", "ls.js"]);
        sb.git(&["commit", "-q", "-m", "ls"]);
        sb.write("ls.js", "var a = 'x\u{2028}y';\nvar b = 2;\n".as_bytes());
    });
}

#[test]
fn trailing_whitespace_is_committed_verbatim_even_with_apply_whitespace_error() {
    clean_in_both_orders(|sb| {
        sb.git(&["config", "apply.whitespace", "error"]);
        sb.edit_line("base.txt", 5, "changed 5   ");
    });
}

#[test]
fn content_that_looks_like_diff_headers() {
    clean_in_both_orders(|sb| {
        let p = sb.root.join("base.txt");
        let mut v: Vec<String> = fs::read_to_string(&p)
            .unwrap()
            .lines()
            .map(String::from)
            .collect();
        v.splice(
            10..10,
            [
                "--- a/fake".into(),
                "+++ b/fake".into(),
                "@@ -1,1 +1,1 @@ fake".into(),
                "diff --git a/x b/x".into(),
            ],
        );
        fs::write(&p, v.join("\n") + "\n").unwrap();
    });
}

// --- files: add, delete, rename, mode, binary, symlink ----------------------

#[test]
fn untracked_files_including_nested_dirs_and_empty_files() {
    let count = clean_in_both_orders(|sb| {
        sb.write("new.txt", b"hello\n");
        sb.write("a/b/deep.txt", b"deep\n");
        sb.write("empty.txt", b"");
        sb.write("new file.txt", b"n\n");
    });
    assert_eq!(count, 4);
}

#[test]
fn delete_and_unstaged_rename() {
    clean_in_both_orders(|sb| fs::remove_file(sb.root.join("base.txt")).unwrap());
    let count = clean_in_both_orders(|sb| {
        sb.git(&["mv", "base.txt", "renamed.txt"]);
        sb.git(&["reset", "-q"]);
    });
    assert_eq!(count, 2);
}

#[test]
fn staged_changes_are_unstaged_first() {
    clean_in_both_orders(|sb| {
        sb.git(&["mv", "base.txt", "renamed.txt"]);
    });
    clean_in_both_orders(|sb| {
        sb.edit_line("base.txt", 5, "staged 5");
        sb.git(&["add", "base.txt"]);
        sb.edit_line("base.txt", 30, "unstaged 30");
    });
    clean_in_both_orders(|sb| {
        sb.write("staged_new.txt", b"x\n");
        sb.git(&["add", "staged_new.txt"]);
    });
}

#[test]
fn mode_changes() {
    clean_in_both_orders(|sb| {
        fs::set_permissions(sb.root.join("base.txt"), fs::Permissions::from_mode(0o755)).unwrap();
    });
    let count = clean_in_both_orders(|sb| {
        fs::set_permissions(sb.root.join("base.txt"), fs::Permissions::from_mode(0o755)).unwrap();
        sb.edit_line("base.txt", 3, "changed 3");
        sb.edit_line("base.txt", 35, "changed 35");
    });
    assert_eq!(count, 2);
    // Both the mode and the content must be in the final tree.
    let sb = Sandbox::with_initial_commit();
    fs::set_permissions(sb.root.join("base.txt"), fs::Permissions::from_mode(0o755)).unwrap();
    sb.edit_line("base.txt", 3, "changed 3");
    sb.edit_line("base.txt", 35, "changed 35");
    run_pipeline(&sb.workflow(true)).unwrap();
    assert!(sb
        .git(&["ls-tree", "HEAD", "base.txt"])
        .starts_with("100755"));
}

#[test]
fn binary_files() {
    clean_in_both_orders(|sb| {
        let mut v: Vec<u8> = Vec::new();
        for _ in 0..4 {
            v.extend((0u8..=255).rev());
        }
        sb.write("bin.dat", &v);
    });
    clean_in_both_orders(|sb| sb.write("img.png", b"\x89PNG\r\n\x1a\n\x00\x01\x02\x03\xff"));
    clean_in_both_orders(|sb| fs::remove_file(sb.root.join("bin.dat")).unwrap());
    clean_in_both_orders(|sb| {
        sb.write(".gitattributes", b"*.dat -diff\n");
        sb.git(&["add", ".gitattributes"]);
        sb.git(&["commit", "-q", "-m", "attrs"]);
        sb.write("bin.dat", b"different");
    });
    clean_in_both_orders(|sb| {
        sb.write(".gitattributes", b"*.txt -diff\n");
        sb.git(&["add", ".gitattributes"]);
        sb.git(&["commit", "-q", "-m", "attrs"]);
        sb.edit_line("base.txt", 5, "changed 5");
    });
}

#[test]
fn symlinks_and_type_change() {
    clean_in_both_orders(|sb| {
        std::os::unix::fs::symlink("base.txt", sb.root.join("link")).unwrap()
    });
    clean_in_both_orders(|sb| {
        std::os::unix::fs::symlink("base.txt", sb.root.join("link")).unwrap();
        sb.git(&["add", "link"]);
        sb.git(&["commit", "-q", "-m", "link"]);
        fs::remove_file(sb.root.join("link")).unwrap();
        std::os::unix::fs::symlink("bin.dat", sb.root.join("link")).unwrap();
    });
    clean_in_both_orders(|sb| {
        fs::remove_file(sb.root.join("base.txt")).unwrap();
        std::os::unix::fs::symlink("bin.dat", sb.root.join("base.txt")).unwrap();
    });
}

#[test]
fn special_filenames() {
    clean_in_both_orders(|sb| {
        sb.write("my file.txt", b"a\n");
        sb.write("ä-ü.txt", b"a\n");
        sb.write("a[1].txt", b"a\n");
        sb.git(&["add", "--", "my file.txt", "ä-ü.txt", "a[1].txt"]);
        sb.git(&["commit", "-q", "-m", "names"]);
        sb.write("my file.txt", b"b\n");
        sb.write("ä-ü.txt", b"b\n");
        sb.write("a[1].txt", b"b\n");
        sb.write("📝notes.txt", b"n\n");
        sb.write("star*.txt", b"n\n");
    });
}

#[test]
fn intent_to_add_and_ignored_files() {
    clean_in_both_orders(|sb| {
        sb.write("ita.txt", b"x\n");
        sb.git(&["add", "-N", "ita.txt"]);
    });
    let sb = Sandbox::with_initial_commit();
    sb.write(".gitignore", b"*.log\n");
    sb.git(&["add", ".gitignore"]);
    sb.git(&["commit", "-q", "-m", "ignore"]);
    sb.write("debug.log", b"x\n");
    assert_eq!(run_pipeline(&sb.workflow(false)).unwrap(), 0);
}

// --- repository states --------------------------------------------------------

#[test]
fn detached_head_and_unborn_head() {
    clean_in_both_orders(|sb| {
        sb.git(&["checkout", "-q", "--detach"]);
        sb.edit_line("base.txt", 5, "changed 5");
    });
    for reverse in [false, true] {
        let sb = Sandbox::new();
        sb.write("first.txt", b"x\n");
        sb.write("second.txt", b"y\n");
        let wf = sb.workflow(reverse);
        assert_eq!(run_pipeline(&wf).unwrap(), 2);
        assert_eq!(sb.status().trim(), "");
        assert_eq!(sb.git(&["rev-list", "--count", "HEAD"]).trim(), "2");
    }
}

#[test]
fn refuses_conflicts_and_in_progress_merges() {
    let sb = Sandbox::with_initial_commit();
    sb.git(&["checkout", "-q", "-b", "other"]);
    sb.edit_line("base.txt", 1, "theirs");
    sb.git(&["commit", "-q", "-am", "theirs"]);
    sb.git(&["checkout", "-q", "main"]);
    sb.edit_line("base.txt", 1, "ours");
    sb.git(&["commit", "-q", "-am", "ours"]);
    sb.git_may_fail(&["merge", "other"]);
    match sb.workflow(false).preflight() {
        Err(WorkflowError::InProgress(InProgress::Merge)) => {}
        Err(WorkflowError::Conflicts(paths)) => assert_eq!(paths, vec!["base.txt"]),
        other => panic!("unexpected: {other:?}"),
    }

    let sb = Sandbox::with_initial_commit();
    sb.git(&["checkout", "-q", "-b", "other"]);
    sb.write("feature.txt", b"f\n");
    sb.git(&["add", "feature.txt"]);
    sb.git(&["commit", "-q", "-m", "feature"]);
    sb.git(&["checkout", "-q", "main"]);
    sb.git(&["merge", "--no-commit", "--no-ff", "other"]);
    let err = sb.workflow(false).preflight().unwrap_err();
    assert!(
        matches!(err, WorkflowError::InProgress(InProgress::Merge)),
        "{err}"
    );
    // Nothing was touched: MERGE_HEAD is still there.
    assert!(sb.root.join(".git/MERGE_HEAD").exists());
}

#[test]
fn unstage_all_keeps_merge_state_but_workflow_never_runs_it_in_a_merge() {
    let sb = Sandbox::with_initial_commit();
    sb.edit_line("base.txt", 5, "staged");
    sb.git(&["add", "base.txt"]);
    let wf = sb.workflow(false);
    assert_eq!(wf.preflight().unwrap().staged, vec!["base.txt"]);
    wf.unstage_all().unwrap();
    assert!(wf.preflight().unwrap().staged.is_empty());
}

#[test]
fn failing_pre_commit_hook_leaves_index_clean() {
    let sb = Sandbox::with_initial_commit();
    let hook = sb.root.join(".git/hooks/pre-commit");
    fs::write(&hook, "#!/bin/sh\necho 'hook says no' >&2\nexit 1\n").unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    sb.edit_line("base.txt", 5, "changed 5");
    let wf = sb.workflow(false);
    let err = run_pipeline(&wf).unwrap_err();
    assert!(matches!(err, WorkflowError::Commit { .. }), "{err}");
    assert!(err.to_string().contains("hook says no"), "{err}");
    sb.git(&["diff", "--cached", "--quiet", "--exit-code"]);
    assert_eq!(sb.status().trim_end(), " M base.txt");
}

#[test]
fn index_lock_held_briefly_by_another_process() {
    // Editors and git clients re-scan the repository after each commit; their
    // `git status` holds `index.lock` for a moment. The commit must wait, not fail.
    let sb = Sandbox::with_initial_commit();
    sb.edit_line("base.txt", 5, "changed 5");
    sb.write("new.txt", b"new\n");
    let lock = sb.root.join(".git/index.lock");
    fs::write(&lock, "").unwrap();
    let holder = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(400));
        fs::remove_file(lock).unwrap();
    });
    let wf = sb.workflow(false);
    let count = run_pipeline(&wf).unwrap();
    holder.join().unwrap();
    assert_eq!(count, 2);
    assert_eq!(sb.status().trim(), "");
    sb.git(&["diff", "HEAD", "--exit-code", "--quiet"]);
    assert!(!sb.root.join(".git/index.lock").exists());
}

#[test]
fn works_from_subdirectory_and_linked_worktree() {
    let sb = Sandbox::with_initial_commit();
    sb.write("subdir/new.txt", b"x\n");
    sb.edit_line("base.txt", 5, "changed 5");
    let wf = sb.workflow_at(&sb.root.join("subdir"), false);
    assert_eq!(wf.repo.root(), sb.root.canonicalize().unwrap());
    assert_eq!(run_pipeline(&wf).unwrap(), 2);
    assert_eq!(sb.status().trim(), "");

    let sb = Sandbox::with_initial_commit();
    let wt = sb.tmp.path().join("wt");
    sb.git(&["worktree", "add", "-q", wt.to_str().unwrap(), "-b", "wt"]);
    fs::write(wt.join("base.txt"), "changed\n").unwrap();
    let wf = sb.workflow_at(&wt, false);
    assert_eq!(run_pipeline(&wf).unwrap(), 1);
    assert_eq!(sb.git_in(&wt, &["status", "--porcelain"]).trim(), "");
}

// --- submodules and nested repositories -------------------------------------

#[test]
fn submodule_pointer_moves_are_committed() {
    let count = clean_in_both_orders(|sb| {
        let sub = sb.add_submodule();
        fs::write(sub.join("more.txt"), "x\n").unwrap();
        sb.git_in(&sub, &["add", "more.txt"]);
        sb.git_in(&sub, &["commit", "-q", "-m", "more"]);
        sb.edit_line("base.txt", 5, "changed 5");
    });
    assert_eq!(count, 2);
}

#[test]
fn dirty_submodules_are_reported_not_committed() {
    for setup in [
        (|_sb: &Sandbox, sub: &Path| fs::write(sub.join("junk.txt"), "x\n").unwrap())
            as fn(&Sandbox, &Path),
        (|sb: &Sandbox, sub: &Path| {
            sb.git_in(sub, &["commit", "-q", "--allow-empty", "-m", "noop"]);
        }) as fn(&Sandbox, &Path),
        (|_sb: &Sandbox, sub: &Path| fs::write(sub.join("inner.txt"), "edited\n").unwrap())
            as fn(&Sandbox, &Path),
    ] {
        let sb = Sandbox::with_initial_commit();
        let sub = sb.add_submodule();
        setup(&sb, &sub);
        let wf = sb.workflow(false);
        let changes = wf.collect().unwrap();
        let inner_dirty = sub.join("junk.txt").exists()
            || fs::read_to_string(sub.join("inner.txt")).unwrap() == "edited\n";
        if inner_dirty {
            assert!(
                changes.warnings.iter().any(|w| w.contains("submodule sub")),
                "{:?}",
                changes.warnings
            );
        }
        let pointer_moved = sb.status().contains(" M sub") && !inner_dirty;
        if pointer_moved {
            assert_eq!(changes.len(), 1);
            run_pipeline(&wf).unwrap();
            assert_eq!(sb.status().trim(), "");
        } else {
            assert!(
                changes.is_empty(),
                "dirty-only submodule must not produce change units"
            );
        }
    }
}

#[test]
fn nested_repository_is_skipped_with_warning() {
    let sb = Sandbox::with_initial_commit();
    let nested = sb.root.join("nested");
    fs::create_dir_all(&nested).unwrap();
    sb.git_in(&nested, &["init", "-q"]);
    fs::write(nested.join("f.txt"), "x\n").unwrap();
    sb.edit_line("base.txt", 5, "changed 5");
    let wf = sb.workflow(false);
    let changes = wf.collect().unwrap();
    assert_eq!(changes.len(), 1);
    assert!(changes.warnings.iter().any(|w| w.contains("nested/")));
    assert_eq!(run_pipeline(&wf).unwrap(), 1);
    assert_eq!(sb.status().trim(), "?? nested/");
}

// --- user configuration must not matter ---------------------------------------

#[test]
fn immune_to_diff_configuration() {
    for (key, value) in [
        ("diff.noprefix", "true"),
        ("diff.mnemonicPrefix", "true"),
        ("diff.external", "/bin/echo"),
        ("diff.submodule", "log"),
        ("diff.submodule", "diff"),
        ("color.ui", "always"),
        ("core.quotePath", "false"),
        ("diff.renames", "true"),
    ] {
        clean_in_both_orders(|sb| {
            sb.git(&["config", key, value]);
            sb.edit_line("base.txt", 5, "changed 5");
            sb.edit_line("base.txt", 30, "changed 30");
            sb.write("ä new.txt", b"n\n");
        });
    }
    clean_in_both_orders(|sb| {
        sb.git(&["config", "diff.submodule", "diff"]);
        let sub = sb.add_submodule();
        fs::write(sub.join("more.txt"), "x\n").unwrap();
        sb.git_in(&sub, &["add", "more.txt"]);
        sb.git_in(&sub, &["commit", "-q", "-m", "more"]);
    });
}

#[test]
fn oversize_files_get_placeholders_but_still_commit() {
    clean_in_both_orders(|sb| {
        let big: String = (0..40_000)
            .map(|i| format!("line {i} with some padding text\n"))
            .collect();
        sb.write("big.txt", big.as_bytes());
    });
    let sb = Sandbox::with_initial_commit();
    let big: String = (0..40_000)
        .map(|i| format!("line {i} with some padding text\n"))
        .collect();
    sb.write("big.txt", big.as_bytes());
    let changes = sb.workflow(false).collect().unwrap();
    let rendered = changes.render_for_model(None);
    assert!(
        rendered.contains("content omitted"),
        "{}",
        &rendered[..200.min(rendered.len())]
    );
    assert!(!rendered.contains("line 39999"));
}

// --- rule derivation ----------------------------------------------------------

#[test]
fn rules_are_derived_from_history_and_written() {
    let sb = Sandbox::with_initial_commit();
    sb.edit_line("base.txt", 5, "changed 5");
    sb.git(&["commit", "-q", "-am", "second: change a line"]);
    sb.write("docs/new.md", b"# doc\n");
    sb.git(&["add", "docs/new.md"]);
    sb.git(&["commit", "-q", "-m", "third: add a doc"]);
    let mut wf = sb.workflow(false);
    wf.config.init.max_lines_per_file = 2;

    let ctx = wf.history_context().unwrap();
    assert_eq!(ctx.commits_available, 3);
    assert_eq!(ctx.commits_included, 3);
    assert!(
        ctx.text.starts_with("Commits in the repository: 3."),
        "{}",
        ctx.text
    );
    assert!(
        ctx.text.contains("## Commit 1:") && ctx.text.contains("third: add a doc"),
        "{}",
        ctx.text
    );
    assert!(
        ctx.text.contains("### docs/new.md (new file, +1 -0)"),
        "{}",
        ctx.text
    );
    assert!(
        ctx.text.contains("### base.txt (modified, +1 -1)"),
        "{}",
        ctx.text
    );
    assert!(ctx.text.contains("more diff lines"), "{}", ctx.text);
    assert!(
        ctx.text
            .contains("File types touched: txt (2), dat (1), md (1)"),
        "{}",
        ctx.text
    );

    assert!(!wf.has_rules());
    let derived = wf.derive_rules().unwrap();
    assert_eq!(derived.rules, "Test rules from 3 commits.");
    let path = wf.write_rules(&derived.rules).unwrap();
    assert_eq!(path, sb.root.canonicalize().unwrap().join(".vibegitrules"));
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "Test rules from 3 commits.\n"
    );
    assert!(wf.has_rules());
    assert!(wf
        .project_instructions
        .as_deref()
        .unwrap()
        .contains("Test rules"));
    // The new rules reach the model on the next proposal.
    sb.edit_line("base.txt", 6, "changed 6");
    let changes = wf.collect().unwrap();
    let context = wf.build_context(&changes, None).unwrap();
    assert!(context.contains("Project Instructions: \"Test rules from 3 commits.\""));
}

#[test]
fn rules_derivation_respects_commit_limit_and_budget() {
    let sb = Sandbox::with_initial_commit();
    for i in 0..5 {
        sb.edit_line("base.txt", 1 + 8 * i, &format!("edit {i}"));
        sb.git(&["commit", "-q", "-am", &format!("commit edit {i}")]);
    }
    let mut wf = sb.workflow(false);
    wf.config.init.commits = 3;
    let ctx = wf.history_context().unwrap();
    assert_eq!(ctx.commits_available, 6);
    assert_eq!(ctx.commits_included, 3);
    assert!(
        ctx.text.contains("Message:\n  commit edit 4") && !ctx.text.contains("commit edit 1"),
        "{}",
        ctx.text
    );
    wf.config.init.commits = 6;
    wf.config.init.max_total_kb = 0; // only the newest commit fits
    let ctx = wf.history_context().unwrap();
    assert_eq!(ctx.commits_included, 1);
}

#[test]
fn rules_derivation_without_commits_reports_no_history() {
    let sb = Sandbox::new();
    let wf = sb.workflow(false);
    assert!(matches!(wf.derive_rules(), Err(WorkflowError::NoHistory)));
}

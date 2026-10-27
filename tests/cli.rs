//! End-to-end tests of the command-line interface through the built binary.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::TempDir;

struct Env {
    _tmp: TempDir,
    home: PathBuf,
    repo: PathBuf,
    config: PathBuf,
}

impl Env {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        fs::write(
            home.join(".gitconfig"),
            "[user]\n\tname = Test\n\temail = test@example.com\n[commit]\n\tgpgsign = false\n[init]\n\tdefaultBranch = main\n",
        )
        .unwrap();
        let config = tmp.path().join("config.toml");
        fs::write(
            &config,
            "watermark = false\n[model]\nname = \"google:gemini-3.8-flash\"\n",
        )
        .unwrap();
        let repo = tmp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        let env = Self {
            _tmp: tmp,
            home,
            repo,
            config,
        };
        env.git(&["init", "-q", "-b", "main"]);
        env
    }

    fn git(&self, args: &[&str]) {
        let out = Command::new("git")
            .args(args)
            .current_dir(&self.repo)
            .env("HOME", &self.home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn commit_file(&self, name: &str, content: &str, message: &str) {
        fs::write(self.repo.join(name), content).unwrap();
        self.git(&["add", name]);
        self.git(&["commit", "-q", "-m", message]);
    }

    fn vibegit(&self, cwd: &Path, args: &[&str]) -> Output {
        self.vibegit_with(cwd, args, true)
    }

    /// Runs the binary; `fake` selects the offline stand-in for the model.
    fn vibegit_with(&self, cwd: &Path, args: &[&str], fake: bool) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_vibegit"));
        cmd.args(args)
            .current_dir(cwd)
            .env("HOME", &self.home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("VIBEGIT_CONFIG_PATH", &self.config)
            .env_remove("GOOGLE_API_KEY")
            .env_remove("GEMINI_API_KEY");
        if fake {
            cmd.env("VIBEGIT_FAKE_PROPOSER", "1");
        } else {
            cmd.env_remove("VIBEGIT_FAKE_PROPOSER");
        }
        cmd.output().unwrap()
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn init_template_writes_the_bundled_rules() {
    let env = Env::new();
    let out = env.vibegit(&env.repo, &["init", "--template"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(
        fs::read_to_string(env.repo.join(".vibegitrules")).unwrap(),
        vibegit::DEFAULT_RULES
    );
    let again = env.vibegit(&env.repo, &["init", "--template"]);
    assert_eq!(again.status.code(), Some(1));
    assert!(text(&again.stderr).contains("already exists"));
    assert!(env
        .vibegit(&env.repo, &["init", "--template", "--force"])
        .status
        .success());
}

#[test]
fn init_derives_rules_from_history_non_interactively() {
    let env = Env::new();
    env.commit_file("a.txt", "a\n", "feat: add a");
    env.commit_file("b.txt", "b\n", "feat: add b");
    env.commit_file("c.txt", "c\n", "fix: add c");

    let printed = env.vibegit(&env.repo, &["init", "--print", "-n", "2"]);
    assert!(printed.status.success(), "{}", text(&printed.stderr));
    assert!(text(&printed.stdout).contains("Fake rules derived from 2 commit(s)."));
    assert!(!env.repo.join(".vibegitrules").exists());

    let subdir = env.repo.join("sub");
    fs::create_dir_all(&subdir).unwrap();
    let written = env.vibegit(&subdir, &["init", "--yes"]);
    assert!(written.status.success(), "{}", text(&written.stderr));
    assert!(text(&written.stdout).contains("Wrote"));
    let rules = fs::read_to_string(env.repo.join(".vibegitrules")).unwrap();
    assert!(
        rules.starts_with("Fake rules derived from 3 commit(s)."),
        "{rules}"
    );

    let refused = env.vibegit(&env.repo, &["init", "--yes"]);
    assert_eq!(refused.status.code(), Some(1));
    assert!(text(&refused.stderr).contains("already exists"));
}

#[test]
fn init_without_commits_falls_back_to_the_template() {
    let env = Env::new();
    let out = env.vibegit(&env.repo, &["init", "--yes"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(text(&out.stderr).contains("no commits yet"));
    assert_eq!(
        fs::read_to_string(env.repo.join(".vibegitrules")).unwrap(),
        vibegit::DEFAULT_RULES
    );
}

#[test]
fn yolo_hints_at_missing_rules_and_commits_everything() {
    let env = Env::new();
    env.commit_file("a.txt", "a\n", "initial");
    fs::write(env.repo.join("a.txt"), "changed\n").unwrap();
    fs::write(env.repo.join("new.txt"), "n\n").unwrap();
    let out = env.vibegit(&env.repo, &["commit", "--yolo"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(text(&out.stderr).contains("vibegit init"));
    assert!(text(&out.stdout).contains("All proposals applied"));
}

#[test]
fn config_set_tests_the_model_and_config_test_repeats_it() {
    let env = Env::new();
    let out = env.vibegit(
        &env.repo,
        &["config", "set", "model.name", "openai:gpt-6.1-terra"],
    );
    assert!(out.status.success(), "{}", text(&out.stderr));
    let stdout = text(&out.stdout);
    assert!(stdout.contains("Set model.name"), "{stdout}");
    assert!(stdout.contains("answered in"), "{stdout}");
    assert!(fs::read_to_string(&env.config)
        .unwrap()
        .contains("openai:gpt-6.1-terra"));

    let quiet = env.vibegit(&env.repo, &["config", "set", "watermark", "true"]);
    assert!(quiet.status.success(), "{}", text(&quiet.stderr));
    assert!(
        !text(&quiet.stdout).contains("Testing"),
        "{}",
        text(&quiet.stdout)
    );

    let again = env.vibegit(&env.repo, &["config", "test"]);
    assert!(again.status.success(), "{}", text(&again.stderr));
    assert!(text(&again.stdout).contains("answered in"));
}

#[test]
fn config_test_reports_a_missing_api_key() {
    let env = Env::new();
    let out = env.vibegit_with(&env.repo, &["config", "test"], false);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        text(&out.stdout).contains("failed"),
        "{}",
        text(&out.stdout)
    );
    assert!(
        text(&out.stderr).contains("GOOGLE_API_KEY"),
        "{}",
        text(&out.stderr)
    );
}

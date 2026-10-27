//! Command-line entry points. Terminal output for the non-interactive paths lives here;
//! the interactive commit flow and the wizard live in `tui`.

use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::Command as Process;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Parser, Subcommand};

use crate::config::{config_path, Config};
use crate::git::{CommitMode, GitRepo};
use crate::llm::{check_model, resolve_model, FakeProposer, LlmProposer, Proposer};
use crate::proposal::ProposalResult;
use crate::tui;
use crate::update::check_for_update;
use crate::workflow::{Workflow, WorkflowError, RULES_FILENAME};
use crate::{DEFAULT_RULES, VERSION};

#[derive(Parser)]
#[command(
    name = "vibegit",
    version,
    about = "Turn a busy Git working tree into focused, reviewable commits.",
    long_about = "VibeGit analyzes your uncommitted changes, groups related hunks into semantic commits with \
generated messages, and lets you review, rename, reorder and apply them.\n\n\
Running `vibegit` without a command starts the commit workflow."
)]
pub struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    #[command(flatten)]
    commit: CommitArgs,
}

#[derive(Subcommand)]
enum Command {
    /// Analyze uncommitted changes and create semantic commits (the default command)
    Commit(CommitArgs),
    /// Create a starter .vibegitrules file in the current directory
    Init(InitArgs),
    /// Show or change the configuration; without a subcommand this runs the setup wizard
    Config {
        #[command(subcommand)]
        action: Option<ConfigAction>,
    },
}

#[derive(Args, Clone, Default, Debug)]
pub struct CommitArgs {
    /// One-off instruction for this run (takes priority over .vibegitrules)
    #[arg(short, long, value_name = "TEXT")]
    pub instruction: Option<String>,
    /// Apply every proposal without the interactive interface
    #[arg(short = 'y', long)]
    pub yolo: bool,
    /// With --yolo: push after committing
    #[arg(long)]
    pub push: bool,
    /// With --yolo: unstage already staged changes without asking
    #[arg(long)]
    pub unstage: bool,
    /// Write prompts and responses to a debug directory and print full error chains
    #[arg(short, long)]
    pub debug: bool,
}

#[derive(Args, Clone, Default, Debug)]
pub struct InitArgs {
    /// Replace an existing .vibegitrules file
    #[arg(short, long)]
    pub force: bool,
    /// Write the derived rules without the interactive review
    #[arg(short = 'y', long)]
    pub yes: bool,
    /// Print the derived rules instead of writing them
    #[arg(long)]
    pub print: bool,
    /// Write VibeGit's static template instead of analyzing the history
    #[arg(long)]
    pub template: bool,
    /// Number of recent commits to analyze (default: init.commits from the config)
    #[arg(short = 'n', long, value_name = "N")]
    pub commits: Option<usize>,
    /// Write prompts and responses to a debug directory
    #[arg(short, long)]
    pub debug: bool,
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Print the configuration (API keys are masked)
    Show,
    /// Print one value, e.g. `model.name`
    Get { path: String },
    /// Set one value, e.g. `model.name openai:gpt-6.1-terra`; use `none` to clear optional values
    Set { path: String, value: String },
    /// Send a short test request to the configured model
    Test,
    /// Print the configuration file path
    Path,
    /// Open the configuration file in your editor
    Open,
    /// Run the interactive setup wizard
    Wizard,
}

/// Runs the CLI and returns the process exit code.
pub fn run() -> i32 {
    let cli = Cli::parse();
    let debug = matches!(&cli.command, Some(Command::Commit(a)) if a.debug) || cli.commit.debug;
    let result = match cli.command {
        None => run_commit(cli.commit),
        Some(Command::Commit(args)) => run_commit(args),
        Some(Command::Init(args)) => run_init(args),
        Some(Command::Config { action }) => run_config(action),
    };
    match result {
        Ok(code) => code,
        Err(err) => {
            if debug {
                eprintln!("error: {err:?}");
            } else {
                eprintln!("error: {err:#}");
            }
            1
        }
    }
}

fn load_or_setup_config(interactive: bool) -> Result<Config> {
    let (config, existed) = Config::load()?;
    if existed {
        return Ok(config);
    }
    if !interactive {
        bail!(
            "no configuration found at {}; run `vibegit config` first",
            config_path().display()
        );
    }
    match tui::run_wizard(config)? {
        Some(config) => Ok(config),
        None => bail!("configuration cancelled"),
    }
}

fn run_commit(args: CommitArgs) -> Result<i32> {
    let config = load_or_setup_config(!args.yolo)?;
    let cwd = std::env::current_dir().context("could not determine the current directory")?;
    let repo =
        GitRepo::discover(&cwd).map_err(|e| anyhow!("not inside a git working tree: {e}"))?;
    let (proposer, model_label) = build_proposer(&config)?;
    let confirmations = config.confirmations;
    let mut workflow = Workflow::new(repo, config, proposer);
    if args.debug {
        let dir = std::env::temp_dir().join("vibegit-debug");
        eprintln!(
            "debug: prompts and responses are written to {}",
            dir.display()
        );
        workflow.debug_dir = Some(dir);
    }
    if args.yolo {
        run_yolo(workflow, &args, &model_label)
    } else {
        tui::run_commit(
            workflow,
            tui::Options {
                instruction: args.instruction.clone(),
                model_label,
                debug: args.debug,
                confirmations,
            },
        )
    }
}

/// The configured model, or the deterministic stand-in when `VIBEGIT_FAKE_PROPOSER` is set.
fn build_proposer(config: &Config) -> Result<(Box<dyn Proposer>, String)> {
    if std::env::var_os("VIBEGIT_FAKE_PROPOSER").is_some() {
        return Ok((Box::new(FakeProposer), "fake proposer".to_string()));
    }
    let model = resolve_model(config)?;
    let label = model.label();
    Ok((Box::new(LlmProposer::new(model)), label))
}

fn confirm(question: &str) -> Result<bool> {
    print!("{question} [y/N] ");
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    Ok(matches!(
        line.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn run_yolo(workflow: Workflow, args: &CommitArgs, model_label: &str) -> Result<i32> {
    let update = std::thread::spawn(|| check_for_update(VERSION));
    let pre = workflow.preflight()?;
    println!(
        "Repository: {} ({})",
        workflow.repo.root().display(),
        pre.state.describe_head()
    );
    if !pre.staged.is_empty() {
        println!("Staged changes found in: {}", pre.staged.join(", "));
        if args.unstage || confirm("VibeGit needs a clean index. Unstage them now?")? {
            workflow.unstage_all()?;
        } else {
            bail!("staged changes present; unstage them or pass --unstage");
        }
    }
    if !workflow.has_rules() {
        eprintln!("hint: no .vibegitrules in this repository; `vibegit init` derives one from your history");
    }
    let mut changes = workflow.collect()?;
    for w in &changes.warnings {
        eprintln!("warning: {w}");
    }
    if changes.is_empty() {
        println!("No uncommitted changes found. Nothing to do.");
        return Ok(0);
    }
    println!(
        "Analyzing {} change unit(s) in {} file(s) with {}...",
        changes.len(),
        changes.files.len(),
        model_label
    );
    let result = workflow.propose(&changes, args.instruction.as_deref())?;
    print_proposals(&result, &changes);

    let total = result.commit_proposals.len();
    let mut created = Vec::new();
    for (i, proposal) in result.commit_proposals.iter().enumerate() {
        println!(
            "\n[{}/{}] {}",
            i + 1,
            total,
            proposal.commit_message.lines().next().unwrap_or("")
        );
        match workflow.apply(
            &mut changes,
            &proposal.change_ids,
            &proposal.commit_message,
            CommitMode::Inherit,
        ) {
            Ok(info) => {
                println!("  committed {} {}", info.short_sha, info.subject);
                created.push(info);
            }
            Err(err) => {
                eprintln!("error: {err:#}");
                eprintln!(
                    "stopping: {} proposal(s) were not applied",
                    total - created.len()
                );
                return Ok(1);
            }
        }
    }

    let leftover = workflow.leftover()?;
    if leftover.is_clean() {
        println!("\nAll proposals applied. The working tree is clean.");
    } else {
        println!(
            "\nAll proposals applied. Remaining: {} changed, {} untracked, {} staged.",
            leftover.changed, leftover.untracked, leftover.staged
        );
    }
    if args.push {
        println!("Pushing...");
        workflow.push(true)?;
    }
    if let Ok(Some(latest)) = update.join() {
        println!("\nA newer VibeGit ({latest}) is available: uv tool upgrade vibegit");
    }
    Ok(0)
}

fn print_proposals(result: &ProposalResult, changes: &crate::changes::ChangeSet) {
    println!("\nProposed commits:");
    for (i, p) in result.commit_proposals.iter().enumerate() {
        let files: Vec<String> = changes
            .files_for(&p.change_ids)
            .iter()
            .map(|f| format!("{} {}", f.letter(), f.path))
            .collect();
        println!(
            "  {}. {}",
            i + 1,
            p.commit_message.lines().next().unwrap_or("")
        );
        println!("     {}", files.join(", "));
    }
    if !result.excluded_groups.is_empty() {
        println!("\nExcluded (not ready to commit):");
        for g in &result.excluded_groups {
            let files: Vec<String> = changes.paths_for(&g.change_ids);
            println!("  - {}: {}", files.join(", "), g.explanation);
        }
    }
}

fn run_init(args: InitArgs) -> Result<i32> {
    if args.template {
        let target = Path::new(RULES_FILENAME);
        if target.exists() && !args.force {
            bail!("{RULES_FILENAME} already exists; use --force to replace it");
        }
        std::fs::write(target, DEFAULT_RULES)
            .with_context(|| format!("could not write {RULES_FILENAME}"))?;
        println!(
            "Created {}",
            std::env::current_dir()?.join(RULES_FILENAME).display()
        );
        return Ok(0);
    }
    let interactive = !(args.yes || args.print);
    let config = load_or_setup_config(interactive)?;
    let cwd = std::env::current_dir().context("could not determine the current directory")?;
    let repo =
        GitRepo::discover(&cwd).map_err(|e| anyhow!("not inside a git working tree: {e}"))?;
    let rules_path = repo.root().join(RULES_FILENAME);
    if rules_path.exists() && !args.force && !args.print {
        bail!(
            "{} already exists; use --force to replace it",
            rules_path.display()
        );
    }
    let (proposer, model_label) = build_proposer(&config)?;
    let confirmations = config.confirmations;
    let mut workflow = Workflow::new(repo, config, proposer);
    if let Some(n) = args.commits {
        workflow.config.init.commits = n.max(1);
    }
    if args.debug {
        workflow.debug_dir = Some(debug_dir());
    }
    if interactive {
        return tui::run_init(
            workflow,
            tui::Options {
                instruction: None,
                model_label,
                debug: args.debug,
                confirmations,
            },
        );
    }
    eprintln!(
        "Analyzing the last {} commits with {}...",
        workflow.config.init.commits, model_label
    );
    let derived = match workflow.derive_rules() {
        Ok(d) => d,
        Err(WorkflowError::NoHistory) => {
            eprintln!("The repository has no commits yet; using the default template.");
            crate::rules::DerivedRules {
                observations: Vec::new(),
                rules: DEFAULT_RULES.trim_end().to_string(),
            }
        }
        Err(e) => return Err(e.into()),
    };
    for o in &derived.observations {
        eprintln!("- {o}");
    }
    if args.print {
        println!("{}", derived.rules);
        return Ok(0);
    }
    let path = workflow.write_rules(&derived.rules)?;
    println!("Wrote {}", path.display());
    Ok(0)
}

fn run_config(action: Option<ConfigAction>) -> Result<i32> {
    let path = config_path();
    match action {
        None | Some(ConfigAction::Wizard) => {
            let (config, _) = Config::load()?;
            match tui::run_wizard(config)? {
                Some(_) => println!("Configuration saved to {}", path.display()),
                None => println!("Configuration cancelled."),
            }
        }
        Some(ConfigAction::Show) => {
            let (config, existed) = Config::load()?;
            if !existed {
                println!(
                    "# no configuration file yet at {} (showing defaults)",
                    path.display()
                );
            }
            print!("{}", config.to_masked_toml()?);
        }
        Some(ConfigAction::Get { path: key }) => {
            let (config, _) = Config::load()?;
            match config.get(&key)? {
                Some(toml::Value::String(s)) => println!("{s}"),
                Some(value) => println!("{value}"),
                None => bail!("{key} is not set"),
            }
        }
        Some(ConfigAction::Set { path: key, value }) => {
            let (mut config, _) = Config::load()?;
            config.set(&key, &value)?;
            config.save()?;
            println!("Set {key}");
            if key.starts_with("model.") || key.starts_with("api_keys.") {
                return run_model_check(&config);
            }
        }
        Some(ConfigAction::Test) => {
            let (config, _) = Config::load()?;
            return run_model_check(&config);
        }
        Some(ConfigAction::Path) => println!("{}", path.display()),
        Some(ConfigAction::Open) => {
            if !path.exists() {
                let (config, _) = Config::load()?;
                config.save()?;
            }
            open_in_editor(&path)?;
        }
    }
    Ok(0)
}

/// Sends the test request to the configured model and reports the outcome.
fn run_model_check(config: &Config) -> Result<i32> {
    print!("Testing the configured model... ");
    io::stdout().flush()?;
    match check_model(config) {
        Ok(check) => {
            println!(
                "ok: {} answered in {:.1}s.",
                check.label,
                check.elapsed.as_secs_f64()
            );
            Ok(0)
        }
        Err(err) => {
            println!("failed.");
            bail!("{err}\n\nThe settings are saved; fix them with `vibegit config` or `vibegit config set`, then run `vibegit config test`.")
        }
    }
}

/// Opens a file with `$VISUAL`/`$EDITOR`, falling back to the platform opener.
pub fn open_in_editor(path: &Path) -> Result<()> {
    let editor = std::env::var("VISUAL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| {
            std::env::var("EDITOR")
                .ok()
                .filter(|v| !v.trim().is_empty())
        });
    let status = if let Some(editor) = editor {
        let mut parts = editor.split_whitespace();
        let program = parts.next().unwrap_or("vi").to_string();
        let args: Vec<String> = parts.map(|s| s.to_string()).collect();
        Process::new(program).args(args).arg(path).status()
    } else if cfg!(target_os = "macos") {
        Process::new("open").arg(path).status()
    } else if cfg!(target_os = "windows") {
        Process::new("cmd")
            .args(["/C", "start", ""])
            .arg(path)
            .status()
    } else {
        Process::new("xdg-open").arg(path).status()
    };
    let status = status.context("could not launch an editor; set $EDITOR")?;
    if !status.success() {
        bail!("editor exited with {status}");
    }
    Ok(())
}

pub fn debug_dir() -> PathBuf {
    std::env::temp_dir().join("vibegit-debug")
}

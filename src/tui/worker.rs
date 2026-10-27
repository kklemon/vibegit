//! Background thread that owns the workflow and performs all slow operations.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};

use crate::changes::{ChangeSet, FileSummary};
use crate::git::CommitMode;
use crate::rules::DerivedRules;
use crate::workflow::{Leftover, Workflow, WorkflowError};
use crate::DEFAULT_RULES;

#[derive(Debug, Clone)]
pub struct ApplyItem {
    /// Index of the proposal in the UI's list, echoed back in events.
    pub index: usize,
    pub change_ids: Vec<u32>,
    pub message: String,
}

#[derive(Debug)]
pub enum Cmd {
    /// Preflight, then (rules offer and) collect and generate.
    Begin,
    /// The user agreed to unstage; continue with the rest of `Begin`.
    Unstage,
    /// Analyze the history and derive `.vibegitrules`.
    DeriveRules,
    /// Write `.vibegitrules`; `then_generate` continues with the commit workflow.
    WriteRules {
        text: String,
        then_generate: bool,
    },
    /// Continue the commit workflow without rules.
    SkipRules,
    /// Collect the remaining changes again and ask the model.
    Generate {
        instruction: Option<String>,
    },
    Apply(Vec<ApplyItem>),
    Push,
    Quit,
}

#[derive(Debug, Clone)]
pub struct ProposalView {
    pub change_ids: Vec<u32>,
    pub message: String,
    pub explanation: String,
    pub files: Vec<FileSummary>,
    pub diff: String,
}

#[derive(Debug, Clone)]
pub struct ExcludedView {
    pub change_ids: Vec<u32>,
    pub explanation: String,
    pub files: Vec<FileSummary>,
    pub diff: String,
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub proposals: Vec<ProposalView>,
    pub excluded: Vec<ExcludedView>,
    pub branch: String,
    pub units: usize,
    pub files: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug)]
pub enum Evt {
    Status(String),
    Fatal(String),
    StagedChanges(Vec<String>),
    /// No `.vibegitrules` exists; the UI may offer to derive one.
    NoRules {
        commits: usize,
    },
    RulesDerived(DerivedRules),
    RulesFailed(String),
    RulesWritten(PathBuf),
    NoChanges {
        branch: String,
    },
    Proposals(Snapshot),
    Committed {
        index: usize,
        sha: String,
        subject: String,
    },
    ApplyFailed {
        index: usize,
        error: String,
    },
    ApplyFinished(Leftover),
    PushResult(Result<String, String>),
}

struct Worker {
    workflow: Workflow,
    changes: Option<ChangeSet>,
    tx: Sender<Evt>,
}

pub fn run(workflow: Workflow, rx: Receiver<Cmd>, tx: Sender<Evt>) {
    let mut worker = Worker {
        workflow,
        changes: None,
        tx,
    };
    let mut pending_instruction: Option<String> = None;
    while let Ok(cmd) = rx.recv() {
        match cmd {
            Cmd::Begin => match worker.begin() {
                Ok(true) => worker.after_preflight(pending_instruction.clone()),
                Ok(false) => {}
                Err(e) => worker.fatal(e),
            },
            Cmd::Unstage => match worker.workflow.unstage_all() {
                Ok(()) => worker.after_preflight(pending_instruction.clone()),
                Err(e) => worker.fatal(e),
            },
            Cmd::DeriveRules => worker.derive_rules(),
            Cmd::WriteRules {
                text,
                then_generate,
            } => {
                match worker.workflow.write_rules(&text) {
                    Ok(path) => worker.send(Evt::RulesWritten(path)),
                    Err(e) => {
                        worker.send(Evt::RulesFailed(format!(
                            "could not write the rules: {e:#}"
                        )));
                        continue;
                    }
                }
                if then_generate {
                    worker.generate(pending_instruction.clone());
                }
            }
            Cmd::SkipRules => worker.generate(pending_instruction.clone()),
            Cmd::Generate { instruction } => {
                pending_instruction = instruction.clone();
                worker.generate(instruction);
            }
            Cmd::Apply(items) => worker.apply(items),
            Cmd::Push => {
                let result = worker.workflow.push(false).map_err(|e| format!("{e:#}"));
                worker.send(Evt::PushResult(result));
            }
            Cmd::Quit => break,
        }
    }
}

impl Worker {
    fn send(&self, evt: Evt) {
        let _ = self.tx.send(evt);
    }

    fn status(&self, text: impl Into<String>) {
        self.send(Evt::Status(text.into()));
    }

    fn fatal(&self, err: WorkflowError) {
        self.send(Evt::Fatal(format!("{err:#}")));
    }

    /// Returns `Ok(true)` when the workflow can continue right away.
    fn begin(&mut self) -> Result<bool, WorkflowError> {
        self.status("Checking the repository...");
        let pre = self.workflow.preflight()?;
        if !pre.staged.is_empty() {
            self.send(Evt::StagedChanges(pre.staged));
            return Ok(false);
        }
        Ok(true)
    }

    /// Offers rule derivation when the project has none, otherwise generates.
    fn after_preflight(&mut self, instruction: Option<String>) {
        if !self.workflow.has_rules() && self.workflow.config.confirmations.derive_rules {
            let commits = self.workflow.repo.commit_count();
            if commits > 0 {
                self.send(Evt::NoRules { commits });
                return;
            }
        }
        self.generate(instruction);
    }

    fn derive_rules(&mut self) {
        let n = self.workflow.config.init.commits;
        self.status(format!("Analyzing the last {n} commits..."));
        match self.workflow.derive_rules() {
            Ok(rules) => self.send(Evt::RulesDerived(rules)),
            Err(WorkflowError::NoHistory) => self.send(Evt::RulesDerived(DerivedRules {
                observations: vec![
                    "The repository has no commits yet, so this is VibeGit's default template."
                        .into(),
                ],
                rules: DEFAULT_RULES.trim_end().to_string(),
            })),
            Err(e) => self.send(Evt::RulesFailed(format!("{e:#}"))),
        }
    }

    fn generate(&mut self, instruction: Option<String>) {
        self.status("Collecting changes...");
        let changes = match self.workflow.collect() {
            Ok(c) => c,
            Err(e) => return self.fatal(e),
        };
        let branch = self
            .workflow
            .repo
            .state()
            .map(|s| s.describe_head())
            .unwrap_or_default();
        if changes.is_empty() {
            self.changes = Some(changes);
            self.send(Evt::NoChanges { branch });
            return;
        }
        self.status(format!(
            "Analyzing {} change unit(s) in {} file(s)...",
            changes.len(),
            changes.files.len()
        ));
        let result = match self.workflow.propose(&changes, instruction.as_deref()) {
            Ok(r) => r,
            Err(e) => return self.fatal(e),
        };
        let proposals = result
            .commit_proposals
            .iter()
            .map(|p| ProposalView {
                change_ids: p.change_ids.clone(),
                message: p.commit_message.trim().to_string(),
                explanation: p.explanation.trim().to_string(),
                files: changes.files_for(&p.change_ids),
                diff: changes.preview(&p.change_ids),
            })
            .collect();
        let excluded = result
            .excluded_groups
            .iter()
            .map(|g| ExcludedView {
                change_ids: g.change_ids.clone(),
                explanation: g.explanation.trim().to_string(),
                files: changes.files_for(&g.change_ids),
                diff: changes.preview(&g.change_ids),
            })
            .collect();
        let snapshot = Snapshot {
            proposals,
            excluded,
            branch,
            units: changes.len(),
            files: changes.files.len(),
            warnings: changes.warnings.clone(),
        };
        self.changes = Some(changes);
        self.send(Evt::Proposals(snapshot));
    }

    fn apply(&mut self, items: Vec<ApplyItem>) {
        let Some(changes) = self.changes.as_mut() else {
            self.send(Evt::Fatal("no changes collected".into()));
            return;
        };
        for item in items {
            self.tx
                .send(Evt::Status(format!(
                    "Committing: {}",
                    item.message.lines().next().unwrap_or("")
                )))
                .ok();
            match self.workflow.apply(
                changes,
                &item.change_ids,
                &item.message,
                CommitMode::Captured,
            ) {
                Ok(info) => {
                    self.tx
                        .send(Evt::Committed {
                            index: item.index,
                            sha: info.short_sha,
                            subject: info.subject,
                        })
                        .ok();
                }
                Err(err) => {
                    self.tx
                        .send(Evt::ApplyFailed {
                            index: item.index,
                            error: format!("{err:#}"),
                        })
                        .ok();
                    break;
                }
            }
        }
        let leftover = self.workflow.leftover().unwrap_or_default();
        self.send(Evt::ApplyFinished(leftover));
    }
}

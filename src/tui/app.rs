//! Interface state and input handling. Rendering lives in `ui.rs`.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};
use ratatui::widgets::ListState;
use tui_textarea::TextArea;

use super::worker::{ApplyItem, Cmd, Evt, ExcludedView, ProposalView, Snapshot};
use std::path::PathBuf;

use crate::config::Confirmations;
use crate::workflow::Leftover;
use crate::DEFAULT_RULES;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Loading,
    Proposals,
    Diff,
    /// Review of a derived `.vibegitrules` before it is written.
    Rules,
    Summary,
    Error,
}

/// What the interface was started for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Commit,
    /// `vibegit init`: derive the rules, review, write, quit.
    Init,
}

pub struct RulesReview {
    pub observations: Vec<String>,
    pub rules: String,
    pub editor: Option<TextArea<'static>>,
    pub scroll: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProposalStatus {
    Pending,
    Committed { sha: String },
    Skipped,
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct ProposalItem {
    pub view: ProposalView,
    pub status: ProposalStatus,
    pub edited: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Proposal(usize),
    Excluded(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confirm {
    CommitAll(usize),
    /// Commit all pending proposals, then push.
    CommitAllAndPush(usize),
    Push,
    Rerun,
    QuitWithPending,
}

impl Confirm {
    fn flag(self, confirmations: &mut Confirmations) -> &mut bool {
        match self {
            Confirm::CommitAll(_) => &mut confirmations.commit_all,
            Confirm::CommitAllAndPush(_) => &mut confirmations.commit_all_and_push,
            Confirm::Push => &mut confirmations.push,
            Confirm::Rerun => &mut confirmations.rerun,
            Confirm::QuitWithPending => &mut confirmations.quit,
        }
    }

    pub fn config_key(self) -> &'static str {
        match self {
            Confirm::CommitAll(_) => "confirmations.commit_all",
            Confirm::CommitAllAndPush(_) => "confirmations.commit_all_and_push",
            Confirm::Push => "confirmations.push",
            Confirm::Rerun => "confirmations.rerun",
            Confirm::QuitWithPending => "confirmations.quit",
        }
    }
}

pub enum Overlay {
    None,
    Unstage(Vec<String>),
    /// Offer to derive `.vibegitrules` from this many commits.
    DeriveRules(usize),
    Confirm(Confirm),
    EditMessage {
        index: usize,
        editor: TextArea<'static>,
    },
    Instruction(TextArea<'static>),
    Menu,
    Help,
    Applying,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Commit,
    CommitAll,
    CommitAllAndPush,
    Edit,
    EditExternal,
    Skip,
    MoveUp,
    MoveDown,
    Diff,
    Instruction,
    Rerun,
    Push,
    Help,
    Quit,
}

impl Action {
    pub const MENU: &'static [Action] = &[
        Action::Commit,
        Action::CommitAll,
        Action::CommitAllAndPush,
        Action::Edit,
        Action::EditExternal,
        Action::Diff,
        Action::Skip,
        Action::MoveUp,
        Action::MoveDown,
        Action::Instruction,
        Action::Rerun,
        Action::Push,
        Action::Help,
        Action::Quit,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Action::Commit => "Commit this proposal",
            Action::CommitAll => "Commit all pending proposals",
            Action::CommitAllAndPush => "Commit all pending proposals, then push",
            Action::Edit => "Edit commit message",
            Action::EditExternal => "Edit commit message in $EDITOR",
            Action::Diff => "Show diff",
            Action::Skip => "Skip / unskip this proposal",
            Action::MoveUp => "Move proposal up",
            Action::MoveDown => "Move proposal down",
            Action::Instruction => "Regenerate with an instruction",
            Action::Rerun => "Regenerate proposals",
            Action::Push => "Push commits",
            Action::Help => "Help",
            Action::Quit => "Quit",
        }
    }

    pub fn key(&self) -> &'static str {
        match self {
            Action::Commit => "c",
            Action::CommitAll => "C",
            Action::CommitAllAndPush => "P",
            Action::Edit => "e",
            Action::EditExternal => "E",
            Action::Diff => "d / Enter",
            Action::Skip => "s",
            Action::MoveUp => "K",
            Action::MoveDown => "J",
            Action::Instruction => "i",
            Action::Rerun => "r",
            Action::Push => "p",
            Action::Help => "?",
            Action::Quit => "q",
        }
    }
}

pub enum Effect {
    Send(Cmd),
    ExternalEdit { index: usize, message: String },
    ExternalEditRules(String),
    SaveConfirmations(Confirmations),
}

/// A focusable, clickable action in the bottom bar. Activating it sends its shortcut key.
#[derive(Debug, Clone, Copy)]
pub struct Button {
    pub label: &'static str,
    pub key_label: &'static str,
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
    pub enabled: bool,
}

fn button(label: &'static str, key_label: &'static str, code: KeyCode, enabled: bool) -> Button {
    Button {
        label,
        key_label,
        code,
        modifiers: KeyModifiers::NONE,
        enabled,
    }
}

#[derive(Debug, Clone)]
pub struct CommitRecord {
    pub sha: String,
    pub subject: String,
}

pub struct App {
    pub screen: Screen,
    pub overlay: Overlay,
    pub proposals: Vec<ProposalItem>,
    pub excluded: Vec<ExcludedView>,
    pub selected: usize,
    pub list_state: ListState,
    pub detail_scroll: u16,
    pub diff_scroll: u16,
    pub diff_row: Option<Row>,
    pub status: String,
    pub error: Option<String>,
    pub spinner: usize,
    pub busy: bool,
    pub repo_label: String,
    pub branch: String,
    pub model_label: String,
    pub units: usize,
    pub files: usize,
    pub warnings: Vec<String>,
    pub update_notice: Option<String>,
    pub commits: Vec<CommitRecord>,
    pub leftover: Option<Leftover>,
    pub push_status: Option<String>,
    /// Set by "commit all and push": push once the apply run finishes cleanly.
    push_after_apply: bool,
    pub instruction: Option<String>,
    pub apply_log: Vec<String>,
    pub menu_index: usize,
    pub debug: bool,
    pub confirmations: Confirmations,
    pub mode: Mode,
    pub rules: Option<RulesReview>,
    pub rules_target: String,
    pub rules_written: Option<PathBuf>,
    /// Index into `buttons()` when the button bar has keyboard focus.
    pub focus: Option<usize>,
    /// Screen rectangles of the buttons drawn last, for mouse hit testing.
    pub button_rects: Vec<(Rect, usize)>,
    pub list_rect: Rect,
    pub detail_rect: Rect,
    pub diff_rect: Rect,
    pub should_quit: bool,
    pub exit_code: i32,
    effects: Vec<Effect>,
}

impl App {
    pub fn new(
        repo_label: String,
        model_label: String,
        instruction: Option<String>,
        debug: bool,
        confirmations: Confirmations,
    ) -> Self {
        let rules_target = format!("{repo_label}/.vibegitrules");
        Self {
            screen: Screen::Loading,
            overlay: Overlay::None,
            proposals: Vec::new(),
            excluded: Vec::new(),
            selected: 0,
            list_state: ListState::default(),
            detail_scroll: 0,
            diff_scroll: 0,
            diff_row: None,
            status: "Starting...".into(),
            error: None,
            spinner: 0,
            busy: true,
            repo_label,
            branch: String::new(),
            model_label,
            units: 0,
            files: 0,
            warnings: Vec::new(),
            update_notice: None,
            commits: Vec::new(),
            leftover: None,
            push_status: None,
            push_after_apply: false,
            instruction,
            apply_log: Vec::new(),
            menu_index: 0,
            debug,
            confirmations,
            mode: Mode::Commit,
            rules: None,
            rules_target,
            rules_written: None,
            focus: None,
            button_rects: Vec::new(),
            list_rect: Rect::default(),
            detail_rect: Rect::default(),
            diff_rect: Rect::default(),
            should_quit: false,
            exit_code: 0,
            effects: Vec::new(),
        }
    }

    pub fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }

    pub fn tick(&mut self) {
        self.spinner = self.spinner.wrapping_add(1);
    }

    /// Lines printed after the terminal is restored.
    pub fn farewell(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(err) = &self.error {
            lines.push(format!("error: {err}"));
        }
        if let Some(path) = &self.rules_written {
            lines.push(format!("Wrote {}", path.display()));
        } else if self.mode == Mode::Init && self.error.is_none() {
            lines.push("No .vibegitrules written.".to_string());
        }
        if !self.commits.is_empty() {
            lines.push(format!("VibeGit created {} commit(s):", self.commits.len()));
            for c in &self.commits {
                lines.push(format!("  {} {}", c.sha, c.subject));
            }
        }
        if let Some(latest) = &self.update_notice {
            lines.push(format!(
                "A newer VibeGit ({latest}) is available: uv tool upgrade vibegit"
            ));
        }
        lines
    }

    // --- rows and selection ---------------------------------------------------

    pub fn rows(&self) -> Vec<Row> {
        let mut rows: Vec<Row> = (0..self.proposals.len()).map(Row::Proposal).collect();
        rows.extend((0..self.excluded.len()).map(Row::Excluded));
        rows
    }

    pub fn selected_row(&self) -> Option<Row> {
        self.rows().get(self.selected).copied()
    }

    fn select(&mut self, index: usize) {
        let count = self.rows().len();
        if count == 0 {
            self.selected = 0;
            self.list_state.select(None);
            return;
        }
        self.selected = index.min(count - 1);
        self.list_state.select(Some(self.selected));
        self.detail_scroll = 0;
    }

    fn move_selection(&mut self, delta: i32) {
        let count = self.rows().len() as i32;
        if count == 0 {
            return;
        }
        let next = (self.selected as i32 + delta).clamp(0, count - 1);
        self.select(next as usize);
    }

    pub fn pending_indices(&self) -> Vec<usize> {
        self.proposals
            .iter()
            .enumerate()
            .filter(|(_, p)| p.status == ProposalStatus::Pending)
            .map(|(i, _)| i)
            .collect()
    }

    pub fn pending_count(&self) -> usize {
        self.pending_indices().len()
    }

    pub fn set_message(&mut self, index: usize, text: String) {
        if let Some(item) = self.proposals.get_mut(index) {
            let text = text.trim().to_string();
            if !text.is_empty() && text != item.view.message {
                item.view.message = text;
                item.edited = true;
                self.status = "Commit message updated.".into();
            }
        }
    }

    pub fn diff_text(&self) -> Option<(&str, String)> {
        match self.diff_row? {
            Row::Proposal(i) => {
                let p = self.proposals.get(i)?;
                Some((
                    &p.view.diff,
                    p.view.message.lines().next().unwrap_or("").to_string(),
                ))
            }
            Row::Excluded(i) => {
                let g = self.excluded.get(i)?;
                Some((&g.diff, format!("Excluded: {}", g.explanation)))
            }
        }
    }

    // --- worker events ---------------------------------------------------------

    pub fn on_event(&mut self, evt: Evt) {
        match evt {
            Evt::Status(text) => self.status = text,
            Evt::Fatal(message) => {
                self.busy = false;
                self.error = Some(message);
                self.screen = Screen::Error;
                self.overlay = Overlay::None;
            }
            Evt::StagedChanges(paths) => self.prompt_unstage(paths),
            Evt::NoRules { commits } => {
                self.busy = false;
                self.overlay = Overlay::DeriveRules(commits);
            }
            Evt::RulesDerived(derived) => {
                self.busy = false;
                self.overlay = Overlay::None;
                self.rules = Some(RulesReview {
                    observations: derived.observations,
                    rules: derived.rules,
                    editor: None,
                    scroll: 0,
                });
                self.screen = Screen::Rules;
                self.status = format!(
                    "Review the proposed rules; Enter writes {}",
                    self.rules_target
                );
            }
            Evt::RulesFailed(message) => match self.mode {
                Mode::Init => {
                    self.busy = false;
                    self.error = Some(message);
                    self.screen = Screen::Error;
                }
                Mode::Commit => {
                    self.status =
                        format!("Could not derive rules ({message}); continuing without.");
                    self.busy = true;
                    self.screen = Screen::Loading;
                    self.effects.push(Effect::Send(Cmd::SkipRules));
                }
            },
            Evt::RulesWritten(path) => {
                self.rules_written = Some(path.clone());
                match self.mode {
                    Mode::Init => {
                        self.busy = false;
                        self.should_quit = true;
                    }
                    Mode::Commit => {
                        self.status = format!("Wrote {}. Generating proposals...", path.display());
                        self.busy = true;
                        self.screen = Screen::Loading;
                    }
                }
            }
            Evt::NoChanges { branch } => {
                self.busy = false;
                self.branch = branch;
                self.status = "No uncommitted changes found.".into();
                self.screen = Screen::Summary;
            }
            Evt::Proposals(snapshot) => self.load_snapshot(snapshot),
            Evt::Committed {
                index,
                sha,
                subject,
            } => {
                if let Some(item) = self.proposals.get_mut(index) {
                    item.status = ProposalStatus::Committed { sha: sha.clone() };
                }
                self.apply_log.push(format!("committed {sha} {subject}"));
                self.commits.push(CommitRecord { sha, subject });
            }
            Evt::ApplyFailed { index, error } => {
                if let Some(item) = self.proposals.get_mut(index) {
                    item.status = ProposalStatus::Failed(error.clone());
                }
                self.apply_log.push(format!("failed: {error}"));
            }
            Evt::ApplyFinished(leftover) => {
                self.busy = false;
                self.leftover = Some(leftover);
                self.overlay = Overlay::None;
                let failed = self
                    .proposals
                    .iter()
                    .any(|p| matches!(p.status, ProposalStatus::Failed(_)));
                let push = std::mem::take(&mut self.push_after_apply);
                if failed {
                    self.status = if push {
                        "A commit failed; see the proposal for details. Remaining proposals were not applied and nothing was pushed.".into()
                    } else {
                        "A commit failed; see the proposal for details. Remaining proposals were not applied.".into()
                    };
                } else if self.pending_count() == 0
                    && !self
                        .proposals
                        .iter()
                        .any(|p| p.status == ProposalStatus::Skipped)
                {
                    self.status = "All proposals committed.".into();
                    self.screen = Screen::Summary;
                } else {
                    self.status = format!("{} proposal(s) still pending.", self.pending_count());
                }
                if push && !failed && !self.commits.is_empty() {
                    self.start_push();
                }
            }
            Evt::PushResult(result) => {
                self.busy = false;
                self.push_status = Some(match result {
                    Ok(text) if text.is_empty() => "pushed".to_string(),
                    Ok(text) => format!("pushed: {text}"),
                    Err(err) => format!("push failed: {err}"),
                });
                self.status = self.push_status.clone().unwrap_or_default();
            }
        }
    }

    fn load_snapshot(&mut self, snapshot: Snapshot) {
        self.busy = false;
        self.proposals = snapshot
            .proposals
            .into_iter()
            .map(|view| ProposalItem {
                view,
                status: ProposalStatus::Pending,
                edited: false,
            })
            .collect();
        self.excluded = snapshot.excluded;
        self.branch = snapshot.branch;
        self.units = snapshot.units;
        self.files = snapshot.files;
        self.warnings = snapshot.warnings;
        self.screen = Screen::Proposals;
        self.overlay = Overlay::None;
        self.select(0);
        self.status = if self.warnings.is_empty() {
            format!(
                "{} proposal(s) for {} change unit(s) in {} file(s).",
                self.proposals.len(),
                self.units,
                self.files
            )
        } else {
            format!(
                "{} proposal(s). Warning: {}",
                self.proposals.len(),
                self.warnings.join(" | ")
            )
        };
    }

    // --- input -----------------------------------------------------------------

    pub fn on_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.exit_code = 130;
            self.should_quit = true;
            return;
        }
        if self.handle_button_focus(key) {
            return;
        }
        match std::mem::replace(&mut self.overlay, Overlay::None) {
            Overlay::None => {}
            Overlay::Unstage(paths) => return self.on_key_unstage(key, paths),
            Overlay::DeriveRules(commits) => return self.on_key_derive_rules(key, commits),
            Overlay::Confirm(kind) => return self.on_key_confirm(key, kind),
            Overlay::EditMessage { index, editor } => {
                return self.on_key_edit_message(key, index, editor)
            }
            Overlay::Instruction(editor) => return self.on_key_instruction(key, editor),
            Overlay::Menu => return self.on_key_menu(key),
            Overlay::Help => {
                // Any key closes help.
                return;
            }
            Overlay::Applying => {
                self.overlay = Overlay::Applying;
                return;
            }
        }
        match self.screen {
            Screen::Loading => {
                if matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) {
                    self.should_quit = true;
                }
            }
            Screen::Error => {
                self.exit_code = 1;
                self.should_quit = true;
            }
            Screen::Summary => match key.code {
                KeyCode::Char('q') | KeyCode::Esc | KeyCode::Enter => self.should_quit = true,
                KeyCode::Char('p') if !self.busy && !self.commits.is_empty() => {
                    self.confirm(Confirm::Push)
                }
                KeyCode::Char('b') if !self.busy && !self.proposals.is_empty() => {
                    self.screen = Screen::Proposals
                }
                _ => {}
            },
            Screen::Diff => self.on_key_diff(key),
            Screen::Rules => self.on_key_rules(key),
            Screen::Proposals => self.on_key_proposals(key),
        }
    }

    fn on_key_unstage(&mut self, key: KeyEvent, paths: Vec<String>) {
        match key.code {
            KeyCode::Char('y') | KeyCode::Enter => self.unstage(),
            KeyCode::Char('Y') => {
                self.confirmations.unstage = false;
                self.effects
                    .push(Effect::SaveConfirmations(self.confirmations));
                self.unstage();
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc | KeyCode::Char('q') => {
                self.should_quit = true;
            }
            _ => self.overlay = Overlay::Unstage(paths),
        }
    }

    fn prompt_unstage(&mut self, paths: Vec<String>) {
        self.busy = false;
        if self.confirmations.unstage {
            self.overlay = Overlay::Unstage(paths);
        } else {
            self.unstage();
        }
    }

    fn unstage(&mut self) {
        self.busy = true;
        self.status = "Unstaging...".into();
        self.effects.push(Effect::Send(Cmd::Unstage));
    }

    /// Asks first unless the user disabled this confirmation with `Y`.
    fn confirm(&mut self, kind: Confirm) {
        if *kind.flag(&mut self.confirmations) {
            self.overlay = Overlay::Confirm(kind);
        } else {
            self.execute(kind);
        }
    }

    fn execute(&mut self, kind: Confirm) {
        match kind {
            Confirm::CommitAll(_) => self.commit_all(),
            Confirm::CommitAllAndPush(_) => {
                if self.pending_count() > 0 {
                    self.push_after_apply = true;
                    self.commit_all();
                }
            }
            Confirm::Push => self.start_push(),
            Confirm::Rerun => self.regenerate(),
            Confirm::QuitWithPending => self.screen = Screen::Summary,
        }
    }
    fn on_key_confirm(&mut self, key: KeyEvent, kind: Confirm) {
        match key.code {
            KeyCode::Char('y') | KeyCode::Enter => self.execute(kind),
            KeyCode::Char('Y') => {
                *kind.flag(&mut self.confirmations) = false;
                self.effects
                    .push(Effect::SaveConfirmations(self.confirmations));
                self.execute(kind);
                self.status = format!(
                    "Won't ask again. Re-enable with: vibegit config set {} true",
                    kind.config_key()
                );
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc | KeyCode::Char('q') => {}
            _ => self.overlay = Overlay::Confirm(kind),
        }
    }
    fn on_key_edit_message(&mut self, key: KeyEvent, index: usize, mut editor: TextArea<'static>) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => {
                self.status = "Edit cancelled.".into();
            }
            KeyCode::Char('s') if ctrl => {
                let text = editor.lines().join("\n");
                self.set_message(index, text);
            }
            _ => {
                editor.input(key);
                self.overlay = Overlay::EditMessage { index, editor };
            }
        }
    }

    fn on_key_instruction(&mut self, key: KeyEvent, mut editor: TextArea<'static>) {
        match key.code {
            KeyCode::Esc => {}
            KeyCode::Enter => {
                let text = editor.lines().join(" ").trim().to_string();
                self.instruction = if text.is_empty() { None } else { Some(text) };
                self.regenerate();
            }
            _ => {
                editor.input(key);
                self.overlay = Overlay::Instruction(editor);
            }
        }
    }

    fn on_key_menu(&mut self, key: KeyEvent) {
        let count = Action::MENU.len();
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('m') => {}
            KeyCode::Up | KeyCode::Char('k') => {
                self.menu_index = (self.menu_index + count - 1) % count;
                self.overlay = Overlay::Menu;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.menu_index = (self.menu_index + 1) % count;
                self.overlay = Overlay::Menu;
            }
            KeyCode::Enter => {
                let action = Action::MENU[self.menu_index];
                self.run_action(action);
            }
            _ => self.overlay = Overlay::Menu,
        }
    }

    fn on_key_diff(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc | KeyCode::Char('d') | KeyCode::Enter => {
                self.screen = Screen::Proposals
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.diff_scroll = self.diff_scroll.saturating_add(1)
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.diff_scroll = self.diff_scroll.saturating_sub(1)
            }
            KeyCode::PageDown | KeyCode::Char(' ') => {
                self.diff_scroll = self.diff_scroll.saturating_add(20)
            }
            KeyCode::PageUp => self.diff_scroll = self.diff_scroll.saturating_sub(20),
            KeyCode::Home | KeyCode::Char('g') => self.diff_scroll = 0,
            KeyCode::End | KeyCode::Char('G') => self.diff_scroll = u16::MAX / 2,
            KeyCode::Char('n') => {
                self.move_selection(1);
                self.diff_row = self.selected_row();
                self.diff_scroll = 0;
            }
            KeyCode::Char('N') | KeyCode::Char('p') => {
                self.move_selection(-1);
                self.diff_row = self.selected_row();
                self.diff_scroll = 0;
            }
            KeyCode::Char('e') => self.run_action(Action::Edit),
            KeyCode::Char('c') => self.run_action(Action::Commit),
            KeyCode::Char('?') => self.overlay = Overlay::Help,
            _ => {}
        }
    }

    fn on_key_proposals(&mut self, key: KeyEvent) {
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Down if alt => self.run_action(Action::MoveDown),
            KeyCode::Up if alt => self.run_action(Action::MoveUp),
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
            KeyCode::PageDown => self.move_selection(10),
            KeyCode::PageUp => self.move_selection(-10),
            KeyCode::Home | KeyCode::Char('g') => self.select(0),
            KeyCode::End | KeyCode::Char('G') => self.select(usize::MAX),
            KeyCode::Char('J') => self.run_action(Action::MoveDown),
            KeyCode::Char('K') => self.run_action(Action::MoveUp),
            KeyCode::Char(']') => self.detail_scroll = self.detail_scroll.saturating_add(3),
            KeyCode::Char('[') => self.detail_scroll = self.detail_scroll.saturating_sub(3),
            KeyCode::Enter | KeyCode::Char('d') => self.run_action(Action::Diff),
            KeyCode::Char('c') => self.run_action(Action::Commit),
            KeyCode::Char('C') | KeyCode::Char('a') => self.run_action(Action::CommitAll),
            KeyCode::Char('P') => self.run_action(Action::CommitAllAndPush),
            KeyCode::Char('e') => self.run_action(Action::Edit),
            KeyCode::Char('E') => self.run_action(Action::EditExternal),
            KeyCode::Char('s') => self.run_action(Action::Skip),
            KeyCode::Char('i') => self.run_action(Action::Instruction),
            KeyCode::Char('r') => self.run_action(Action::Rerun),
            KeyCode::Char('p') => self.run_action(Action::Push),
            KeyCode::Char('m') | KeyCode::Char(' ') => {
                self.menu_index = 0;
                self.overlay = Overlay::Menu;
            }
            KeyCode::Char('?') | KeyCode::F(1) => self.run_action(Action::Help),
            KeyCode::Char('q') | KeyCode::Esc => self.run_action(Action::Quit),
            _ => {}
        }
    }

    // --- button bar ---------------------------------------------------------------

    /// The buttons shown in the bottom bar for the current screen or overlay.
    pub fn buttons(&self) -> Vec<Button> {
        use KeyCode::{Char, Enter, Esc};
        let yes_no = |third: &'static str| {
            vec![
                button("Yes", "y", Char('y'), true),
                button("Yes, don't ask again", "Y", Char('Y'), true),
                button(third, "n", Char('n'), true),
            ]
        };
        match &self.overlay {
            Overlay::Unstage(_) => return yes_no("Quit"),
            Overlay::DeriveRules(_) => {
                return vec![
                    button("Yes, derive rules", "y", Char('y'), true),
                    button("Not now", "n", Char('n'), true),
                    button("Never ask", "N", Char('N'), true),
                ]
            }
            Overlay::Confirm(_) => return yes_no("No"),
            Overlay::EditMessage { .. } => {
                return vec![
                    Button {
                        label: "Save",
                        key_label: "Ctrl+S",
                        code: Char('s'),
                        modifiers: KeyModifiers::CONTROL,
                        enabled: true,
                    },
                    button("Cancel", "Esc", Esc, true),
                ]
            }
            Overlay::Instruction(_) => {
                return vec![
                    button("Regenerate", "Enter", Enter, true),
                    button("Cancel", "Esc", Esc, true),
                ]
            }
            Overlay::Menu => {
                return vec![
                    button("Run", "Enter", Enter, true),
                    button("Close", "Esc", Esc, true),
                ]
            }
            Overlay::Help => return vec![button("Close", "Esc", Esc, true)],
            Overlay::Applying => return Vec::new(),
            Overlay::None => {}
        }
        match self.screen {
            Screen::Loading => vec![button("Quit", "q", Char('q'), true)],
            Screen::Rules => {
                if self.rules_editing() {
                    return vec![
                        Button {
                            label: "Apply",
                            key_label: "Ctrl+S",
                            code: Char('s'),
                            modifiers: KeyModifiers::CONTROL,
                            enabled: true,
                        },
                        button("Cancel edit", "Esc", Esc, true),
                    ];
                }
                let mut out = vec![
                    button("Write .vibegitrules", "Enter", Enter, !self.busy),
                    button("Edit", "e", Char('e'), !self.busy),
                    button("Edit in $EDITOR", "E", Char('E'), !self.busy),
                    button("Regenerate", "r", Char('r'), !self.busy),
                ];
                match self.mode {
                    Mode::Init => {
                        out.push(button("Use template", "t", Char('t'), !self.busy));
                        out.push(button("Cancel", "q", Char('q'), true));
                    }
                    Mode::Commit => out.push(button("Skip", "s", Char('s'), !self.busy)),
                }
                out
            }
            Screen::Error => vec![button("Exit", "Enter", Enter, true)],
            Screen::Summary => vec![
                button(
                    "Push",
                    "p",
                    Char('p'),
                    !self.busy && !self.commits.is_empty(),
                ),
                button(
                    "Back to proposals",
                    "b",
                    Char('b'),
                    !self.busy && !self.proposals.is_empty(),
                ),
                button("Quit", "q", Char('q'), true),
            ],
            Screen::Diff => {
                let (editable, committable) = self.selected_capabilities();
                vec![
                    button("Back", "q", Char('q'), true),
                    button("Previous", "p", Char('p'), self.selected > 0),
                    button(
                        "Next",
                        "n",
                        Char('n'),
                        self.selected + 1 < self.rows().len(),
                    ),
                    button("Edit", "e", Char('e'), !self.busy && editable),
                    button("Commit", "c", Char('c'), !self.busy && committable),
                ]
            }
            Screen::Proposals => {
                let (editable, committable) = self.selected_capabilities();
                let busy = self.busy;
                let is_proposal = matches!(self.selected_row(), Some(Row::Proposal(_)));
                vec![
                    button("Commit", "c", Char('c'), !busy && committable),
                    button(
                        "Commit all",
                        "C",
                        Char('C'),
                        !busy && self.pending_count() > 0,
                    ),
                    button(
                        "Commit all & push",
                        "P",
                        Char('P'),
                        !busy && self.pending_count() > 0,
                    ),
                    button("Edit", "e", Char('e'), !busy && editable),
                    button("Diff", "d", Char('d'), self.selected_row().is_some()),
                    button("Skip", "s", Char('s'), !busy && is_proposal),
                    button("Instruct", "i", Char('i'), !busy),
                    button("Regenerate", "r", Char('r'), !busy),
                    button("Push", "p", Char('p'), !busy && !self.commits.is_empty()),
                    button("Help", "?", Char('?'), true),
                    button("Quit", "q", Char('q'), true),
                ]
            }
        }
    }

    /// `(editable, committable)` for the selected row.
    fn selected_capabilities(&self) -> (bool, bool) {
        match self.selected_row() {
            Some(Row::Proposal(i)) => match self.proposals[i].status {
                ProposalStatus::Pending | ProposalStatus::Failed(_) => (true, true),
                _ => (false, false),
            },
            _ => (false, false),
        }
    }

    fn editor_overlay_active(&self) -> bool {
        matches!(
            self.overlay,
            Overlay::EditMessage { .. } | Overlay::Instruction(_)
        ) || self.rules_editing()
    }

    fn rules_editing(&self) -> bool {
        self.screen == Screen::Rules
            && matches!(self.overlay, Overlay::None)
            && self.rules.as_ref().is_some_and(|r| r.editor.is_some())
    }

    /// Tab moves keyboard focus into the button bar. Returns true when the key was consumed.
    fn handle_button_focus(&mut self, key: KeyEvent) -> bool {
        if self.editor_overlay_active() {
            return false;
        }
        let buttons = self.buttons();
        let enabled: Vec<usize> = buttons
            .iter()
            .enumerate()
            .filter(|(_, b)| b.enabled)
            .map(|(i, _)| i)
            .collect();
        let Some(current) = self.focus else {
            return match key.code {
                KeyCode::Tab if !enabled.is_empty() => {
                    self.focus = enabled.first().copied();
                    true
                }
                KeyCode::BackTab if !enabled.is_empty() => {
                    self.focus = enabled.last().copied();
                    true
                }
                _ => false,
            };
        };
        let pos = enabled.iter().position(|&i| i == current);
        match key.code {
            KeyCode::Tab => {
                self.focus = pos.and_then(|p| enabled.get(p + 1).copied());
                true
            }
            KeyCode::BackTab => {
                self.focus = pos
                    .and_then(|p| p.checked_sub(1))
                    .and_then(|p| enabled.get(p).copied());
                true
            }
            KeyCode::Right | KeyCode::Char('l') => {
                if let Some(p) = pos {
                    self.focus = Some(enabled[(p + 1) % enabled.len()]);
                }
                true
            }
            KeyCode::Left | KeyCode::Char('h') => {
                if let Some(p) = pos {
                    self.focus = Some(enabled[(p + enabled.len() - 1) % enabled.len()]);
                }
                true
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                self.activate_button(current);
                true
            }
            KeyCode::Esc | KeyCode::Up => {
                self.focus = None;
                true
            }
            _ => {
                // A shortcut was typed: let it through and release the focus.
                self.focus = None;
                false
            }
        }
    }

    /// Runs the button's shortcut as if the key had been pressed.
    pub fn activate_button(&mut self, index: usize) {
        let buttons = self.buttons();
        let Some(b) = buttons.get(index).copied() else {
            return;
        };
        if !b.enabled {
            self.status = format!("{} is not available right now.", b.label);
            return;
        }
        let focused = self.focus.take();
        self.on_key(KeyEvent::new(b.code, b.modifiers));
        if focused.is_some() {
            let count = self.buttons().len();
            self.focus = (count > 0).then(|| index.min(count - 1));
        }
    }

    pub fn on_mouse(&mut self, mouse: MouseEvent) {
        let at = Position::new(mouse.column, mouse.row);
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some((_, index)) = self
                    .button_rects
                    .iter()
                    .find(|(r, _)| r.contains(at))
                    .copied()
                {
                    self.activate_button(index);
                } else if matches!(self.overlay, Overlay::None)
                    && self.screen == Screen::Proposals
                    && self.list_rect.contains(at)
                {
                    let row = (mouse.row - self.list_rect.y) as usize;
                    if row >= 1 {
                        let index = self.list_state.offset() + row - 1;
                        if index < self.rows().len() {
                            self.select(index);
                        }
                    }
                }
            }
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                if !matches!(self.overlay, Overlay::None) {
                    return;
                }
                let delta: i32 = if mouse.kind == MouseEventKind::ScrollDown {
                    3
                } else {
                    -3
                };
                match self.screen {
                    Screen::Diff => {
                        self.diff_scroll = (i32::from(self.diff_scroll) + delta).max(0) as u16
                    }
                    Screen::Proposals if self.detail_rect.contains(at) => {
                        self.detail_scroll = (i32::from(self.detail_scroll) + delta).max(0) as u16
                    }
                    Screen::Proposals => self.move_selection(delta.signum()),
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn on_key_derive_rules(&mut self, key: KeyEvent, commits: usize) {
        match key.code {
            KeyCode::Char('y') | KeyCode::Enter => self.start_derivation(),
            KeyCode::Char('n') | KeyCode::Esc => self.skip_rules(),
            KeyCode::Char('N') => {
                self.confirmations.derive_rules = false;
                self.effects
                    .push(Effect::SaveConfirmations(self.confirmations));
                self.skip_rules();
            }
            KeyCode::Char('q') => self.should_quit = true,
            _ => self.overlay = Overlay::DeriveRules(commits),
        }
    }

    fn start_derivation(&mut self) {
        self.busy = true;
        self.overlay = Overlay::None;
        self.screen = Screen::Loading;
        self.status = "Analyzing the commit history...".into();
        self.effects.push(Effect::Send(Cmd::DeriveRules));
    }

    fn skip_rules(&mut self) {
        self.busy = true;
        self.overlay = Overlay::None;
        self.screen = Screen::Loading;
        self.status = "Continuing without project rules...".into();
        self.effects.push(Effect::Send(Cmd::SkipRules));
    }

    fn write_rules(&mut self) {
        let Some(text) = self.rules.as_ref().map(|r| r.rules.clone()) else {
            return;
        };
        if text.trim().is_empty() {
            self.status = "The rules are empty; nothing written.".into();
            return;
        }
        self.busy = true;
        self.status = format!("Writing {}...", self.rules_target);
        self.effects.push(Effect::Send(Cmd::WriteRules {
            text,
            then_generate: self.mode == Mode::Commit,
        }));
    }

    /// Replaces the proposed rules text (after an external edit).
    pub fn set_rules_text(&mut self, text: String) {
        if let Some(review) = self.rules.as_mut() {
            if !text.trim().is_empty() {
                review.rules = text.trim_end().to_string();
                review.scroll = 0;
                self.status = "Rules updated.".into();
            }
        }
    }

    fn on_key_rules(&mut self, key: KeyEvent) {
        let Some(review) = self.rules.as_mut() else {
            return;
        };
        if let Some(editor) = review.editor.as_mut() {
            let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
            match key.code {
                KeyCode::Esc => review.editor = None,
                KeyCode::Char('s') if ctrl => {
                    let text = editor.lines().join("\n");
                    review.editor = None;
                    self.set_rules_text(text);
                }
                _ => {
                    editor.input(key);
                }
            }
            return;
        }
        if self.busy {
            return;
        }
        match key.code {
            KeyCode::Enter => self.write_rules(),
            KeyCode::Char('e') => {
                let lines: Vec<String> = review.rules.lines().map(String::from).collect();
                let mut editor = TextArea::new(lines);
                editor.set_cursor_line_style(ratatui::style::Style::default());
                review.editor = Some(editor);
            }
            KeyCode::Char('E') => {
                let text = review.rules.clone();
                self.effects.push(Effect::ExternalEditRules(text));
            }
            KeyCode::Char('r') => self.start_derivation(),
            KeyCode::Char('t') if self.mode == Mode::Init => {
                review.rules = DEFAULT_RULES.trim_end().to_string();
                review.observations = vec!["VibeGit's default template.".to_string()];
                review.scroll = 0;
                self.status = "Template loaded; Enter writes it.".into();
            }
            KeyCode::Char('s') if self.mode == Mode::Commit => self.skip_rules(),
            KeyCode::Char('q') | KeyCode::Esc => match self.mode {
                Mode::Init => self.should_quit = true,
                Mode::Commit => self.skip_rules(),
            },
            KeyCode::Down | KeyCode::Char('j') => review.scroll = review.scroll.saturating_add(1),
            KeyCode::Up | KeyCode::Char('k') => review.scroll = review.scroll.saturating_sub(1),
            KeyCode::PageDown => review.scroll = review.scroll.saturating_add(15),
            KeyCode::PageUp => review.scroll = review.scroll.saturating_sub(15),
            KeyCode::Home | KeyCode::Char('g') => review.scroll = 0,
            KeyCode::End | KeyCode::Char('G') => review.scroll = u16::MAX / 2,
            KeyCode::Char('?') => self.overlay = Overlay::Help,
            _ => {}
        }
    }

    // --- actions ---------------------------------------------------------------

    pub fn run_action(&mut self, action: Action) {
        if self.busy && !matches!(action, Action::Help | Action::Quit) {
            self.status = "Please wait for the current operation to finish.".into();
            return;
        }
        let row = self.selected_row();
        match action {
            Action::Commit => {
                if let Some(Row::Proposal(i)) = row {
                    match self.proposals[i].status {
                        ProposalStatus::Pending => self.apply_indices(vec![i]),
                        ProposalStatus::Skipped => {
                            self.status =
                                "This proposal is skipped; press s to unskip it first.".into()
                        }
                        ProposalStatus::Committed { .. } => {
                            self.status = "Already committed.".into()
                        }
                        ProposalStatus::Failed(_) => {
                            // Allow a retry after the user fixed the cause (e.g. a hook).
                            self.proposals[i].status = ProposalStatus::Pending;
                            self.apply_indices(vec![i]);
                        }
                    }
                } else {
                    self.status = "Excluded groups cannot be committed; regenerate with an instruction if they should be.".into();
                }
            }
            Action::CommitAll | Action::CommitAllAndPush => {
                let pending = self.pending_count();
                if pending == 0 {
                    self.status = "Nothing pending to commit.".into();
                } else if action == Action::CommitAll {
                    self.confirm(Confirm::CommitAll(pending));
                } else {
                    self.confirm(Confirm::CommitAllAndPush(pending));
                }
            }
            Action::Edit => {
                if let Some(Row::Proposal(i)) = row {
                    if self.proposals[i].status == ProposalStatus::Pending
                        || matches!(self.proposals[i].status, ProposalStatus::Failed(_))
                    {
                        let lines: Vec<String> = self.proposals[i]
                            .view
                            .message
                            .lines()
                            .map(String::from)
                            .collect();
                        let mut editor = TextArea::new(lines);
                        editor.set_cursor_line_style(ratatui::style::Style::default());
                        editor.move_cursor(tui_textarea::CursorMove::Bottom);
                        editor.move_cursor(tui_textarea::CursorMove::End);
                        self.overlay = Overlay::EditMessage { index: i, editor };
                    } else {
                        self.status = "Only pending proposals can be edited.".into();
                    }
                }
            }
            Action::EditExternal => {
                if let Some(Row::Proposal(i)) = row {
                    if self.proposals[i].status == ProposalStatus::Pending {
                        self.effects.push(Effect::ExternalEdit {
                            index: i,
                            message: self.proposals[i].view.message.clone(),
                        });
                    }
                }
            }
            Action::Skip => {
                if let Some(Row::Proposal(i)) = row {
                    let item = &mut self.proposals[i];
                    item.status = match item.status {
                        ProposalStatus::Pending => ProposalStatus::Skipped,
                        ProposalStatus::Skipped => ProposalStatus::Pending,
                        ref other => other.clone(),
                    };
                }
            }
            Action::MoveUp | Action::MoveDown => {
                if let Some(Row::Proposal(i)) = row {
                    let target = if action == Action::MoveUp {
                        i.checked_sub(1)
                    } else if i + 1 < self.proposals.len() {
                        Some(i + 1)
                    } else {
                        None
                    };
                    if let Some(t) = target {
                        self.proposals.swap(i, t);
                        self.select(t);
                    }
                }
            }
            Action::Diff => {
                if row.is_some() {
                    self.diff_row = row;
                    self.diff_scroll = 0;
                    self.screen = Screen::Diff;
                }
            }
            Action::Instruction => {
                let mut editor = TextArea::new(vec![self.instruction.clone().unwrap_or_default()]);
                editor.set_cursor_line_style(ratatui::style::Style::default());
                editor.set_placeholder_text("e.g. group all test changes together");
                editor.move_cursor(tui_textarea::CursorMove::End);
                self.overlay = Overlay::Instruction(editor);
            }
            Action::Rerun => self.confirm(Confirm::Rerun),
            Action::Push => {
                if self.commits.is_empty() {
                    self.status = "Nothing to push yet.".into();
                } else {
                    self.confirm(Confirm::Push);
                }
            }
            Action::Help => self.overlay = Overlay::Help,
            Action::Quit => {
                if self.pending_count() > 0 && !self.commits.is_empty() {
                    self.confirm(Confirm::QuitWithPending);
                } else if !self.commits.is_empty() {
                    self.screen = Screen::Summary;
                } else {
                    self.should_quit = true;
                }
            }
        }
    }

    fn commit_all(&mut self) {
        let indices = self.pending_indices();
        self.apply_indices(indices);
    }

    fn start_push(&mut self) {
        self.busy = true;
        self.status = "Pushing...".into();
        self.effects.push(Effect::Send(Cmd::Push));
    }

    fn apply_indices(&mut self, indices: Vec<usize>) {
        if indices.is_empty() {
            return;
        }
        let items: Vec<ApplyItem> = indices
            .iter()
            .map(|&i| ApplyItem {
                index: i,
                change_ids: self.proposals[i].view.change_ids.clone(),
                message: self.proposals[i].view.message.clone(),
            })
            .collect();
        self.busy = true;
        self.apply_log.clear();
        self.overlay = Overlay::Applying;
        self.effects.push(Effect::Send(Cmd::Apply(items)));
    }

    fn regenerate(&mut self) {
        self.busy = true;
        self.screen = Screen::Loading;
        self.overlay = Overlay::None;
        self.status = "Regenerating proposals...".into();
        self.effects.push(Effect::Send(Cmd::Generate {
            instruction: self.instruction.clone(),
        }));
    }
}

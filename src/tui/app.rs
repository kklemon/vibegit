//! Interface state and input handling. Rendering lives in `ui.rs`.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::ListState;
use tui_textarea::TextArea;

use super::worker::{ApplyItem, Cmd, Evt, ExcludedView, ProposalView, Snapshot};
use crate::workflow::Leftover;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Loading,
    Proposals,
    Diff,
    Summary,
    Error,
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
    Push,
    Rerun,
    QuitWithPending,
}

pub enum Overlay {
    None,
    Unstage(Vec<String>),
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
    pub instruction: Option<String>,
    pub apply_log: Vec<String>,
    pub menu_index: usize,
    pub debug: bool,
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
    ) -> Self {
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
            instruction,
            apply_log: Vec::new(),
            menu_index: 0,
            debug,
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
            Evt::StagedChanges(paths) => {
                self.busy = false;
                self.overlay = Overlay::Unstage(paths);
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
                if failed {
                    self.status = "A commit failed; see the proposal for details. Remaining proposals were not applied.".into();
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
        match std::mem::replace(&mut self.overlay, Overlay::None) {
            Overlay::None => {}
            Overlay::Unstage(paths) => return self.on_key_unstage(key, paths),
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
                    self.overlay = Overlay::Confirm(Confirm::Push)
                }
                KeyCode::Char('b') if !self.busy && !self.proposals.is_empty() => {
                    self.screen = Screen::Proposals
                }
                _ => {}
            },
            Screen::Diff => self.on_key_diff(key),
            Screen::Proposals => self.on_key_proposals(key),
        }
    }

    fn on_key_unstage(&mut self, key: KeyEvent, paths: Vec<String>) {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                self.busy = true;
                self.status = "Unstaging...".into();
                self.effects.push(Effect::Send(Cmd::Unstage));
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc | KeyCode::Char('q') => {
                self.should_quit = true;
            }
            _ => self.overlay = Overlay::Unstage(paths),
        }
    }

    fn on_key_confirm(&mut self, key: KeyEvent, kind: Confirm) {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => match kind {
                Confirm::CommitAll(_) => self.commit_all(),
                Confirm::Push => {
                    self.busy = true;
                    self.status = "Pushing...".into();
                    self.effects.push(Effect::Send(Cmd::Push));
                }
                Confirm::Rerun => self.regenerate(),
                Confirm::QuitWithPending => self.screen = Screen::Summary,
            },
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
            KeyCode::Tab | KeyCode::Char('n') => {
                self.move_selection(1);
                self.diff_row = self.selected_row();
                self.diff_scroll = 0;
            }
            KeyCode::BackTab | KeyCode::Char('N') | KeyCode::Char('p') => {
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
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
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
            KeyCode::Tab if shift => self.move_selection(-1),
            KeyCode::Tab => self.move_selection(1),
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
            Action::CommitAll => {
                let pending = self.pending_count();
                if pending == 0 {
                    self.status = "Nothing pending to commit.".into();
                } else {
                    self.overlay = Overlay::Confirm(Confirm::CommitAll(pending));
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
            Action::Rerun => self.overlay = Overlay::Confirm(Confirm::Rerun),
            Action::Push => {
                if self.commits.is_empty() {
                    self.status = "Nothing to push yet.".into();
                } else {
                    self.overlay = Overlay::Confirm(Confirm::Push);
                }
            }
            Action::Help => self.overlay = Overlay::Help,
            Action::Quit => {
                if self.pending_count() > 0 && !self.commits.is_empty() {
                    self.overlay = Overlay::Confirm(Confirm::QuitWithPending);
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

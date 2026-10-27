//! Rendering of every screen and overlay.

use ratatui::prelude::*;
use ratatui::widgets::*;

use super::app::{Action, App, Button, Confirm, Overlay, ProposalStatus, Row, Screen};
use super::widgets::{
    centered_fixed, centered_rect, diff_lines, first_line, shorten_path, spinner_frame,
    status_symbol,
};

pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let buttons = app.buttons();
    let rows = button_rows(&buttons, area.width);
    let footer_height = 1 + rows.len().max(1) as u16;
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(footer_height),
    ])
    .areas(area);
    render_header(frame, app, header);
    match app.screen {
        Screen::Loading => render_loading(frame, app, body),
        Screen::Proposals => render_proposals(frame, app, body),
        Screen::Diff => render_diff(frame, app, body),
        Screen::Summary => render_summary(frame, app, body),
        Screen::Error => render_error(frame, app, body),
        Screen::Rules => render_rules(frame, app, body),
    }
    render_footer(frame, app, footer, &buttons, &rows);
    render_overlay(frame, app, area);
}

fn render_header(frame: &mut Frame, app: &App, area: Rect) {
    let repo = shorten_path(&app.repo_label, 40);
    let mut spans = vec![
        Span::styled(
            " VibeGit ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(repo, Style::default().add_modifier(Modifier::BOLD)),
    ];
    if !app.branch.is_empty() {
        spans.push(Span::raw(" on "));
        spans.push(Span::styled(
            app.branch.clone(),
            Style::default().fg(Color::Magenta),
        ));
    }
    spans.push(Span::raw("  model "));
    spans.push(Span::styled(
        app.model_label.clone(),
        Style::default().fg(Color::Blue),
    ));
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
    if let Some(latest) = &app.update_notice {
        let notice = Paragraph::new(Line::from(Span::styled(
            format!("update available: {latest} "),
            Style::default().fg(Color::Yellow),
        )))
        .alignment(Alignment::Right);
        frame.render_widget(notice, area);
    }
}

fn button_width(b: &Button) -> u16 {
    (b.key_label.chars().count() + b.label.chars().count() + 3) as u16
}

/// Greedy line wrapping of the buttons for the available width.
fn button_rows(buttons: &[Button], width: u16) -> Vec<Vec<usize>> {
    let mut rows: Vec<Vec<usize>> = Vec::new();
    let mut current: Vec<usize> = Vec::new();
    let mut used: u16 = 0;
    for (i, b) in buttons.iter().enumerate() {
        let w = button_width(b);
        let needed = if current.is_empty() { w } else { used + 1 + w };
        if !current.is_empty() && needed > width {
            rows.push(std::mem::take(&mut current));
            used = 0;
        }
        used = if current.is_empty() { w } else { used + 1 + w };
        current.push(i);
    }
    if !current.is_empty() {
        rows.push(current);
    }
    rows
}

fn button_spans(b: &Button, focused: bool) -> Vec<Span<'static>> {
    let key = format!(" {} ", b.key_label);
    let label = format!("{} ", b.label);
    let (key_style, label_style) = if !b.enabled {
        (
            Style::default().fg(Color::DarkGray),
            Style::default().fg(Color::DarkGray),
        )
    } else if focused {
        (
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(Color::Black).bg(Color::Cyan),
        )
    } else {
        (
            Style::default()
                .fg(Color::Yellow)
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(Color::White).bg(Color::DarkGray),
        )
    };
    vec![
        Span::styled(key, key_style),
        Span::styled(label, label_style),
    ]
}

fn render_footer(
    frame: &mut Frame,
    app: &mut App,
    area: Rect,
    buttons: &[Button],
    rows: &[Vec<usize>],
) {
    let [status_area, bar_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(area);
    let mut status = Vec::new();
    if app.busy {
        status.push(Span::styled(
            format!("{} ", spinner_frame(app.spinner)),
            Style::default().fg(Color::Cyan),
        ));
    }
    status.push(Span::raw(app.status.clone()));
    frame.render_widget(Paragraph::new(Line::from(status)), status_area);

    app.button_rects.clear();
    if rows.is_empty() {
        let text = if matches!(app.overlay, Overlay::Applying) {
            "committing, please wait"
        } else {
            ""
        };
        frame.render_widget(
            Paragraph::new(Span::styled(text, Style::default().fg(Color::DarkGray))),
            bar_area,
        );
        return;
    }
    for (r, row) in rows.iter().enumerate() {
        let y = bar_area.y + r as u16;
        if y >= bar_area.y + bar_area.height {
            break;
        }
        let mut spans = Vec::new();
        let mut x = bar_area.x;
        for (n, &i) in row.iter().enumerate() {
            if n > 0 {
                spans.push(Span::raw(" "));
                x += 1;
            }
            let b = &buttons[i];
            let w = button_width(b);
            spans.extend(button_spans(b, app.focus == Some(i)));
            app.button_rects.push((Rect::new(x, y, w, 1), i));
            x += w;
        }
        frame.render_widget(
            Paragraph::new(Line::from(spans)),
            Rect::new(bar_area.x, y, bar_area.width, 1),
        );
    }
}

fn render_loading(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::bordered()
        .title(" Working ")
        .border_style(Style::default().fg(Color::Cyan));
    let inner = centered_fixed(70, 7 + app.warnings.len() as u16, area);
    frame.render_widget(Clear, inner);
    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(
                format!("  {} ", spinner_frame(app.spinner)),
                Style::default().fg(Color::Cyan),
            ),
            Span::raw(app.status.clone()),
        ]),
        Line::from(""),
    ];
    if let Some(i) = &app.instruction {
        lines.push(Line::from(Span::styled(
            format!("  instruction: {i}"),
            Style::default().fg(Color::DarkGray),
        )));
    }
    for w in &app.warnings {
        lines.push(Line::from(Span::styled(
            format!("  warning: {w}"),
            Style::default().fg(Color::Yellow),
        )));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        inner,
    );
}

fn render_proposals(frame: &mut Frame, app: &mut App, area: Rect) {
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(42), Constraint::Percentage(58)]).areas(area);
    app.list_rect = left;
    app.detail_rect = right;

    let rows = app.rows();
    let items: Vec<ListItem> = rows
        .iter()
        .map(|row| match row {
            Row::Proposal(i) => {
                let item = &app.proposals[*i];
                let (symbol, color) = status_symbol(&item.status);
                let dim = matches!(
                    item.status,
                    ProposalStatus::Committed { .. } | ProposalStatus::Skipped
                );
                let mut style = Style::default();
                if dim {
                    style = style.fg(Color::DarkGray);
                }
                let subject = first_line(&item.view.message).to_string();
                ListItem::new(Line::from(vec![
                    Span::styled(format!("{symbol} "), Style::default().fg(color)),
                    Span::styled(
                        format!("{:>2} ", i + 1),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(subject, style),
                    Span::styled(
                        if item.edited { " *" } else { "" },
                        Style::default().fg(Color::Yellow),
                    ),
                ]))
            }
            Row::Excluded(i) => {
                let group = &app.excluded[*i];
                let label = group
                    .files
                    .first()
                    .map(|f| f.path.clone())
                    .unwrap_or_else(|| "excluded changes".into());
                ListItem::new(Line::from(vec![
                    Span::styled("! ", Style::default().fg(Color::Yellow)),
                    Span::styled(
                        format!("excluded: {label}"),
                        Style::default().fg(Color::Yellow),
                    ),
                ]))
            }
        })
        .collect();
    let title = format!(
        " Proposals ({} pending, {} committed) ",
        app.pending_count(),
        app.commits.len()
    );
    let list = List::new(items)
        .block(Block::bordered().title(title))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD))
        .highlight_symbol("▶ ");
    frame.render_stateful_widget(list, left, &mut app.list_state);

    let (title, lines) = match app.selected_row() {
        Some(Row::Proposal(i)) => proposal_detail(app, i),
        Some(Row::Excluded(i)) => excluded_detail(app, i),
        None => (" Details ".to_string(), vec![Line::from("No proposals.")]),
    };
    let paragraph = Paragraph::new(lines)
        .block(Block::bordered().title(title))
        .wrap(Wrap { trim: false })
        .scroll((app.detail_scroll, 0));
    frame.render_widget(paragraph, right);
}

fn proposal_detail(app: &App, i: usize) -> (String, Vec<Line<'static>>) {
    let item = &app.proposals[i];
    let title = format!(" Proposal {} of {} ", i + 1, app.proposals.len());
    let heading = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let mut lines: Vec<Line<'static>> = Vec::new();
    match &item.status {
        ProposalStatus::Committed { sha } => lines.push(Line::from(Span::styled(
            format!("✔ committed as {sha}"),
            Style::default().fg(Color::Green),
        ))),
        ProposalStatus::Skipped => lines.push(Line::from(Span::styled(
            "⊘ skipped (press s to unskip)",
            Style::default().fg(Color::DarkGray),
        ))),
        ProposalStatus::Failed(err) => {
            lines.push(Line::from(Span::styled(
                "✖ commit failed",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            )));
            for l in err.lines() {
                lines.push(Line::from(Span::styled(
                    l.to_string(),
                    Style::default().fg(Color::Red),
                )));
            }
        }
        ProposalStatus::Pending => {}
    }
    if !lines.is_empty() {
        lines.push(Line::from(""));
    }
    lines.push(Line::from(Span::styled("Message", heading)));
    for l in item.view.message.lines() {
        lines.push(Line::from(format!("  {l}")));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Explanation", heading)));
    lines.push(Line::from(format!("  {}", item.view.explanation)));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("Files ({})", item.view.files.len()),
        heading,
    )));
    lines.extend(file_lines(&item.view.files));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("Change IDs: {:?}", item.view.change_ids),
        Style::default().fg(Color::DarkGray),
    )));
    (title, lines)
}

fn excluded_detail(app: &App, i: usize) -> (String, Vec<Line<'static>>) {
    let group = &app.excluded[i];
    let heading = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let mut lines = vec![
        Line::from(Span::styled("Excluded from the proposals", heading)),
        Line::from("  The model considers these changes not ready to commit."),
        Line::from(""),
        Line::from(Span::styled("Reason", heading)),
        Line::from(format!("  {}", group.explanation)),
        Line::from(""),
        Line::from(Span::styled(
            format!("Files ({})", group.files.len()),
            heading,
        )),
    ];
    lines.extend(file_lines(&group.files));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Use i to regenerate with an instruction if they should be committed.",
        Style::default().fg(Color::DarkGray),
    )));
    (format!(" Excluded group {} ", i + 1), lines)
}

fn file_lines(files: &[crate::changes::FileSummary]) -> Vec<Line<'static>> {
    files
        .iter()
        .map(|f| {
            let color = match f.letter() {
                'A' => Color::Green,
                'D' => Color::Red,
                'T' | 'S' => Color::Magenta,
                _ => Color::Yellow,
            };
            let mut spans = vec![
                Span::raw("  "),
                Span::styled(f.letter().to_string(), Style::default().fg(color)),
                Span::raw(format!(" {} ", f.path)),
            ];
            if f.binary {
                spans.push(Span::styled(
                    "(binary)",
                    Style::default().fg(Color::DarkGray),
                ));
            } else {
                spans.push(Span::styled(
                    format!("+{}", f.added),
                    Style::default().fg(Color::Green),
                ));
                spans.push(Span::raw(" "));
                spans.push(Span::styled(
                    format!("-{}", f.removed),
                    Style::default().fg(Color::Red),
                ));
            }
            Line::from(spans)
        })
        .collect()
}

fn render_diff(frame: &mut Frame, app: &mut App, area: Rect) {
    app.diff_rect = area;
    let (text, subject) = match app.diff_text() {
        Some((t, s)) => (t.to_string(), s),
        None => (String::new(), String::new()),
    };
    let lines = diff_lines(&text);
    let total = lines.len() as u16;
    let visible = area.height.saturating_sub(2);
    let max_scroll = total.saturating_sub(visible);
    if app.diff_scroll > max_scroll {
        app.diff_scroll = max_scroll;
    }
    let paragraph = Paragraph::new(lines)
        .block(Block::bordered().title(format!(" Diff: {subject} ")))
        .scroll((app.diff_scroll, 0));
    frame.render_widget(paragraph, area);
    let mut state = ScrollbarState::new(total as usize).position(app.diff_scroll as usize);
    frame.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight),
        area.inner(Margin {
            vertical: 1,
            horizontal: 0,
        }),
        &mut state,
    );
}

fn render_summary(frame: &mut Frame, app: &App, area: Rect) {
    let heading = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let mut lines: Vec<Line<'static>> = vec![Line::from("")];
    if app.commits.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No commits were created.",
            heading,
        )));
        lines.push(Line::from(format!("  {}", app.status)));
    } else {
        lines.push(Line::from(Span::styled(
            format!("  VibeGit created {} commit(s)", app.commits.len()),
            heading,
        )));
        for c in &app.commits {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("    {} ", c.sha),
                    Style::default().fg(Color::Yellow),
                ),
                Span::raw(c.subject.clone()),
            ]));
        }
    }
    lines.push(Line::from(""));
    if let Some(left) = &app.leftover {
        if left.is_clean() {
            lines.push(Line::from(Span::styled(
                "  The working tree is clean. 😎",
                Style::default().fg(Color::Green),
            )));
        } else {
            lines.push(Line::from(Span::styled(
                format!(
                    "  Remaining: {} changed, {} untracked, {} staged.",
                    left.changed, left.untracked, left.staged
                ),
                Style::default().fg(Color::Yellow),
            )));
        }
    }
    let skipped = app
        .proposals
        .iter()
        .filter(|p| p.status == ProposalStatus::Skipped)
        .count();
    if skipped > 0 {
        lines.push(Line::from(format!("  {skipped} proposal(s) were skipped.")));
    }
    if let Some(p) = &app.push_status {
        lines.push(Line::from(""));
        lines.push(Line::from(format!("  Push: {p}")));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::bordered().title(" Summary "))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_error(frame: &mut Frame, app: &App, area: Rect) {
    let message = app.error.clone().unwrap_or_else(|| "unknown error".into());
    let lines: Vec<Line> = std::iter::once(Line::from(""))
        .chain(message.lines().map(|l| Line::from(format!("  {l}"))))
        .collect();
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::bordered()
                    .title(" Error ")
                    .border_style(Style::default().fg(Color::Red)),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_overlay(frame: &mut Frame, app: &mut App, area: Rect) {
    match &mut app.overlay {
        Overlay::None => {}
        Overlay::Unstage(paths) => {
            let mut lines = vec![
                Line::from(
                    "VibeGit needs an index that equals HEAD, but these paths have staged changes:",
                ),
                Line::from(""),
            ];
            for p in paths.iter().take(12) {
                lines.push(Line::from(Span::styled(
                    format!("  {p}"),
                    Style::default().fg(Color::Yellow),
                )));
            }
            if paths.len() > 12 {
                lines.push(Line::from(format!("  … and {} more", paths.len() - 12)));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(
                "Unstage them now? Your working tree is not touched.",
            ));
            lines.push(Line::from(
                "  [y] yes   [Y] yes, don't ask again   [n] quit",
            ));
            modal(
                frame,
                area,
                " Staged changes ",
                lines,
                Color::Yellow,
                80,
                (paths.len().min(12) + 7) as u16,
            );
        }
        Overlay::Confirm(kind) => {
            let question = match kind {
                Confirm::CommitAll(n) => {
                    format!("Commit all {n} pending proposal(s) in list order?")
                }
                Confirm::CommitAllAndPush(n) => {
                    format!("Commit all {n} pending proposal(s) in list order, then push?")
                }
                Confirm::Push => "Push the new commits to the remote?".to_string(),
                Confirm::Rerun => {
                    "Discard the current proposals and generate new ones?".to_string()
                }
                Confirm::QuitWithPending => {
                    "Some proposals are still pending. Quit anyway?".to_string()
                }
            };
            let lines = vec![
                Line::from(""),
                Line::from(format!("  {question}")),
                Line::from(""),
                Line::from("  [y] yes   [Y] yes, don't ask again   [n] no"),
            ];
            modal(frame, area, " Confirm ", lines, Color::Cyan, 76, 6);
        }
        Overlay::EditMessage { editor, .. } => {
            let rect = centered_rect(80, 60, area);
            frame.render_widget(Clear, rect);
            editor.set_block(
                Block::bordered()
                    .title(" Edit commit message  (Ctrl+S save · Esc cancel) ")
                    .border_style(Style::default().fg(Color::Cyan)),
            );
            frame.render_widget(&*editor, rect);
        }
        Overlay::Instruction(editor) => {
            let rect = centered_fixed(90.min(area.width), 3, area);
            frame.render_widget(Clear, rect);
            editor.set_block(
                Block::bordered()
                    .title(" Instruction for regenerating the proposals  (Enter regenerate · Esc cancel) ")
                    .border_style(Style::default().fg(Color::Cyan)),
            );
            frame.render_widget(&*editor, rect);
        }
        Overlay::Menu => {
            let items: Vec<ListItem> = Action::MENU
                .iter()
                .map(|a| {
                    ListItem::new(Line::from(vec![
                        Span::styled(format!("{:<10}", a.key()), Style::default().fg(Color::Cyan)),
                        Span::raw(a.label()),
                    ]))
                })
                .collect();
            let rect = centered_fixed(56, Action::MENU.len() as u16 + 2, area);
            frame.render_widget(Clear, rect);
            let mut state = ListState::default().with_selected(Some(app.menu_index));
            let list = List::new(items)
                .block(
                    Block::bordered()
                        .title(" Actions ")
                        .border_style(Style::default().fg(Color::Cyan)),
                )
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
            frame.render_stateful_widget(list, rect, &mut state);
        }
        Overlay::Help => {
            let rows = [
                ("↑ ↓ j k", "select proposal"),
                ("Enter / d", "show the diff of the selection"),
                ("c", "stage and commit the selected proposal"),
                ("C / a", "commit all pending proposals"),
                ("P", "commit all pending proposals, then push"),
                ("e", "edit the commit message in place"),
                ("E", "edit the commit message in $EDITOR"),
                ("s", "skip or unskip a proposal"),
                ("J / K, Alt+↓ ↑", "move the proposal down / up"),
                ("[ ]", "scroll the detail pane"),
                ("i", "regenerate with a one-off instruction"),
                ("r", "regenerate proposals"),
                ("p", "push the created commits"),
                ("m / Space", "open the action menu"),
                (
                    "Tab / Shift+Tab",
                    "focus the button bar; ← → choose, Enter runs",
                ),
                ("mouse", "click buttons or proposals; wheel scrolls"),
                ("q / Esc", "quit (summary first when commits exist)"),
                ("Ctrl+C", "abort immediately"),
            ];
            let lines: Vec<Line> = rows
                .iter()
                .map(|(k, v)| {
                    Line::from(vec![
                        Span::styled(format!("  {k:<16}"), Style::default().fg(Color::Cyan)),
                        Span::raw(v.to_string()),
                    ])
                })
                .collect();
            modal(
                frame,
                area,
                " Keys ",
                lines,
                Color::Cyan,
                64,
                rows.len() as u16 + 2,
            );
        }
        Overlay::DeriveRules(commits) => {
            let lines = vec![
                Line::from(""),
                Line::from("  No .vibegitrules found in this repository."),
                Line::from(""),
                Line::from(format!(
                    "  VibeGit can analyze the last {commits} commit(s) and derive this project's"
                )),
                Line::from("  commit conventions (message style, granularity, grouping) into a"),
                Line::from("  .vibegitrules file, which you review before it is written."),
                Line::from(""),
                Line::from("  [y] yes, derive rules   [n] not now   [N] never ask"),
            ];
            modal(frame, area, " Project rules ", lines, Color::Cyan, 78, 10);
        }
        Overlay::Applying => {
            let mut lines = vec![Line::from(vec![
                Span::styled(
                    format!("  {} ", spinner_frame(app.spinner)),
                    Style::default().fg(Color::Cyan),
                ),
                Span::raw(app.status.clone()),
            ])];
            lines.push(Line::from(""));
            let start = app.apply_log.len().saturating_sub(8);
            for l in &app.apply_log[start..] {
                let style = if l.starts_with("failed") {
                    Style::default().fg(Color::Red)
                } else {
                    Style::default().fg(Color::Green)
                };
                lines.push(Line::from(Span::styled(format!("  {l}"), style)));
            }
            modal(frame, area, " Committing ", lines, Color::Cyan, 80, 12);
        }
    }
}

fn render_rules(frame: &mut Frame, app: &mut App, area: Rect) {
    let target = shorten_path(&app.rules_target, 60);
    let Some(review) = app.rules.as_mut() else {
        return;
    };
    let obs_height = (review.observations.len() as u16 + 2)
        .max(3)
        .min(area.height / 3);
    let [obs_area, rules_area] =
        Layout::vertical([Constraint::Length(obs_height), Constraint::Min(3)]).areas(area);
    let obs_lines: Vec<Line> = review
        .observations
        .iter()
        .map(|o| Line::from(format!("• {o}")))
        .collect();
    frame.render_widget(
        Paragraph::new(obs_lines)
            .block(Block::bordered().title(" Observed conventions "))
            .wrap(Wrap { trim: false }),
        obs_area,
    );
    if let Some(editor) = review.editor.as_mut() {
        editor.set_block(
            Block::bordered()
                .title(" Edit .vibegitrules  (Ctrl+S apply · Esc cancel) ")
                .border_style(Style::default().fg(Color::Cyan)),
        );
        frame.render_widget(&*editor, rules_area);
        return;
    }
    let lines: Vec<Line> = review
        .rules
        .lines()
        .map(|l| Line::from(l.to_string()))
        .collect();
    let total = lines.len() as u16;
    let visible = rules_area.height.saturating_sub(2);
    let max_scroll = total.saturating_sub(visible);
    if review.scroll > max_scroll {
        review.scroll = max_scroll;
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::bordered().title(format!(" Proposed {target} ")))
            .wrap(Wrap { trim: false })
            .scroll((review.scroll, 0)),
        rules_area,
    );
}

fn modal(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    lines: Vec<Line>,
    color: Color,
    width: u16,
    height: u16,
) {
    let rect = centered_fixed(width.min(area.width), height.min(area.height), area);
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::bordered()
                    .title(title.to_string())
                    .border_style(Style::default().fg(color)),
            )
            .wrap(Wrap { trim: false }),
        rect,
    );
}

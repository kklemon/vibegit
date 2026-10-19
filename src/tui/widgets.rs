//! Small rendering helpers shared by the screens.

use ratatui::prelude::*;

use super::app::ProposalStatus;

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub fn spinner_frame(tick: usize) -> &'static str {
    SPINNER[tick % SPINNER.len()]
}

/// A rectangle of the given percentage size centered in `area`.
pub fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let [_, middle, _] = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ])
    .areas(area);
    let [_, center, _] = Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .areas(middle);
    center
}

/// A rectangle of fixed size (clamped to `area`) centered in `area`.
pub fn centered_fixed(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    let x = area.x + (area.width - width) / 2;
    let y = area.y + (area.height - height) / 2;
    Rect::new(x, y, width, height)
}

pub fn status_symbol(status: &ProposalStatus) -> (&'static str, Color) {
    match status {
        ProposalStatus::Pending => ("○", Color::White),
        ProposalStatus::Committed { .. } => ("✔", Color::Green),
        ProposalStatus::Skipped => ("⊘", Color::DarkGray),
        ProposalStatus::Failed(_) => ("✖", Color::Red),
    }
}

/// Colours a unified diff line by line.
pub fn diff_lines(text: &str) -> Vec<Line<'static>> {
    text.lines()
        .map(|line| {
            let owned = line.to_string();
            let style = if line.starts_with("+++") || line.starts_with("---") {
                Style::default().add_modifier(Modifier::BOLD)
            } else if line.starts_with("diff --git") {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else if line.starts_with("@@") {
                Style::default().fg(Color::Cyan)
            } else if line.starts_with('+') {
                Style::default().fg(Color::Green)
            } else if line.starts_with('-') {
                Style::default().fg(Color::Red)
            } else if line.starts_with("index ")
                || line.starts_with("new file mode")
                || line.starts_with("deleted file mode")
                || line.starts_with("old mode")
                || line.starts_with("new mode")
                || line.starts_with("Binary files")
                || line.starts_with("GIT binary patch")
            {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default()
            };
            Line::from(Span::styled(owned, style))
        })
        .collect()
}

/// `key label` pairs rendered as a hint bar.
pub fn hint_line(pairs: &[(&str, &str)]) -> Line<'static> {
    let mut spans = Vec::new();
    for (i, (key, label)) in pairs.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(
            key.to_string(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(format!(" {label}")));
    }
    Line::from(spans)
}

/// Keeps the tail of a path so it fits in `max` characters.
pub fn shorten_path(path: &str, max: usize) -> String {
    let count = path.chars().count();
    if count <= max {
        return path.to_string();
    }
    let keep = max.saturating_sub(1);
    let tail: String = path.chars().skip(count - keep).collect();
    format!("…{tail}")
}

pub fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or("")
}

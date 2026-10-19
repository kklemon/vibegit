//! Render smoke tests: every screen and overlay must draw without panicking,
//! including on very small terminals.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::Terminal;

use super::app::{App, Overlay, Screen};
use super::ui;
use super::worker::{Evt, ExcludedView, ProposalView, Snapshot};
use crate::changes::FileSummary;
use crate::git::FileKind;
use crate::workflow::Leftover;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn snapshot() -> Snapshot {
    let file = FileSummary {
        path: "src/lib.rs".into(),
        kind: FileKind::Modified,
        binary: false,
        added: 3,
        removed: 1,
    };
    Snapshot {
        proposals: (0..3)
            .map(|i| ProposalView {
                change_ids: vec![i + 1],
                message: format!("commit number {i}\n\nwith a body"),
                explanation: "because".into(),
                files: vec![file.clone()],
                diff: "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n".into(),
            })
            .collect(),
        excluded: vec![ExcludedView {
            change_ids: vec![4],
            explanation: "debug prints".into(),
            files: vec![file],
            diff: String::new(),
        }],
        branch: "main".into(),
        units: 4,
        files: 1,
        warnings: vec!["submodule sub is dirty".into()],
    }
}

fn draw(app: &mut App, width: u16, height: u16) {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| ui::render(f, app)).unwrap();
}

#[test]
fn renders_every_state_at_several_sizes() {
    let mut app = App::new(
        "/tmp/repo".into(),
        "google:gemini".into(),
        Some("x".into()),
        false,
        Confirmations::default(),
    );
    for (w, h) in [(120, 40), (80, 24), (30, 8), (5, 3)] {
        draw(&mut app, w, h); // loading
        app.on_event(Evt::StagedChanges(vec!["a.txt".into(); 20]));
        draw(&mut app, w, h);
        app.on_key(key(KeyCode::Char('y')));
        app.on_event(Evt::Proposals(snapshot()));
        draw(&mut app, w, h);
        for code in [
            KeyCode::Down,
            KeyCode::Down,
            KeyCode::Down,
            KeyCode::Down,
            KeyCode::Up,
        ] {
            app.on_key(key(code));
            draw(&mut app, w, h);
        }
        app.on_key(key(KeyCode::Char('d')));
        assert_eq!(app.screen, Screen::Diff);
        app.on_key(key(KeyCode::Char('G')));
        draw(&mut app, w, h);
        app.on_key(key(KeyCode::Char('q')));
        for code in [
            KeyCode::Char('e'),
            KeyCode::Esc,
            KeyCode::Char('i'),
            KeyCode::Esc,
            KeyCode::Char('m'),
            KeyCode::Esc,
            KeyCode::Char('?'),
            KeyCode::Esc,
            KeyCode::Char('C'),
        ] {
            app.on_key(key(code));
            draw(&mut app, w, h);
        }
        assert!(matches!(app.overlay, Overlay::Confirm(_)));
        app.on_key(key(KeyCode::Char('n')));
        app.on_key(key(KeyCode::Char('s')));
        draw(&mut app, w, h);
        app.on_key(key(KeyCode::Char('s')));
        app.on_key(key(KeyCode::Char('J')));
        app.on_key(key(KeyCode::Char('K')));
        app.on_key(key(KeyCode::Char('c')));
        assert!(matches!(app.overlay, Overlay::Applying));
        draw(&mut app, w, h);
        app.on_event(Evt::Committed {
            index: 0,
            sha: "abc1234".into(),
            subject: "commit number 0".into(),
        });
        app.on_event(Evt::ApplyFinished(Leftover::default()));
        draw(&mut app, w, h);
        app.on_key(key(KeyCode::Char('q')));
        assert!(
            matches!(app.overlay, Overlay::Confirm(_)),
            "pending proposals need confirmation"
        );
        app.on_key(key(KeyCode::Char('y')));
        assert_eq!(app.screen, Screen::Summary);
        draw(&mut app, w, h);
        app.on_event(Evt::PushResult(Err("no remote".into())));
        draw(&mut app, w, h);
        app.on_event(Evt::Fatal("boom".into()));
        draw(&mut app, w, h);
        app = App::new(
            "/tmp/repo".into(),
            "google:gemini".into(),
            None,
            false,
            Confirmations::default(),
        );
    }
}

#[test]
fn editing_a_message_marks_it_edited() {
    let mut app = App::new(
        "r".into(),
        "m".into(),
        None,
        false,
        Confirmations::default(),
    );
    app.on_event(Evt::Proposals(snapshot()));
    app.on_key(key(KeyCode::Char('e')));
    assert!(matches!(app.overlay, Overlay::EditMessage { .. }));
    app.on_key(key(KeyCode::Char('!')));
    app.on_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    assert!(app.proposals[0].edited);
    assert!(app.proposals[0].view.message.ends_with("with a body!"));
    draw(&mut app, 80, 24);
}

#[test]
fn capital_y_disables_a_confirmation_persistently() {
    let mut app = App::new(
        "r".into(),
        "m".into(),
        None,
        false,
        Confirmations::default(),
    );
    app.on_event(Evt::Proposals(snapshot()));
    app.on_key(key(KeyCode::Char('C')));
    assert!(matches!(
        app.overlay,
        Overlay::Confirm(Confirm::CommitAll(3))
    ));
    app.on_key(KeyEvent::new(KeyCode::Char('Y'), KeyModifiers::SHIFT));
    assert!(!app.confirmations.commit_all);
    assert!(matches!(app.overlay, Overlay::Applying));
    let effects = app.take_effects();
    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::SaveConfirmations(c) if !c.commit_all && c.push)));
    // The next commit-all goes straight through.
    app.on_event(Evt::ApplyFailed {
        index: 0,
        error: "hook".into(),
    });
    app.on_event(Evt::ApplyFinished(Leftover::default()));
    app.on_key(key(KeyCode::Char('C')));
    assert!(matches!(app.overlay, Overlay::Applying));
    draw(&mut app, 80, 24);
}

#[test]
fn disabled_confirmations_skip_the_dialog() {
    let confirmations = Confirmations {
        unstage: false,
        push: false,
        ..Confirmations::default()
    };
    let mut app = App::new("r".into(), "m".into(), None, false, confirmations);
    app.on_event(Evt::StagedChanges(vec!["a.txt".into()]));
    assert!(matches!(app.overlay, Overlay::None));
    assert!(app.busy, "unstaging started without a prompt");
    app.on_event(Evt::Proposals(snapshot()));
    app.on_event(Evt::Committed {
        index: 0,
        sha: "abc".into(),
        subject: "s".into(),
    });
    app.on_key(key(KeyCode::Char('p')));
    assert!(matches!(app.overlay, Overlay::None));
    assert!(app.status.starts_with("Pushing"));
    draw(&mut app, 80, 24);
}

fn click(column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

#[test]
fn button_bar_focus_navigation_and_activation() {
    let mut app = App::new(
        "r".into(),
        "m".into(),
        None,
        false,
        Confirmations::default(),
    );
    app.on_event(Evt::Proposals(snapshot()));
    assert_eq!(app.focus, None);
    app.on_key(key(KeyCode::Tab));
    assert_eq!(app.focus, Some(0), "Tab focuses the first enabled button");
    for _ in 0..3 {
        app.on_key(key(KeyCode::Right));
    }
    assert_eq!(app.buttons()[app.focus.unwrap()].label, "Diff");
    app.on_key(key(KeyCode::Enter));
    assert_eq!(
        app.screen,
        Screen::Diff,
        "Enter activates the focused button"
    );
    app.on_key(key(KeyCode::Esc));
    assert_eq!(app.focus, None, "Esc releases the focus");
    app.on_key(key(KeyCode::Char('q')));
    assert_eq!(app.screen, Screen::Proposals);
    // Disabled buttons are skipped: on the excluded row, Commit is unavailable.
    app.on_key(key(KeyCode::End));
    app.on_key(key(KeyCode::Tab));
    assert_eq!(app.buttons()[app.focus.unwrap()].label, "Commit all");
    assert!(!app.buttons()[0].enabled);
    // Tab past the last button returns to the content.
    while app.focus.is_some() {
        app.on_key(key(KeyCode::Tab));
    }
    draw(&mut app, 80, 24);
    draw(&mut app, 20, 10);
}

#[test]
fn mouse_clicks_buttons_and_rows() {
    let mut app = App::new(
        "r".into(),
        "m".into(),
        None,
        false,
        Confirmations::default(),
    );
    app.on_event(Evt::Proposals(snapshot()));
    draw(&mut app, 120, 30);
    let (rect, _) = *app
        .button_rects
        .iter()
        .find(|(_, i)| app.buttons()[*i].label == "Commit all")
        .expect("commit-all button drawn");
    app.on_mouse(click(rect.x + 1, rect.y));
    assert!(matches!(
        app.overlay,
        Overlay::Confirm(Confirm::CommitAll(3))
    ));
    draw(&mut app, 120, 30);
    let (no_rect, _) = *app
        .button_rects
        .iter()
        .find(|(_, i)| app.buttons()[*i].label == "No")
        .expect("no button drawn");
    app.on_mouse(click(no_rect.x, no_rect.y));
    assert!(matches!(app.overlay, Overlay::None));
    // Clicking the third list row selects it; the wheel moves the selection.
    let list = app.list_rect;
    app.on_mouse(click(list.x + 3, list.y + 3));
    assert_eq!(app.selected, 2);
    app.on_mouse(MouseEvent {
        kind: MouseEventKind::ScrollUp,
        column: list.x + 2,
        row: list.y + 2,
        modifiers: KeyModifiers::NONE,
    });
    assert_eq!(app.selected, 1);
}

#[test]
fn button_bar_renders_and_wraps() {
    let mut app = App::new(
        "r".into(),
        "m".into(),
        None,
        false,
        Confirmations::default(),
    );
    app.on_event(Evt::Proposals(snapshot()));
    app.on_key(key(KeyCode::Tab));
    for (width, expected_rows) in [(120u16, 1usize), (60, 2)] {
        let mut terminal = Terminal::new(TestBackend::new(width, 30)).unwrap();
        terminal.draw(|f| ui::render(f, &mut app)).unwrap();
        let buf = terminal.backend().buffer();
        let row = |y: u16| -> String {
            (0..width)
                .map(|x| {
                    buf.cell((x, y))
                        .map(|c| c.symbol().to_string())
                        .unwrap_or_default()
                })
                .collect()
        };
        let bottom: Vec<String> = (27..30).map(row).collect();
        eprintln!("--- {width} columns ---\n{}", bottom.join("\n"));
        let joined = bottom.join("\n");
        assert!(joined.contains(" C Commit all ") && joined.contains(" q Quit "));
        let bar_rows = bottom
            .iter()
            .filter(|l| l.contains(" Commit ") || l.contains(" Quit "))
            .count();
        assert_eq!(bar_rows, expected_rows, "{joined}");
    }
}

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
        app = App::new("/tmp/repo".into(), "google:gemini".into(), None, false);
    }
}

#[test]
fn editing_a_message_marks_it_edited() {
    let mut app = App::new("r".into(), "m".into(), None, false);
    app.on_event(Evt::Proposals(snapshot()));
    app.on_key(key(KeyCode::Char('e')));
    assert!(matches!(app.overlay, Overlay::EditMessage { .. }));
    app.on_key(key(KeyCode::Char('!')));
    app.on_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    assert!(app.proposals[0].edited);
    assert!(app.proposals[0].view.message.ends_with("with a body!"));
    draw(&mut app, 80, 24);
}

use std::sync::{Arc, Mutex};

use pam_desktop_ui::animator::Timer;
use pam_desktop_ui::backend::{DrawCommand, KeyEvent, MAIN_WINDOW, Modifiers, PointerPhase};
use pam_desktop_ui::context::{HostCommand, event_kind};
use pam_desktop_ui::geometry::Rect;
use pam_desktop_ui::modules::terminal::{TerminalHub, TerminalInput};
use pam_desktop_ui::scene::Change;
use pam_desktop_ui::terminal::{KeyOutcome, Terminals};
use pam_desktop_ui::testing::Harness;
use pam_desktop_ui::text::{TextBlock, TextMetrics};
use pam_desktop_ui::wiremap::WireMap;
use pam_desktop_wire::{NodeKind, PropKey, PropValue};

#[derive(Default)]
struct Remote(Mutex<Vec<String>>);

impl TerminalInput for Remote {
    fn write(&self, bytes: &[u8]) -> bool {
        self.0
            .lock()
            .unwrap()
            .push(String::from_utf8_lossy(bytes).into_owned());
        true
    }
    fn resize(&self, cols: u16, rows: u16) -> bool {
        self.0.lock().unwrap().push(format!("{cols}x{rows}"));
        true
    }
}

/// An 8 × 16 monospace font with its baseline at 12.
fn measure(block: &TextBlock, _width: f32) -> TextMetrics {
    let characters: usize = block.runs.iter().map(|run| run.text.chars().count()).sum();
    TextMetrics {
        width: characters as f32 * 8.0,
        height: 16.0,
        first_baseline: 12.0,
        line_count: 1,
    }
}

fn drawings(harness: &mut Harness) -> Vec<Vec<DrawCommand>> {
    harness
        .commands
        .drain(..)
        .filter_map(|command| match command {
            HostCommand::SetDrawing { id: 2, commands } => Some(commands),
            _ => None,
        })
        .collect()
}

fn texts(commands: &[DrawCommand]) -> Vec<(String, f32, f32)> {
    commands
        .iter()
        .filter_map(|command| match command {
            DrawCommand::Text { text, baseline, .. } => {
                Some((text.clone(), baseline.x, baseline.y))
            }
            _ => None,
        })
        .collect()
}

fn key(name: &str, modifiers: Modifiers) -> KeyEvent {
    KeyEvent {
        window: MAIN_WINDOW,
        pressed: true,
        key: name.to_owned(),
        modifiers,
        repeat: false,
    }
}

#[test]
fn a_terminal_binds_sizes_paints_and_types_through_the_hub() {
    let hub: &'static TerminalHub = Box::leak(Box::new(TerminalHub::new()));
    let remote = Arc::new(Remote::default());
    let channel = hub
        .register("C", Arc::clone(&remote) as Arc<dyn TerminalInput>)
        .unwrap();
    channel.output(b"before ");
    let mut harness = Harness::with_tree(&[
        (
            1,
            0,
            NodeKind::Column,
            &[],
            Rect::new(0.0, 0.0, 800.0, 600.0),
        ),
        (
            2,
            1,
            NodeKind::Terminal,
            &[
                (PropKey::TerminalChannel, PropValue::String("C".into())),
                (PropKey::Padding, PropValue::Float(4.0)),
                (PropKey::OnTerminalResize, PropValue::Boolean(true)),
                (PropKey::OnTerminalTitle, PropValue::Boolean(true)),
            ],
            // 80 × 24 cells of 8 × 16 plus 4 of padding on each side.
            Rect::new(0.0, 0.0, 648.0, 392.0),
        ),
    ]);
    let notified = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&notified);
    let mut terminals = Terminals::new(hub, Arc::new(move |id| sink.lock().unwrap().push(id)));
    terminals.on_change(&mut harness.ctx(), Change::Created(2, NodeKind::Terminal));
    terminals.evaluate(&mut harness.ctx(), &mut measure);

    assert_eq!(terminals.grid(2), Some((80, 24)));
    assert_eq!(*remote.0.lock().unwrap(), vec!["80x24"]);
    assert_eq!(harness.event_kinds(), vec![event_kind::TERMINAL_RESIZE]);
    assert_eq!(
        harness.events[0].payload,
        WireMap::new()
            .with("cols", 80_usize)
            .with("rows", 24_usize)
            .encode()
    );
    // The backlog recorded before the terminal mounted is on screen.
    let first = drawings(&mut harness);
    assert_eq!(first.len(), 1);
    assert_eq!(texts(&first[0]), vec![("before".to_owned(), 4.0, 16.0)]);

    // Output arrives on another thread: one notification, one paint.
    harness.now_ms = 1000;
    channel.output(b"\x1b]0;remote title\x07hi");
    channel.output(b"!");
    assert_eq!(*notified.lock().unwrap(), vec![2]);
    terminals.on_output(&mut harness.ctx(), 2);
    terminals.evaluate(&mut harness.ctx(), &mut measure);
    assert_eq!(
        harness.event_kinds(),
        vec![event_kind::TERMINAL_RESIZE, event_kind::TERMINAL_TITLE]
    );
    let painted = drawings(&mut harness);
    assert_eq!(painted.len(), 1);
    assert_eq!(
        texts(&painted[0]),
        vec![("before hi!".to_owned(), 4.0, 16.0)]
    );

    // Within the same frame interval the next paint waits for the timer.
    channel.output(b"?");
    terminals.on_output(&mut harness.ctx(), 2);
    terminals.evaluate(&mut harness.ctx(), &mut measure);
    assert!(drawings(&mut harness).is_empty());
    assert!(harness.timers.is_pending(Timer::TerminalFrame(2)));
    harness.now_ms += 16;
    terminals.on_timer(&mut harness.ctx(), Timer::TerminalFrame(2));
    terminals.evaluate(&mut harness.ctx(), &mut measure);
    assert_eq!(drawings(&mut harness).len(), 1);

    // Nothing changed: nothing is painted.
    terminals.evaluate(&mut harness.ctx(), &mut measure);
    assert!(drawings(&mut harness).is_empty());

    // Keys of the focused terminal go to the remote end.
    terminals.on_focus(&mut harness.ctx(), 2, true);
    let none = Modifiers::default();
    assert_eq!(
        terminals.on_key(&mut harness.ctx(), Some(2), &key("ArrowUp", none)),
        KeyOutcome::Handled
    );
    assert_eq!(
        terminals.on_key(
            &mut harness.ctx(),
            Some(2),
            &key(
                "c",
                Modifiers {
                    control: true,
                    ..none
                }
            )
        ),
        KeyOutcome::Handled
    );
    assert_eq!(
        terminals.on_key(&mut harness.ctx(), None, &key("x", none)),
        KeyOutcome::Ignored
    );
    assert_eq!(remote.0.lock().unwrap()[1..], ["\x1b[A", "\x03"]);

    // Text the input method composed is written as UTF-8.
    terminals.on_text(&mut harness.ctx(), 2, "ção");
    assert_eq!(
        remote.0.lock().unwrap().last().map(String::as_str),
        Some("ção")
    );
    remote.0.lock().unwrap().pop();

    // A drag selects cells; the copy shortcut copies them.
    let mut down = harness.pointer(PointerPhase::Down, 4.0 + 7.0 * 8.0 + 1.0, 10.0);
    down.click_count = 1;
    assert!(terminals.on_pointer(&mut harness.ctx(), down));
    assert!(
        harness
            .commands
            .iter()
            .any(|command| matches!(command, HostCommand::Focus(Some(2))))
    );
    let drag = harness.pointer(PointerPhase::Move, 4.0 + 8.0 * 8.0 + 7.0, 10.0);
    assert!(terminals.on_pointer(&mut harness.ctx(), drag));
    let up = harness.pointer(PointerPhase::Up, 4.0 + 8.0 * 8.0 + 7.0, 10.0);
    assert!(terminals.on_pointer(&mut harness.ctx(), up));
    let copy = Modifiers {
        control: true,
        shift: true,
        ..none
    };
    assert_eq!(
        terminals.on_key(&mut harness.ctx(), Some(2), &key("C", copy)),
        KeyOutcome::Copy("hi".to_owned())
    );
    assert_eq!(
        terminals.on_key(&mut harness.ctx(), Some(2), &key("V", copy)),
        KeyOutcome::Paste {
            channel: "C".to_owned(),
            bracketed: false
        }
    );

    // The shell ends: the terminal reports it once and lets go of the hub.
    hub.unregister("C", Some(0));
    terminals.on_output(&mut harness.ctx(), 2);
    terminals.on_change(&mut harness.ctx(), Change::Removed(2, NodeKind::Terminal));
    assert!(!terminals.contains(2));
}

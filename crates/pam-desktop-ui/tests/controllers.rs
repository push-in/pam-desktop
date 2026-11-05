//! Controllers driven directly over a scene: what they emit to PHP and what
//! they ask the host to do, without a backend or a worker.

use pam_desktop_ui::animator::{Owner, Timer};
use pam_desktop_ui::backend::{MAIN_WINDOW, PointerPhase};
use pam_desktop_ui::context::{HostCommand, event_kind};
use pam_desktop_ui::dragdrop::DragDrop;
use pam_desktop_ui::geometry::{Affine, Point, Rect};
use pam_desktop_ui::gestures::{Gestures, Outcome};
use pam_desktop_ui::marquee::Marquees;
use pam_desktop_ui::observers::Observers;
use pam_desktop_ui::scene::LayerSlot;
use pam_desktop_ui::testing::Harness;
use pam_desktop_ui::text::{TextBlock, TextMetrics};
use pam_desktop_ui::toast::Toasts;
use pam_desktop_ui::windows::Windows;
use pam_desktop_ui::wiremap::WireMap;
use pam_desktop_wire::{NodeKind, PropKey, PropValue};

fn measure(block: &TextBlock, _width: f32) -> TextMetrics {
    let characters: usize = block.runs.iter().map(|run| run.text.chars().count()).sum();
    TextMetrics {
        width: characters as f32 * 7.0,
        height: 18.0,
        first_baseline: 14.0,
        line_count: 1,
    }
}

#[test]
fn a_draggable_node_starts_a_drag_past_the_threshold_and_a_target_receives_the_drop() {
    let mut harness = Harness::with_tree(&[
        (
            1,
            0,
            NodeKind::Column,
            &[],
            Rect::new(0.0, 0.0, 400.0, 300.0),
        ),
        (
            2,
            1,
            NodeKind::Pressable,
            &[
                (PropKey::Draggable, PropValue::Boolean(true)),
                (PropKey::DragData, PropValue::String("card-7".into())),
                (PropKey::OnDragStart, PropValue::Boolean(true)),
                (PropKey::OnDragEnd, PropValue::Boolean(true)),
            ],
            Rect::new(10.0, 10.0, 100.0, 40.0),
        ),
        (
            3,
            1,
            NodeKind::Pressable,
            &[
                (PropKey::DropEnabled, PropValue::Boolean(true)),
                (PropKey::OnDrop, PropValue::Boolean(true)),
            ],
            Rect::new(200.0, 10.0, 150.0, 200.0),
        ),
    ]);
    let mut drag_drop = DragDrop::default();
    let down = harness.pointer(PointerPhase::Down, 20.0, 20.0);
    let small = harness.pointer(PointerPhase::Move, 22.0, 21.0);
    let far = harness.pointer(PointerPhase::Move, 40.0, 30.0);
    assert!(!drag_drop.on_pointer(&mut harness.ctx(), down));
    assert!(
        !drag_drop.on_pointer(&mut harness.ctx(), small),
        "a small movement is still a press"
    );
    assert!(
        drag_drop.on_pointer(&mut harness.ctx(), far),
        "past the threshold the drag starts"
    );
    assert_eq!(harness.event_kinds(), vec![event_kind::DRAG_START]);
    assert_eq!(
        harness.commands,
        vec![HostCommand::BeginTextDrag {
            source: 2,
            text: "card-7".into()
        }]
    );

    drag_drop.on_text_dropped(&mut harness.ctx(), Point::new(250.0, 100.0), "card-7");
    drag_drop.on_drag_finished(&mut harness.ctx(), 2);
    assert_eq!(
        harness.event_kinds(),
        vec![
            event_kind::DRAG_START,
            event_kind::DROP,
            event_kind::DRAG_END
        ]
    );
    assert_eq!(
        (harness.events[1].node, harness.events[1].payload.clone()),
        (3, WireMap::new().with("data", "card-7").encode())
    );

    // Text dropped where nothing accepts it is ignored.
    drag_drop.on_text_dropped(&mut harness.ctx(), Point::new(20.0, 250.0), "elsewhere");
    assert_eq!(harness.events.len(), 3);
}

#[test]
fn observers_report_once_when_installed_and_then_only_changes() {
    let mut harness = Harness::with_tree(&[
        (
            1,
            0,
            NodeKind::Column,
            &[],
            Rect::new(0.0, 0.0, 400.0, 300.0),
        ),
        (
            2,
            1,
            NodeKind::View,
            &[
                (PropKey::OnIntersect, PropValue::Boolean(true)),
                (PropKey::OnResize, PropValue::Boolean(true)),
            ],
            Rect::new(0.0, 0.0, 100.0, 50.0),
        ),
    ]);
    // The first flush resolves layers, which visibility depends on.
    let mut backend = pam_desktop_ui::testing::RecordingBackend::new();
    harness.scene.flush(&mut backend, MAIN_WINDOW);
    let mut observers = Observers::default();
    harness.settle(|ctx, change| observers.on_change(ctx, change));
    observers.evaluate(&mut harness.ctx());
    assert_eq!(
        harness.event_kinds(),
        vec![event_kind::INTERSECT, event_kind::RESIZE]
    );
    assert_eq!(harness.events[0].payload, b"1");
    assert_eq!(
        harness.events[1].payload,
        WireMap::new()
            .with("width", 100.0_f32)
            .with("height", 50.0_f32)
            .encode()
    );

    // Nothing changed: nothing is reported again.
    observers.invalidate();
    observers.evaluate(&mut harness.ctx());
    assert_eq!(harness.events.len(), 2);
}

#[test]
fn a_pan_detector_recognises_after_its_distance_and_reports_begin_update_end() {
    let handlers: &[(PropKey, PropValue)] = &[
        (PropKey::GestureType, PropValue::Integer(2)),
        (PropKey::GestureMinDistance, PropValue::Float(8.0)),
        (PropKey::OnGestureBegin, PropValue::Boolean(true)),
        (PropKey::OnGestureUpdate, PropValue::Boolean(true)),
        (PropKey::OnGestureEnd, PropValue::Boolean(true)),
    ];
    let mut harness = Harness::with_tree(&[
        (
            1,
            0,
            NodeKind::Column,
            &[],
            Rect::new(0.0, 0.0, 400.0, 300.0),
        ),
        (
            2,
            1,
            NodeKind::Pressable,
            handlers,
            Rect::new(0.0, 0.0, 400.0, 100.0),
        ),
        (3, 2, NodeKind::View, &[], Rect::new(0.0, 0.0, 400.0, 100.0)),
    ]);
    let mut backend = pam_desktop_ui::testing::RecordingBackend::new();
    harness.scene.flush(&mut backend, MAIN_WINDOW);
    let mut gestures = Gestures::default();
    harness.settle(|ctx, change| gestures.on_change(ctx, change));
    gestures.after_commit(&mut harness.ctx());

    let down = harness.pointer(PointerPhase::Down, 50.0, 50.0);
    assert_eq!(
        gestures.on_pointer(&mut harness.ctx(), down),
        Outcome::Ignored
    );
    harness.now_ms = 16;
    let near = harness.pointer(PointerPhase::Move, 54.0, 50.0);
    assert_eq!(
        gestures.on_pointer(&mut harness.ctx(), near),
        Outcome::Ignored,
        "below the minimum distance"
    );
    harness.now_ms = 32;
    let far = harness.pointer(PointerPhase::Move, 70.0, 52.0);
    assert_eq!(
        gestures.on_pointer(&mut harness.ctx(), far),
        Outcome::Recognised
    );
    harness.now_ms = 48;
    let further = harness.pointer(PointerPhase::Move, 90.0, 52.0);
    assert_eq!(
        gestures.on_pointer(&mut harness.ctx(), further),
        Outcome::Consumed
    );
    gestures.flush(&mut harness.ctx());
    harness.now_ms = 64;
    let up = harness.pointer(PointerPhase::Up, 90.0, 52.0);
    assert_eq!(
        gestures.on_pointer(&mut harness.ctx(), up),
        Outcome::Consumed
    );
    assert_eq!(
        harness.event_kinds(),
        vec![
            event_kind::GESTURE_BEGIN,
            event_kind::GESTURE_UPDATE,
            event_kind::GESTURE_END
        ]
    );
}

#[test]
fn a_toast_is_host_chrome_that_goes_away_when_dismissed() {
    let mut harness = Harness::with_tree(&[(
        1,
        0,
        NodeKind::Column,
        &[],
        Rect::new(0.0, 0.0, 400.0, 300.0),
    )]);
    let mut toasts = Toasts::default();
    let request = WireMap::new()
        .with("title", "Saved")
        .with("message", "All good");
    let nodes_before = harness.scene.len();
    let mut measurer = measure;
    toasts.show(&mut harness.ctx(), &mut measurer, &request);
    assert_eq!(toasts.roots().len(), 1);
    assert!(
        harness.scene.len() > nodes_before,
        "the card is made of scene nodes"
    );
    // A second toast replaces the first.
    toasts.show(
        &mut harness.ctx(),
        &mut measurer,
        &WireMap::new().with("message", "Copied"),
    );
    assert_eq!(toasts.roots().len(), 1);
    toasts.remove(&mut harness.ctx());
    assert!(toasts.roots().is_empty());
    assert_eq!(harness.scene.len(), nodes_before);
}

#[test]
fn a_window_surface_opens_resizes_and_closes_a_native_window() {
    let window: &[(PropKey, PropValue)] = &[
        (PropKey::WindowSurface, PropValue::Boolean(true)),
        (PropKey::WindowTitle, PropValue::String("Inspector".into())),
        (PropKey::WindowWidth, PropValue::Float(360.0)),
        (PropKey::WindowHeight, PropValue::Float(260.0)),
        (PropKey::OnModalRequestClose, PropValue::Boolean(true)),
    ];
    let mut harness = Harness::with_tree(&[
        (
            1,
            0,
            NodeKind::Column,
            &[],
            Rect::new(0.0, 0.0, 400.0, 300.0),
        ),
        (
            2,
            1,
            NodeKind::Modal,
            window,
            Rect::new(0.0, 0.0, 360.0, 260.0),
        ),
    ]);
    let mut windows = Windows::default();
    harness.settle(|ctx, change| windows.on_change(ctx, change));
    windows.after_commit(&mut harness.ctx());
    let opened = harness.commands.iter().find_map(|command| match command {
        HostCommand::OpenWindow { window, config } => {
            Some((*window, config.title.clone(), config.size))
        }
        _ => None,
    });
    let (id, title, size) = opened.unwrap_or_else(|| panic!("the window must open"));
    assert_ne!(id, MAIN_WINDOW);
    assert_eq!(
        (title.as_str(), size.width, size.height),
        ("Inspector", 360.0, 260.0)
    );
    assert!(harness.commands.contains(&HostCommand::SetWindowRoot {
        window: id,
        root: Some(2)
    }));
    assert_eq!(windows.root_of(id), Some(2));
    assert_eq!(windows.take_resized(), vec![(2, Some(size))]);

    // The user resizes it, then asks to close it: the application decides.
    windows.on_resized(id, pam_desktop_ui::geometry::Size::new(500.0, 400.0));
    assert_eq!(
        windows.take_resized(),
        vec![(2, Some(pam_desktop_ui::geometry::Size::new(500.0, 400.0)))]
    );
    windows.on_close_requested(&mut harness.ctx(), id);
    assert_eq!(harness.event_kinds(), vec![event_kind::MODAL_REQUEST_CLOSE]);
}

#[test]
fn a_marquee_line_slides_only_when_it_does_not_fit() {
    let line = |text: &str| {
        vec![
            (PropKey::Text, PropValue::String(text.into())),
            (PropKey::NumberOfLines, PropValue::Integer(1)),
            (PropKey::TextEllipsizeMode, PropValue::Integer(5)),
        ]
    };
    // 50 characters are 350 wide here; the box is 200.
    let long = line("A line that is much too long for the box it got...");
    let short = line("Fits");
    let mut harness = Harness::with_tree(&[
        (
            1,
            0,
            NodeKind::Column,
            &[],
            Rect::new(0.0, 0.0, 400.0, 300.0),
        ),
        (
            2,
            1,
            NodeKind::Text,
            &long,
            Rect::new(10.0, 10.0, 200.0, 20.0),
        ),
        (
            3,
            1,
            NodeKind::Text,
            &short,
            Rect::new(10.0, 40.0, 200.0, 20.0),
        ),
    ]);
    let mut marquees = Marquees::default();
    let mut measurer = measure;
    harness.settle(|ctx, change| marquees.on_change(ctx, change));
    let nodes = harness.scene.len();
    marquees.evaluate(&mut harness.ctx(), &mut measurer);

    // The long line got a clipping box with a copy in it; the short one nothing.
    assert_eq!(harness.scene.len(), nodes + 2);
    let clip = harness.scene.node(2).unwrap().children[0];
    let copy = harness.scene.node(clip).unwrap().children[0];
    assert_eq!(harness.scene.node(clip).unwrap().frame.width, 200.0);
    assert!(harness.scene.node(copy).unwrap().frame.width > 350.0);
    assert!(harness.scene.node(3).unwrap().children.is_empty());
    assert!(harness.timers.is_pending(Timer::Marquee(2)));
    assert!(
        harness.animator.is_idle(),
        "nothing moves while the line rests"
    );

    // After the rest it slides by what does not fit, at 30 units a second.
    harness.now_ms = 1200;
    for timer in harness.timers.take_due(1200) {
        if let Timer::Marquee(id) = timer {
            marquees.on_timer(&mut harness.ctx(), id);
        }
    }
    assert!(harness.animator.is_running(Owner::Marquee(2)));
    harness.now_ms = 1200 + 5000 + 16;
    for sample in harness.animator.tick(harness.now_ms) {
        marquees.on_sample(&mut harness.ctx(), 2, sample.value, sample.finished);
    }
    assert_eq!(
        harness
            .scene
            .layer_override(copy, LayerSlot::Presenter)
            .transform,
        Affine::translation(-150.0, 0.0)
    );
    // It rests again at the end, with nothing running.
    assert!(harness.animator.is_idle());
    assert!(harness.timers.is_pending(Timer::Marquee(2)));

    // A text that fits paints itself again.
    harness
        .scene
        .set_prop(2, PropKey::Text, Some(PropValue::String("Short".into())));
    harness.settle(|ctx, change| marquees.on_change(ctx, change));
    marquees.evaluate(&mut harness.ctx(), &mut measurer);
    assert_eq!(harness.scene.len(), nodes);
    assert!(!harness.timers.is_pending(Timer::Marquee(2)));
}

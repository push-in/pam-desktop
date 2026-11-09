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
fn unused_native_wheel_reaches_scroller_below_transparent_portal() {
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
            NodeKind::Scroll,
            &[],
            Rect::new(0.0, 0.0, 400.0, 300.0),
        ),
        (
            3,
            2,
            NodeKind::Column,
            &[],
            Rect::new(0.0, 0.0, 400.0, 900.0),
        ),
        (
            4,
            1,
            NodeKind::Column,
            &[(PropKey::PointerEvents, PropValue::Integer(3))],
            Rect::new(0.0, 0.0, 400.0, 300.0),
        ),
    ]);
    let mut scrolling = pam_desktop_ui::scrolling::Scrolling::default();
    assert!(scrolling.on_wheel(
        &mut harness.ctx(),
        Point::new(200.0, 150.0),
        Point::new(0.0, 100.0)
    ));
    assert!(harness.commands.contains(&HostCommand::ScrollTo {
        id: 2,
        offset: Point::new(0.0, 100.0),
        animated: false
    }));
    harness.commands.clear();
    assert!(!scrolling.on_wheel(
        &mut harness.ctx(),
        Point::new(200.0, 150.0),
        Point::new(0.0, -100.0)
    ));
    assert!(harness.commands.is_empty());
    assert!(!scrolling.on_wheel(
        &mut harness.ctx(),
        Point::new(200.0, 150.0),
        Point::new(0.0, f32::NAN)
    ));
    harness
        .scene
        .set_prop(2, PropKey::ScrollEnabled, Some(PropValue::Boolean(false)));
    assert!(!scrolling.on_wheel(
        &mut harness.ctx(),
        Point::new(200.0, 150.0),
        Point::new(0.0, 100.0)
    ));
    harness
        .scene
        .set_prop(2, PropKey::ScrollEnabled, Some(PropValue::Boolean(true)));
    harness
        .scene
        .set_prop(4, PropKey::PointerEvents, Some(PropValue::Integer(1)));
    assert!(
        !scrolling.on_wheel(
            &mut harness.ctx(),
            Point::new(200.0, 150.0),
            Point::new(0.0, 100.0)
        ),
        "An opaque overlay must block underlying scrolling"
    );
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
            window: MAIN_WINDOW,
            source: 2,
            text: "card-7".into()
        }]
    );

    drag_drop.on_text_dropped(
        &mut harness.ctx(),
        MAIN_WINDOW,
        Point::new(250.0, 100.0),
        "card-7",
    );
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
    drag_drop.on_text_dropped(
        &mut harness.ctx(),
        MAIN_WINDOW,
        Point::new(20.0, 250.0),
        "elsewhere",
    );
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

#[test]
fn composite_key_capture_precedes_default_activation_and_ignores_unlisted_keys() {
    use pam_desktop_ui::backend::{KeyEvent, Modifiers};
    use pam_desktop_ui::interaction::Interaction;
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
                (PropKey::OnKeyDown, PropValue::Boolean(true)),
                (PropKey::OnPress, PropValue::Boolean(true)),
                (
                    PropKey::CaptureKeys,
                    PropValue::String("Enter,Space,Escape,ArrowDown".into()),
                ),
            ],
            Rect::new(0.0, 0.0, 200.0, 40.0),
        ),
    ]);
    let mut interaction = Interaction::default();
    interaction.on_focus(&mut harness.ctx(), 2, true);
    let mut key = KeyEvent {
        window: MAIN_WINDOW,
        pressed: true,
        key: "Enter".into(),
        modifiers: Modifiers::default(),
        repeat: false,
    };
    for captured in ["Enter", " ", "Escape", "ArrowDown"] {
        key.key = captured.into();
        assert!(interaction.capture_key(&mut harness.ctx(), &key));
    }
    assert_eq!(harness.event_kinds(), vec![event_kind::KEY_DOWN; 4]);
    key.key = "Tab".into();
    assert!(!interaction.capture_key(&mut harness.ctx(), &key));
    key.key = "Enter".into();
    key.modifiers.control = true;
    assert!(!interaction.capture_key(&mut harness.ctx(), &key));
    interaction.on_focus(&mut harness.ctx(), 2, false);
    key.modifiers.control = false;
    assert!(!interaction.capture_key(&mut harness.ctx(), &key));
}

#[test]
fn typed_drag_uses_opaque_nonce_and_rejects_external_wrong_type_and_cross_window_drops() {
    let mut harness = Harness::with_tree(&[
        (1, 0, NodeKind::Column, &[], Rect::new(0., 0., 400., 300.)),
        (
            2,
            1,
            NodeKind::Column,
            &[
                (PropKey::Draggable, PropValue::Boolean(true)),
                (PropKey::DragData, PropValue::String("order-7".into())),
                (PropKey::DragType, PropValue::Integer(1)),
                (PropKey::OnDragStart, PropValue::Boolean(true)),
            ],
            Rect::new(10., 10., 100., 40.),
        ),
        (
            3,
            1,
            NodeKind::Column,
            &[
                (PropKey::DropEnabled, PropValue::Boolean(true)),
                (PropKey::DropType, PropValue::Integer(1)),
                (PropKey::OnDrop, PropValue::Boolean(true)),
                (PropKey::OnDragEnter, PropValue::Boolean(true)),
                (PropKey::OnDragLeave, PropValue::Boolean(true)),
            ],
            Rect::new(200., 10., 150., 200.),
        ),
    ]);
    let mut drag = DragDrop::default();
    let down = harness.pointer(PointerPhase::Down, 20., 20.);
    let movement = harness.pointer(PointerPhase::Move, 40., 30.);
    drag.on_pointer(&mut harness.ctx(), down);
    assert!(drag.on_pointer(&mut harness.ctx(), movement));
    let token = match &harness.commands[0] {
        HostCommand::BeginTextDrag { text, .. } => text.clone(),
        _ => panic!("Missing native drag"),
    };
    assert_ne!(token, "order-7");
    drag.on_text_dropped(
        &mut harness.ctx(),
        MAIN_WINDOW,
        Point::new(250., 100.),
        "order-7",
    );
    assert_eq!(harness.events.len(), 1);
    drag.on_text_dropped(
        &mut harness.ctx(),
        MAIN_WINDOW + 1,
        Point::new(250., 100.),
        &token,
    );
    assert_eq!(harness.events.len(), 1);
    drag.on_motion(
        &mut harness.ctx(),
        MAIN_WINDOW,
        Some(Point::new(250., 100.)),
    );
    drag.on_motion(
        &mut harness.ctx(),
        MAIN_WINDOW,
        Some(Point::new(260., 100.)),
    );
    assert_eq!(
        harness.event_kinds(),
        vec![event_kind::DRAG_START, event_kind::DRAG_ENTER]
    );
    drag.on_text_dropped(
        &mut harness.ctx(),
        MAIN_WINDOW,
        Point::new(250., 100.),
        &token,
    );
    let drop = WireMap::decode(&harness.events[2].payload).unwrap();
    assert_eq!(drop.text("data"), Some("order-7"));
    assert_eq!(drop.integer("type"), Some(1));
    drag.on_drag_finished(&mut harness.ctx(), 2);
    assert_eq!(harness.event_kinds().last(), Some(&event_kind::DRAG_LEAVE));
    drag.on_text_dropped(
        &mut harness.ctx(),
        MAIN_WINDOW,
        Point::new(250., 100.),
        &token,
    );
    assert_eq!(harness.events.len(), 4);
}

#[test]
fn typed_drag_edge_scroll_stops_when_cancelled() {
    let mut harness = Harness::with_tree(&[
        (1, 0, NodeKind::Column, &[], Rect::new(0., 0., 400., 300.)),
        (
            2,
            1,
            NodeKind::Scroll,
            &[(PropKey::ScrollHorizontal, PropValue::Boolean(true))],
            Rect::new(0., 0., 400., 300.),
        ),
        (3, 2, NodeKind::Row, &[], Rect::new(0., 0., 1200., 300.)),
        (
            4,
            3,
            NodeKind::Column,
            &[
                (PropKey::Draggable, PropValue::Boolean(true)),
                (PropKey::DragData, PropValue::String("order-7".into())),
                (PropKey::DragType, PropValue::Integer(1)),
            ],
            Rect::new(10., 10., 100., 40.),
        ),
    ]);
    let mut drag = DragDrop::default();
    let mut scrolling = pam_desktop_ui::scrolling::Scrolling::default();
    let down = harness.pointer(PointerPhase::Down, 20., 20.);
    let movement = harness.pointer(PointerPhase::Move, 40., 30.);
    drag.on_pointer(&mut harness.ctx(), down);
    assert!(drag.on_pointer(&mut harness.ctx(), movement));
    drag.on_motion(
        &mut harness.ctx(),
        MAIN_WINDOW,
        Some(Point::new(390., 150.)),
    );
    assert!(harness.timers.is_pending(Timer::DragScroll));
    drag.scroll_tick(&mut harness.ctx(), &mut scrolling);
    assert!(harness.commands.contains(&HostCommand::ScrollTo {
        id: 2,
        offset: Point::new(12., 0.),
        animated: false,
    }));
    drag.cancel(&mut harness.ctx());
    assert!(!harness.timers.is_pending(Timer::DragScroll));
    harness.commands.clear();
    drag.scroll_tick(&mut harness.ctx(), &mut scrolling);
    assert!(harness.commands.is_empty());
}

#[test]
fn focus_within_tracks_descendants_siblings_blur_removal_and_style_rebuild() {
    use pam_desktop_ui::interaction::Interaction;
    use pam_desktop_ui::scene::Change;
    let styles = PropValue::String(r#"{"10":{"37":4278255360,"480":4278255360,"481":4278255360,"482":4278255360,"483":4278255360}}"#.into());
    let mut h = Harness::with_tree(&[
        (1, 0, NodeKind::Column, &[], Rect::new(0., 0., 400., 300.)),
        (
            2,
            1,
            NodeKind::Row,
            &[
                (PropKey::NativeStateStyles, styles.clone()),
                (PropKey::BorderTopColor, PropValue::Integer(4278190080)),
                (PropKey::BorderRightColor, PropValue::Integer(4278190081)),
                (PropKey::BorderBottomColor, PropValue::Integer(4278190082)),
                (PropKey::BorderLeftColor, PropValue::Integer(4278190083)),
            ],
            Rect::new(0., 0., 300., 40.),
        ),
        (3, 2, NodeKind::Input, &[], Rect::new(0., 0., 200., 40.)),
        (
            4,
            2,
            NodeKind::Pressable,
            &[],
            Rect::new(200., 0., 40., 40.),
        ),
        (5, 1, NodeKind::Input, &[], Rect::new(0., 80., 200., 40.)),
    ]);
    let mut interaction = Interaction::default();
    interaction.on_change(&mut h.ctx(), Change::Created(2, NodeKind::Row));
    let color = |h: &Harness| {
        h.scene
            .effective_props(2)
            .unwrap()
            .integer(PropKey::BorderColor)
    };
    assert_eq!(color(&h), None);
    interaction.on_focus(&mut h.ctx(), 3, true);
    assert_eq!(color(&h), Some(4278255360));
    for key in [
        PropKey::BorderTopColor,
        PropKey::BorderRightColor,
        PropKey::BorderBottomColor,
        PropKey::BorderLeftColor,
    ] {
        assert_eq!(
            h.scene.effective_props(2).unwrap().integer(key),
            Some(4278255360)
        );
    }

    interaction.on_change(&mut h.ctx(), Change::Updated(2, PropKey::NativeStateStyles));
    assert_eq!(color(&h), Some(4278255360));
    interaction.on_focus(&mut h.ctx(), 4, true);
    interaction.on_focus(&mut h.ctx(), 3, false); // Late blur must not erase sibling focus.
    assert_eq!(color(&h), Some(4278255360));
    interaction.on_focus(&mut h.ctx(), 5, true);
    assert_eq!(color(&h), None);
    interaction.on_focus(&mut h.ctx(), 3, true);
    h.scene
        .apply(vec![pam_desktop_wire::Mutation::Remove { id: 3 }]);
    interaction.on_change(&mut h.ctx(), Change::Removed(3, NodeKind::Input));
    assert_eq!(color(&h), None);
    interaction.on_focus(&mut h.ctx(), 4, true);
    interaction.on_focus(&mut h.ctx(), 4, false);
    h.scene.apply(vec![pam_desktop_wire::Mutation::Create(
        pam_desktop_wire::Node {
            id: 3,
            parent: 2,
            index: 0,
            kind: NodeKind::Input,
            properties: Default::default(),
        },
    )]);
    interaction.on_change(&mut h.ctx(), Change::Created(3, NodeKind::Input));
    interaction.on_focus(&mut h.ctx(), 3, true);
    assert_eq!(color(&h), Some(4278255360));
    interaction.on_focus(&mut h.ctx(), 3, false);
    assert_eq!(color(&h), None);
    for (index, key) in [
        PropKey::BorderTopColor,
        PropKey::BorderRightColor,
        PropKey::BorderBottomColor,
        PropKey::BorderLeftColor,
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            h.scene.effective_props(2).unwrap().integer(key),
            Some(4278190080 + index as i64)
        );
    }
}

#[test]
fn a_window_layer_modal_stays_above_dialogs_and_lets_presses_through() {
    use pam_desktop_ui::presenter::modal::Modals;
    let overlay: &[(PropKey, PropValue)] = &[
        (PropKey::ModalPresentation, PropValue::Integer(4)),
        (PropKey::PointerEvents, PropValue::Integer(3)),
    ];
    let dialog: &[(PropKey, PropValue)] = &[(PropKey::ModalPresentation, PropValue::Integer(2))];
    let passthrough: &[(PropKey, PropValue)] = &[(PropKey::PointerEvents, PropValue::Integer(3))];
    let window = Rect::new(0.0, 0.0, 400.0, 300.0);
    // The window layer is mounted first (the toast stack), the dialog later.
    let mut harness = Harness::with_tree(&[
        (1, 0, NodeKind::Column, &[], window),
        (2, 1, NodeKind::Modal, overlay, window),
        (3, 2, NodeKind::Column, passthrough, window),
        (
            4,
            3,
            NodeKind::Column,
            &[],
            Rect::new(280.0, 10.0, 110.0, 40.0),
        ),
        (5, 1, NodeKind::Modal, dialog, window),
        (
            6,
            5,
            NodeKind::Column,
            &[],
            Rect::new(100.0, 50.0, 200.0, 200.0),
        ),
    ]);
    let mut modals = Modals::default();
    harness.settle(|ctx, change| modals.on_change(ctx, change));
    modals.after_commit(&mut harness.ctx());
    assert_eq!(
        harness.scene.overlays(),
        &[5, 2],
        "the layer is above the dialog"
    );
    assert_eq!(modals.top(), Some(5), "Escape and sheets see the dialog");
    // The toast takes its own press; elsewhere the press reaches the dialog.
    assert_eq!(harness.scene.hit_test(Point::new(300.0, 20.0)), Some(4));
    assert_eq!(harness.scene.hit_test(Point::new(150.0, 100.0)), Some(6));
    let outside = harness.scene.hit_test(Point::new(10.0, 290.0));
    assert!(
        outside.is_some_and(|hit| harness.scene.is_within(hit, 5)),
        "outside the toast, the dialog (its backdrop) takes the press"
    );
    // Escape closes the dialog, never the layer.
    assert!(modals.request_close_top(&mut harness.ctx()));
    assert!(modals.is_presented(2));
    assert!(!modals.request_close_top(&mut harness.ctx()));
    assert_eq!(harness.scene.hit_test(Point::new(10.0, 290.0)), Some(1));
}

#[test]
fn dialog_shadow_and_fills_move_onto_a_compositing_backdrop() {
    use pam_desktop_ui::presenter::modal::Modals;
    let window = Rect::new(0.0, 0.0, 400.0, 300.0);
    let modal: &[(PropKey, PropValue)] = &[
        (PropKey::ModalPresentation, PropValue::Integer(2)),
        (PropKey::ModalAnimationType, PropValue::Integer(1)),
        (PropKey::ModalBackdropColor, PropValue::Integer(0xB302_0618)),
        (PropKey::BackdropBlurRadius, PropValue::Float(8.0)),
    ];
    let dialog: &[(PropKey, PropValue)] = &[
        (PropKey::BackgroundColor, PropValue::Integer(0xF202_0618)),
        (PropKey::BorderRadius, PropValue::Float(8.0)),
        (
            PropKey::BoxShadows,
            PropValue::String(format!("[[0,24,64,0,{}]]", 0xAD00_0000_u32)),
        ),
    ];
    let card: &[(PropKey, PropValue)] =
        &[(PropKey::BackgroundColor, PropValue::Integer(0x0DFF_FFFF))];
    let button: &[(PropKey, PropValue)] =
        &[(PropKey::BackgroundColor, PropValue::Integer(0x0DFF_FFFF))];
    let mut harness = Harness::with_tree(&[
        (1, 0, NodeKind::Column, &[], window),
        (2, 1, NodeKind::Modal, modal, window),
        (3, 2, NodeKind::Column, &[], window),
        (
            4,
            3,
            NodeKind::Column,
            dialog,
            Rect::new(100.0, 50.0, 200.0, 150.0),
        ),
        (
            5,
            4,
            NodeKind::View,
            card,
            Rect::new(110.0, 60.0, 80.0, 40.0),
        ),
        (
            6,
            4,
            NodeKind::Pressable,
            button,
            Rect::new(110.0, 110.0, 80.0, 30.0),
        ),
    ]);
    harness.scene.environment.backdrop_boxes = pam_desktop_ui::backend::BackdropBoxes::Topmost;
    let mut modals = Modals::default();
    harness.settle(|ctx, change| modals.on_change(ctx, change));
    modals.after_commit(&mut harness.ctx());
    let backdrop = harness
        .scene
        .node(2)
        .and_then(|node| node.children.iter().copied().find(|child| *child != 3))
        .expect("backdrop");
    let boxes = harness.scene.backdrop_boxes(backdrop).to_vec();
    assert_eq!(boxes.len(), 2, "the dialog and its card: {boxes:?}");
    assert_eq!(boxes[0].rect, Rect::new(100.0, 50.0, 200.0, 150.0));
    assert!(boxes[0].shadow.is_some() && boxes[0].fill.is_some());
    assert_eq!(boxes[1].rect, Rect::new(110.0, 60.0, 80.0, 40.0));
    let shadows = harness
        .scene
        .effective_props(4)
        .and_then(|props| props.text(PropKey::BoxShadows).map(str::to_owned));
    assert_eq!(
        shadows.as_deref(),
        Some("[]"),
        "the dialog paints no shadow"
    );
    assert!(harness.scene.fill_composited(4) && harness.scene.fill_composited(5));
    assert!(
        !harness.scene.fill_composited(6),
        "pressables restyle on hover"
    );
    // Without a compositing backend nothing moves.
    let mut plain = Harness::with_tree(&[
        (1, 0, NodeKind::Column, &[], window),
        (2, 1, NodeKind::Modal, modal, window),
        (3, 2, NodeKind::Column, &[], window),
        (
            4,
            3,
            NodeKind::Column,
            dialog,
            Rect::new(100.0, 50.0, 200.0, 150.0),
        ),
    ]);
    let mut modals = Modals::default();
    plain.settle(|ctx, change| modals.on_change(ctx, change));
    modals.after_commit(&mut plain.ctx());
    assert!(!plain.scene.fill_composited(4));
}

#[test]
fn dialogs_shown_in_one_commit_stack_in_tree_order() {
    use pam_desktop_ui::presenter::modal::Modals;
    let dialog: &[(PropKey, PropValue)] = &[(PropKey::ModalPresentation, PropValue::Integer(2))];
    let window = Rect::new(0.0, 0.0, 400.0, 300.0);
    // A detail, a dialog opened from it and a third on top, all in one render
    // (ids deliberately out of tree order).
    let mut harness = Harness::with_tree(&[
        (1, 0, NodeKind::Column, &[], window),
        (9, 1, NodeKind::Modal, dialog, window),
        (
            10,
            9,
            NodeKind::Column,
            &[],
            Rect::new(10.0, 10.0, 380.0, 280.0),
        ),
        (3, 1, NodeKind::Modal, dialog, window),
        (
            4,
            3,
            NodeKind::Column,
            &[],
            Rect::new(100.0, 50.0, 200.0, 200.0),
        ),
        (7, 1, NodeKind::Modal, dialog, window),
        (
            8,
            7,
            NodeKind::Column,
            &[],
            Rect::new(150.0, 100.0, 100.0, 100.0),
        ),
    ]);
    let mut modals = Modals::default();
    harness.settle(|ctx, change| modals.on_change(ctx, change));
    modals.after_commit(&mut harness.ctx());
    assert_eq!(harness.scene.overlays(), &[9, 3, 7], "later siblings above");
    assert_eq!(modals.top(), Some(7), "Escape reaches the last one");
    assert_eq!(harness.scene.hit_test(Point::new(200.0, 150.0)), Some(8));
    assert!(modals.request_close_top(&mut harness.ctx()));
}

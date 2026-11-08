use pam_desktop_ui::backend::PointerPhase;
use pam_desktop_ui::context::event_kind;
use pam_desktop_ui::drawing::Drawings;
use pam_desktop_ui::geometry::Rect;
use pam_desktop_ui::scene::Change;
use pam_desktop_ui::testing::Harness;
use pam_desktop_ui::wiremap::WireMap;
use pam_desktop_wire::{NodeKind, PropKey, PropValue};

#[test]
fn canvas_hover_is_local_coalesced_and_cleared_when_leaving_or_removed() {
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
            NodeKind::Canvas,
            &[(PropKey::OnNativeEvent, PropValue::Boolean(true))],
            Rect::new(10.0, 20.0, 200.0, 100.0),
        ),
    ]);
    let mut drawings = Drawings::default();
    drawings.on_change(&mut harness.ctx(), Change::Created(2, NodeKind::Canvas));
    for x in [30.0, 40.0, 50.0] {
        let pointer = harness.pointer(PointerPhase::Move, x, 40.0);
        assert!(!drawings.on_pointer(&mut harness.ctx(), pointer));
    }
    assert!(harness.events.is_empty());
    drawings.flush(&mut harness.ctx());
    assert_eq!(harness.event_kinds(), vec![event_kind::NATIVE]);
    assert_eq!(
        harness.events[0].payload,
        WireMap::new()
            .with("event", 2_i64)
            .with("x", 40.0_f32)
            .with("y", 20.0_f32)
            .encode()
    );
    let outside = harness.pointer(PointerPhase::Move, 350.0, 250.0);
    drawings.on_pointer(&mut harness.ctx(), outside);
    drawings.flush(&mut harness.ctx());
    assert_eq!(harness.events.len(), 2);
    assert_eq!(
        harness.events[1].payload,
        WireMap::new()
            .with("event", 4_i64)
            .with("x", 0.0_f32)
            .with("y", 0.0_f32)
            .encode()
    );
    let inside = harness.pointer(PointerPhase::Move, 60.0, 40.0);
    drawings.on_pointer(&mut harness.ctx(), inside);
    drawings.on_change(&mut harness.ctx(), Change::Removed(2, NodeKind::Canvas));
    drawings.flush(&mut harness.ctx());
    assert_eq!(
        harness.events.len(),
        2,
        "Removed canvases cannot emit queued hover callbacks."
    );
}

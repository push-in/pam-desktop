//! Development aid for layout parity: with `PAM_DESKTOP_DUMP_FRAMES=<path>`
//! every flush that moves a frame rewrites `<path>` (atomically) with the
//! frame of every node, as laid out (Blink LayoutUnit grid, 1/64 px) and as
//! painted (snapped to device pixels), in window coordinates. Off by default:
//! the variable is read once and an unset variable costs one branch per
//! flush. `tools/parity/compare_frames.py` compares a dump with Chromium's
//! `getBoundingClientRect()` boxes (`tools/parity/electron-dom.cjs`).

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::geometry::Rect;
use crate::scene::Scene;

/// Characters of text kept per node: enough to match it with the DOM.
const TEXT_LIMIT: usize = 120;

fn target() -> Option<&'static Path> {
    static TARGET: OnceLock<Option<PathBuf>> = OnceLock::new();
    TARGET
        .get_or_init(|| {
            std::env::var_os("PAM_DESKTOP_DUMP_FRAMES")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        })
        .as_deref()
}

/// Whether frames are dumped in this process.
#[must_use]
pub fn enabled() -> bool {
    target().is_some()
}

fn rect(rect: Rect) -> serde_json::Value {
    serde_json::json!([rect.x, rect.y, rect.width, rect.height])
}

/// The dump of `scene` as JSON (also used by tests).
#[must_use]
pub fn document(scene: &Scene, window: u64) -> serde_json::Value {
    let scale = scene.environment.scale;
    let nodes: Vec<serde_json::Value> = scene
        .nodes_in_order()
        .into_iter()
        .filter_map(|id| {
            let node = scene.node(id)?;
            let origin = scene.window_origin(id);
            let layout = Rect::new(origin.x, origin.y, node.frame.width, node.frame.height);
            let props = node.props();
            let text = scene
                .plain_text(id)
                .map(|text| text.chars().take(TEXT_LIMIT).collect::<String>());
            // The paragraph's box inside padding and border, like a DOM
            // content box.
            let content = scene.text_insets(id).map(|insets| {
                rect(Rect::new(
                    layout.x + insets.left,
                    layout.y + insets.top,
                    (layout.width - insets.horizontal()).max(0.0),
                    (layout.height - insets.vertical()).max(0.0),
                ))
            });
            // Corner radii as declared (TL, TR, BR, BL), to compare with
            // Chromium's computed border-radius.
            let radii = crate::style::Decoration::resolve(node.kind, props).radii;
            Some(serde_json::json!({
                "id": id,
                "radius": [radii.top_left, radii.top_right, radii.bottom_right, radii.bottom_left],
                "parent": node.parent,
                "kind": format!("{:?}", node.kind),
                "testId": props.text_or_empty(pam_desktop_wire::PropKey::TestId),
                "label": props.text_or_empty(pam_desktop_wire::PropKey::AccessibilityLabel),
                "text": text,
                "layout": rect(layout),
                "content": content,
                "painted": rect(layout.snapped(scale)),
            }))
        })
        .collect();
    serde_json::json!({ "window": window, "scale": scale, "nodes": nodes })
}

/// Rewrites the dump file; errors are reported once on stderr and ignored.
pub fn write(scene: &Scene, window: u64) {
    let Some(path) = target() else {
        return;
    };
    let path = PathBuf::from(
        path.to_string_lossy()
            .replace("{window}", &window.to_string()),
    );
    let temporary = path.with_extension("tmp");
    let result = serde_json::to_vec(&document(scene, window))
        .map_err(std::io::Error::other)
        .and_then(|bytes| std::fs::write(&temporary, bytes))
        .and_then(|()| std::fs::rename(&temporary, &path));
    if let Err(error) = result {
        static REPORTED: OnceLock<()> = OnceLock::new();
        if REPORTED.set(()).is_ok() {
            eprintln!(
                "PAM_DESKTOP_DUMP_FRAMES: cannot write {}: {error}",
                path.display()
            );
        }
    }
}

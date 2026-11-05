//! Child windows over the Direct2D scene (text inputs, web views): they
//! follow their node's box, are cut by clipping ancestors and hide with
//! their view. Placement is computed from the retained tree under the
//! state borrow and applied to the windows after it.

use std::rc::Rc;

use pam_desktop_ui::backend::{NodeId, WindowId};
use windows::Win32::Foundation::HWND;

use crate::app::{App, State};
use crate::scene::Placement;

/// Moves every child window of `window` to where its node is now.
pub fn sync(app: &Rc<App>, window: WindowId) {
    let plan = app.with_state(|state| {
        let parent = state.scene.windows.get(&window)?.hwnd;
        let placements: Vec<(NodeId, Option<Placement>)> = state
            .scene
            .natives
            .iter()
            .map(|id| (*id, state.scene.placement(*id)))
            // Views of other windows are placed when those windows paint.
            .filter(|(_, placement)| placement.is_none_or(|placement| placement.window == window))
            .collect();
        Some((parent, placements))
    });
    let Some(Some((parent, placements))) = plan else {
        return;
    };
    for (id, placement) in placements {
        match placement {
            Some(placement) => {
                crate::input::place(app, id, &placement);
                crate::webview::place(app, id, &placement);
            }
            // A view that left the tree keeps its child window, hidden.
            None => {
                crate::input::detach(app, id, parent);
                crate::webview::detach(app, id, parent);
            }
        }
    }
}

/// A top-level window was destroyed: its child windows are gone.
pub fn parent_destroyed(state: &mut State, parent: HWND) {
    for id in &state.scene.natives {
        let Some(view) = state.scene.views.get_mut(id) else {
            continue;
        };
        if let Some(input) = view.input.as_mut() {
            crate::input::parent_destroyed(input, parent);
        }
        if let Some(web) = view.web.as_mut() {
            crate::webview::parent_destroyed(web, parent);
        }
    }
}

//! Keyboard focus: which view has it, Tab traversal and the events the
//! core expects. A focused box keeps the Win32 focus on the top-level
//! window; a focused text input owns it with its child control.

use std::rc::Rc;

use pam_desktop_ui::backend::{BackendEvent, NodeId, WindowId};
use pam_desktop_ui::geometry::Rect;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::IsChild;

use crate::app::{App, State};
use crate::scene::Scene;

/// Whether the view keeps the Win32 focus in a child window of its own.
fn is_native(scene: &Scene, id: NodeId) -> bool {
    scene.views.get(&id).is_some_and(|view| view.input.is_some() || view.web.is_some())
}

/// A focus request for a control that does not exist yet is void once
/// the focus is asked to go elsewhere.
fn cancel_pending(state: &mut State) {
    for id in &state.scene.natives {
        if let Some(input) = state.scene.views.get_mut(id).and_then(|view| view.input.as_mut()) {
            input.cancel_focus();
        }
    }
}

/// `Backend::focus`. Runs outside any state borrow.
pub fn set(app: &Rc<App>, id: Option<NodeId>) {
    let Some(id) = id else { return clear(app) };
    app.with_state(cancel_pending);
    // Inputs and web pages take the focus with their own window, which
    // reports the change.
    if crate::input::focus(app, id) || crate::webview::focus(app, id) {
        return;
    }
    let target = app
        .with_state(|state| {
            let window = state.scene.window_of(id)?;
            let hwnd = state.scene.windows.get(&window)?.hwnd;
            let previous = state.scene.focus;
            if previous == Some(id) {
                return None;
            }
            state.scene.focus = Some(id);
            state.scene.damage_view(id);
            if let Some(previous) = previous {
                state.scene.damage_view(previous);
            }
            // A previously focused native control reports its own loss when
            // it loses the Win32 focus below.
            let previous_box = previous.filter(|previous| !is_native(&state.scene, *previous));
            Some((hwnd, previous_box))
        })
        .flatten();
    let Some((hwnd, previous_box)) = target else { return };
    // SAFETY: `hwnd` is a live window of this thread; moving the focus
    // dispatches messages and no state borrow is held.
    unsafe {
        if GetFocus() != hwnd {
            let _ = SetFocus(Some(hwnd));
        }
    }
    if let Some(previous) = previous_box {
        app.emit(BackendEvent::FocusChanged { id: previous, focused: false });
    }
    app.emit(BackendEvent::FocusChanged { id, focused: true });
    crate::accessibility::focus_changed(app, id);
}

fn clear(app: &Rc<App>) {
    let previous = app.with_state(|state| {
        cancel_pending(state);
        let previous = state.scene.focus?;
        let native = is_native(&state.scene, previous);
        let hwnd = state.scene.window_of(previous).and_then(|window| state.scene.windows.get(&window)).map(|entry| entry.hwnd);
        if !native {
            state.scene.focus = None;
            state.scene.damage_view(previous);
        }
        Some((previous, native, hwnd))
    });
    let Some(Some((previous, native, hwnd))) = previous else { return };
    if native {
        // Taking the Win32 focus back makes the control report the loss.
        if let Some(hwnd) = hwnd {
            // SAFETY: live windows of this thread; no state borrow is held.
            unsafe {
                let focused = GetFocus();
                if focused != hwnd && IsChild(hwnd, focused).as_bool() {
                    let _ = SetFocus(Some(hwnd));
                }
            }
        }
    } else {
        app.emit(BackendEvent::FocusChanged { id: previous, focused: false });
    }
}

/// A native control (text input, web page) gained or lost the Win32
/// focus: the scene and the core follow. The event is drained after the
/// control's own procedure returned.
pub fn native_changed(app: &Rc<App>, id: NodeId, focused: bool) {
    app.with_state(|state| {
        let previous = state.scene.focus;
        if focused {
            state.scene.focus = Some(id);
            if let Some(previous) = previous.filter(|previous| *previous != id) {
                // A focused box loses the focus to the control.
                if !is_native(&state.scene, previous) {
                    state.events.push(BackendEvent::FocusChanged { id: previous, focused: false });
                    state.scene.damage_view(previous);
                }
            }
        } else if previous == Some(id) {
            state.scene.focus = None;
        }
    });
    app.emit_later(BackendEvent::FocusChanged { id, focused });
}

fn collect(scene: &Scene, id: NodeId, out: &mut Vec<NodeId>, depth: usize) {
    let Some(view) = scene.views.get(&id) else { return };
    if !view.layer.visible || view.layer.opacity <= 0.001 || depth > 512 {
        return;
    }
    if view.focusable || view.web.is_some() || view.input.as_ref().is_some_and(|input| input.is_enabled()) {
        out.push(id);
    }
    for child in &view.children {
        collect(scene, *child, out, depth + 1);
    }
}

/// Views that take part in Tab traversal, in tree order. A layer that
/// covers the window (a modal) hides the layers under it from the keyboard.
fn candidates(scene: &Scene, window: WindowId) -> Vec<NodeId> {
    let mut found = Vec::new();
    let Some(entry) = scene.windows.get(&window) else { return found };
    let layers: Vec<NodeId> = entry.root.iter().chain(entry.overlays.iter()).copied().collect();
    let bounds = Rect::from_size(entry.size);
    let covers = |id: &NodeId| {
        scene.views.get(id).is_some_and(|view| view.layer.visible && view.frame.x <= 0.5 && view.frame.y <= 0.5 && view.frame.right() >= bounds.right() - 0.5 && view.frame.bottom() >= bounds.bottom() - 0.5)
    };
    let first = layers.iter().rposition(covers).unwrap_or(0);
    for layer in &layers[first..] {
        collect(scene, *layer, &mut found, 0);
    }
    found
}

/// Scrolls the scroll views around `id` so it is inside their viewports.
fn reveal(state: &mut State, id: NodeId) {
    let Some(view) = state.scene.views.get(&id) else { return };
    let mut rect = view.frame;
    let mut ancestor = view.parent;
    while let Some(current_id) = ancestor {
        let Some(current) = state.scene.views.get(&current_id) else { break };
        let frame = current.frame;
        ancestor = current.parent;
        match current.scroll.as_ref().map(|scroll| scroll.offset) {
            Some(offset) => {
                let viewport = frame.size();
                let mut wanted = offset;
                if rect.x < offset.x {
                    wanted.x = rect.x;
                } else if rect.right() > offset.x + viewport.width {
                    wanted.x = rect.right() - viewport.width;
                }
                if rect.y < offset.y {
                    wanted.y = rect.y;
                } else if rect.bottom() > offset.y + viewport.height {
                    wanted.y = rect.bottom() - viewport.height;
                }
                if wanted != offset {
                    state.scrolling().scroll_to(current_id, wanted, true);
                }
                // Continue with the scroll view's own box in its parent.
                rect = frame;
            }
            None => rect = rect.translate(frame.x, frame.y),
        }
    }
    state.schedule_scroll_work();
}

/// Moves the focus to the next (or previous) focusable view of `window`.
/// Returns whether there was one.
pub fn advance(app: &Rc<App>, window: WindowId, forward: bool) -> bool {
    let next = app
        .with_state(|state| {
            let order = candidates(&state.scene, window);
            if order.is_empty() {
                return None;
            }
            let current = state.scene.focus.and_then(|focus| order.iter().position(|id| *id == focus));
            let index = match (current, forward) {
                (Some(index), true) => (index + 1) % order.len(),
                (Some(index), false) => (index + order.len() - 1) % order.len(),
                (None, true) => 0,
                (None, false) => order.len() - 1,
            };
            let next = order[index];
            state.scene.focus_visible = true;
            reveal(state, next);
            Some(next)
        })
        .flatten();
    match next {
        Some(next) => {
            set(app, Some(next));
            true
        }
        None => false,
    }
}

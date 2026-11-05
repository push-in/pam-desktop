//! Windows: the `NSWindow` subclass that sees every pointer, key and focus
//! change before AppKit dispatches it, its delegate, and the content view
//! that hosts the root node and the overlays.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyClass, AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{
    AnyThread, ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class,
    msg_send, sel,
};
use objc2_app_kit::{
    NSAppearanceCustomization, NSApplication, NSBackingStoreType, NSDragOperation,
    NSDraggingContext, NSDraggingDestination, NSDraggingInfo, NSDraggingSession, NSDraggingSource,
    NSEvent, NSEventModifierFlags, NSEventPhase, NSEventType, NSPasteboard,
    NSPasteboardTypeFileURL, NSPasteboardTypeString, NSPasteboardURLReadingFileURLsOnlyKey,
    NSResponder, NSTrackingArea, NSTrackingAreaOptions, NSView, NSWindow, NSWindowDelegate,
    NSWindowStyleMask,
};
use objc2_core_foundation::CGRect;
use objc2_foundation::{
    NSArray, NSDictionary, NSNotification, NSNumber, NSPoint, NSRect, NSString, NSURL,
};
use pam_desktop_ui::backend::{
    BackendEvent, GesturePhase, KeyEvent, NodeId, PointerButton, PointerEvent, PointerPhase,
    WindowId,
};
use pam_desktop_ui::geometry::{Point, Size};
use pam_desktop_ui::shell::WindowState;

use crate::events::{self, Events};
use crate::util;
use crate::view::{PamView, node_view_of, responder_view};

/// Logical units one notch of a mouse wheel scrolls.
const WHEEL_NOTCH: f64 = 100.0;

fn button_bit(button: PointerButton) -> u8 {
    match button {
        PointerButton::Primary => 1,
        PointerButton::Secondary => 2,
        PointerButton::Middle => 4,
    }
}

pub struct WindowIvars {
    id: WindowId,
    events: Rc<Events>,
    /// The node whose view contains the first responder.
    focused: RefCell<Option<(NodeId, Weak<PamView>)>>,
    /// Buttons whose press was reported to the core (`button_bit`).
    pressed: Cell<u8>,
    /// How the press of the left button was reported: with Control it is
    /// a secondary click.
    left_button: Cell<PointerButton>,
    /// The press being handled, needed to start window and content drags.
    last_press: RefCell<Option<Retained<NSEvent>>>,
    /// A pointer event is being reported from `sendEvent:`.
    reporting_pointer: Cell<bool>,
    /// The core asked to move the window with the press being reported.
    drag_requested: Cell<bool>,
    /// Totals of the pinch and rotation gestures in progress.
    magnification: Cell<f32>,
    rotation: Cell<f32>,
}

define_class!(
    // SAFETY: `NSWindow` allows subclassing; every override forwards to
    // `super` unless it documents why not. Main thread only.
    #[unsafe(super(NSWindow, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = WindowIvars]
    pub struct PamWindow;

    impl PamWindow {
        /// Every event of the window passes here first: pointer events are
        /// reported to the core, then dispatched to native controls.
        #[unsafe(method(sendEvent:))]
        fn send_event(&self, event: &NSEvent) {
            let kind = event.r#type();
            if kind == NSEventType::LeftMouseDown || kind == NSEventType::RightMouseDown || kind == NSEventType::OtherMouseDown {
                self.mouse_down(event);
            } else if kind == NSEventType::LeftMouseUp || kind == NSEventType::RightMouseUp || kind == NSEventType::OtherMouseUp {
                self.mouse_up(event);
            } else if kind == NSEventType::LeftMouseDragged || kind == NSEventType::RightMouseDragged || kind == NSEventType::OtherMouseDragged {
                self.mouse_dragged(event);
            } else if kind == NSEventType::ScrollWheel && event.modifierFlags().contains(NSEventModifierFlags::Control) {
                // Control + wheel zooms and never scrolls: it is taken
                // before any scrolling view sees it.
                self.report_wheel(event);
            } else {
                self.dispatch(event);
            }
        }

        /// Every change of first responder passes here: the node that
        /// holds keyboard focus is tracked for all kinds of views at once.
        #[unsafe(method(makeFirstResponder:))]
        fn make_first_responder(&self, responder: Option<&NSResponder>) -> bool {
            // SAFETY: forwards the same selector and argument to `NSWindow`.
            let changed: bool = unsafe { msg_send![super(self), makeFirstResponder: responder] };
            self.focus_moved();
            changed
        }

        /// Keys no view consumed end here. `super` is not called: it would
        /// only beep.
        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            let ivars = self.ivars();
            let key = events::key_name(event);
            let modifiers = events::modifiers(event.modifierFlags());
            let tab = key == "Tab" && !modifiers.control && !modifiers.alt && !modifiers.meta;
            ivars.events.push(BackendEvent::Key(KeyEvent { window: ivars.id, pressed: true, key, modifiers, repeat: event.isARepeat() }));
            if tab {
                // Keyboard focus traversal is the platform's.
                if modifiers.shift {
                    self.selectPreviousKeyView(None);
                } else {
                    self.selectNextKeyView(None);
                }
            }
        }

        #[unsafe(method(keyUp:))]
        fn key_up(&self, event: &NSEvent) {
            let ivars = self.ivars();
            ivars.events.push(BackendEvent::Key(KeyEvent {
                window: ivars.id,
                pressed: false,
                key: events::key_name(event),
                modifiers: events::modifiers(event.modifierFlags()),
                repeat: false,
            }));
        }

        /// A modifier key went down or up.
        #[unsafe(method(flagsChanged:))]
        fn flags_changed(&self, event: &NSEvent) {
            let ivars = self.ivars();
            ivars.events.push(BackendEvent::Key(KeyEvent {
                window: ivars.id,
                pressed: events::modifier_pressed(event),
                key: events::key_name(event),
                modifiers: events::modifiers(event.modifierFlags()),
                repeat: false,
            }));
        }

        /// Scrolling no scroll view consumed.
        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &NSEvent) {
            self.report_wheel(event);
        }

        /// A three-finger swipe to the left is the system's "back".
        #[unsafe(method(swipeWithEvent:))]
        fn swipe(&self, event: &NSEvent) {
            let ivars = self.ivars();
            if event.deltaX() > 0.0 {
                ivars.events.push(BackendEvent::BackRequested { window: ivars.id });
            }
        }

        #[unsafe(method(magnifyWithEvent:))]
        fn magnify(&self, event: &NSEvent) {
            let ivars = self.ivars();
            let phase = gesture_phase(event.phase());
            if phase == GesturePhase::Begin {
                ivars.magnification.set(0.0);
            }
            // Each event carries the change since the previous one.
            ivars.magnification.set(ivars.magnification.get() + event.magnification() as f32);
            ivars.events.push(BackendEvent::Magnify {
                window: ivars.id,
                position: self.content_point(event.locationInWindow()),
                phase,
                scale: (1.0 + ivars.magnification.get()).max(0.01),
            });
        }

        #[unsafe(method(rotateWithEvent:))]
        fn rotate(&self, event: &NSEvent) {
            let ivars = self.ivars();
            let phase = gesture_phase(event.phase());
            if phase == GesturePhase::Begin {
                ivars.rotation.set(0.0);
            }
            // AppKit reports degrees counter-clockwise.
            ivars.rotation.set(ivars.rotation.get() - event.rotation().to_radians());
            ivars.events.push(BackendEvent::Rotate {
                window: ivars.id,
                position: self.content_point(event.locationInWindow()),
                phase,
                radians: ivars.rotation.get(),
            });
        }

        /// The editing shortcuts of text controls come from the Edit menu.
        /// An application whose menu bar has none still gets them.
        #[unsafe(method(performKeyEquivalent:))]
        fn perform_key_equivalent(&self, event: &NSEvent) -> bool {
            // SAFETY: forwards the same selector and argument to `NSWindow`.
            let handled: bool = unsafe { msg_send![super(self), performKeyEquivalent: event] };
            handled || self.perform_edit_shortcut(event)
        }

        /// Windows without a system title bar still take keyboard focus.
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key_window(&self) -> bool {
            true
        }

        #[unsafe(method(canBecomeMainWindow))]
        fn can_become_main_window(&self) -> bool {
            true
        }
    }
);

fn gesture_phase(phase: NSEventPhase) -> GesturePhase {
    if phase.contains(NSEventPhase::Began) || phase.contains(NSEventPhase::MayBegin) {
        GesturePhase::Begin
    } else if phase.contains(NSEventPhase::Ended) {
        GesturePhase::End
    } else if phase.contains(NSEventPhase::Cancelled) {
        GesturePhase::Cancel
    } else {
        GesturePhase::Update
    }
}

impl PamWindow {
    pub fn new(
        mtm: MainThreadMarker,
        id: WindowId,
        events: &Rc<Events>,
        content: NSRect,
        style: NSWindowStyleMask,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(WindowIvars {
            id,
            events: Rc::clone(events),
            focused: RefCell::new(None),
            pressed: Cell::new(0),
            left_button: Cell::new(PointerButton::Primary),
            last_press: RefCell::new(None),
            reporting_pointer: Cell::new(false),
            drag_requested: Cell::new(false),
            magnification: Cell::new(0.0),
            rotation: Cell::new(0.0),
        });
        // SAFETY: the designated initializer of `NSWindow`, with the
        // argument types its signature declares.
        let window: Retained<Self> = unsafe {
            msg_send![super(this), initWithContentRect: content, styleMask: style, backing: NSBackingStoreType::Buffered, defer: false]
        };
        // SAFETY: the window is owned by its `Retained`: closing it must
        // not release it a second time.
        unsafe { window.setReleasedWhenClosed(false) };
        window
    }

    /// Handles the standard editing chords before the menu bar sees them:
    /// the first responder gets the matching action (a text control
    /// copies), and when nothing native takes it the core gets the key
    /// (it copies the text selected in its own views). Left to the menu
    /// bar, a disabled Edit item would swallow the chord.
    fn perform_edit_shortcut(&self, event: &NSEvent) -> bool {
        let flags = event.modifierFlags() & NSEventModifierFlags::DeviceIndependentFlagsMask;
        let chord = flags
            & (NSEventModifierFlags::Command
                | NSEventModifierFlags::Shift
                | NSEventModifierFlags::Control
                | NSEventModifierFlags::Option);
        let shift = chord == NSEventModifierFlags::Command | NSEventModifierFlags::Shift;
        if event.r#type() != NSEventType::KeyDown
            || !(chord == NSEventModifierFlags::Command || shift)
        {
            return false;
        }
        let key = event
            .charactersIgnoringModifiers()
            .map(|characters| characters.to_string().to_lowercase())
            .unwrap_or_default();
        let action = match (key.as_str(), shift) {
            ("c", false) => sel!(copy:),
            ("v", false) => sel!(paste:),
            ("x", false) => sel!(cut:),
            ("a", false) => sel!(selectAll:),
            ("z", false) => sel!(undo:),
            ("z", true) => sel!(redo:),
            _ => return false,
        };
        // SAFETY: the actions take the sender; a nil target sends them
        // down the responder chain of the key window.
        let native = unsafe {
            NSApplication::sharedApplication(self.mtm()).sendAction_to_from(
                action,
                None,
                Some(self),
            )
        };
        if !native {
            let ivars = self.ivars();
            ivars.events.push(BackendEvent::Key(KeyEvent {
                window: ivars.id,
                pressed: true,
                key: events::key_name(event),
                modifiers: events::modifiers(event.modifierFlags()),
                repeat: event.isARepeat(),
            }));
        }
        true
    }

    fn report_wheel(&self, event: &NSEvent) {
        let ivars = self.ivars();
        let unit = if event.hasPreciseScrollingDeltas() {
            1.0
        } else {
            WHEEL_NOTCH
        };
        // AppKit's deltas say where the content moves; the core's say
        // where the viewport moves.
        let delta = Point::new(
            (-event.scrollingDeltaX() * unit) as f32,
            (-event.scrollingDeltaY() * unit) as f32,
        );
        if delta != Point::ZERO {
            ivars.events.push(BackendEvent::Wheel {
                window: ivars.id,
                position: self.content_point(event.locationInWindow()),
                delta,
                modifiers: events::modifiers(event.modifierFlags()),
            });
        }
    }

    pub fn id(&self) -> WindowId {
        self.ivars().id
    }

    fn dispatch(&self, event: &NSEvent) {
        // SAFETY: forwards the same selector and argument to `NSWindow`.
        let _: () = unsafe { msg_send![super(self), sendEvent: event] };
    }

    /// A window location in the content view's coordinates (top-left origin).
    fn content_point(&self, location: NSPoint) -> Point {
        match self.contentView() {
            Some(content) => util::point(content.convertPoint_fromView(location, None)),
            None => Point::ZERO,
        }
    }

    fn inside_content(&self, point: Point) -> bool {
        self.contentView().is_some_and(|content| {
            let bounds = content.bounds();
            point.x >= 0.0
                && point.y >= 0.0
                && f64::from(point.x) < bounds.size.width
                && f64::from(point.y) < bounds.size.height
        })
    }

    fn report_pointer(
        &self,
        phase: PointerPhase,
        position: Point,
        button: PointerButton,
        event: &NSEvent,
        click_count: isize,
    ) {
        let ivars = self.ivars();
        ivars.reporting_pointer.set(true);
        ivars.events.push(BackendEvent::Pointer(PointerEvent {
            window: ivars.id,
            phase,
            position,
            button,
            modifiers: events::modifiers(event.modifierFlags()),
            click_count: click_count.clamp(1, 255) as u8,
            time_ms: events::time_ms(event),
        }));
        ivars.reporting_pointer.set(false);
    }

    /// Whether the press lands on a native control that must not open its
    /// own context menu.
    fn context_menu_hidden_at(&self, event: &NSEvent) -> bool {
        let Some(content) = self.contentView() else {
            return false;
        };
        // SAFETY: the superview is only used to convert the location.
        let location = match unsafe { content.superview() } {
            Some(frame) => frame.convertPoint_fromView(event.locationInWindow(), None),
            None => event.locationInWindow(),
        };
        content
            .hitTest(location)
            .and_then(|hit| node_view_of(&hit))
            .is_some_and(|node| node.context_menu_hidden())
    }

    fn mouse_down(&self, event: &NSEvent) {
        let ivars = self.ivars();
        let kind = event.r#type();
        let left = kind == NSEventType::LeftMouseDown;
        let button = if left {
            // Control-click is the secondary click of a one-button mouse.
            if event
                .modifierFlags()
                .contains(NSEventModifierFlags::Control)
            {
                PointerButton::Secondary
            } else {
                PointerButton::Primary
            }
        } else if kind == NSEventType::RightMouseDown {
            PointerButton::Secondary
        } else if event.buttonNumber() == 2 {
            PointerButton::Middle
        } else {
            // Side buttons: "back" is reported when released.
            return self.dispatch(event);
        };
        let position = self.content_point(event.locationInWindow());
        let inside = self.inside_content(position);
        ivars.drag_requested.set(false);
        if inside {
            ivars.pressed.set(ivars.pressed.get() | button_bit(button));
            if left {
                ivars.left_button.set(button);
            }
            *ivars.last_press.borrow_mut() = Some(event.retain());
            self.report_pointer(
                PointerPhase::Down,
                position,
                button,
                event,
                event.clickCount(),
            );
        }
        if ivars.drag_requested.replace(false) {
            // The press grabbed a drag region: the system moves the window
            // until the button is released.
            self.performWindowDragWithEvent(event);
        } else if !(inside
            && button == PointerButton::Secondary
            && self.context_menu_hidden_at(event))
        {
            ivars.events.dispatching_mouse_down.set(left);
            self.dispatch(event);
            ivars.events.dispatching_mouse_down.set(false);
        }
        self.release_if_swallowed(button, event);
    }

    /// Native tracking loops (text selection, window moves, scrollers)
    /// consume the release of the button: the core still has to see it.
    fn release_if_swallowed(&self, button: PointerButton, event: &NSEvent) {
        let ivars = self.ivars();
        let still_reported = ivars.pressed.get() & button_bit(button) != 0;
        let number = event.buttonNumber().clamp(0, 31) as usize;
        if still_reported && NSEvent::pressedMouseButtons() & (1 << number) == 0 {
            ivars.pressed.set(ivars.pressed.get() & !button_bit(button));
            let position = self.content_point(self.mouseLocationOutsideOfEventStream());
            self.report_pointer(
                PointerPhase::Up,
                position,
                button,
                event,
                event.clickCount(),
            );
        }
    }

    /// The button a release or drag event is about, as its press was reported.
    fn reported_button(&self, event: &NSEvent) -> Option<PointerButton> {
        let kind = event.r#type();
        if kind == NSEventType::LeftMouseUp || kind == NSEventType::LeftMouseDragged {
            Some(self.ivars().left_button.get())
        } else if kind == NSEventType::RightMouseUp || kind == NSEventType::RightMouseDragged {
            Some(PointerButton::Secondary)
        } else if event.buttonNumber() == 2 {
            Some(PointerButton::Middle)
        } else {
            None
        }
    }

    fn mouse_up(&self, event: &NSEvent) {
        let ivars = self.ivars();
        match self.reported_button(event) {
            Some(button) => {
                if ivars.pressed.get() & button_bit(button) != 0 {
                    ivars.pressed.set(ivars.pressed.get() & !button_bit(button));
                    self.report_pointer(
                        PointerPhase::Up,
                        self.content_point(event.locationInWindow()),
                        button,
                        event,
                        event.clickCount(),
                    );
                }
            }
            // The "back" side button of a mouse.
            None if event.buttonNumber() == 3 => ivars
                .events
                .push(BackendEvent::BackRequested { window: ivars.id }),
            None => {}
        }
        self.dispatch(event);
    }

    fn mouse_dragged(&self, event: &NSEvent) {
        let ivars = self.ivars();
        if let Some(button) = self
            .reported_button(event)
            .filter(|button| ivars.pressed.get() & button_bit(*button) != 0)
        {
            self.report_pointer(
                PointerPhase::Move,
                self.content_point(event.locationInWindow()),
                button,
                event,
                1,
            );
            if ivars.drag_requested.replace(false) {
                self.performWindowDragWithEvent(event);
                return self.release_if_swallowed(button, event);
            }
        }
        self.dispatch(event);
    }

    /// Starts moving the window with the press being handled.
    pub fn begin_drag(&self) {
        let ivars = self.ivars();
        if ivars.reporting_pointer.get() {
            // `sendEvent:` starts the move once the core has finished with
            // the event: the system's tracking loop must not run inside it.
            ivars.drag_requested.set(true);
            return;
        }
        let Some(press) = ivars.last_press.borrow().clone() else {
            return;
        };
        if ivars.pressed.get() == 0 {
            return;
        }
        let window = self.retain();
        ivars.events.defer(move || {
            window.performWindowDragWithEvent(&press);
            let button = window.ivars().left_button.get();
            window.release_if_swallowed(button, &press);
        });
    }

    /// The press being handled, for drags that start from it.
    pub fn press_event(&self) -> Option<Retained<NSEvent>> {
        self.ivars().last_press.borrow().clone()
    }

    fn focus_moved(&self) {
        let ivars = self.ivars();
        let node = self
            .firstResponder()
            .and_then(|responder| responder_view(&responder))
            .and_then(|view| node_view_of(&view));
        let next = node.as_ref().map(|node| node.node_id());
        let previous = {
            let mut focused = ivars.focused.borrow_mut();
            if focused.as_ref().map(|(id, _)| *id) == next {
                return;
            }
            std::mem::replace(
                &mut *focused,
                node.as_ref()
                    .map(|node| (node.node_id(), Weak::from_retained(node))),
            )
        };
        if let Some((id, view)) = previous {
            if let Some(view) = view.load() {
                view.notify_focus(false);
            }
            ivars
                .events
                .push(BackendEvent::FocusChanged { id, focused: false });
        }
        if let Some(node) = node {
            node.notify_focus(true);
            ivars.events.push(BackendEvent::FocusChanged {
                id: node.node_id(),
                focused: true,
            });
        }
    }
}

pub fn window_state(window: &NSWindow) -> WindowState {
    let full_screen = window.styleMask().contains(NSWindowStyleMask::FullScreen);
    WindowState {
        // A full screen window also reports itself as zoomed.
        maximized: window.isZoomed() && !full_screen,
        minimized: window.isMiniaturized(),
        full_screen,
        focused: window.isKeyWindow(),
    }
}

/// Size of the content area and device pixels per logical unit.
pub fn content_metrics(window: &NSWindow) -> (Size, f32) {
    let size = window
        .contentView()
        .map_or(Size::ZERO, |content| util::size(content.bounds().size));
    (size, window.backingScaleFactor() as f32)
}

pub struct DelegateIvars {
    id: WindowId,
    events: Rc<Events>,
    /// What was last reported, to report each change once.
    state: Cell<WindowState>,
    metrics: Cell<(Size, f32)>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements. Main thread only.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = DelegateIvars]
    pub struct WindowDelegate;

    unsafe impl NSObjectProtocol for WindowDelegate {}

    unsafe impl NSWindowDelegate for WindowDelegate {
        /// The core decides whether the window closes.
        #[unsafe(method(windowShouldClose:))]
        fn window_should_close(&self, _sender: &NSWindow) -> bool {
            let ivars = self.ivars();
            ivars
                .events
                .push(BackendEvent::WindowCloseRequested { window: ivars.id });
            false
        }

        #[unsafe(method(windowDidResize:))]
        fn window_did_resize(&self, notification: &NSNotification) {
            self.report(notification);
        }

        #[unsafe(method(windowDidChangeBackingProperties:))]
        fn window_did_change_backing_properties(&self, notification: &NSNotification) {
            self.report(notification);
        }

        #[unsafe(method(windowDidBecomeKey:))]
        fn window_did_become_key(&self, notification: &NSNotification) {
            let ivars = self.ivars();
            ivars.events.push(BackendEvent::WindowFocusChanged {
                window: ivars.id,
                focused: true,
            });
            self.report(notification);
        }

        #[unsafe(method(windowDidResignKey:))]
        fn window_did_resign_key(&self, notification: &NSNotification) {
            let ivars = self.ivars();
            ivars.events.push(BackendEvent::WindowFocusChanged {
                window: ivars.id,
                focused: false,
            });
            self.report(notification);
        }

        #[unsafe(method(windowDidMiniaturize:))]
        fn window_did_miniaturize(&self, notification: &NSNotification) {
            self.report(notification);
        }

        #[unsafe(method(windowDidDeminiaturize:))]
        fn window_did_deminiaturize(&self, notification: &NSNotification) {
            self.report(notification);
        }

        #[unsafe(method(windowDidEnterFullScreen:))]
        fn window_did_enter_full_screen(&self, notification: &NSNotification) {
            self.report(notification);
        }

        #[unsafe(method(windowDidExitFullScreen:))]
        fn window_did_exit_full_screen(&self, notification: &NSNotification) {
            self.report(notification);
        }
    }
);

impl WindowDelegate {
    pub fn new(
        mtm: MainThreadMarker,
        id: WindowId,
        events: &Rc<Events>,
        window: &NSWindow,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(DelegateIvars {
            id,
            events: Rc::clone(events),
            state: Cell::new(window_state(window)),
            metrics: Cell::new(content_metrics(window)),
        });
        // SAFETY: `init` is the designated initializer of `NSObject`.
        unsafe { msg_send![super(this), init] }
    }

    /// Reports whatever changed in the window's size, scale and state.
    fn report(&self, notification: &NSNotification) {
        let ivars = self.ivars();
        let Some(window) = notification
            .object()
            .and_then(|object| object.downcast::<NSWindow>().ok())
        else {
            return;
        };
        let metrics = content_metrics(&window);
        if ivars.metrics.replace(metrics) != metrics {
            ivars.events.push(BackendEvent::WindowResized {
                window: ivars.id,
                size: metrics.0,
                scale: metrics.1,
            });
        }
        let state = window_state(&window);
        if ivars.state.replace(state) != state {
            ivars.events.push(BackendEvent::WindowStateChanged {
                window: ivars.id,
                state,
            });
        }
    }
}

pub struct HostIvars {
    window: WindowId,
    events: Rc<Events>,
    /// The node whose text is being dragged out of the window.
    drag_source: Cell<Option<NodeId>>,
}

define_class!(
    // SAFETY: `NSView` allows subclassing and the overrides keep its
    // contracts. Main thread only.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = HostIvars]
    /// The content view of a window: the parent of the root node and the
    /// overlays, the pointer's tracking area and the drop target.
    pub struct HostView;

    impl HostView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(mouseDownCanMoveWindow))]
        fn mouse_down_can_move_window(&self) -> bool {
            false
        }

        /// The pointer moved with no button down (drags arrive through
        /// the window's `sendEvent:`).
        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) {
            self.report_pointer(PointerPhase::Move, util::point(self.convertPoint_fromView(event.locationInWindow(), None)), event);
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, event: &NSEvent) {
            self.report_pointer(PointerPhase::Leave, Point::ZERO, event);
        }

        /// The system (or the application) switched between light and dark.
        #[unsafe(method(viewDidChangeEffectiveAppearance))]
        fn view_did_change_effective_appearance(&self) {
            // SAFETY: forwards the same selector to `NSView`.
            let _: () = unsafe { msg_send![super(self), viewDidChangeEffectiveAppearance] };
            let ivars = self.ivars();
            // While the application forces an appearance, the one seen
            // here is not the system's.
            if !ivars.events.appearance_forced.get() {
                ivars.events.push(BackendEvent::AppearanceChanged { dark: crate::shell::is_dark(&self.effectiveAppearance()) });
            }
        }
    }

    unsafe impl NSObjectProtocol for HostView {}

    unsafe impl NSDraggingDestination for HostView {
        #[unsafe(method(draggingEntered:))]
        fn dragging_entered(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
            Self::drop_operation(sender)
        }

        #[unsafe(method(draggingUpdated:))]
        fn dragging_updated(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
            Self::drop_operation(sender)
        }

        #[unsafe(method(performDragOperation:))]
        fn perform_drag_operation(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> bool {
            let ivars = self.ivars();
            let position = util::point(self.convertPoint_fromView(sender.draggingLocation(), None));
            let pasteboard = sender.draggingPasteboard();
            let paths = dropped_files(&pasteboard);
            let event = if !paths.is_empty() {
                Some(BackendEvent::FilesDropped { window: ivars.window, position, paths })
            } else {
                // SAFETY: the type name is an immutable string exported by AppKit.
                pasteboard
                    .stringForType(unsafe { NSPasteboardTypeString })
                    .map(|text| BackendEvent::TextDropped { window: ivars.window, position, text: text.to_string() })
            };
            let accepted = event.is_some();
            if let Some(event) = event {
                // Handled once the drag session has finished with the view.
                ivars.events.push_later(event);
            }
            accepted
        }
    }

    unsafe impl NSDraggingSource for HostView {
        #[unsafe(method(draggingSession:sourceOperationMaskForDraggingContext:))]
        fn source_operation_mask(&self, _session: &NSDraggingSession, _context: NSDraggingContext) -> NSDragOperation {
            NSDragOperation::Copy
        }

        #[unsafe(method(draggingSession:endedAtPoint:operation:))]
        fn dragging_session_ended(&self, _session: &NSDraggingSession, _point: NSPoint, operation: NSDragOperation) {
            let ivars = self.ivars();
            if let Some(source) = ivars.drag_source.take() {
                ivars.events.push_later(BackendEvent::DragFinished { source, dropped: operation != NSDragOperation::None });
            }
        }
    }
);

/// The files of a drag, when it carries any.
fn dropped_files(pasteboard: &NSPasteboard) -> Vec<PathBuf> {
    let classes: Retained<NSArray<AnyClass>> = NSArray::from_slice(&[NSURL::class()]);
    let yes = NSNumber::new_bool(true);
    // SAFETY: the key is an immutable string exported by AppKit.
    let key: &NSString = unsafe { NSPasteboardURLReadingFileURLsOnlyKey };
    let options: Retained<NSDictionary<NSString, AnyObject>> =
        NSDictionary::from_slices(&[key], &[yes.as_ref()]);
    // SAFETY: `NSURL` is a pasteboard-reading class and the option takes a
    // boolean number; the result holds instances of the classes asked for.
    let Some(objects) =
        (unsafe { pasteboard.readObjectsForClasses_options(&classes, Some(&options)) })
    else {
        return Vec::new();
    };
    objects
        .iter()
        .filter_map(|object| object.downcast::<NSURL>().ok())
        .filter_map(|url| url.path())
        .map(|path| PathBuf::from(path.to_string()))
        .collect()
}

impl HostView {
    pub fn new(mtm: MainThreadMarker, window: WindowId, events: &Rc<Events>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(HostIvars {
            window,
            events: Rc::clone(events),
            drag_source: Cell::new(None),
        });
        // SAFETY: `initWithFrame:` is the designated initializer of `NSView`.
        let view: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: CGRect::ZERO] };
        view.setWantsLayer(true);
        // One area that always matches the visible part of the view; it
        // reports while the window is in the background too (hover).
        let options = NSTrackingAreaOptions::MouseEnteredAndExited
            | NSTrackingAreaOptions::MouseMoved
            | NSTrackingAreaOptions::ActiveAlways
            | NSTrackingAreaOptions::InVisibleRect;
        // SAFETY: the owner implements the mouse methods above; a tracking
        // area does not retain its owner, and it is removed with the view.
        let area = unsafe {
            NSTrackingArea::initWithRect_options_owner_userInfo(
                NSTrackingArea::alloc(),
                CGRect::ZERO,
                options,
                Some(&view),
                None,
            )
        };
        view.addTrackingArea(&area);
        // SAFETY: the type names are immutable strings exported by AppKit.
        let types =
            unsafe { NSArray::from_slice(&[NSPasteboardTypeFileURL, NSPasteboardTypeString]) };
        view.registerForDraggedTypes(&types);
        view
    }

    fn report_pointer(&self, phase: PointerPhase, position: Point, event: &NSEvent) {
        let ivars = self.ivars();
        ivars.events.push(BackendEvent::Pointer(PointerEvent {
            window: ivars.window,
            phase,
            position,
            button: PointerButton::Primary,
            modifiers: events::modifiers(event.modifierFlags()),
            click_count: 1,
            time_ms: events::time_ms(event),
        }));
    }

    fn drop_operation(sender: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
        // SAFETY: the type names are immutable strings exported by AppKit.
        let types =
            unsafe { NSArray::from_slice(&[NSPasteboardTypeFileURL, NSPasteboardTypeString]) };
        if sender
            .draggingPasteboard()
            .availableTypeFromArray(&types)
            .is_some()
        {
            NSDragOperation::Copy
        } else {
            NSDragOperation::None
        }
    }

    /// Remembers which node a drag session started from.
    pub fn set_drag_source(&self, source: NodeId) {
        self.ivars().drag_source.set(Some(source));
    }
}

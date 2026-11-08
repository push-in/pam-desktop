//! How native callbacks reach the runtime: the event queue, the wake-up of
//! the main thread from other threads, and the translation of `NSEvent`s.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Mutex;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{AnyThread, define_class, msg_send, sel};
use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventType};
use objc2_foundation::NSObjectNSThreadPerformAdditions;
use pam_desktop_ui::backend::{BackendEvent, Modifiers};
use pam_desktop_ui::runtime::{self, EventQueue};

/// Work other threads hand to the main thread (decoded images…).
static MAIN_JOBS: Mutex<Vec<Box<dyn FnOnce() + Send>>> = Mutex::new(Vec::new());

thread_local! {
    /// A drain of the runtime is already scheduled on the main run loop.
    static PUMP_SCHEDULED: Cell<bool> = const { Cell::new(false) };
    /// Work the main thread postponed to the next turn of its run loop.
    static DEFERRED: RefCell<Vec<Box<dyn FnOnce()>>> = const { RefCell::new(Vec::new()) };
    /// The queue of the running backend, for work that arrives from other
    /// threads and so cannot carry a reference to it.
    static CURRENT: RefCell<Option<Rc<Events>>> = const { RefCell::new(None) };
}

/// Drops the work that was still postponed when the run loop ended, and
/// with it the native objects that work kept alive.
pub fn discard_deferred() {
    let deferred = DEFERRED.with(|deferred| std::mem::take(&mut *deferred.borrow_mut()));
    drop(deferred);
}

/// Reports an event from a job other threads handed to the main thread.
pub fn post(event: BackendEvent) {
    let events = CURRENT.with(|current| current.borrow().clone());
    if let Some(events) = events {
        events.push(event);
    }
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements; the class has no
    // instance variables and does not implement `Drop`.
    #[unsafe(super(NSObject))]
    struct MainPump;

    impl MainPump {
        /// Runs on the main thread, from the run loop.
        #[unsafe(method(pump:))]
        fn pump(&self, _argument: Option<&AnyObject>) {
            // Cleared first: a job may run a nested run loop (a menu), and
            // events reported meanwhile must be able to schedule a drain.
            PUMP_SCHEDULED.with(|scheduled| scheduled.set(false));
            let jobs = MAIN_JOBS.lock().map(|mut jobs| std::mem::take(&mut *jobs)).unwrap_or_default();
            for job in jobs {
                job();
            }
            let deferred = DEFERRED.with(|deferred| std::mem::take(&mut *deferred.borrow_mut()));
            for job in deferred {
                job();
            }
            runtime::pump();
        }
    }
);

/// A handle any thread can use to wake the main thread. The wake-up is
/// queued in the common run loop modes, so it also arrives while a menu is
/// tracking, a window is being resized or a modal panel is open.
#[derive(Clone)]
pub struct MainThread {
    pump: Retained<MainPump>,
}

impl MainThread {
    pub fn new() -> Self {
        let this = MainPump::alloc().set_ivars(());
        // SAFETY: `init` is the designated initializer of `NSObject`.
        let pump: Retained<MainPump> = unsafe { msg_send![super(this), init] };
        Self { pump }
    }

    /// Schedules a drain of the runtime on the main thread.
    pub fn wake(&self) {
        // SAFETY: `pump:` is implemented by `MainPump` and takes one object
        // argument, here nil. The call only enqueues the message.
        unsafe {
            self.pump
                .performSelectorOnMainThread_withObject_waitUntilDone(sel!(pump:), None, false)
        };
    }

    /// Runs `job` on the main thread, then drains the runtime.
    pub fn run(&self, job: impl FnOnce() + Send + 'static) {
        if let Ok(mut jobs) = MAIN_JOBS.lock() {
            jobs.push(Box::new(job));
        }
        self.wake();
    }
}

/// The backend's side of the runtime's event queue.
///
/// Native callbacks may fire while the runtime is inside a `Backend` method
/// (a programmatic scroll reports its new offset, a focus change reports
/// itself). The runtime does not drain re-entrantly, so such events wait in
/// the queue and a drain is scheduled for the next turn of the run loop.
pub struct Events {
    queue: Rc<EventQueue>,
    main: MainThread,
    /// Tells whether the runtime is draining right now.
    busy: RefCell<Option<Box<dyn Fn() -> bool>>>,
    /// Set while a mouse-down is being dispatched by AppKit: a press must
    /// not move keyboard focus to a focusable box (only Tab does).
    pub dispatching_mouse_down: Cell<bool>,
    /// The last input was the keyboard: only then is the focus ring of a
    /// box drawn, like `:focus-visible` in GTK.
    pub focus_visible: Cell<bool>,
    /// The application forces light or dark chrome: the appearance views
    /// see is then not the system's.
    pub appearance_forced: Cell<bool>,
}

impl Events {
    pub fn new(queue: Rc<EventQueue>, main: MainThread) -> Rc<Self> {
        Rc::new(Self {
            queue,
            main,
            busy: RefCell::new(None),
            dispatching_mouse_down: Cell::new(false),
            focus_visible: Cell::new(false),
            appearance_forced: Cell::new(false),
        })
    }

    /// Makes this the queue `post` reports to; `None` forgets it.
    pub fn set_current(events: Option<&Rc<Self>>) {
        CURRENT.with(|current| *current.borrow_mut() = events.cloned());
    }

    pub fn main(&self) -> &MainThread {
        &self.main
    }

    /// Installed once the runtime exists; until then every event waits.
    pub fn set_busy_probe(&self, probe: impl Fn() -> bool + 'static) {
        *self.busy.borrow_mut() = Some(Box::new(probe));
    }

    fn is_busy(&self) -> bool {
        self.busy.borrow().as_ref().is_none_or(|probe| probe())
    }

    /// Reports an event, draining the runtime at once when it is free.
    pub fn push(&self, event: BackendEvent) {
        if self.is_busy() {
            self.push_later(event);
        } else {
            self.queue.push(event);
        }
    }

    /// Runs `job` on the next turn of the run loop, outside any `Backend`
    /// call: whatever spins a nested run loop (a menu, a window drag) or
    /// answers a request "later" must not hold the runtime while it does.
    pub fn defer(&self, job: impl FnOnce() + 'static) {
        DEFERRED.with(|deferred| deferred.borrow_mut().push(Box::new(job)));
        self.schedule_pump();
    }

    fn schedule_pump(&self) {
        if !PUMP_SCHEDULED.with(|scheduled| scheduled.replace(true)) {
            self.main.wake();
        }
    }

    /// Reports an event that is handled on the next turn of the run loop:
    /// the core may answer by changing the very control that is in the
    /// middle of reporting (an input mask replacing the text being typed).
    pub fn push_later(&self, event: BackendEvent) {
        self.queue.push_deferred(event);
        self.schedule_pump();
    }
}

pub fn modifiers(flags: NSEventModifierFlags) -> Modifiers {
    Modifiers {
        shift: flags.contains(NSEventModifierFlags::Shift),
        control: flags.contains(NSEventModifierFlags::Control),
        alt: flags.contains(NSEventModifierFlags::Option),
        meta: flags.contains(NSEventModifierFlags::Command),
    }
}

/// `NSEvent` timestamps are seconds since boot, the same clock as
/// `CACurrentMediaTime`.
pub fn time_ms(event: &NSEvent) -> u64 {
    (event.timestamp().max(0.0) * 1000.0) as u64
}

pub fn now_ms() -> u64 {
    (objc2_quartz_core::CACurrentMediaTime().max(0.0) * 1000.0) as u64
}

/// The W3C UI Events `key` value of a virtual key that has a name.
fn named_key(code: u16) -> Option<&'static str> {
    Some(match code {
        36 | 76 => "Enter",
        53 => "Escape",
        48 => "Tab",
        51 => "Backspace",
        117 => "Delete",
        114 => "Insert",
        123 => "ArrowLeft",
        124 => "ArrowRight",
        125 => "ArrowDown",
        126 => "ArrowUp",
        115 => "Home",
        119 => "End",
        116 => "PageUp",
        121 => "PageDown",
        49 => " ",
        56 | 60 => "Shift",
        59 | 62 => "Control",
        58 | 61 => "Alt",
        55 | 54 => "Meta",
        57 => "CapsLock",
        110 => "ContextMenu",
        122 => "F1",
        120 => "F2",
        99 => "F3",
        118 => "F4",
        96 => "F5",
        97 => "F6",
        98 => "F7",
        100 => "F8",
        101 => "F9",
        109 => "F10",
        103 => "F11",
        111 => "F12",
        105 => "F13",
        107 => "F14",
        113 => "F15",
        106 => "F16",
        64 => "F17",
        79 => "F18",
        80 => "F19",
        90 => "F20",
        _ => return None,
    })
}

/// The W3C `key` value of a key-down, key-up or flags-changed event.
pub fn key_name(event: &NSEvent) -> String {
    if let Some(name) = named_key(event.keyCode()) {
        return name.to_owned();
    }
    if event.r#type() == NSEventType::FlagsChanged {
        return String::from("Unidentified");
    }
    // Ignoring Control, Option and Command (but not Shift) gives the letter
    // of a chord instead of a control code or an alternate glyph.
    let text = event
        .charactersIgnoringModifiers()
        .map(|characters| characters.to_string())
        .unwrap_or_default();
    let mut characters = text.chars();
    match (characters.next(), characters.next()) {
        // Function keys without a W3C name live in a private use area.
        (Some(character), None)
            if !character.is_control() && !('\u{F700}'..='\u{F8FF}').contains(&character) =>
        {
            text
        }
        _ => String::from("Unidentified"),
    }
}

/// Whether the modifier key of a flags-changed event went down.
pub fn modifier_pressed(event: &NSEvent) -> bool {
    let flags = event.modifierFlags();
    match event.keyCode() {
        56 | 60 => flags.contains(NSEventModifierFlags::Shift),
        59 | 62 => flags.contains(NSEventModifierFlags::Control),
        58 | 61 => flags.contains(NSEventModifierFlags::Option),
        55 | 54 => flags.contains(NSEventModifierFlags::Command),
        57 => flags.contains(NSEventModifierFlags::CapsLock),
        _ => false,
    }
}

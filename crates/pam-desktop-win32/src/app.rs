//! The UI thread's shared state and plumbing: the retained scene behind a
//! `RefCell`, the message-only window that receives wake-ups, the frame
//! clock thread and the queue of deferred (modal) work.
//!
//! Borrowing rule: nothing that can dispatch a window message (creating,
//! moving, focusing or destroying a window, modal loops, cross-process COM
//! calls) runs while `App::state` is borrowed. Backend methods and window
//! procedures look things up under a short borrow, release it, and only
//! then call into Win32. Events are queued with `App::emit` and the runtime
//! is drained by `App::pump` once no borrow is held.

use std::cell::{Cell, RefCell, RefMut};
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

use pam_desktop_ui::backend::BackendEvent;
use pam_desktop_ui::color::Color;
use pam_desktop_ui::runtime::{self, EventQueue};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Dwm::DwmFlush;
use windows::Win32::Graphics::Gdi::{GetSysColor, COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT, COLOR_WINDOW, COLOR_WINDOWTEXT};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, IsIconic, IsWindowVisible, KillTimer, PostMessageW, RegisterClassExW, SetTimer, HWND_MESSAGE, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_APP, WM_HOTKEY, WM_TIMER, WNDCLASSEXW,
};

use crate::gfx::Graphics;
use crate::images::Images;
use crate::paint::ShadowCache;
use crate::scene::Scene;
use crate::scroll::ScrollContext;
use crate::shell::ShellState;
use crate::text::TextEngine;
use crate::util::from_colorref;

/// Another thread posted to the runtime's inbox.
pub const WM_APP_WAKE: u32 = WM_APP + 1;
/// The frame clock thread saw a vertical blank.
pub const WM_APP_FRAME: u32 = WM_APP + 2;
/// Deferred tasks are waiting.
pub const WM_APP_TASKS: u32 = WM_APP + 3;
/// Worker threads left mail.
pub const WM_APP_MAIL: u32 = WM_APP + 4;
/// Queued events must be drained once the current message is done.
pub const WM_APP_PUMP: u32 = WM_APP + 5;
/// The notification area icon was used (sent to the tray window).
pub const WM_APP_TRAY: u32 = WM_APP + 6;

/// Shortest time between two frame messages.
const MIN_FRAME_INTERVAL: std::time::Duration = std::time::Duration::from_millis(4);

const TIMER_WAKE: usize = 1;
const TIMER_BARS: usize = 2;

const MESSAGE_CLASS: PCWSTR = w!("PamDesktopMessages");

/// Events on their way to the core. Pushing never drains the runtime (the
/// state may be borrowed); `App::pump` does, and only when something is
/// waiting.
pub struct Outbox {
    queue: Rc<EventQueue>,
    pending: Cell<bool>,
}

impl Outbox {
    pub fn push(&self, event: BackendEvent) {
        self.queue.push_deferred(event);
        self.pending.set(true);
    }
}

/// Work that runs from the message loop, outside the runtime and outside
/// any state borrow (modal loops, callbacks into the runtime).
pub type Task = Box<dyn FnOnce(&Rc<App>)>;

/// Results that worker threads hand to the UI thread.
pub enum Mail {
    Image(crate::images::Decoded),
    /// A toast notification was activated.
    Notification(String),
    /// A Media Foundation engine event: player token, event, parameters.
    Media { player: u64, event: u32, first: usize, second: u32 },
}

pub struct Mailbox {
    queue: Mutex<Vec<Mail>>,
    /// The message-only window, as an integer so the mailbox is `Send`.
    hwnd: isize,
}

impl Mailbox {
    pub fn post(&self, mail: Mail) {
        if let Ok(mut queue) = self.queue.lock() {
            queue.push(mail);
        }
        // SAFETY: posting to a window handle is valid from any thread; a
        // stale handle only makes the call fail.
        unsafe {
            let _ = PostMessageW(Some(HWND(self.hwnd as *mut _)), WM_APP_MAIL, WPARAM(0), LPARAM(0));
        }
    }

    fn take(&self) -> Vec<Mail> {
        self.queue.lock().map(|mut queue| std::mem::take(&mut *queue)).unwrap_or_default()
    }

    /// Wakes the UI thread so it drains the runtime's inbox.
    pub fn wake(&self) {
        // SAFETY: as in `post`.
        unsafe {
            let _ = PostMessageW(Some(HWND(self.hwnd as *mut _)), WM_APP_WAKE, WPARAM(0), LPARAM(0));
        }
    }
}

struct ClockState {
    requested: bool,
    stopped: bool,
}

struct ClockShared {
    state: Mutex<ClockState>,
    signal: Condvar,
    hwnd: isize,
}

/// Delivers one `WM_APP_FRAME` per request, on the next vertical blank.
/// The thread sleeps on a condition variable while nobody asks.
pub struct FrameClock {
    shared: Arc<ClockShared>,
}

impl FrameClock {
    fn new(hwnd: HWND) -> Self {
        let shared = Arc::new(ClockShared { state: Mutex::new(ClockState { requested: false, stopped: false }), signal: Condvar::new(), hwnd: hwnd.0 as isize });
        let thread_shared = Arc::clone(&shared);
        let spawned = std::thread::Builder::new().name("pam-frame-clock".into()).spawn(move || loop {
            {
                let Ok(mut state) = thread_shared.state.lock() else { return };
                while !state.requested && !state.stopped {
                    state = match thread_shared.signal.wait(state) {
                        Ok(state) => state,
                        Err(_) => return,
                    };
                }
                if state.stopped {
                    return;
                }
                state.requested = false;
            }
            let started = Instant::now();
            // SAFETY: DwmFlush blocks until the compositor's next pass and
            // has no preconditions; PostMessageW is valid from any thread.
            unsafe {
                if DwmFlush().is_err() {
                    std::thread::sleep(std::time::Duration::from_millis(16));
                }
                // With the display off or in some remote sessions the flush
                // returns at once: frames are never delivered faster than this.
                let waited = started.elapsed();
                if waited < MIN_FRAME_INTERVAL {
                    std::thread::sleep(MIN_FRAME_INTERVAL - waited);
                }
                let _ = PostMessageW(Some(HWND(thread_shared.hwnd as *mut _)), WM_APP_FRAME, WPARAM(0), LPARAM(0));
            }
        });
        // Without the thread, frames never arrive; animations then jump to
        // their end through the core's timers. Nothing else depends on it.
        drop(spawned);
        Self { shared }
    }

    pub fn request(&self) {
        if let Ok(mut state) = self.shared.state.lock() {
            state.requested = true;
        }
        self.shared.signal.notify_one();
    }

    fn stop(&self) {
        if let Ok(mut state) = self.shared.state.lock() {
            state.stopped = true;
        }
        self.shared.signal.notify_one();
    }
}

/// Colors that follow the light or dark appearance.
#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub dark: bool,
    /// Behind every view.
    pub window: Color,
    /// Text runs without a color of their own.
    pub text: Color,
    pub highlight: Color,
    pub highlight_text: Color,
    /// Focus rings.
    pub accent: Color,
}

impl Theme {
    pub fn new(dark: bool) -> Self {
        // SAFETY: GetSysColor has no preconditions.
        let (window, text, highlight, highlight_text) = unsafe {
            (
                from_colorref(GetSysColor(COLOR_WINDOW)),
                from_colorref(GetSysColor(COLOR_WINDOWTEXT)),
                from_colorref(GetSysColor(COLOR_HIGHLIGHT)),
                from_colorref(GetSysColor(COLOR_HIGHLIGHTTEXT)),
            )
        };
        if dark {
            // The classic system colors have no dark variant.
            Self { dark, window: Color::rgba(32, 32, 32, 255), text: Color::rgba(255, 255, 255, 255), highlight, highlight_text, accent: highlight }
        } else {
            Self { dark, window, text, highlight, highlight_text, accent: highlight }
        }
    }
}

pub struct State {
    pub graphics: Graphics,
    pub text: TextEngine,
    pub scene: Scene,
    pub images: Images,
    pub shadows: ShadowCache,
    pub shell: ShellState,
    pub media: crate::media::MediaShared,
    pub theme: Theme,
    pub system_dark: bool,
    /// The appearance the application forced, if any.
    pub forced_dark: Option<bool>,
    pub reduced_motion: bool,
    /// Windows are kept out of screenshots and recordings.
    pub content_protected: bool,
    /// The core asked for a `Frame` event.
    pub core_frame_requested: bool,
    /// A `WM_APP_FRAME` is on its way.
    tick_pending: bool,
    last_tick_ms: u64,
    pub events: Rc<Outbox>,
    clock: FrameClock,
    pub message_window: HWND,
    started: Instant,
}

impl State {
    pub fn now_ms(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// Asks for one internal tick on the next vertical blank.
    pub fn request_tick(&mut self) {
        if !self.tick_pending {
            self.tick_pending = true;
            self.clock.request();
        }
    }

    /// Whether some window is on screen. Frames for the core are held
    /// back while none is (minimized, hidden): its animations wait.
    fn any_window_visible(&self) -> bool {
        self.scene.windows.values().any(|entry| {
            // SAFETY: plain queries on live windows; they send no message.
            entry.presented && unsafe { IsWindowVisible(entry.hwnd).as_bool() && !IsIconic(entry.hwnd).as_bool() }
        })
    }

    /// A window is visible again: a frame the core is waiting for is delivered.
    pub fn resume_frames(&mut self) {
        if self.core_frame_requested {
            self.request_tick();
        }
    }

    pub fn scrolling(&mut self) -> ScrollContext<'_> {
        let now_ms = self.now_ms();
        ScrollContext { scene: &mut self.scene, events: &self.events, now_ms, reduced_motion: self.reduced_motion }
    }

    /// Schedules what active scroll views need next: a tick while one
    /// animates or fades, a timer while its bars only wait to fade.
    pub fn schedule_scroll_work(&mut self) {
        let now = self.now_ms();
        let mut animating = false;
        let mut deadline: Option<u64> = None;
        for id in &self.scene.active_scrolls {
            let Some(scroll) = self.scene.views.get(id).and_then(|view| view.scroll.as_ref()) else { continue };
            if scroll.animation != crate::scroll::Animation::None {
                animating = true;
            } else if scroll.bar_opacity > 0.0 && !scroll.bar_held {
                if now >= scroll.bar_deadline {
                    animating = true;
                } else {
                    deadline = Some(deadline.map_or(scroll.bar_deadline, |known| known.min(scroll.bar_deadline)));
                }
            }
        }
        if animating {
            self.request_tick();
        } else if let Some(deadline) = deadline {
            let delay = u32::try_from(deadline.saturating_sub(now)).unwrap_or(u32::MAX).max(1);
            // SAFETY: SetTimer on our own window; it replaces a pending timer with the same id.
            unsafe { SetTimer(Some(self.message_window), TIMER_BARS, delay, None) };
        }
    }

    /// The appearance in effect (forced by the application or the system's).
    pub fn dark(&self) -> bool {
        self.forced_dark.unwrap_or(self.system_dark)
    }

    /// Drops every device resource after a device loss. Windows repaint
    /// with a new device; images decode again from their sources.
    pub fn device_lost(&mut self) {
        // Swap chains go first: a new one cannot be made for a window while
        // the old one still exists, and Direct3D defers destruction until
        // the context is flushed.
        for entry in self.scene.windows.values_mut() {
            entry.surface = None;
        }
        if let Some(device) = &self.graphics.device {
            // SAFETY: plain calls on the live immediate context.
            unsafe {
                device.d3d_context.ClearState();
                device.d3d_context.Flush();
            }
        }
        self.shadows.clear();
        for view in self.scene.views.values_mut() {
            view.cache = None;
            if let Some(text) = &mut view.text {
                // Layouts hold brushes of the old device as drawing effects.
                text.layout = None;
                text.colored_for = 0;
            }
        }
        self.images.device_lost();
        crate::media::device_lost(self);
        self.graphics.lose_device();
        let windows: Vec<_> = self.scene.windows.keys().copied().collect();
        for window in windows {
            self.scene.damage_all(window);
        }
    }
}

pub struct App {
    pub events: Rc<Outbox>,
    pub state: RefCell<State>,
    pub mailbox: Arc<Mailbox>,
    pub instance: HINSTANCE,
    pub application_id: String,
    pub message_window: HWND,
    tasks: RefCell<VecDeque<Task>>,
    tasks_posted: Cell<bool>,
    pump_posted: Cell<bool>,
    /// A paint could not run because the state was busy; repaint later.
    repaint_pending: Cell<bool>,
    /// The runtime is draining (it is on the stack).
    draining: Cell<bool>,
    /// `quit` ran: windows are gone and the loop is ending.
    pub quitting: Cell<bool>,
}

thread_local! {
    static APP: RefCell<Option<Rc<App>>> = const { RefCell::new(None) };
}

impl App {
    /// The application of the calling thread (the UI thread).
    pub fn current() -> Option<Rc<Self>> {
        APP.with(|slot| slot.borrow().clone())
    }

    /// Creates the message window, the graphics factories and the shared
    /// state, and installs them for this thread.
    pub fn install(instance: HINSTANCE, application_id: &str, events: Rc<EventQueue>) -> windows::core::Result<Rc<Self>> {
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(message_proc),
            hInstance: instance,
            lpszClassName: MESSAGE_CLASS,
            ..Default::default()
        };
        // SAFETY: the class structure is fully initialized and the window
        // procedure is a valid `extern "system"` function for the process lifetime.
        let message_window = unsafe {
            // Registering twice (a second `run` in the same process) fails harmlessly.
            RegisterClassExW(&class);
            CreateWindowExW(WINDOW_EX_STYLE(0), MESSAGE_CLASS, PCWSTR::null(), WINDOW_STYLE(0), 0, 0, 0, 0, Some(HWND_MESSAGE), None, Some(instance), None)?
        };
        let graphics = Graphics::new()?;
        let text = TextEngine::new(&graphics.dwrite);
        let system_dark = crate::shell::system_prefers_dark();
        let events = Rc::new(Outbox { queue: events, pending: Cell::new(false) });
        let mailbox = Arc::new(Mailbox { queue: Mutex::new(Vec::new()), hwnd: message_window.0 as isize });
        let state = State {
            graphics,
            text,
            scene: Scene::default(),
            images: Images::default(),
            shadows: ShadowCache::default(),
            shell: ShellState::default(),
            media: crate::media::MediaShared::default(),
            theme: Theme::new(system_dark),
            system_dark,
            forced_dark: None,
            reduced_motion: crate::shell::reduced_motion(),
            content_protected: false,
            core_frame_requested: false,
            tick_pending: false,
            last_tick_ms: 0,
            events: Rc::clone(&events),
            clock: FrameClock::new(message_window),
            message_window,
            started: Instant::now(),
        };
        let app = Rc::new(Self {
            events,
            state: RefCell::new(state),
            mailbox,
            instance,
            application_id: application_id.to_owned(),
            message_window,
            tasks: RefCell::new(VecDeque::new()),
            tasks_posted: Cell::new(false),
            pump_posted: Cell::new(false),
            repaint_pending: Cell::new(false),
            draining: Cell::new(false),
            quitting: Cell::new(false),
        });
        APP.with(|slot| *slot.borrow_mut() = Some(Rc::clone(&app)));
        Ok(app)
    }

    /// Releases the thread's application: stops the clock, frees device
    /// resources and destroys the message window.
    pub fn uninstall(&self) {
        if let Ok(state) = self.state.try_borrow() {
            state.clock.stop();
        }
        APP.with(|slot| *slot.borrow_mut() = None);
        // SAFETY: the message window belongs to this thread.
        unsafe {
            let _ = DestroyWindow(self.message_window);
        }
    }

    /// The state, for code that runs outside any other borrow. `None`
    /// means a caller up the stack still holds it (a broken borrowing
    /// rule): the operation is skipped instead of aborting the process.
    pub fn state(&self) -> Option<RefMut<'_, State>> {
        let state = self.state.try_borrow_mut().ok();
        debug_assert!(state.is_some(), "pam-desktop-win32: state borrowed re-entrantly");
        state
    }

    /// Runs `call` with the state when it is free (window procedures that
    /// may be entered while a borrow is held fall back to the default).
    pub fn with_state<T>(&self, call: impl FnOnce(&mut State) -> T) -> Option<T> {
        self.state.try_borrow_mut().ok().map(|mut state| call(&mut state))
    }

    pub fn now_ms(&self) -> u64 {
        self.with_state(|state| state.now_ms()).unwrap_or(0)
    }

    /// Queues an event for the core; `pump` delivers it.
    pub fn emit(&self, event: BackendEvent) {
        self.events.push(event);
    }

    /// Queues an event and drains once the message being handled is over.
    /// Native controls report through here: the core may answer by
    /// changing or destroying the control that is still inside its own
    /// window procedure.
    pub fn emit_later(&self, event: BackendEvent) {
        self.events.push(event);
        self.post_pump();
    }

    pub fn post_pump(&self) {
        if !self.pump_posted.replace(true) {
            // SAFETY: posting to our own message window.
            unsafe {
                let _ = PostMessageW(Some(self.message_window), WM_APP_PUMP, WPARAM(0), LPARAM(0));
            }
        }
    }

    /// Asks for a drain after the current message when events are waiting
    /// (for code running inside a control's own procedure).
    pub fn post_pump_if_pending(&self) {
        if self.events.pending.get() {
            self.post_pump();
        }
    }

    /// Something outside the event queue (the inbox, a callback into the
    /// runtime) needs a drain at the next `pump`.
    pub fn mark_pending(&self) {
        self.events.pending.set(true);
    }

    /// Drains the runtime when events are waiting, unless the state is
    /// borrowed up the stack (the drain would call back into the backend):
    /// then it is postponed.
    pub fn pump(&self) {
        if !self.events.pending.get() && !self.repaint_pending.get() {
            return;
        }
        // A drain up the stack only picks new events up while it is still
        // in its event loop; whatever it leaves is drained right after it
        // returns (see below).
        if self.draining.get() {
            return;
        }
        if self.state.try_borrow_mut().is_ok() {
            if self.events.pending.replace(false) {
                self.draining.set(true);
                runtime::pump();
                self.draining.set(false);
                // Events queued while the runtime was flushing to the
                // backend were not seen by that drain.
                if self.events.pending.get() {
                    self.post_pump();
                }
            }
            if self.repaint_pending.replace(false) {
                self.with_state(|state| {
                    let windows: Vec<_> = state.scene.windows.keys().copied().collect();
                    for window in windows {
                        state.scene.damage_all(window);
                    }
                });
            }
        } else {
            self.post_pump();
        }
    }

    pub fn note_missed_paint(&self) {
        self.repaint_pending.set(true);
        self.post_pump();
    }

    /// Runs `task` from the message loop, after the current message.
    pub fn defer(&self, task: Task) {
        self.tasks.borrow_mut().push_back(task);
        if !self.tasks_posted.replace(true) {
            // SAFETY: posting to our own message window.
            unsafe {
                let _ = PostMessageW(Some(self.message_window), WM_APP_TASKS, WPARAM(0), LPARAM(0));
            }
        }
    }

    fn run_tasks(self: &Rc<Self>) {
        self.tasks_posted.set(false);
        loop {
            // The queue borrow ends before the task runs: tasks may defer more.
            let Some(task) = self.tasks.borrow_mut().pop_front() else { break };
            task(self);
        }
        // Tasks answer the runtime through callbacks.
        self.mark_pending();
    }

    /// One vertical blank: internal animations advance and the core gets
    /// its `Frame` when it asked for one.
    fn on_frame(self: &Rc<Self>) {
        let frame = self.with_state(|state| {
            state.tick_pending = false;
            let now = state.now_ms();
            let elapsed = now.saturating_sub(state.last_tick_ms);
            // After a pause the first tick is one nominal frame, not the pause.
            let elapsed_ms = if elapsed == 0 || elapsed > 100 { 16.0 } else { elapsed as f32 };
            state.last_tick_ms = now;
            let mut again = state.scrolling().tick(elapsed_ms);
            again |= crate::media::tick(state);
            if again {
                state.request_tick();
            }
            state.schedule_scroll_work();
            // With nothing on screen the core's frame waits for a window
            // (see `resume_frames`): no animation runs unseen.
            if !state.any_window_visible() {
                return None;
            }
            std::mem::take(&mut state.core_frame_requested).then_some(now)
        });
        if let Some(Some(time_ms)) = frame {
            self.emit(BackendEvent::Frame { time_ms });
        }
    }

    fn on_mail(self: &Rc<Self>) {
        for mail in self.mailbox.take() {
            match mail {
                Mail::Image(decoded) => crate::images::decoded(self, decoded),
                Mail::Notification(id) => self.emit(BackendEvent::NotificationActivated { id }),
                Mail::Media { player, event, first, second } => crate::media::on_engine_event(self, player, event, first, second),
            }
        }
    }

    /// Arms the single wake timer (`schedule_wake`).
    pub fn schedule_wake(&self, delay_ms: u32) {
        // SAFETY: timers on our own message window; a new SetTimer with the
        // same id replaces the pending one.
        unsafe {
            if delay_ms == 0 {
                let _ = KillTimer(Some(self.message_window), TIMER_WAKE);
                let _ = PostMessageW(Some(self.message_window), WM_TIMER, WPARAM(TIMER_WAKE), LPARAM(0));
            } else {
                SetTimer(Some(self.message_window), TIMER_WAKE, delay_ms, None);
            }
        }
    }
}

/// Window procedure of the message-only window.
extern "system" fn message_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let handled = std::panic::catch_unwind(|| {
        let Some(app) = App::current() else { return false };
        match message {
            WM_APP_WAKE => app.mark_pending(),
            WM_APP_PUMP => app.pump_posted.set(false),
            WM_APP_TASKS => app.run_tasks(),
            WM_APP_FRAME => app.on_frame(),
            WM_APP_MAIL => app.on_mail(),
            WM_TIMER => match wparam.0 {
                TIMER_WAKE => {
                    // SAFETY: the timer belongs to this window.
                    unsafe {
                        let _ = KillTimer(Some(hwnd), TIMER_WAKE);
                    }
                    app.emit(BackendEvent::Wake);
                }
                TIMER_BARS => {
                    // SAFETY: as above.
                    unsafe {
                        let _ = KillTimer(Some(hwnd), TIMER_BARS);
                    }
                    app.with_state(State::schedule_scroll_work);
                }
                _ => return false,
            },
            WM_HOTKEY => crate::shell::hot_key(&app, wparam.0 as i32),
            _ => return false,
        }
        app.pump();
        true
    });
    match handled {
        Ok(true) => LRESULT(0),
        // SAFETY: forwarding the unmodified message to the default procedure.
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

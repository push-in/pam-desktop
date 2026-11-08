//! The frame clock and the wake-up timer. Neither runs while nothing was
//! asked of it: the display link is paused between frames and the timers
//! fire once.

use std::cell::{Cell, RefCell};
use std::ptr::NonNull;
use std::rc::{Rc, Weak};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::NSView;
use objc2_foundation::{NSRunLoop, NSRunLoopCommonModes, NSTimer};
use objc2_quartz_core::CADisplayLink;
use pam_desktop_ui::backend::BackendEvent;

use crate::events::{self, Events};

/// Frame period of the timer used where views have no display link.
const FALLBACK_FRAME: f64 = 1.0 / 60.0;

struct Shared {
    events: Rc<Events>,
    /// A frame was asked for and not delivered yet.
    requested: Cell<bool>,
    link: RefCell<Option<Retained<CADisplayLink>>>,
    /// A fallback timer shot is on its way.
    timer_pending: Cell<bool>,
}

impl Shared {
    fn tick(&self) {
        if self.requested.replace(false) {
            self.events.push(BackendEvent::Frame {
                time_ms: events::now_ms(),
            });
        }
        // Nobody asked for another frame while this one was handled: the
        // display link rests until the next request.
        if !self.requested.get() {
            let link = self.link.borrow().clone();
            if let Some(link) = link {
                link.setPaused(true);
            }
        }
    }
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements. Main thread only.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Weak<Shared>]
    /// What the display link and the fallback timer call. It only holds a
    /// weak reference: both retain their target.
    struct FrameTarget;

    impl FrameTarget {
        #[unsafe(method(displayTick:))]
        fn display_tick(&self, _link: &CADisplayLink) {
            if let Some(shared) = self.ivars().upgrade() {
                shared.tick();
            }
        }

        #[unsafe(method(timerTick:))]
        fn timer_tick(&self, _timer: &NSTimer) {
            if let Some(shared) = self.ivars().upgrade() {
                shared.timer_pending.set(false);
                shared.tick();
            }
        }
    }

    unsafe impl NSObjectProtocol for FrameTarget {}
);

pub struct FrameClock {
    shared: Rc<Shared>,
    target: Retained<FrameTarget>,
}

impl FrameClock {
    pub fn new(mtm: MainThreadMarker, events: &Rc<Events>) -> Self {
        let shared = Rc::new(Shared {
            events: Rc::clone(events),
            requested: Cell::new(false),
            link: RefCell::new(None),
            timer_pending: Cell::new(false),
        });
        let this = FrameTarget::alloc(mtm).set_ivars(Rc::downgrade(&shared));
        // SAFETY: `init` is the designated initializer of `NSObject`.
        let target: Retained<FrameTarget> = unsafe { msg_send![super(this), init] };
        Self { shared, target }
    }

    /// Asks for one frame on the next refresh of the display `view` is on.
    pub fn request(&self, view: Option<&NSView>) {
        let shared = &self.shared;
        if shared.requested.replace(true) {
            return;
        }
        let link = shared.link.borrow().clone();
        if let Some(link) = link {
            link.setPaused(false);
            return;
        }
        // Views vend display links since macOS 14; they follow the screen
        // the view is on and stop while it is not visible.
        if let Some(view) =
            view.filter(|view| view.respondsToSelector(sel!(displayLinkWithTarget:selector:)))
        {
            // SAFETY: the target implements `displayTick:`, which takes the
            // display link; the link is invalidated in `reset`.
            let link = unsafe {
                let link = view.displayLinkWithTarget_selector(&self.target, sel!(displayTick:));
                link.addToRunLoop_forMode(&NSRunLoop::mainRunLoop(), NSRunLoopCommonModes);
                link
            };
            *shared.link.borrow_mut() = Some(link);
            return;
        }
        if !shared.timer_pending.replace(true) {
            // SAFETY: the target implements `timerTick:`, which takes the
            // timer; a one-shot timer releases its target after firing.
            unsafe {
                let timer = NSTimer::timerWithTimeInterval_target_selector_userInfo_repeats(
                    FALLBACK_FRAME,
                    &self.target,
                    sel!(timerTick:),
                    None,
                    false,
                );
                NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes);
            }
        }
    }

    /// Drops the display link (its view is going away). A frame that was
    /// asked for is asked again by the caller.
    pub fn reset(&self) {
        let link = self.shared.link.borrow_mut().take();
        if let Some(link) = link {
            link.invalidate();
        }
        self.shared.requested.set(false);
    }

    pub fn is_requested(&self) -> bool {
        self.shared.requested.get()
    }
}

impl Drop for FrameClock {
    fn drop(&mut self) {
        self.reset();
    }
}

/// The single pending `Wake`.
#[derive(Default)]
pub struct WakeTimer {
    timer: Option<Retained<NSTimer>>,
}

impl WakeTimer {
    /// Reports one `Wake` after `delay_ms`, replacing a pending one.
    pub fn schedule(&mut self, events: &Rc<Events>, delay_ms: u32) {
        self.cancel();
        let events = Rc::clone(events);
        let block = RcBlock::new(move |_: NonNull<NSTimer>| events.push(BackendEvent::Wake));
        // SAFETY: the block runs on the main run loop, where the queue it
        // captures lives; the mode is an immutable string of Foundation.
        let timer = unsafe {
            let timer = NSTimer::timerWithTimeInterval_repeats_block(
                f64::from(delay_ms) / 1000.0,
                false,
                &block,
            );
            NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes);
            timer
        };
        self.timer = Some(timer);
    }

    pub fn cancel(&mut self) {
        if let Some(timer) = self.timer.take() {
            timer.invalidate();
        }
    }
}

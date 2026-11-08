//! Gives the heap memory freed by the interface back to the system once the
//! windows settle.
//!
//! glibc's allocator keeps what a frame freed (the widgets, layouts and
//! render nodes of rows scrolled away, of a screen left behind) in its
//! arenas for the next allocation, so the resident memory of the host only
//! grows. Half a second after the last painted frame, when the allocator
//! holds more than a megabyte of free memory, `malloc_trim` returns the free
//! pages: never while frames are painted, at most once per pause, and
//! nothing runs while nothing paints.

use gtk::glib;
use gtk::prelude::*;
use std::cell::Cell;
use std::time::{Duration, Instant};

/// Quiet time after the last painted frame before the heap is trimmed.
const SETTLE: Duration = Duration::from_millis(500);

/// The one later look at the heap after a pause.
const FOLLOW_UP: Duration = Duration::from_secs(3);

/// Free heap memory below this is not worth a trim.
#[cfg(target_env = "gnu")]
const WORTH_TRIMMING: usize = 1 << 20;

thread_local! {
    static LAST_PAINT: Cell<Option<Instant>> = const { Cell::new(None) };
    static ARMED: Cell<bool> = const { Cell::new(false) };
}

/// Trims the heap once `window` stops painting for a while.
pub fn watch(window: &impl IsA<gtk::Widget>) {
    window.connect_realize(|widget| {
        if let Some(clock) = widget.frame_clock() {
            clock.connect_after_paint(|_| painted());
        }
    });
}

fn painted() {
    LAST_PAINT.with(|last| last.set(Some(Instant::now())));
    if !ARMED.with(|armed| armed.replace(true)) {
        arm(SETTLE);
    }
}

fn arm(delay: Duration) {
    glib::timeout_add_local_once(delay, || {
        let quiet = LAST_PAINT
            .with(Cell::get)
            .map_or(SETTLE, |last| last.elapsed());
        if quiet < SETTLE {
            arm(SETTLE - quiet);
            return;
        }
        ARMED.with(|armed| armed.set(false));
        release();
        // What the interface releases without painting (decoders and
        // caches let go of after the first frames) is returned once more
        // a few seconds later, unless painting started again.
        glib::timeout_add_local_once(FOLLOW_UP, || {
            if !ARMED.with(Cell::get) {
                release();
            }
        });
    });
}

#[cfg(target_env = "gnu")]
fn release() {
    thread_local! {
        /// Free heap bytes left after the last trim (none yet: trim).
        static LEFT: Cell<Option<usize>> = const { Cell::new(None) };
    }
    // SAFETY: mallinfo2 only reads the allocator's own statistics.
    let free = unsafe { libc::mallinfo2() }.fordblks;
    // Free chunks the allocator cannot give back stay counted: trim again
    // only when more than a megabyte was freed since the last time.
    if LEFT
        .with(Cell::get)
        .is_none_or(|left| free >= left + WORTH_TRIMMING)
    {
        // SAFETY: malloc_trim only walks the allocator's own arenas.
        unsafe {
            libc::malloc_trim(0);
        }
        // SAFETY: as above.
        LEFT.with(|left| left.set(Some(unsafe { libc::mallinfo2() }.fordblks)));
    }
}

#[cfg(not(target_env = "gnu"))]
fn release() {}

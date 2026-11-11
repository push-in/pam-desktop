//! Time spent in backend calls between two painted frames (PAM_DESKTOP_PROFILE=1).

use std::cell::RefCell;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

thread_local! {
    static SPENT: RefCell<Vec<(&'static str, Duration, u32)>> = const { RefCell::new(Vec::new()) };
}

pub(crate) fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("PAM_DESKTOP_PROFILE").as_deref() == Ok("1"))
}

pub(crate) struct Span(Option<(&'static str, Instant)>);

impl Span {
    pub(crate) fn new(name: &'static str) -> Self {
        Self(enabled().then(|| (name, Instant::now())))
    }
}

impl Drop for Span {
    fn drop(&mut self) {
        if let Some((name, started)) = self.0 {
            let spent = started.elapsed();
            SPENT.with(|totals| {
                let mut totals = totals.borrow_mut();
                match totals.iter_mut().find(|(known, ..)| *known == name) {
                    Some(entry) => {
                        entry.1 += spent;
                        entry.2 += 1;
                    }
                    None => totals.push((name, spent, 1)),
                }
            });
        }
    }
}

/// The calls since the last report, heaviest first, when they add up to 2 ms.
pub(crate) fn report() -> Option<String> {
    SPENT.with(|totals| {
        let mut totals = std::mem::take(&mut *totals.borrow_mut());
        let total: Duration = totals.iter().map(|(_, spent, _)| *spent).sum();
        if total < Duration::from_millis(2) {
            return None;
        }
        totals.sort_by_key(|(_, spent, _)| std::cmp::Reverse(*spent));
        let parts: Vec<String> = totals
            .iter()
            .take(8)
            .map(|(name, spent, calls)| {
                format!("{name} {:.1}×{calls}", spent.as_secs_f64() * 1000.0)
            })
            .collect();
        Some(format!(
            "{:.1} ms: {}",
            total.as_secs_f64() * 1000.0,
            parts.join(", ")
        ))
    })
}

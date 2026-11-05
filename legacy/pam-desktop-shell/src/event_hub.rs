use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pam_desktop_protocol::ClientEvent;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::Notify;

const EVENT_HISTORY_LIMIT: usize = 256;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishedEvent {
    pub id: u64,
    pub name: String,
    pub payload: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_id: Option<String>,
}

#[derive(Clone, Default)]
pub struct EventHub {
    inner: Arc<Mutex<EventState>>,
    notify: Arc<Notify>,
}

#[derive(Default)]
struct EventState {
    next_id: u64,
    events: VecDeque<PublishedEvent>,
}

impl EventHub {
    pub fn publish(&self, event: ClientEvent) {
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.next_id = state.next_id.saturating_add(1);
        let published = PublishedEvent {
            id: state.next_id,
            name: event.name,
            payload: event.payload,
            window_id: event.window_id,
        };
        state.events.push_back(published);
        while state.events.len() > EVENT_HISTORY_LIMIT {
            state.events.pop_front();
        }
        drop(state);
        self.notify.notify_waiters();
    }

    pub async fn poll(
        &self,
        after: u64,
        window_id: &str,
        timeout: Duration,
    ) -> Vec<PublishedEvent> {
        let deadline = Instant::now() + timeout;
        loop {
            let notified = self.notify.notified();
            let events = self.events_after(after, window_id);
            if !events.is_empty() {
                return events;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() || tokio::time::timeout(remaining, notified).await.is_err() {
                return Vec::new();
            }
        }
    }

    #[must_use]
    pub fn latest_id(&self) -> u64 {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .next_id
    }

    fn events_after(&self, after: u64, window_id: &str) -> Vec<PublishedEvent> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .events
            .iter()
            .filter(|event| {
                event.id > after
                    && event
                        .window_id
                        .as_deref()
                        .is_none_or(|target| target == window_id)
            })
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn retains_order_and_filters_target_windows() {
        let hub = EventHub::default();
        hub.publish(ClientEvent {
            name: "global".to_owned(),
            payload: Value::Null,
            window_id: None,
        });
        hub.publish(ClientEvent {
            name: "settings.only".to_owned(),
            payload: serde_json::json!({"open": true}),
            window_id: Some("settings".to_owned()),
        });

        let main = hub.poll(0, "main", Duration::ZERO).await;
        assert_eq!(main.len(), 1);
        assert_eq!(main[0].name, "global");

        let settings = hub.poll(0, "settings", Duration::ZERO).await;
        assert_eq!(settings.len(), 2);
        assert_eq!(settings[1].id, 2);
    }
}

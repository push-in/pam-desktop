use std::path::Path;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

const PREFIX: &str = "@pam-event ";
static SEQUENCE: AtomicU64 = AtomicU64::new(0);
static SESSION_ID: LazyLock<String> =
    LazyLock::new(|| format!("{}-{}", std::process::id(), unix_milliseconds()));

#[derive(Clone, Copy)]
#[repr(u8)]
#[allow(
    dead_code,
    reason = "all cross-host event codes are reserved by schema 1"
)]
pub enum EventCode {
    SessionStarting = 1,
    SessionReady = 2,
    ChangeDetected = 3,
    ReloadStarted = 4,
    ReloadSucceeded = 5,
    ReloadFailed = 6,
    RuntimeExited = 7,
    SessionStopped = 8,
}

#[derive(Clone, Copy)]
#[repr(u8)]
#[allow(
    dead_code,
    reason = "all cross-host surface codes are reserved by schema 1"
)]
pub enum SurfaceCode {
    Server = 1,
    Android = 2,
    Ios = 3,
    Desktop = 4,
}

pub fn emit(event: EventCode, project_root: &Path, data: &Value) {
    if !std::env::var("PAM_DEV_EVENTS")
        .is_ok_and(|value| matches!(value.to_ascii_lowercase().as_str(), "1" | "json" | "jsonl"))
    {
        return;
    }
    let envelope = json!({
        "schemaVersion": 1,
        "eventCode": event as u8,
        "surfaceCode": SurfaceCode::Desktop as u8,
        "sessionId": SESSION_ID.as_str(),
        "sequence": SEQUENCE.fetch_add(1, Ordering::Relaxed) + 1,
        "occurredAtUnixMs": unix_milliseconds(),
        "projectRoot": project_root,
        "data": data,
    });
    eprintln!("{PREFIX}{envelope}");
}

fn unix_milliseconds() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_codes_match_the_cross_host_contract() {
        assert_eq!(EventCode::SessionStarting as u8, 1);
        assert_eq!(EventCode::SessionReady as u8, 2);
        assert_eq!(EventCode::ChangeDetected as u8, 3);
        assert_eq!(EventCode::ReloadStarted as u8, 4);
        assert_eq!(EventCode::ReloadSucceeded as u8, 5);
        assert_eq!(EventCode::ReloadFailed as u8, 6);
        assert_eq!(EventCode::RuntimeExited as u8, 7);
        assert_eq!(EventCode::SessionStopped as u8, 8);
        assert_eq!(SurfaceCode::Server as u8, 1);
        assert_eq!(SurfaceCode::Android as u8, 2);
        assert_eq!(SurfaceCode::Ios as u8, 3);
        assert_eq!(SurfaceCode::Desktop as u8, 4);
    }
}

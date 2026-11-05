use std::fs;
use std::path::Path;
use std::time::Duration;

use pam_desktop_protocol::{ConnectivityState, PowerState};
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemSnapshot {
    os: &'static str,
    architecture: &'static str,
    cpu_logical_cores: usize,
    memory_total_bytes: Option<u64>,
    memory_available_bytes: Option<u64>,
    uptime_seconds: Option<u64>,
    connectivity_state: ConnectivityState,
    power_state: PowerState,
    battery_percentage: Option<u8>,
}

pub fn snapshot() -> SystemSnapshot {
    let memory = memory();
    let (power_state, battery_percentage) = power();
    SystemSnapshot {
        os: std::env::consts::OS,
        architecture: std::env::consts::ARCH,
        cpu_logical_cores: std::thread::available_parallelism().map_or(1, usize::from),
        memory_total_bytes: memory.0,
        memory_available_bytes: memory.1,
        uptime_seconds: uptime(),
        connectivity_state: connectivity(),
        power_state,
        battery_percentage,
    }
}

fn memory() -> (Option<u64>, Option<u64>) {
    let Ok(contents) = fs::read_to_string("/proc/meminfo") else {
        return (None, None);
    };
    let mut total = None;
    let mut available = None;
    for line in contents.lines() {
        let mut fields = line.split_whitespace();
        match fields.next() {
            Some("MemTotal:") => total = kibibytes(fields.next()),
            Some("MemAvailable:") => available = kibibytes(fields.next()),
            _ => {}
        }
    }
    (total, available)
}

fn kibibytes(value: Option<&str>) -> Option<u64> {
    value?.parse::<u64>().ok()?.checked_mul(1_024)
}

fn uptime() -> Option<u64> {
    fs::read_to_string("/proc/uptime")
        .ok()?
        .split_whitespace()
        .next()?
        .parse::<f64>()
        .ok()
        .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
        .map(|seconds| Duration::from_secs_f64(seconds).as_secs())
}

fn connectivity() -> ConnectivityState {
    let Ok(entries) = fs::read_dir("/sys/class/net") else {
        return ConnectivityState::Offline;
    };
    for entry in entries.flatten() {
        if entry.file_name() == "lo" {
            continue;
        }
        if fs::read_to_string(entry.path().join("operstate"))
            .is_ok_and(|state| state.trim() == "up")
        {
            return ConnectivityState::Online;
        }
    }
    ConnectivityState::Offline
}

fn power() -> (PowerState, Option<u8>) {
    let root = Path::new("/sys/class/power_supply");
    let Ok(entries) = fs::read_dir(root) else {
        return (PowerState::Unknown, None);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !fs::read_to_string(path.join("type")).is_ok_and(|kind| kind.trim() == "Battery") {
            continue;
        }
        let state =
            fs::read_to_string(path.join("status"))
                .ok()
                .map_or(PowerState::Unknown, |status| match status.trim() {
                    "Charging" => PowerState::Charging,
                    "Discharging" => PowerState::Discharging,
                    "Full" => PowerState::Full,
                    _ => PowerState::Unknown,
                });
        let percentage = fs::read_to_string(path.join("capacity"))
            .ok()
            .and_then(|value| value.trim().parse::<u8>().ok())
            .map(|value| value.min(100));
        return (state, percentage);
    }
    (PowerState::Unknown, None)
}

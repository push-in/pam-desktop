use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq)]
pub struct MonitorGeometry {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct SavedWindowState {
    monitor: String,
    relative_x: i32,
    relative_y: i32,
    width: u32,
    height: u32,
    scale: f64,
    maximized: bool,
    fullscreen: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RestoredWindowState {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
    pub fullscreen: bool,
}

pub struct WindowStateStore {
    path: PathBuf,
    states: Mutex<HashMap<String, SavedWindowState>>,
}

impl WindowStateStore {
    pub fn open(application_id: &str) -> Result<Self, String> {
        Self::open_at(data_path(application_id)?)
    }

    fn open_at(path: PathBuf) -> Result<Self, String> {
        let states = if path.is_file() {
            let bytes = std::fs::read(&path)
                .map_err(|error| format!("cannot read window state: {error}"))?;
            serde_json::from_slice(&bytes)
                .map_err(|error| format!("cannot decode window state: {error}"))?
        } else {
            HashMap::new()
        };
        Ok(Self {
            path,
            states: Mutex::new(states),
        })
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "window snapshots deliberately cross the platform boundary as primitives"
    )]
    pub fn record(
        &self,
        id: &str,
        monitor: &MonitorGeometry,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        maximized: bool,
        fullscreen: bool,
    ) -> Result<(), String> {
        let state = SavedWindowState {
            monitor: monitor.name.clone(),
            relative_x: x.saturating_sub(monitor.x),
            relative_y: y.saturating_sub(monitor.y),
            width,
            height,
            scale: monitor.scale.max(0.25),
            maximized,
            fullscreen,
        };
        let mut states = self
            .states
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        states.insert(id.to_owned(), state);
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("cannot create window state directory: {error}"))?;
        }
        let bytes = serde_json::to_vec(&*states)
            .map_err(|error| format!("cannot encode window state: {error}"))?;
        let temporary = self.path.with_extension("json.tmp");
        std::fs::write(&temporary, bytes)
            .map_err(|error| format!("cannot persist window state: {error}"))?;
        std::fs::rename(temporary, &self.path)
            .map_err(|error| format!("cannot publish window state: {error}"))
    }

    pub fn restore(
        &self,
        id: &str,
        monitors: &[MonitorGeometry],
        remember_monitor: bool,
    ) -> Option<RestoredWindowState> {
        let state = self.states.lock().ok()?.get(id)?.clone();
        let monitor = if remember_monitor {
            monitors
                .iter()
                .find(|monitor| monitor.name == state.monitor)
                .or_else(|| monitors.first())?
        } else {
            monitors.first()?
        };
        Some(remap(&state, monitor))
    }
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "scale is finite and clamped; final geometry is clamped to monitor integer bounds"
)]
fn remap(state: &SavedWindowState, monitor: &MonitorGeometry) -> RestoredWindowState {
    const MIN_VISIBLE: i32 = 64;
    let ratio = (monitor.scale / state.scale.max(0.25)).clamp(0.25, 4.0);
    let width =
        ((f64::from(state.width) * ratio).round() as u32).clamp(320, monitor.width.max(320));
    let height =
        ((f64::from(state.height) * ratio).round() as u32).clamp(240, monitor.height.max(240));
    let proposed_x = monitor
        .x
        .saturating_add((f64::from(state.relative_x) * ratio).round() as i32);
    let proposed_y = monitor
        .y
        .saturating_add((f64::from(state.relative_y) * ratio).round() as i32);
    let min_x = monitor
        .x
        .saturating_sub(i32::try_from(width).unwrap_or(i32::MAX))
        .saturating_add(MIN_VISIBLE);
    let max_x = monitor
        .x
        .saturating_add(i32::try_from(monitor.width).unwrap_or(i32::MAX))
        .saturating_sub(MIN_VISIBLE);
    let min_y = monitor.y;
    let max_y = monitor
        .y
        .saturating_add(i32::try_from(monitor.height).unwrap_or(i32::MAX))
        .saturating_sub(MIN_VISIBLE);
    RestoredWindowState {
        x: proposed_x.clamp(min_x, max_x.max(min_x)),
        y: proposed_y.clamp(min_y, max_y.max(min_y)),
        width,
        height,
        maximized: state.maximized,
        fullscreen: state.fullscreen,
    }
}

fn data_path(application_id: &str) -> Result<PathBuf, String> {
    let base = if cfg!(target_os = "windows") {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })
    }
    .ok_or_else(|| "cannot locate operating-system data directory for windows".to_owned())?;
    Ok(base
        .join("pam-desktop")
        .join(application_id)
        .join("windows.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remaps_removed_monitor_and_changed_scale_inside_visible_bounds() {
        let root = std::env::temp_dir().join(format!(
            "pam-window-state-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock should be valid")
                .as_nanos(),
        ));
        let path = root.join("windows.json");
        let old = MonitorGeometry {
            name: "removed".to_owned(),
            x: 1920,
            y: 0,
            width: 1920,
            height: 1080,
            scale: 1.0,
        };
        let current = MonitorGeometry {
            name: "primary".to_owned(),
            x: 0,
            y: 0,
            width: 1280,
            height: 720,
            scale: 2.0,
        };
        let store = WindowStateStore::open_at(path.clone()).expect("store should open");
        store
            .record("main", &old, 3700, 900, 1000, 800, true, false)
            .expect("state should persist");
        drop(store);
        let reopened = WindowStateStore::open_at(path).expect("store should reopen");
        let state = reopened
            .restore("main", &[current], true)
            .expect("state should restore");
        assert!(state.x <= 1216);
        assert!(state.y <= 656);
        assert_eq!(state.width, 1280);
        assert_eq!(state.height, 720);
        assert!(state.maximized);
        std::fs::remove_dir_all(root).expect("fixture should be removable");
    }
}

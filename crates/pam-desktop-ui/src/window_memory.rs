//! The size and maximized state of the main window, kept between launches
//! (`window.remember` in `pam-desktop.json`, on by default).
//!
//! Only the restored size is kept: a maximized or full-screen window
//! remembers the size it had before, and whether it was maximized. Window
//! positions are left to the system (Wayland does not let clients choose).

use std::path::{Path, PathBuf};

use crate::backend::WindowConfig;
use crate::geometry::Size;
use crate::modules::storage::write_atomically;
use crate::shell::WindowState;

#[derive(Clone, Debug, PartialEq)]
pub struct WindowMemory {
    path: PathBuf,
    size: Option<Size>,
    maximized: bool,
    saved: (Option<Size>, bool),
}

impl WindowMemory {
    #[must_use]
    pub fn load(state_directory: &Path) -> Self {
        let path = state_directory.join("window.json");
        let stored = std::fs::read_to_string(&path)
            .ok()
            .and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok());
        let number = |key: &str| {
            stored
                .as_ref()
                .and_then(|stored| stored.get(key))
                .and_then(serde_json::Value::as_f64)
                .filter(|value| value.is_finite())
                .map(|value| value as f32)
        };
        let size = match (number("width"), number("height")) {
            (Some(width), Some(height)) => Some(Size::new(width, height)),
            _ => None,
        };
        let maximized = stored
            .as_ref()
            .and_then(|stored| stored.get("maximized"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        Self {
            path,
            size,
            maximized,
            saved: (size, maximized),
        }
    }

    /// Puts the remembered size into `config` (within its minimum and the
    /// largest window size) and tells whether to maximize after opening.
    #[must_use]
    pub fn restore(&self, config: &mut WindowConfig) -> bool {
        if !config.remember {
            return false;
        }
        if let Some(size) = self.size {
            let minimum = config.min_size.unwrap_or(Size::new(120.0, 120.0));
            config.size = Size::new(
                size.width.clamp(minimum.width.max(120.0), 16_384.0),
                size.height.clamp(minimum.height.max(120.0), 16_384.0),
            );
        }
        self.maximized && config.resizable
    }

    /// The window was resized: kept only while it is in its normal state.
    pub fn resized(&mut self, size: Size, state: WindowState) {
        if !(state.maximized || state.minimized || state.full_screen)
            && size.width >= 1.0
            && size.height >= 1.0
        {
            self.size = Some(size);
        }
    }

    pub fn state_changed(&mut self, state: WindowState) {
        if !state.minimized && !state.full_screen {
            self.maximized = state.maximized;
        }
    }

    /// Writes the file when something changed since it was read or written.
    pub fn save(&mut self) -> std::io::Result<()> {
        let current = (self.size, self.maximized);
        if current == self.saved {
            return Ok(());
        }
        let Some(size) = self.size else {
            return Ok(());
        };
        let json = serde_json::json!({
            "width": size.width.round(),
            "height": size.height.round(),
            "maximized": self.maximized,
        });
        write_atomically(&self.path, json.to_string().as_bytes())?;
        self.saved = current;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn normal() -> WindowState {
        WindowState::default()
    }

    #[test]
    fn remembers_the_normal_size_and_the_maximized_state() {
        let directory = std::env::temp_dir().join(format!("pam-window-memory-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let mut memory = WindowMemory::load(&directory);
        let mut config = WindowConfig::default();
        assert!(!memory.restore(&mut config));
        assert_eq!(config.size, Size::new(960.0, 640.0));

        memory.resized(Size::new(1280.4, 800.0), normal());
        let maximized = WindowState {
            maximized: true,
            ..normal()
        };
        memory.state_changed(maximized);
        memory.resized(Size::new(1920.0, 1080.0), maximized);
        memory.save().unwrap();

        let memory = WindowMemory::load(&directory);
        let mut config = WindowConfig {
            min_size: Some(Size::new(1300.0, 200.0)),
            ..WindowConfig::default()
        };
        assert!(memory.restore(&mut config));
        assert_eq!(config.size, Size::new(1300.0, 800.0), "the minimum still applies");

        let mut forgetful = WindowConfig {
            remember: false,
            ..WindowConfig::default()
        };
        assert!(!memory.restore(&mut forgetful));
        assert_eq!(forgetful.size, Size::new(960.0, 640.0));
        std::fs::remove_dir_all(&directory).unwrap();
    }
}

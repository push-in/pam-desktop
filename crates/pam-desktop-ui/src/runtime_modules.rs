//! Module calls that need the window, the scene or the backend: typed system
//! operations, appearance, text scale, window flags and imperative view
//! commands.

use std::path::{Path, PathBuf};

use pam_desktop_wire::{NodeKind, PropKey};

use crate::backend::{AlertButton, AlertButtonStyle, AlertConfig, Backend, NodeId, MAIN_WINDOW};
use crate::context::HostCommand;
use crate::ctx;
use crate::modules::system::{can_open_url, locale_tag, open_url, time_zone};
use crate::modules::{request_map, storage::write_atomically, Reply};
use crate::runtime::Runtime;
use crate::text::utf16_len;
use crate::wiremap::WireMap;

const APPEARANCE_SYSTEM: u8 = 1;
const APPEARANCE_LIGHT: u8 = 2;
const APPEARANCE_DARK: u8 = 3;

const MAX_CLIPBOARD_BYTES: usize = 1024 * 1024;

/// Preferences the application sets at run time and expects back at the next launch.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    path: PathBuf,
    /// 1 follow the system, 2 light, 3 dark.
    pub appearance_mode: u8,
    /// Extra text scale chosen inside the app.
    pub text_multiplier: f32,
    /// Cap on the system text scale (0 = uncapped).
    pub text_max_system: f32,
}

impl Settings {
    #[must_use]
    pub fn load(state_directory: &Path) -> Self {
        let path = state_directory.join("settings.json");
        let stored = std::fs::read_to_string(&path).ok().and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok());
        let number = |key: &str| stored.as_ref().and_then(|stored| stored.get(key)).and_then(serde_json::Value::as_f64);
        Self {
            path,
            appearance_mode: number("appearanceMode").map_or(APPEARANCE_SYSTEM, |mode| (mode as u8).clamp(APPEARANCE_SYSTEM, APPEARANCE_DARK)),
            text_multiplier: number("textMultiplier").map_or(1.0, |value| (value as f32).clamp(0.5, 3.0)),
            text_max_system: number("textMaxSystemScale").map_or(0.0, |value| (value as f32).max(0.0)),
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let json = serde_json::json!({
            "appearanceMode": self.appearance_mode,
            "textMultiplier": self.text_multiplier,
            "textMaxSystemScale": self.text_max_system,
        });
        write_atomically(&self.path, json.to_string().as_bytes())
    }

    /// The appearance the app forces, or `None` to follow the system.
    #[must_use]
    pub fn forced_dark(&self) -> Option<bool> {
        match self.appearance_mode {
            APPEARANCE_LIGHT => Some(false),
            APPEARANCE_DARK => Some(true),
            _ => None,
        }
    }

    /// The text scale views use: the (capped) system scale times the app's own.
    #[must_use]
    pub fn effective_font_scale(&self, system: f32) -> f32 {
        let system = if self.text_max_system >= 1.0 { system.min(self.text_max_system) } else { system };
        (system * self.text_multiplier).clamp(0.25, 8.0)
    }
}

impl<B: Backend> Runtime<B> {
    fn reply(&self, request: u64) -> Reply {
        Reply::new(std::sync::Arc::clone(&self.inbox), self.generation, request)
    }

    pub(crate) fn call_module(&mut self, request: u64, module: &str, method: &str, payload: &[u8]) {
        let reply = self.reply(request);
        // The file pickers need the native dialog before the files module imports.
        if module == "files" && matches!(method, "pick" | "pickMany") {
            return self.pick_files(method == "pickMany", payload, reply);
        }
        let Some(reply) = self.modules.call(module, method, payload, reply) else { return };
        match module {
            "appearance" => self.appearance_module(method, payload, reply),
            "accessibility" => self.accessibility_module(method, payload, reply),
            "window" => match method {
                // Excluding a window from capture is a per-platform call; no
                // backend offers it yet, so it reports as unsupported.
                "secure" | "isSecure" => reply.map(&WireMap::new().with("enabled", false).with("supported", false)),
                other => reply.failure(format!("Unknown window method {other}")),
            },
            "view-ref" => self.view_ref_module(method, payload, reply),
            "desktop" => self.desktop_module(method, payload, reply),
            // Unknown modules fail fast so application callbacks never hang.
            other => reply.failure(format!("Unknown native module {other}")),
        }
    }

    /// `files.pick` / `files.pickMany`: the native open dialog, then a copy of
    /// the selection into the sandbox (off the UI thread).
    fn pick_files(&mut self, many: bool, payload: &[u8], reply: Reply) {
        use crate::modules::files;
        use crate::shell::{FileDialogConfig, FileDialogKind, FileFilter};
        let request = match request_map(payload).and_then(|map| files::pick_request(&map)) {
            Ok(request) => request,
            Err(error) => return reply.failure(error),
        };
        let extensions: Vec<String> = request.extensions().into_iter().map(str::to_owned).collect();
        let name = match request.kind {
            files::PickKind::Image => "Images",
            files::PickKind::Video => "Videos",
            files::PickKind::Audio => "Audio",
            files::PickKind::Media => "Media",
            files::PickKind::Any => "Files",
        };
        let config = FileDialogConfig {
            kind: if many { FileDialogKind::OpenMultiple } else { FileDialogKind::Open },
            title: String::new(),
            default_name: String::new(),
            directory: None,
            filters: if extensions.is_empty() { Vec::new() } else { vec![FileFilter { name: name.into(), extensions }] },
        };
        let root = files::sandbox_root(&self.directories.data);
        self.backend.pick_files(
            MAIN_WINDOW,
            &config,
            Box::new(move |paths| {
                if paths.is_empty() {
                    // Cancelled: an empty success (`null` / no items in PHP).
                    return if many { reply.map(&files::items_map(&[])) } else { reply.done() };
                }
                let import = move || {
                    if many {
                        match files::import_selection(&root, &paths, &request) {
                            Ok(items) => reply.map(&files::items_map(&items)),
                            Err(error) => reply.failure(error),
                        }
                    } else {
                        match files::import_picked(&root, &paths[0], request.maximum_bytes) {
                            Ok(file) => reply.map(&file.to_map()),
                            Err(error) => reply.failure(error),
                        }
                    }
                };
                if std::thread::Builder::new().name("pam-pick-import".into()).spawn(import).is_err() {
                    // The reply moved into the closure; a failed spawn drops it, which PHP sees as a failure.
                }
            }),
        );
    }

    pub(crate) fn call_typed(&mut self, request: u64, operation: u16, payload: &[u8]) {
        let reply = self.reply(request);
        let map = match request_map(payload) {
            Ok(map) => map,
            Err(error) => return reply.failure(error),
        };
        let route = |runtime: &mut Self, module: &str, method: &str, reply: Reply| {
            if let Some(reply) = runtime.modules.call(module, method, payload, reply) {
                reply.failure(format!("Unknown native module {module}"));
            }
        };
        match operation {
            1 => route(self, "http", "get", reply),
            2 => route(self, "storage", "get", reply),
            3 => route(self, "storage", "set", reply),
            4 => self.alert(&map, reply),
            5 => route(self, "toast", "show", reply),
            6 => reply.failure("Sharing is not available on this platform"),
            7 => match map.text("url") {
                Some(url) => match open_url(url) {
                    Ok(()) => reply.done(),
                    Err(message) => reply.failure(message),
                },
                None => reply.failure("Missing text value url"),
            },
            8 => reply.map(&WireMap::new().with("supported", map.text("url").is_some_and(can_open_url))),
            // Vibration and haptics have no desktop equivalent.
            9 | 15 => reply.done(),
            10 => {
                let info = self.device_info();
                reply.map(&info);
            }
            11 => {
                self.commands.push(HostCommand::Focus(None));
                reply.done();
            }
            12 | 13 => reply.map(&WireMap::new().with("granted", true)),
            // CloseApp: PHP has nowhere left to go back to.
            14 => {
                reply.done();
                if let Some(worker) = &mut self.worker {
                    worker.shutdown();
                }
                self.finished = true;
            }
            16 => match map.text("text") {
                Some(text) if text.len() > MAX_CLIPBOARD_BYTES => reply.failure("Clipboard text exceeds one megabyte"),
                Some(text) => {
                    self.backend.clipboard_set_text(text);
                    reply.done();
                }
                None => reply.failure("Missing text value text"),
            },
            17 => self.backend.clipboard_text(Box::new(move |text| {
                let text = text.filter(|text| text.len() <= MAX_CLIPBOARD_BYTES).unwrap_or_default();
                reply.map(&WireMap::new().with("text", text));
            })),
            18 => self.backend.clipboard_text(Box::new(move |text| {
                reply.map(&WireMap::new().with("hasText", text.is_some_and(|text| !text.is_empty())));
            })),
            19 => reply.failure("Sensors moved to the pushinbr/pam-desktop-sensors plugin"),
            other => reply.failure(format!("Unknown native operation {other}")),
        }
    }

    fn device_info(&self) -> WireMap {
        let metrics = self.metrics;
        WireMap::new()
            .with("width", metrics.size.width)
            .with("height", metrics.size.height)
            .with("density", metrics.scale)
            .with("appearance", if metrics.dark { 2_i64 } else { 1_i64 })
            .with("appState", if self.active { 1_i64 } else { 2_i64 })
            .with("safeAreaTop", 0.0_f64)
            .with("safeAreaRight", 0.0_f64)
            .with("safeAreaBottom", 0.0_f64)
            .with("safeAreaLeft", 0.0_f64)
            .with("timeZone", time_zone())
            .with("memoryClassMb", 0_i64)
            .with("lowRamDevice", false)
            .with("powerSaveMode", false)
            .with("appVersion", self.app_version.as_str())
            .with("buildNumber", self.build_number.as_str())
            .with("locale", locale_tag())
            .with("manufacturer", "")
            .with("model", "")
    }

    /// Typed operation 4: a native message dialog.
    fn alert(&mut self, request: &WireMap, reply: Reply) {
        let title = request.text("title").unwrap_or_default().to_owned();
        let message = request.text("message").unwrap_or_default().to_owned();
        let Some(count) = request.integer("buttonCount") else {
            // Legacy alert: one OK button, answered when it is dismissed.
            let alert = AlertConfig { title, message, buttons: Vec::new() };
            return self.backend.show_alert(MAIN_WINDOW, &alert, Box::new(move |_| reply.done()));
        };
        if !(1..=3).contains(&count) {
            return reply.failure("Alert supports between 1 and 3 buttons");
        }
        let mut buttons = Vec::new();
        for index in 0..count {
            let Some(label) = request.text(&format!("button{index}Label")) else {
                return reply.failure(format!("Missing alert button label {index}"));
            };
            if label.trim().is_empty() {
                return reply.failure("Alert button labels cannot be empty");
            }
            let style = match request.integer(&format!("button{index}Style")).unwrap_or(1) {
                1 => AlertButtonStyle::Default,
                2 => AlertButtonStyle::Cancel,
                3 => AlertButtonStyle::Destructive,
                other => return reply.failure(format!("Unknown alert button style {other}")),
            };
            buttons.push(AlertButton { label: label.to_owned(), style });
        }
        if buttons.iter().filter(|button| button.style == AlertButtonStyle::Cancel).count() > 1 {
            return reply.failure("Alert supports one cancel button");
        }
        // Closing the dialog another way answers as its Cancel button.
        let cancel = buttons.iter().position(|button| button.style == AlertButtonStyle::Cancel);
        let alert = AlertConfig { title, message, buttons };
        self.backend.show_alert(
            MAIN_WINDOW,
            &alert,
            Box::new(move |choice| {
                let index = choice.or(cancel).map_or(-1, |index| index as i64);
                reply.map(&WireMap::new().with("index", index));
            }),
        );
    }

    // ---- appearance ----------------------------------------------------

    fn appearance_snapshot(&self) -> WireMap {
        WireMap::new()
            .with("mode", i64::from(self.settings.appearance_mode))
            .with("appearance", if self.metrics.dark { 2_i64 } else { 1_i64 })
            .with("systemAppearance", if self.system_dark { 2_i64 } else { 1_i64 })
    }

    fn appearance_module(&mut self, method: &str, payload: &[u8], reply: Reply) {
        match method {
            "get" => reply.map(&self.appearance_snapshot()),
            "set" => {
                let mode = request_map(payload).ok().and_then(|request| request.integer("mode"));
                let Some(mode) = mode.filter(|mode| (1..=3).contains(mode)) else {
                    return reply.failure("Appearance mode must be 1 (System), 2 (Light) or 3 (Dark)");
                };
                self.settings.appearance_mode = mode as u8;
                if self.settings.save().is_err() {
                    return reply.failure("Appearance preference could not be persisted");
                }
                self.apply_appearance();
                reply.map(&self.appearance_snapshot());
            }
            other => reply.failure(format!("Unknown appearance method {other}")),
        }
    }

    /// Recomputes the effective appearance from the system's and the app's
    /// preference; PHP learns about it through a Dimensions event.
    pub(crate) fn apply_appearance(&mut self) {
        self.backend.set_appearance(self.settings.forced_dark());
        let dark = self.settings.forced_dark().unwrap_or(self.system_dark);
        if self.metrics.dark != dark {
            self.metrics.dark = dark;
            self.interaction.dark = dark;
            self.controls.dark = dark;
            let mut ctx = ctx!(self);
            self.interaction.refresh_defaults(&mut ctx);
            self.controls.refresh(&mut ctx);
            if self.error_overlay.is_visible() {
                self.refresh_error_overlay();
            }
        }
        self.send_dimensions();
    }

    // ---- accessibility -------------------------------------------------

    fn text_scale_snapshot(&self) -> WireMap {
        WireMap::new()
            .with("multiplier", self.settings.text_multiplier)
            .with("maxSystemScale", self.settings.text_max_system)
            // Preferences apply at once on desktop: applied always equals stored.
            .with("appliedMultiplier", self.settings.text_multiplier)
            .with("appliedMaxSystemScale", self.settings.text_max_system)
            .with("systemScale", self.system_font_scale)
            .with("effectiveScale", self.metrics.font_scale)
    }

    fn accessibility_module(&mut self, method: &str, payload: &[u8], reply: Reply) {
        let request = match request_map(payload) {
            Ok(request) => request,
            Err(error) => return reply.failure(error),
        };
        match method {
            "announce" => match request.text("text").map(str::trim) {
                Some(text) if !text.is_empty() && text.chars().count() <= 4096 => {
                    reply.map(&WireMap::new().with("delivered", false));
                }
                Some(_) => reply.failure("Announcement text is invalid"),
                None => reply.failure("Announcement text is required"),
            },
            "status" => reply.map(&WireMap::new().with("enabled", false).with("touchExploration", false)),
            "textScale" => reply.map(&self.text_scale_snapshot()),
            "setTextScale" => {
                let Some(multiplier) = request.number("multiplier") else { return reply.failure("Text scale multiplier is required") };
                self.settings.text_multiplier = (multiplier as f32).clamp(0.5, 3.0);
                self.settings.text_max_system = request.number("maxSystemScale").map_or(0.0, |value| (value as f32).max(0.0));
                if self.settings.save().is_err() {
                    return reply.failure("Text scale could not be persisted");
                }
                self.apply_text_scale();
                reply.map(&self.text_scale_snapshot());
            }
            other => reply.failure(format!("Unknown accessibility method {other}")),
        }
    }

    /// Applies a new effective text scale to layout and paint, live.
    pub(crate) fn apply_text_scale(&mut self) {
        let scale = self.settings.effective_font_scale(self.system_font_scale);
        if (self.metrics.font_scale - scale).abs() < f32::EPSILON {
            return;
        }
        self.metrics.font_scale = scale;
        self.scene.environment.font_scale = scale;
        let _ = self.engine.set_text_scale(scale);
        self.engine.invalidate_text_measurements();
        self.scene.invalidate_all();
        self.relayout();
        self.send_dimensions();
    }

    // ---- view-ref ------------------------------------------------------

    fn node_by_ref(&self, reference: &str, within: Option<&str>) -> Option<NodeId> {
        let matches = |id: NodeId, wanted: &str| self.scene.node(id).is_some_and(|node| node.props().text(PropKey::NativeRef) == Some(wanted));
        let ancestor = match within {
            Some(within) => {
                let mut found = self.scene.nodes().filter(|node| matches(node.id, within)).map(|node| node.id);
                let first = found.next()?;
                // `within` must name a unique ancestor.
                if found.next().is_some() {
                    return None;
                }
                Some(first)
            }
            None => None,
        };
        let mut candidates: Vec<NodeId> = self
            .scene
            .nodes()
            .filter(|node| matches(node.id, reference))
            .map(|node| node.id)
            .filter(|id| ancestor.is_none_or(|ancestor| *id != ancestor && self.scene.is_within(*id, ancestor)))
            .collect();
        // Prefer what is on screen (a covered route may carry the same ref).
        candidates.sort_by_key(|id| !self.scene.is_effectively_visible(*id));
        candidates.first().copied()
    }

    /// The text input a view-ref command targets: the view, or the first input inside it.
    fn input_under(&self, id: NodeId) -> Option<NodeId> {
        let node = self.scene.node(id)?;
        if node.kind == NodeKind::Input {
            return Some(id);
        }
        self.scene.ordered_children(id).into_iter().find_map(|child| self.input_under(child))
    }

    fn view_ref_module(&mut self, method: &str, payload: &[u8], reply: Reply) {
        let Ok(request) = request_map(payload) else { return reply.failure("Invalid view ref request") };
        let Some(reference) = request.text("ref").filter(|reference| !reference.is_empty()) else {
            return reply.failure("A valid nativeRef is required.");
        };
        let within = request.text("within").filter(|within| !within.is_empty());
        let Some(id) = self.node_by_ref(reference, within) else {
            let scope = within.map(|within| format!(" within {within}")).unwrap_or_default();
            return reply.failure(format!("No mounted view has nativeRef {reference}{scope}."));
        };
        let input = self.input_under(id);
        let not_input = format!("The view {reference} is not a text input.");
        match method {
            "focus" => match input {
                Some(input) if self.scene.is_effectively_visible(input) => {
                    self.commands.push(HostCommand::Focus(Some(input)));
                    reply.done();
                }
                Some(_) => reply.failure(format!("The input {reference} is not visible.")),
                None => reply.failure(not_input),
            },
            "blur" => {
                self.commands.push(HostCommand::Focus(None));
                reply.done();
            }
            "clear" => match input {
                Some(input) => {
                    let mut ctx = ctx!(self);
                    // Like the user clearing it: the change event fires with ''.
                    self.inputs.set_text(&mut ctx, input, "");
                    reply.done();
                }
                None => reply.failure(not_input),
            },
            "setSelection" => match input {
                Some(input) => {
                    let length = self.inputs.text(input).map_or(0, utf16_len);
                    let start = (request.integer("start").unwrap_or(0).max(0) as u32).min(length);
                    let end = (request.integer("end").unwrap_or(0).max(0) as u32).clamp(start, length);
                    self.commands.push(HostCommand::InputSelection { id: input, start, end });
                    reply.done();
                }
                None => reply.failure(not_input),
            },
            "isFocused" => {
                let focused = input.is_some_and(|input| self.inputs.is_focused(input)) || self.interaction.focused() == Some(id);
                reply.map(&WireMap::new().with("focused", focused));
            }
            "measure" => {
                let Some(node) = self.scene.node(id) else { return reply.failure(format!("The view {reference} has not been laid out.")) };
                let parent_origin = self.scene.node(node.parent).map_or(crate::geometry::Point::ZERO, |parent| parent.frame.origin());
                let window = self.scene.window_origin(id);
                reply.map(
                    &WireMap::new()
                        .with("x", node.frame.x - parent_origin.x)
                        .with("y", node.frame.y - parent_origin.y)
                        .with("width", node.frame.width)
                        .with("height", node.frame.height)
                        .with("windowX", window.x)
                        .with("windowY", window.y)
                        .with("pageX", window.x)
                        .with("pageY", window.y),
                );
            }
            other => reply.failure(format!("Unknown view ref method {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_and_resolve_preferences() {
        let directory = std::env::temp_dir().join(format!("pam-settings-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let mut settings = Settings::load(&directory);
        assert_eq!(settings.appearance_mode, APPEARANCE_SYSTEM);
        assert_eq!(settings.forced_dark(), None);
        assert_eq!(settings.effective_font_scale(1.5), 1.5);
        settings.appearance_mode = APPEARANCE_DARK;
        settings.text_multiplier = 1.2;
        settings.text_max_system = 1.25;
        settings.save().unwrap();
        let loaded = Settings::load(&directory);
        assert_eq!(loaded.forced_dark(), Some(true));
        assert!((loaded.effective_font_scale(2.0) - 1.5).abs() < 1e-5);
        let _ = std::fs::remove_dir_all(&directory);
    }
}

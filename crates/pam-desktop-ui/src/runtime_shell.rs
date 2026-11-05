//! The `desktop` module: the application menu, tray, shortcuts, file dialogs,
//! notifications and window control, plus the event channel that brings
//! their activations back to PHP.

use std::collections::VecDeque;
use std::path::PathBuf;

use pam_desktop_wire::PropKey;

use crate::backend::{Backend, NodeId, MAIN_WINDOW};
use crate::context::event_kind;
use crate::geometry::{Point, Size};
use crate::modules::{request_map, Reply};
use crate::runtime::Runtime;
use crate::shell::{
    parse_menu, Accelerator, FileDialogConfig, FileDialogKind, FileFilter, MenuNode, NotificationConfig, Shortcut, TrayConfig,
    WindowCommand, WindowState,
};
use crate::wiremap::WireMap;

/// Events waiting for PHP's next long poll.
const MAX_PENDING_EVENTS: usize = 64;

/// State of the `desktop` module.
#[derive(Default)]
pub struct Shell {
    pending: VecDeque<Vec<u8>>,
    waiter: Option<Reply>,
    /// PHP wants to decide whether the main window closes.
    pub(crate) intercepts_close: bool,
    shortcuts: Vec<Shortcut>,
}

impl Shell {
    /// Queues an event for PHP: `type`, `id` and a JSON `data` string.
    pub(crate) fn emit(&mut self, kind: &str, id: &str, data: &str) {
        let event = WireMap::new().with("type", kind).with("id", id).with("data", data).encode();
        match self.waiter.take() {
            Some(waiter) => waiter.success(event),
            None => {
                if self.pending.len() == MAX_PENDING_EVENTS {
                    self.pending.pop_front();
                }
                self.pending.push_back(event);
            }
        }
    }

    /// The PHP worker restarted: its listener and registrations are gone.
    pub(crate) fn reset(&mut self) {
        self.waiter = None;
        self.pending.clear();
        self.intercepts_close = false;
        self.shortcuts.clear();
    }

    /// App-scoped shortcuts are matched here, on keys no control consumed.
    pub(crate) fn shortcut_for(&self, key: &str, modifiers: crate::backend::Modifiers) -> Option<String> {
        self.shortcuts
            .iter()
            .find(|shortcut| !shortcut.global && shortcut.accelerator.matches(key, modifiers))
            .map(|shortcut| shortcut.id.clone())
    }
}

fn state_json(state: WindowState, size: Size) -> String {
    serde_json::json!({
        "maximized": state.maximized,
        "minimized": state.minimized,
        "fullScreen": state.full_screen,
        "focused": state.focused,
        "width": size.width,
        "height": size.height,
    })
    .to_string()
}

fn parse_filters(json: &str) -> Vec<FileFilter> {
    let Ok(serde_json::Value::Array(entries)) = serde_json::from_str::<serde_json::Value>(json) else { return Vec::new() };
    entries
        .iter()
        .take(32)
        .filter_map(|entry| {
            let name = entry.get("name")?.as_str()?.chars().take(128).collect::<String>();
            let extensions: Vec<String> = entry
                .get("extensions")?
                .as_array()?
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(|extension| extension.trim_start_matches('.').to_ascii_lowercase())
                .filter(|extension| !extension.is_empty() && extension.len() <= 16 && extension.chars().all(|c| c.is_ascii_alphanumeric() || c == '*'))
                .take(64)
                .collect();
            (!extensions.is_empty()).then_some(FileFilter { name, extensions })
        })
        .collect()
}

impl<B: Backend> Runtime<B> {
    pub(crate) fn desktop_module(&mut self, method: &str, payload: &[u8], reply: Reply) {
        let request = match request_map(payload) {
            Ok(request) => request,
            Err(error) => return reply.failure(error),
        };
        match method {
            // The event channel: one long poll at a time, re-armed by PHP.
            "next" => {
                if let Some(event) = self.shell.pending.pop_front() {
                    reply.success(event);
                } else if self.shell.waiter.is_some() {
                    reply.failure("Only one desktop event listener can wait at a time");
                } else {
                    self.shell.waiter = Some(reply);
                }
            }
            "menu" => match parse_menu(request.text("items").unwrap_or("[]")) {
                Some(menu) => {
                    self.backend.set_menu_bar(&menu);
                    reply.done();
                }
                None => reply.failure("The application menu is malformed"),
            },
            "tray" => {
                if request.boolean("visible") == Some(false) {
                    self.backend.set_tray(None);
                    return reply.done();
                }
                let Some(menu) = parse_menu(request.text("items").unwrap_or("[]")) else {
                    return reply.failure("The tray menu is malformed");
                };
                let tray = TrayConfig {
                    icon: request.text("icon").filter(|icon| !icon.is_empty()).map(PathBuf::from),
                    tooltip: request.text("tooltip").unwrap_or_default().chars().take(256).collect(),
                    menu,
                };
                self.backend.set_tray(Some(&tray));
                reply.done();
            }
            "shortcuts" => {
                let Ok(serde_json::Value::Array(entries)) = serde_json::from_str::<serde_json::Value>(request.text("items").unwrap_or("[]")) else {
                    return reply.failure("Shortcuts are malformed");
                };
                let mut shortcuts = Vec::new();
                for entry in entries.iter().take(256) {
                    let id = entry.get("id").and_then(serde_json::Value::as_str).unwrap_or_default();
                    let accelerator = entry.get("shortcut").and_then(serde_json::Value::as_str).and_then(Accelerator::parse);
                    let (Some(accelerator), false) = (accelerator, id.is_empty()) else {
                        return reply.failure("A shortcut needs an id and a valid key combination");
                    };
                    shortcuts.push(Shortcut {
                        id: id.to_owned(),
                        accelerator,
                        global: entry.get("global").and_then(serde_json::Value::as_bool).unwrap_or(false),
                    });
                }
                self.backend.set_shortcuts(&shortcuts);
                self.shell.shortcuts = shortcuts;
                reply.done();
            }
            "window" => self.window_method(&request, reply),
            "windowState" => {
                let state = self.backend.window_state(MAIN_WINDOW);
                reply.map(&WireMap::new().with("state", state_json(state, self.metrics.size)));
            }
            "openFile" | "saveFile" | "selectFolder" => {
                let kind = match (method, request.boolean("multiple").unwrap_or(false)) {
                    ("saveFile", _) => FileDialogKind::Save,
                    ("selectFolder", _) => FileDialogKind::SelectFolder,
                    (_, true) => FileDialogKind::OpenMultiple,
                    (_, false) => FileDialogKind::Open,
                };
                let config = FileDialogConfig {
                    kind,
                    title: request.text("title").unwrap_or_default().chars().take(256).collect(),
                    default_name: request.text("defaultName").unwrap_or_default().chars().take(255).collect(),
                    directory: request.text("directory").filter(|directory| !directory.is_empty()).map(PathBuf::from),
                    filters: parse_filters(request.text("filters").unwrap_or("[]")),
                };
                self.backend.pick_files(
                    MAIN_WINDOW,
                    &config,
                    Box::new(move |paths| {
                        let paths: Vec<String> = paths.iter().map(|path| path.display().to_string()).collect();
                        let json = serde_json::to_string(&paths).unwrap_or_else(|_| String::from("[]"));
                        reply.map(&WireMap::new().with("paths", json));
                    }),
                );
            }
            "notify" => {
                let title: String = request.text("title").unwrap_or_default().chars().take(256).collect();
                if title.is_empty() {
                    return reply.failure("A notification needs a title");
                }
                self.backend.notify(&NotificationConfig {
                    id: request.text("id").unwrap_or_default().chars().take(128).collect(),
                    title,
                    body: request.text("body").unwrap_or_default().chars().take(4096).collect(),
                });
                reply.done();
            }
            "quit" => {
                reply.done();
                if let Some(worker) = &mut self.worker {
                    worker.shutdown();
                }
                self.finished = true;
            }
            other => reply.failure(format!("Unknown desktop method {other}")),
        }
    }

    fn window_method(&mut self, request: &WireMap, reply: Reply) {
        let number = |key: &str| request.number(key).map(|value| value as f32).filter(|value| value.is_finite());
        let size = || match (number("width"), number("height")) {
            (Some(width), Some(height)) if width >= 1.0 && height >= 1.0 => Some(Size::new(width.min(16_384.0), height.min(16_384.0))),
            _ => None,
        };
        let command = match request.text("command").unwrap_or_default() {
            "title" => WindowCommand::SetTitle(request.text("title").unwrap_or_default().chars().take(512).collect()),
            "size" => match size() {
                Some(size) => WindowCommand::SetSize(size),
                None => return reply.failure("A window size needs a width and a height"),
            },
            "minSize" => WindowCommand::SetMinSize(size()),
            "resizable" => WindowCommand::SetResizable(request.boolean("enabled").unwrap_or(true)),
            "minimize" => WindowCommand::Minimize,
            "maximize" => WindowCommand::Maximize,
            "restore" => WindowCommand::Restore,
            "fullScreen" => WindowCommand::SetFullScreen(request.boolean("enabled").unwrap_or(true)),
            "focus" => WindowCommand::Focus,
            "center" => WindowCommand::Center,
            "alwaysOnTop" => WindowCommand::SetAlwaysOnTop(request.boolean("enabled").unwrap_or(true)),
            "show" => WindowCommand::Show,
            "hide" => WindowCommand::Hide,
            "progress" => WindowCommand::SetProgress(number("value").map(|value| value.clamp(0.0, 1.0))),
            "interceptClose" => {
                self.shell.intercepts_close = request.boolean("enabled").unwrap_or(true);
                return reply.done();
            }
            "close" => {
                reply.done();
                if let Some(worker) = &mut self.worker {
                    worker.shutdown();
                }
                self.finished = true;
                return;
            }
            other => return reply.failure(format!("Unknown window command {other}")),
        };
        // A hidden window puts the application in the background.
        match command {
            WindowCommand::Hide => self.hidden = true,
            WindowCommand::Show | WindowCommand::Focus => self.hidden = false,
            _ => {}
        }
        self.backend.window_command(MAIN_WINDOW, &command);
        self.report_app_state();
        reply.done();
    }

    /// The user asked to close the main window.
    pub(crate) fn close_requested(&mut self) {
        if self.shell.intercepts_close && self.worker.is_some() {
            // PHP decides (unsaved changes…) and closes through `window close`.
            self.shell.emit("window", "close-requested", "{}");
            return;
        }
        if let Some(worker) = &mut self.worker {
            worker.shutdown();
        }
        self.finished = true;
    }

    pub(crate) fn window_state_changed(&mut self, state: WindowState) {
        let json = state_json(state, self.metrics.size);
        self.shell.emit("window", "state", &json);
    }

    pub(crate) fn files_dropped(&mut self, position: Point, paths: &[PathBuf]) {
        let paths: Vec<String> = paths.iter().map(|path| path.display().to_string()).collect();
        // The element under the pointer (or an ancestor) may take the drop.
        let mut current = self.scene.hit_test(position);
        while let Some(id) = current {
            let Some(node) = self.scene.node(id) else { break };
            if node.handles(PropKey::OnFilesDrop) {
                let json = serde_json::to_string(&paths).unwrap_or_else(|_| String::from("[]"));
                let origin = self.scene.window_origin(id);
                let payload = WireMap::new().with("paths", json).with("x", position.x - origin.x).with("y", position.y - origin.y).encode();
                return self.send_event(id, event_kind::FILES_DROP, &payload);
            }
            current = (node.parent != 0).then_some(node.parent);
        }
        let data = serde_json::json!({ "paths": paths, "x": position.x, "y": position.y }).to_string();
        self.shell.emit("drop", "files", &data);
    }

    /// A secondary click: opens the node's context menu, if it has one.
    pub(crate) fn open_context_menu(&mut self, window: crate::backend::WindowId, position: Point) {
        if let Some(hit) = self.scene.hit_test(position) {
            self.open_context_menu_of(window, hit, Some(position));
        }
    }

    /// Opens the menu declared by `from` or its nearest ancestor, at
    /// `position` or else in the middle of the element. Answers whether a
    /// menu opened.
    pub(crate) fn open_context_menu_of(&mut self, window: crate::backend::WindowId, from: NodeId, position: Option<Point>) -> bool {
        let mut current = from;
        let target = loop {
            let Some(node) = self.scene.node(current) else { return false };
            if node.props().bytes(PropKey::ContextMenuItems).is_some() {
                break current;
            }
            if node.parent == 0 {
                return false;
            }
            current = node.parent;
        };
        let Some(node) = self.scene.node(target) else { return false };
        let Some(menu) = node.props().bytes(PropKey::ContextMenuItems).and_then(context_menu).filter(|menu| !menu.is_empty()) else { return false };
        let position = position.unwrap_or_else(|| {
            let origin = self.scene.window_origin(target);
            Point::new(origin.x + node.frame.width / 2.0, origin.y + node.frame.height / 2.0)
        });
        let inbox = std::sync::Arc::clone(&self.inbox);
        self.backend.popup_menu(
            window,
            position,
            &menu,
            Box::new(move |choice| {
                if let Some(id) = choice {
                    inbox.post(crate::runtime::Inbound::ContextMenuAction { node: target, id });
                }
            }),
        );
        true
    }

    pub(crate) fn context_menu_chosen(&mut self, node: NodeId, id: &str) {
        if self.scene.node(node).is_some_and(|node| node.handles(PropKey::OnMenuAction)) {
            let payload = WireMap::new().with("id", id).encode();
            self.send_event(node, event_kind::MENU_ACTION, &payload);
        }
    }
}

/// `ContextMenuItems`: a JSON array of `{id, title, destructive, disabled}`
/// entries, which may also be `{separator: true}`, carry `checked` or nest
/// `children` (at most 64 entries per level, four levels).
fn context_menu(bytes: &[u8]) -> Option<Vec<MenuNode>> {
    context_menu_level(&serde_json::from_slice::<serde_json::Value>(bytes).ok()?, 0)
}

fn context_menu_level(value: &serde_json::Value, depth: usize) -> Option<Vec<MenuNode>> {
    let entries = value.as_array()?;
    if depth > 3 {
        return None;
    }
    let flag = |entry: &serde_json::Value, key: &str| entry.get(key).and_then(serde_json::Value::as_bool);
    Some(
        entries
            .iter()
            .take(64)
            .filter_map(|entry| {
                if flag(entry, "separator") == Some(true) {
                    return Some(MenuNode::Separator);
                }
                let label: String = entry.get("title")?.as_str()?.chars().take(256).collect();
                let enabled = flag(entry, "disabled") != Some(true);
                if let Some(children) = entry.get("children") {
                    return Some(MenuNode::Submenu { label, enabled, children: context_menu_level(children, depth + 1)? });
                }
                Some(MenuNode::Item {
                    id: entry.get("id")?.as_str()?.to_owned(),
                    label,
                    accelerator: None,
                    enabled,
                    checked: flag(entry, "checked"),
                    role: None,
                    destructive: flag(entry, "destructive") == Some(true),
                })
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_keep_only_safe_extensions() {
        let filters = parse_filters(r#"[{"name":"Images","extensions":[".PNG","jpg","../x",""]},{"name":"Empty","extensions":[]}]"#);
        assert_eq!(filters, vec![FileFilter { name: "Images".into(), extensions: vec!["png".into(), "jpg".into()] }]);
        assert!(parse_filters("not json").is_empty());
    }

    #[test]
    fn context_menus_decode_the_protocol_items() {
        let menu = context_menu(br#"[{"id":"copy","title":"Copy"},{"id":"delete","title":"Delete","destructive":true,"disabled":true},{"title":"No id"}]"#)
            .expect("valid");
        assert_eq!(menu.len(), 2);
        assert!(matches!(&menu[1], MenuNode::Item { destructive: true, enabled: false, .. }));
    }

    #[test]
    fn events_queue_until_php_polls() {
        let mut shell = Shell::default();
        shell.emit("menu", "file.new", "{}");
        assert_eq!(shell.pending.len(), 1);
        let (reply, receiver) = Reply::channel();
        shell.waiter = Some(reply);
        shell.emit("shortcut", "search", "{}");
        let (status, payload) = receiver.try_recv().expect("delivered to the waiter");
        assert_eq!(status, crate::runtime::MODULE_SUCCESS);
        assert_eq!(WireMap::decode(&payload).unwrap().text("id"), Some("search"));
        for index in 0..80 {
            shell.emit("menu", &index.to_string(), "{}");
        }
        assert_eq!(shell.pending.len(), MAX_PENDING_EVENTS);
    }
}

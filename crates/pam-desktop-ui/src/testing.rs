//! A backend that records every command, for tests of the render core and of
//! anything built on it.

use std::collections::HashMap;

use crate::backend::{
    AlertConfig, Backend, ImageContent, ImageKey, ImageSource, InputConfig, NodeId, ScrollConfig, TextContent, ViewClass,
    WindowConfig, WindowId, WindowMetrics,
};
use crate::geometry::{Point, Rect, Size};
use crate::shell::{Cursor, FileDialogConfig, MenuNode, NotificationConfig, Shortcut, TrayConfig, WindowCommand, WindowState};
use crate::style::{Decoration, LayerState};
use crate::text::{TextBlock, TextMetrics};

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    OpenWindow(WindowId),
    CloseWindow(WindowId),
    SetWindowRoot(WindowId, Option<NodeId>),
    Overlays(WindowId, Vec<NodeId>),
    Create(NodeId, ViewClass),
    Destroy(NodeId),
    Children(NodeId, Vec<NodeId>),
    Frame(NodeId, Rect),
    Decoration(NodeId, Decoration),
    Layer(NodeId, LayerState),
    Text(NodeId, Option<String>),
    Focusable(NodeId, bool),
    Focus(Option<NodeId>),
    InputConfig(NodeId, Box<InputConfig>),
    InputText(NodeId, String, Option<(u32, u32)>),
    InputSelection(NodeId, u32, u32),
    LoadImage(ImageKey),
    ReleaseImage(ImageKey),
    Image(NodeId, Option<ImageContent>),
    ScrollContent(NodeId, Size),
    ScrollConfig(NodeId, ScrollConfig),
    ScrollTo(NodeId, Point, bool),
    ClipboardSet(String),
    Alert(AlertConfig),
    Appearance(Option<bool>),
    MenuBar(Vec<MenuNode>),
    PopupMenu(Point, Vec<MenuNode>),
    Tray(Option<TrayConfig>),
    PickFiles(FileDialogConfig),
    Notify(NotificationConfig),
    Window(WindowId, WindowCommand),
    Shortcuts(Vec<Shortcut>),
    Cursor(NodeId, Cursor),
    Tooltip(NodeId, Option<String>),
    BeginWindowDrag(WindowId),
    RequestFrame,
    ScheduleWake(u32),
    Quit,
}

/// Records commands and mirrors the view tree so tests can assert on the
/// resulting native state as well as on the command stream.
#[derive(Default)]
pub struct RecordingBackend {
    pub commands: Vec<Command>,
    pub views: HashMap<NodeId, ViewClass>,
    pub children: HashMap<NodeId, Vec<NodeId>>,
    pub frames: HashMap<NodeId, Rect>,
    pub texts: HashMap<NodeId, TextContent>,
    /// Text last set on each native input.
    pub inputs: HashMap<NodeId, String>,
    pub clipboard: Option<String>,
    /// What `show_alert` answers.
    pub alert_choice: Option<usize>,
    /// What `popup_menu` answers.
    pub menu_choice: Option<String>,
    /// What `pick_files` answers.
    pub picked_files: Vec<std::path::PathBuf>,
    pub window_state: WindowState,
    pub metrics: WindowMetrics,
    /// Width of one character used by the fake text measurer.
    pub character_width: f32,
}

impl RecordingBackend {
    #[must_use]
    pub fn new() -> Self {
        Self { character_width: 8.0, ..Self::default() }
    }

    /// Returns and clears the commands recorded so far.
    pub fn take(&mut self) -> Vec<Command> {
        std::mem::take(&mut self.commands)
    }

    #[must_use]
    pub fn count(&self, matches: impl Fn(&Command) -> bool) -> usize {
        self.commands.iter().filter(|command| matches(command)).count()
    }
}

impl Backend for RecordingBackend {
    fn open_window(&mut self, window: WindowId, config: &WindowConfig) {
        self.metrics.size = config.size;
        self.commands.push(Command::OpenWindow(window));
    }

    fn close_window(&mut self, window: WindowId) {
        self.commands.push(Command::CloseWindow(window));
    }

    fn window_metrics(&self, _window: WindowId) -> WindowMetrics {
        self.metrics
    }

    fn set_window_root(&mut self, window: WindowId, root: Option<NodeId>) {
        self.commands.push(Command::SetWindowRoot(window, root));
    }

    fn set_window_overlays(&mut self, window: WindowId, overlays: &[NodeId]) {
        for overlay in overlays {
            assert!(self.views.contains_key(overlay), "overlay {overlay} has no view");
        }
        self.commands.push(Command::Overlays(window, overlays.to_vec()));
    }

    fn create_view(&mut self, id: NodeId, class: ViewClass) {
        assert!(self.views.insert(id, class).is_none(), "view {id} created twice");
        self.commands.push(Command::Create(id, class));
    }

    fn destroy_view(&mut self, id: NodeId) {
        assert!(self.views.remove(&id).is_some(), "view {id} destroyed but never created");
        self.children.remove(&id);
        self.frames.remove(&id);
        self.texts.remove(&id);
        for children in self.children.values_mut() {
            children.retain(|child| *child != id);
        }
        self.commands.push(Command::Destroy(id));
    }

    fn set_children(&mut self, parent: NodeId, children: &[NodeId]) {
        assert!(self.views.contains_key(&parent), "children set on missing view {parent}");
        for child in children {
            assert!(self.views.contains_key(child), "child {child} of {parent} has no view");
            for (other, siblings) in &mut self.children {
                if *other != parent {
                    siblings.retain(|sibling| sibling != child);
                }
            }
        }
        self.children.insert(parent, children.to_vec());
        self.commands.push(Command::Children(parent, children.to_vec()));
    }

    fn set_frame(&mut self, id: NodeId, frame: Rect) {
        assert!(self.views.contains_key(&id), "frame set on missing view {id}");
        self.frames.insert(id, frame);
        self.commands.push(Command::Frame(id, frame));
    }

    fn set_decoration(&mut self, id: NodeId, decoration: &Decoration) {
        self.commands.push(Command::Decoration(id, decoration.clone()));
    }

    fn set_layer(&mut self, id: NodeId, layer: &LayerState) {
        self.commands.push(Command::Layer(id, *layer));
    }

    fn set_text(&mut self, id: NodeId, text: Option<&TextContent>) {
        match text {
            Some(content) => {
                self.texts.insert(id, content.clone());
            }
            None => {
                self.texts.remove(&id);
            }
        }
        self.commands.push(Command::Text(id, text.map(|content| content.block.plain_text())));
    }

    fn set_focusable(&mut self, id: NodeId, focusable: bool) {
        self.commands.push(Command::Focusable(id, focusable));
    }

    fn focus(&mut self, id: Option<NodeId>) {
        self.commands.push(Command::Focus(id));
    }

    fn set_input_config(&mut self, id: NodeId, config: &InputConfig) {
        self.commands.push(Command::InputConfig(id, Box::new(config.clone())));
    }

    fn set_input_text(&mut self, id: NodeId, text: &str, selection: Option<(u32, u32)>) {
        self.inputs.insert(id, text.to_owned());
        self.commands.push(Command::InputText(id, text.to_owned(), selection));
    }

    fn set_input_selection(&mut self, id: NodeId, start: u32, end: u32) {
        self.commands.push(Command::InputSelection(id, start, end));
    }

    fn load_image(&mut self, key: ImageKey, _source: ImageSource) {
        self.commands.push(Command::LoadImage(key));
    }

    fn release_image(&mut self, key: ImageKey) {
        self.commands.push(Command::ReleaseImage(key));
    }

    fn set_image(&mut self, id: NodeId, image: Option<&ImageContent>) {
        self.commands.push(Command::Image(id, image.copied()));
    }

    fn set_scroll_content(&mut self, id: NodeId, size: Size) {
        self.commands.push(Command::ScrollContent(id, size));
    }

    fn set_scroll_config(&mut self, id: NodeId, config: &ScrollConfig) {
        self.commands.push(Command::ScrollConfig(id, *config));
    }

    fn scroll_to(&mut self, id: NodeId, offset: Point, animated: bool) {
        self.commands.push(Command::ScrollTo(id, offset, animated));
    }

    /// Monospace model: every character is `character_width` wide and lines
    /// are 1.25 × the first run's font size tall.
    fn measure_text(&mut self, block: &TextBlock, max_width: f32) -> TextMetrics {
        let characters: usize = block.runs.iter().map(|run| run.text.chars().count()).sum();
        let font_size = block.runs.first().map_or(14.0, |run| run.font.size);
        let line_height = if block.line_height > 0.0 { block.line_height } else { font_size * 1.25 };
        let natural = characters as f32 * self.character_width;
        let (width, mut lines) = if max_width.is_finite() && natural > max_width && max_width > 0.0 {
            (max_width, (natural / max_width).ceil() as u32)
        } else {
            (natural, 1)
        };
        if block.max_lines > 0 {
            lines = lines.min(block.max_lines);
        }
        TextMetrics { width, height: lines as f32 * line_height, first_baseline: font_size, line_count: lines }
    }

    fn request_frame(&mut self) {
        self.commands.push(Command::RequestFrame);
    }

    fn clipboard_set_text(&mut self, text: &str) {
        self.clipboard = Some(text.to_owned());
        self.commands.push(Command::ClipboardSet(text.to_owned()));
    }

    fn clipboard_text(&mut self, done: Box<dyn FnOnce(Option<String>)>) {
        done(self.clipboard.clone());
    }

    /// Answers with `alert_choice` right away.
    fn show_alert(&mut self, _window: WindowId, alert: &AlertConfig, done: Box<dyn FnOnce(Option<usize>)>) {
        self.commands.push(Command::Alert(alert.clone()));
        done(self.alert_choice);
    }

    fn set_appearance(&mut self, dark: Option<bool>) {
        self.commands.push(Command::Appearance(dark));
    }

    fn set_menu_bar(&mut self, menu: &[MenuNode]) {
        self.commands.push(Command::MenuBar(menu.to_vec()));
    }

    /// Answers with `menu_choice` right away.
    fn popup_menu(&mut self, _window: WindowId, position: Point, menu: &[MenuNode], done: Box<dyn FnOnce(Option<String>)>) {
        self.commands.push(Command::PopupMenu(position, menu.to_vec()));
        done(self.menu_choice.clone());
    }

    fn set_tray(&mut self, tray: Option<&TrayConfig>) {
        self.commands.push(Command::Tray(tray.cloned()));
    }

    /// Answers with `picked_files` right away.
    fn pick_files(&mut self, _window: WindowId, config: &FileDialogConfig, done: Box<dyn FnOnce(Vec<std::path::PathBuf>)>) {
        self.commands.push(Command::PickFiles(config.clone()));
        done(self.picked_files.clone());
    }

    fn notify(&mut self, notification: &NotificationConfig) {
        self.commands.push(Command::Notify(notification.clone()));
    }

    fn window_command(&mut self, window: WindowId, command: &WindowCommand) {
        self.commands.push(Command::Window(window, command.clone()));
    }

    fn window_state(&self, _window: WindowId) -> WindowState {
        self.window_state
    }

    fn set_shortcuts(&mut self, shortcuts: &[Shortcut]) {
        self.commands.push(Command::Shortcuts(shortcuts.to_vec()));
    }

    fn set_cursor(&mut self, id: NodeId, cursor: Cursor) {
        self.commands.push(Command::Cursor(id, cursor));
    }

    fn set_tooltip(&mut self, id: NodeId, text: Option<&str>) {
        self.commands.push(Command::Tooltip(id, text.map(str::to_owned)));
    }

    fn begin_window_drag(&mut self, window: WindowId) {
        self.commands.push(Command::BeginWindowDrag(window));
    }

    fn schedule_wake(&mut self, delay_ms: u32) {
        self.commands.push(Command::ScheduleWake(delay_ms));
    }

    fn quit(&mut self) {
        self.commands.push(Command::Quit);
    }
}

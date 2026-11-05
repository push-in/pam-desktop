//! The contract every platform backend implements.
//!
//! The render core owns the scene (nodes, resolved styles, interaction state)
//! and drives a backend with small retained-mode commands. A backend owns the
//! native objects: GTK widgets, AppKit views, or a Direct2D display list with
//! native child controls. Backends never interpret wire properties.

use crate::color::Color;
use crate::geometry::{Insets, Point, Rect, Size};
use crate::shell::{Cursor, FileDialogConfig, MenuNode, NotificationConfig, Shortcut, TrayConfig, WindowCommand, WindowState};
use crate::style::{Decoration, LayerState};
use crate::text::{TextBlock, TextMetrics};

pub type NodeId = u64;
pub type WindowId = u32;

/// The main application window.
pub const MAIN_WINDOW: WindowId = 1;

/// What kind of native object backs a node.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ViewClass {
    /// Decorated box with optional text and image content, and children.
    Box,
    /// Scrolling viewport; its children live in content coordinates.
    Scroll,
    /// Native single-line text field.
    Input,
    /// Native multi-line text editor.
    TextArea,
    /// Pixels painted by the core (canvas, drawing surface).
    Surface,
    /// A video or audio player; the box may also show a poster image.
    Media,
    /// An embedded web page.
    WebView,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum VerticalAlign {
    #[default]
    Top,
    Center,
    Bottom,
}

/// Text painted inside a box.
#[derive(Clone, Debug, PartialEq)]
pub struct TextContent {
    pub block: TextBlock,
    /// Space between the box edges and the text (padding plus borders).
    pub insets: Insets,
    pub vertical: VerticalAlign,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PointerButton {
    Primary,
    Secondary,
    Middle,
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct Modifiers {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    /// Command on macOS, the Windows/Super key elsewhere.
    pub meta: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PointerPhase {
    Down,
    Up,
    Move,
    /// The pointer left the window or the gesture was taken by the system.
    Cancel,
    Leave,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointerEvent {
    pub window: WindowId,
    pub phase: PointerPhase,
    /// Position in the window's content area, logical units.
    pub position: Point,
    pub button: PointerButton,
    pub modifiers: Modifiers,
    /// 1 for a single click, 2 for a double click, …
    pub click_count: u8,
    /// Monotonic milliseconds.
    pub time_ms: u64,
}

/// A key press or release that no native control consumed.
#[derive(Clone, Debug, PartialEq)]
pub struct KeyEvent {
    pub window: WindowId,
    pub pressed: bool,
    /// The W3C UI Events `key` value: `"Enter"`, `"Escape"`, `"ArrowLeft"`,
    /// `"Tab"`, `" "`, `"a"`, `"F5"`, …
    pub key: String,
    pub modifiers: Modifiers,
    pub repeat: bool,
}

/// Everything a backend reports to the core. Backends push these into the
/// shared queue and ask the runtime to drain it.
#[derive(Clone, Debug, PartialEq)]
pub enum BackendEvent {
    Pointer(PointerEvent),
    Key(KeyEvent),
    /// The user asked to go back (mouse back button, system back gesture).
    BackRequested { window: WindowId },
    /// Keyboard focus entered or left a focusable view.
    FocusChanged { id: NodeId, focused: bool },
    Input { id: NodeId, event: InputEvent },
    /// A menu bar or tray menu item was chosen.
    MenuAction { id: String },
    /// A registered shortcut fired.
    Shortcut { id: String },
    /// The tray icon itself was activated.
    TrayActivated,
    NotificationActivated { id: String },
    /// Files were dropped on a window.
    FilesDropped { window: WindowId, position: Point, paths: Vec<std::path::PathBuf> },
    WindowStateChanged { window: WindowId, state: WindowState },
    WindowResized { window: WindowId, size: Size, scale: f32 },
    WindowCloseRequested { window: WindowId },
    WindowFocusChanged { window: WindowId, focused: bool },
    AppearanceChanged { dark: bool },
    /// A display frame for running animations (monotonic milliseconds).
    Frame { time_ms: u64 },
    /// The deadline asked with `schedule_wake` passed.
    Wake,
    ScrollChanged { id: NodeId, offset: Point },
    /// Wheel or two-finger scrolling that no scrolling view consumed, in
    /// logical units (positive `y` scrolls the content up).
    Wheel { window: WindowId, position: Point, delta: Point, modifiers: Modifiers },
    /// A pinch: `scale` is relative to the start of the gesture (1 = none).
    Magnify { window: WindowId, position: Point, phase: GesturePhase, scale: f32 },
    /// A two-finger rotation, in radians since the start of the gesture
    /// (positive is clockwise).
    Rotate { window: WindowId, position: Point, phase: GesturePhase, radians: f32 },
    /// Plain text was dropped on a window (from this or another application).
    TextDropped { window: WindowId, position: Point, text: String },
    /// The drag started with `begin_text_drag` ended; `dropped` tells
    /// whether a target accepted it.
    DragFinished { source: NodeId, dropped: bool },
    Media { id: NodeId, event: MediaEvent },
    WebView { id: NodeId, event: WebViewEvent },
    /// Assistive technology ran an action on a view: `activate` (the default
    /// press) or the name of one of its custom actions.
    AccessibilityAction { id: NodeId, name: String },
    /// An image finished decoding; `size` is in image pixels.
    ImageLoaded { key: ImageKey, size: Size },
    ImageFailed { key: ImageKey, message: String },
}

/// What the field is for: backends map it to input hints, on-screen
/// keyboards and autofill.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum InputPurpose {
    #[default]
    Text,
    Email,
    /// Whole numbers.
    Number,
    /// Signed decimals.
    Decimal,
    Phone,
    Url,
    Search,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Capitalization {
    None,
    #[default]
    Sentences,
    Words,
    Characters,
}

/// Everything a native text field or editor needs besides its text.
#[derive(Clone, Debug, PartialEq)]
pub struct InputConfig {
    pub placeholder: String,
    pub placeholder_color: Option<Color>,
    pub font: crate::text::FontSpec,
    /// `None` uses the platform's text color.
    pub color: Option<Color>,
    pub align: crate::text::TextAlign,
    pub vertical: VerticalAlign,
    /// Space between the box edges and the text (padding plus borders).
    pub insets: Insets,
    pub letter_spacing: f32,
    /// Password masking (single-line only).
    pub secure: bool,
    /// `false` is read-only: focusable and selectable, never modified.
    pub editable: bool,
    pub enabled: bool,
    /// Maximum length in UTF-16 code units.
    pub max_length: Option<u32>,
    pub purpose: InputPurpose,
    pub capitalization: Capitalization,
    pub spellcheck: bool,
    pub selection_color: Option<Color>,
    pub caret_color: Option<Color>,
    pub caret_hidden: bool,
    pub context_menu_hidden: bool,
    pub select_on_focus: bool,
    /// Enter submits instead of inserting a line break (editors only; a
    /// single-line field always reports Enter).
    pub submit_on_enter: bool,
}

impl Default for InputConfig {
    fn default() -> Self {
        Self {
            placeholder: String::new(),
            placeholder_color: None,
            font: crate::text::FontSpec::default(),
            color: None,
            align: crate::text::TextAlign::Start,
            vertical: VerticalAlign::Center,
            insets: Insets::ZERO,
            letter_spacing: 0.0,
            secure: false,
            editable: true,
            enabled: true,
            max_length: None,
            purpose: InputPurpose::Text,
            capitalization: Capitalization::Sentences,
            spellcheck: true,
            selection_color: None,
            caret_color: None,
            caret_hidden: false,
            context_menu_hidden: false,
            select_on_focus: false,
            submit_on_enter: false,
        }
    }
}

/// What the user did in a native text control. Offsets are UTF-16 code units.
#[derive(Clone, Debug, PartialEq)]
pub enum InputEvent {
    /// The text changed by typing, pasting, deleting or an input method.
    /// Never sent for text set through `set_input_text`.
    Edited { text: String, selection: (u32, u32) },
    SelectionChanged { start: u32, end: u32 },
    /// A key about to edit the text: `"Enter"`, `"Backspace"` or the inserted text.
    KeyPressed { key: String },
    /// Enter in a single-line field, or in an editor with `submit_on_enter`.
    Submitted,
    /// Size of the laid out text, for controls that grow with their content.
    ContentSizeChanged { size: Size },
}

/// Identifies a decoded image owned by the backend.
pub type ImageKey = u64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageSource {
    /// Encoded bytes (PNG, JPEG, WebP, GIF, SVG… whatever the platform decodes).
    Bytes(std::sync::Arc<[u8]>),
    File(std::path::PathBuf),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ImageFit {
    /// Fill the box, cropping the overflow (aspect ratio kept).
    #[default]
    Cover,
    /// Fit inside the box (aspect ratio kept).
    Contain,
    /// Stretch to the box.
    Fill,
    /// Natural size, centered.
    Center,
    /// Natural size, tiled from the top-left corner.
    Repeat,
}

/// What a media view plays.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MediaSource {
    /// An `http://` or `https://` stream.
    Url(String),
    File(std::path::PathBuf),
}

/// Everything a media view should be doing. The backend compares it with
/// the previous state: a new `source` replaces the player (`None` releases
/// it and its decoder), a new `seek` serial seeks to that position (and is
/// the start position of a new source), the rest applies live.
#[derive(Clone, Debug, PartialEq)]
pub struct MediaState {
    pub source: Option<MediaSource>,
    /// Play when prepared; pause otherwise. Never resets the position.
    pub playing: bool,
    /// Serial of the last seek request and its position in seconds.
    pub seek: (u32, f64),
    /// Show the platform's transport controls.
    pub controls: bool,
    pub looping: bool,
    pub muted: bool,
    /// 0..1.
    pub volume: f32,
    /// 0.25..4; backends without rate control play at normal speed.
    pub rate: f32,
    /// How the video fills the view (`Repeat` never appears).
    pub fit: ImageFit,
}

/// What a platform player reports.
#[derive(Clone, Debug, PartialEq)]
pub enum MediaEvent {
    /// The media is prepared: video size in pixels (0 for audio) and
    /// duration in seconds (0 when unknown).
    Ready { width: u32, height: u32, duration: f64 },
    /// The first video frame is on screen.
    Rendered,
    /// Playback stalled waiting for data, or resumed.
    Buffering(bool),
    /// Playback reached the end (not reported while looping).
    Ended,
    Failed(String),
    /// The player started or paused by itself (native controls).
    Playing(bool),
}

/// What a web view shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum WebSource {
    /// Nothing new to load; the page on screen stays.
    #[default]
    None,
    Url(String),
    /// Inline HTML, loaded with an `about:blank` origin.
    Html(String),
}

/// Everything a web view should be. The backend compares it with the
/// previous state: a new `source` navigates, a new `stop` serial stops the
/// load in progress, a new non-empty `injected_script` is evaluated at once
/// when a page is loaded, the rest applies live.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebViewState {
    pub source: WebSource,
    pub stop: u32,
    pub javascript: bool,
    /// Local and session storage.
    pub dom_storage: bool,
    /// Empty keeps the engine's own.
    pub user_agent: String,
    /// Evaluated after every page finish (when JavaScript is enabled).
    pub injected_script: String,
    /// Media may play without a user gesture.
    pub inline_media: bool,
    /// Lower-cased hosts main-frame navigations are limited to; empty
    /// allows everything. See `webview::navigation_allowed`.
    pub allowed_hosts: Vec<String>,
    pub scroll_enabled: bool,
}

/// What a platform web view reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WebViewEvent {
    /// A main-frame page finished loading; `url` is its address.
    Loaded { url: String },
    /// A main-frame load failed (not an HTTP error status).
    Failed(String),
    /// The backend refused a main-frame navigation the allow-list excludes.
    Blocked { url: String },
    /// The page called `PamNative.postMessage(text)`; `url` is the main
    /// frame's address at that moment.
    Message { message: String, url: String },
}

/// One primitive of a drawing surface, in the view's logical coordinates.
/// Commands paint in order, each over the previous ones, anti-aliased.
#[derive(Clone, Debug, PartialEq)]
pub enum DrawCommand {
    /// A filled rectangle; `radius` rounds every corner with a circular arc.
    Rect { rect: Rect, radius: f32, color: Color },
    Circle { center: Point, radius: f32, color: Color },
    /// A segment with round caps. A zero width is one device pixel.
    Line { from: Point, to: Point, width: f32, color: Color },
    /// A polyline with round caps and joins; a single point is a dot.
    /// `erase` clears what the current group holds instead of painting.
    Stroke { points: Vec<Point>, width: f32, color: Color, erase: bool },
    /// Opens an offscreen group clipped to `clip`: erasing strokes only
    /// reach what the group holds, never what is under the view.
    PushGroup { clip: Rect },
    /// Composites the group opened by `PushGroup`.
    PopGroup,
}

/// An image painted inside a box, above its background and below its text.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImageContent {
    pub key: ImageKey,
    pub fit: ImageFit,
    /// Replaces every opaque pixel with this color (template images, icons).
    pub tint: Option<Color>,
    /// Painted over the image.
    pub overlay: Option<Color>,
    /// Space between the box edges and the image (borders).
    pub insets: Insets,
    /// 0..1, used for fade-in.
    pub opacity: f32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AlertButtonStyle {
    #[default]
    Default,
    Cancel,
    Destructive,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlertButton {
    pub label: String,
    pub style: AlertButtonStyle,
}

/// A native message dialog with up to three buttons.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AlertConfig {
    pub title: String,
    pub message: String,
    /// Empty means a single "OK" button.
    pub buttons: Vec<AlertButton>,
}

/// How a scroll view behaves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollConfig {
    pub horizontal: bool,
    /// User scrolling (wheel, drag, scrollbars) is allowed.
    pub enabled: bool,
    pub shows_indicators: bool,
}

impl Default for ScrollConfig {
    fn default() -> Self {
        Self { horizontal: false, enabled: true, shows_indicators: true }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct WindowConfig {
    pub title: String,
    pub size: Size,
    pub min_size: Option<Size>,
    pub resizable: bool,
    /// The system title bar and borders. Without them the application
    /// draws its own and moves the window through `begin_window_drag`.
    pub decorated: bool,
    /// The window this one belongs to: it stays above it and closes with it.
    pub parent: Option<WindowId>,
    /// Blocks input to `parent` while open (a dialog).
    pub modal: bool,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self { title: String::from("PAM Desktop"), size: Size::new(960.0, 640.0), min_size: None, resizable: true, decorated: true, parent: None, modal: false }
    }
}

/// Phase of a continuous trackpad or touch gesture.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GesturePhase {
    Begin,
    Update,
    End,
    Cancel,
}

/// Display and accessibility environment of a window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowMetrics {
    pub size: Size,
    /// Device pixels per logical unit.
    pub scale: f32,
    /// Accessibility text scale (1.0 = default).
    pub font_scale: f32,
    pub dark: bool,
    pub refresh_rate: f32,
    pub reduced_motion: bool,
}

impl Default for WindowMetrics {
    fn default() -> Self {
        Self { size: Size::new(960.0, 640.0), scale: 1.0, font_scale: 1.0, dark: false, refresh_rate: 60.0, reduced_motion: false }
    }
}

pub trait Backend {
    // ---- windows -------------------------------------------------------

    /// Creates (or reconfigures) a native window. It becomes visible when its
    /// first root is attached.
    fn open_window(&mut self, window: WindowId, config: &WindowConfig);
    fn close_window(&mut self, window: WindowId);
    fn window_metrics(&self, window: WindowId) -> WindowMetrics;
    /// Attaches the root view of a window's content.
    fn set_window_root(&mut self, window: WindowId, root: Option<NodeId>);
    /// Views stacked above the root, back to front, each placed by its own
    /// frame: presented modals cover the window, host chrome (error
    /// reports, toasts) may be smaller.
    fn set_window_overlays(&mut self, window: WindowId, overlays: &[NodeId]);

    // ---- views ---------------------------------------------------------

    fn create_view(&mut self, id: NodeId, class: ViewClass);
    fn destroy_view(&mut self, id: NodeId);
    /// Replaces the ordered children of a view (back to front).
    fn set_children(&mut self, parent: NodeId, children: &[NodeId]);
    /// Frame relative to the parent view (content coordinates under a scroll view).
    fn set_frame(&mut self, id: NodeId, frame: Rect);
    fn set_decoration(&mut self, id: NodeId, decoration: &Decoration);
    fn set_layer(&mut self, id: NodeId, layer: &LayerState);
    fn set_text(&mut self, id: NodeId, text: Option<&TextContent>);
    /// Whether the view takes part in keyboard focus traversal (Tab).
    fn set_focusable(&mut self, id: NodeId, focusable: bool);
    /// What assistive technology is told about a view; `None` restores the
    /// plain defaults (a group, or static text when the view shows text).
    fn set_accessibility(&mut self, id: NodeId, accessibility: Option<&crate::accessibility::Accessibility>);
    /// Moves keyboard focus to a view, or clears it with `None`.
    fn focus(&mut self, id: Option<NodeId>);

    // ---- text input ----------------------------------------------------

    /// Configures a `ViewClass::Input` or `ViewClass::TextArea` view.
    fn set_input_config(&mut self, id: NodeId, config: &InputConfig);
    /// Replaces the text (and optionally the selection, UTF-16 offsets)
    /// without reporting an edit back.
    fn set_input_text(&mut self, id: NodeId, text: &str, selection: Option<(u32, u32)>);
    fn set_input_selection(&mut self, id: NodeId, start: u32, end: u32);

    // ---- images --------------------------------------------------------

    /// Decodes an image off the UI thread and reports `ImageLoaded` or
    /// `ImageFailed` for `key`. With `max_dimension`, an image whose longest
    /// side is larger is decoded (or scaled) down to it, keeping its ratio.
    fn load_image(&mut self, key: ImageKey, source: ImageSource, max_dimension: Option<u32>);
    /// Drops a decoded image no view shows any more.
    fn release_image(&mut self, key: ImageKey);
    fn set_image(&mut self, id: NodeId, image: Option<&ImageContent>);
    /// The UTF-16 offset of the character boundary nearest to `point` (in
    /// the view's own coordinates) in the text the view shows; `None` when
    /// it shows no text.
    fn text_index_at(&self, id: NodeId, point: Point) -> Option<u32>;
    /// Paints a range of the view's text as selected (UTF-16 offsets,
    /// `start <= end`, with the platform's selection colors); `None` clears it.
    fn set_text_selection(&mut self, id: NodeId, range: Option<(u32, u32)>);
    /// Starts a system drag of plain text from the pointer press being
    /// handled in `window`, with a snapshot of the `source` view under the
    /// pointer. Reports `DragFinished` when the drag ends.
    fn begin_text_drag(&mut self, window: WindowId, source: NodeId, text: &str);
    /// Brings the player of a `Media` view to `state`.
    fn set_media(&mut self, id: NodeId, state: &MediaState);
    /// Current position and duration of a prepared player, in seconds.
    fn media_position(&self, id: NodeId) -> Option<(f64, f64)>;
    /// Brings the page of a `WebView` view to `state`. A backend without a
    /// web engine reports `WebViewEvent::Failed` for any source.
    fn set_web_view(&mut self, id: NodeId, state: &WebViewState);
    /// Replaces what a view draws above its image and below its children.
    /// The view is clipped to its bounds; an empty list draws nothing.
    fn set_drawing(&mut self, id: NodeId, commands: &[DrawCommand]);

    // ---- scrolling -----------------------------------------------------

    /// Extent of the scrollable content of a `ViewClass::Scroll` view.
    fn set_scroll_content(&mut self, id: NodeId, size: Size);
    fn set_scroll_config(&mut self, id: NodeId, config: &ScrollConfig);
    /// Moves the viewport of a scroll view to `offset` (content coordinates).
    fn scroll_to(&mut self, id: NodeId, offset: Point, animated: bool);

    // ---- text ----------------------------------------------------------

    /// Measures a paragraph with the platform's own shaper, exactly as
    /// `set_text` will draw it. `max_width` is infinite when unconstrained.
    fn measure_text(&mut self, block: &TextBlock, max_width: f32) -> TextMetrics;

    // ---- frames --------------------------------------------------------

    /// Requests one `BackendEvent::Frame` on the next display refresh.
    fn request_frame(&mut self);
    /// Requests one `BackendEvent::Wake` after `delay_ms` (long press and
    /// debounce deadlines). A newer request replaces a pending one.
    fn schedule_wake(&mut self, delay_ms: u32);

    // ---- system services -----------------------------------------------

    fn clipboard_set_text(&mut self, text: &str);
    /// Reads the clipboard text; `done` runs on the UI thread, later.
    fn clipboard_text(&mut self, done: Box<dyn FnOnce(Option<String>)>);
    /// Shows a native message dialog. `done` receives the index of the
    /// pressed button, or `None` when it was dismissed another way.
    fn show_alert(&mut self, window: WindowId, alert: &AlertConfig, done: Box<dyn FnOnce(Option<usize>)>);
    /// Forces light (`Some(false)`) or dark (`Some(true)`) native chrome, or
    /// follows the system (`None`).
    fn set_appearance(&mut self, dark: Option<bool>);

    // ---- desktop shell -------------------------------------------------

    /// Replaces the application menu bar (empty removes it).
    fn set_menu_bar(&mut self, menu: &[MenuNode]);
    /// Opens a context menu at `position` (window coordinates). `done`
    /// receives the id of the chosen item, or `None` when dismissed.
    fn popup_menu(&mut self, window: WindowId, position: Point, menu: &[MenuNode], done: Box<dyn FnOnce(Option<String>)>);
    /// Shows, updates or removes (`None`) the status area icon.
    fn set_tray(&mut self, tray: Option<&TrayConfig>);
    /// Opens a native file dialog; `done` receives the chosen paths (empty
    /// when cancelled).
    fn pick_files(&mut self, window: WindowId, config: &FileDialogConfig, done: Box<dyn FnOnce(Vec<std::path::PathBuf>)>);
    fn notify(&mut self, notification: &NotificationConfig);
    fn window_command(&mut self, window: WindowId, command: &WindowCommand);
    fn window_state(&self, window: WindowId) -> WindowState;
    /// Replaces the registered shortcuts; each one reports `Shortcut` events.
    fn set_shortcuts(&mut self, shortcuts: &[Shortcut]);
    fn set_cursor(&mut self, id: NodeId, cursor: Cursor);
    fn set_tooltip(&mut self, id: NodeId, text: Option<&str>);
    /// Starts moving the window with the pointer (the press that is being
    /// handled grabbed a drag region).
    fn begin_window_drag(&mut self, window: WindowId);

    // ---- application ---------------------------------------------------

    /// Closes every window and leaves the platform's main loop.
    fn quit(&mut self);
}

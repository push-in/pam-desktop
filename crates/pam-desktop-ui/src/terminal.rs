//! The `Terminal` element: a VT terminal bound to a channel of the
//! [`TerminalHub`], painted on a drawing surface.
//!
//! Output never passes through PHP. The hub calls the subscriber on the
//! producer's thread (`pam-ssh`); it feeds the bytes to the emulator there
//! (alacritty_terminal: parser, grid, scrollback, selection), marks the
//! terminal changed and posts one notification to the inbox until the UI
//! thread has taken it. The UI thread repaints at most once per frame
//! interval and only when something changed: no output, no cursor or
//! selection change, no work. The cursor does not blink.
//!
//! Painting is a list of [`DrawCommand`]s: one rectangle per run of cells
//! with the same background, one text command per run of cells with the
//! same style on a row (each non-ASCII character on its own, at its cell, so
//! fallback fonts cannot shift the grid), underlines, the selection and the
//! cursor.
//!
//! Keys of a focused terminal become bytes ([`encode_key`]) written to the
//! hub; the pointer selects (cells, words on a double click, lines on a
//! triple click); the wheel scrolls the scrollback (or sends arrows in the
//! alternate screen).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use alacritty_terminal::Term;
use alacritty_terminal::event::{Event as TermEvent, EventListener, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point as GridPoint, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, TermMode};
use alacritty_terminal::vte::ansi::{
    Color as AnsiColor, CursorShape, NamedColor, Processor, Rgb, StdSyncHandler,
};
use pam_desktop_wire::{NodeKind, PropKey};

use crate::animator::Timer;
use crate::backend::{
    DrawCommand, KeyEvent, Modifiers, NodeId, PointerButton, PointerEvent, PointerPhase,
};
use crate::chrome::Measure;
use crate::color::Color;
use crate::context::{Ctx, HostCommand, event_kind};
use crate::geometry::{Point, Rect, Size};
use crate::modules::terminal::{TerminalEvent, TerminalHub, TerminalSubscription};
use crate::scene::{Change, IdMap};
use crate::text::{FontSpec, TextAlign, TextBlock, TextRun};
use crate::wiremap::WireMap;

/// Keys a focused terminal takes from focus traversal and scrolling.
pub const CAPTURED_KEYS: &str = "Tab,Escape,Enter,Space,Backspace,Delete,Insert,ArrowUp,ArrowDown,ArrowLeft,ArrowRight,Home,End,PageUp,PageDown,F1,F2,F3,F4,F5,F6,F7,F8,F9,F10,F11,F12";
pub const DEFAULT_FONT_FAMILY: &str =
    "'JetBrains Mono', 'DejaVu Sans Mono', 'Cascadia Mono', Menlo, Consolas, monospace";
pub const DEFAULT_FONT_SIZE: f32 = 13.0;
pub const DEFAULT_SCROLLBACK: usize = 5000;
/// Hard cap of the scrollback, whatever the property says.
pub const MAX_SCROLLBACK: usize = 100_000;
pub const MAX_COLS: usize = 1000;
pub const MAX_ROWS: usize = 1000;
/// The shortest interval between two paints.
const FRAME_MS: u64 = 16;
/// How long the grid size must hold before the remote end hears it.
const RESIZE_DEBOUNCE_MS: u64 = 80;
/// Pasted text larger than this is cut.
pub const MAX_PASTE: usize = 1024 * 1024;

pub const DEFAULT_PALETTE: [u32; 16] = [
    0x282C34, 0xE06C75, 0x98C379, 0xE5C07B, 0x61AFEF, 0xC678DD, 0x56B6C2, 0xABB2BF, 0x5C6370,
    0xF0828B, 0xB5E08F, 0xF2D08F, 0x82C1F5, 0xD99BE8, 0x7CCFD9, 0xFFFFFF,
];
pub const DEFAULT_BACKGROUND: u32 = 0xFF1B1D23;
pub const DEFAULT_FOREGROUND: u32 = 0xFFD0D4DC;
pub const DEFAULT_CURSOR: u32 = 0xFFE8E8E8;
pub const DEFAULT_SELECTION: u32 = 0xFF3E4A5E;

/// Colours of a terminal.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub background: Color,
    pub foreground: Color,
    pub cursor: Color,
    pub selection: Color,
    pub palette: [Color; 16],
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            background: Color::from_argb(DEFAULT_BACKGROUND),
            foreground: Color::from_argb(DEFAULT_FOREGROUND),
            cursor: Color::from_argb(DEFAULT_CURSOR),
            selection: Color::from_argb(DEFAULT_SELECTION),
            palette: DEFAULT_PALETTE.map(|rgb| Color::from_argb(0xFF00_0000 | rgb)),
        }
    }
}

/// `#rrggbb` (or `#rgb`, `#aarrggbb`) entries separated by commas; entries
/// that do not parse keep the default.
#[must_use]
pub fn parse_palette(text: &str) -> [Color; 16] {
    let mut palette = Theme::default().palette;
    for (index, entry) in text.split(',').take(16).enumerate() {
        if let Some(color) = parse_hex(entry.trim()) {
            palette[index] = color;
        }
    }
    palette
}

fn parse_hex(text: &str) -> Option<Color> {
    let hex = text.strip_prefix('#')?;
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let value = u32::from_str_radix(hex, 16).ok()?;
    match hex.len() {
        3 => {
            let (r, g, b) = ((value >> 8) & 0xF, (value >> 4) & 0xF, value & 0xF);
            Some(Color::from_argb(
                0xFF00_0000 | ((r * 0x11) << 16) | ((g * 0x11) << 8) | (b * 0x11),
            ))
        }
        6 => Some(Color::from_argb(0xFF00_0000 | value)),
        8 => Some(Color::from_argb(value)),
        _ => None,
    }
}

fn rgb(color: Rgb) -> Color {
    Color::rgba(color.r, color.g, color.b, 255)
}

fn to_rgb(color: Color) -> Rgb {
    let argb = color.to_argb();
    Rgb {
        r: (argb >> 16) as u8,
        g: (argb >> 8) as u8,
        b: argb as u8,
    }
}

fn dim(color: Color) -> Color {
    let argb = color.to_argb();
    let scale = |channel: u32| ((channel & 0xFF) * 2 / 3) as u8;
    Color::rgba(
        scale(argb >> 16),
        scale(argb >> 8),
        scale(argb),
        (argb >> 24) as u8,
    )
}

/// The xterm 256-colour table beyond the 16 named colours.
fn indexed(index: u8) -> Color {
    match index {
        16..=231 => {
            let index = index - 16;
            let level = |value: u8| if value == 0 { 0 } else { 55 + value * 40 };
            Color::rgba(
                level(index / 36),
                level((index / 6) % 6),
                level(index % 6),
                255,
            )
        }
        232..=255 => {
            let gray = 8 + (index - 232) * 10;
            Color::rgba(gray, gray, gray, 255)
        }
        _ => Color::rgba(0, 0, 0, 255),
    }
}

/// Resolves terminal colours against the theme and the colours the remote
/// end set (OSC 4, 10, 11).
struct Palette<'a> {
    theme: &'a Theme,
    colors: &'a alacritty_terminal::term::color::Colors,
}

impl Palette<'_> {
    fn index(&self, index: usize) -> Color {
        if let Some(color) = self.colors[index] {
            return rgb(color);
        }
        match index {
            0..=15 => self.theme.palette[index],
            16..=255 => indexed(index as u8),
            256 => self.theme.foreground,
            257 => self.theme.background,
            258 => self.theme.cursor,
            _ => self.theme.foreground,
        }
    }

    fn resolve(&self, color: AnsiColor, bold: bool) -> Color {
        match color {
            AnsiColor::Spec(color) => rgb(color),
            AnsiColor::Indexed(index) if bold && index < 8 => self.index(usize::from(index) + 8),
            AnsiColor::Indexed(index) => self.index(usize::from(index)),
            AnsiColor::Named(named) => {
                let value = named as usize;
                match named {
                    _ if value < 8 && bold => self.index(value + 8),
                    _ if value < 16 => self.index(value),
                    NamedColor::Foreground | NamedColor::BrightForeground => self.index(256),
                    NamedColor::Background => self.index(257),
                    NamedColor::Cursor => self.index(258),
                    NamedColor::DimForeground => dim(self.index(256)),
                    // DimBlack (259) … DimWhite (266).
                    _ if (259..=266).contains(&value) => dim(self.index(value - 259)),
                    _ => self.index(256),
                }
            }
        }
    }

    /// Foreground and background of a cell, attributes applied.
    fn cell(&self, cell: &Cell) -> (Color, Color) {
        let bold = cell.flags.contains(Flags::BOLD);
        let mut foreground = self.resolve(cell.fg, bold);
        let mut background = self.resolve(cell.bg, false);
        if cell.flags.contains(Flags::DIM) {
            foreground = dim(foreground);
        }
        if cell.flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut foreground, &mut background);
        }
        if cell.flags.contains(Flags::HIDDEN) {
            foreground = background;
        }
        (foreground, background)
    }
}

/// Bytes a key sends to the remote end, or `None` when it sends nothing
/// (modifiers alone, unknown keys, Command/Super shortcuts).
#[must_use]
pub fn encode_key(key: &str, modifiers: Modifiers, application_cursor: bool) -> Option<Vec<u8>> {
    if modifiers.meta {
        return None;
    }
    // xterm modifier parameter: 1 + shift + 2 alt + 4 control.
    let parameter = 1
        + u8::from(modifiers.shift)
        + 2 * u8::from(modifiers.alt)
        + 4 * u8::from(modifiers.control);
    let modified = parameter > 1;
    let csi_letter = |letter: char| -> Vec<u8> {
        if modified {
            format!("\x1b[1;{parameter}{letter}").into_bytes()
        } else if application_cursor {
            format!("\x1bO{letter}").into_bytes()
        } else {
            format!("\x1b[{letter}").into_bytes()
        }
    };
    let tilde = |code: u8| -> Vec<u8> {
        if modified {
            format!("\x1b[{code};{parameter}~").into_bytes()
        } else {
            format!("\x1b[{code}~").into_bytes()
        }
    };
    let alt = |bytes: Vec<u8>| -> Vec<u8> {
        if modifiers.alt {
            let mut prefixed = vec![0x1b];
            prefixed.extend(bytes);
            prefixed
        } else {
            bytes
        }
    };
    let bytes = match key {
        "Enter" => alt(vec![b'\r']),
        "Backspace" => alt(vec![if modifiers.control { 0x08 } else { 0x7f }]),
        "Tab" if modifiers.shift => b"\x1b[Z".to_vec(),
        "Tab" => alt(vec![b'\t']),
        "Escape" => alt(vec![0x1b]),
        "ArrowUp" => csi_letter('A'),
        "ArrowDown" => csi_letter('B'),
        "ArrowRight" => csi_letter('C'),
        "ArrowLeft" => csi_letter('D'),
        "Home" => csi_letter('H'),
        "End" => csi_letter('F'),
        "Insert" => tilde(2),
        "Delete" => tilde(3),
        "PageUp" => tilde(5),
        "PageDown" => tilde(6),
        "F1" | "F2" | "F3" | "F4" => {
            let letter = (b'P' + key.as_bytes()[1] - b'1') as char;
            if modified {
                format!("\x1b[1;{parameter}{letter}").into_bytes()
            } else {
                format!("\x1bO{letter}").into_bytes()
            }
        }
        "F5" => tilde(15),
        "F6" => tilde(17),
        "F7" => tilde(18),
        "F8" => tilde(19),
        "F9" => tilde(20),
        "F10" => tilde(21),
        "F11" => tilde(23),
        "F12" => tilde(24),
        _ => {
            let mut characters = key.chars();
            let character = characters.next()?;
            if characters.next().is_some() {
                // A key name ("Shift", "Unidentified", "Dead_acute"…).
                return None;
            }
            if modifiers.control {
                let control = match character {
                    'a'..='z' => character as u8 - b'a' + 1,
                    'A'..='Z' => character as u8 - b'A' + 1,
                    ' ' | '@' | '2' => 0,
                    '[' | '3' => 0x1b,
                    '\\' | '4' => 0x1c,
                    ']' | '5' => 0x1d,
                    '^' | '6' => 0x1e,
                    '_' | '-' | '7' => 0x1f,
                    '?' | '8' => 0x7f,
                    _ => return None,
                };
                alt(vec![control])
            } else {
                let mut buffer = [0; 4];
                alt(character.encode_utf8(&mut buffer).as_bytes().to_vec())
            }
        }
    };
    Some(bytes)
}

/// What the runtime does after a key reached a terminal.
#[derive(Debug, PartialEq, Eq)]
pub enum KeyOutcome {
    /// Not a terminal's key.
    Ignored,
    /// Sent to the remote end (or swallowed).
    Handled,
    /// Put this text on the clipboard.
    Copy(String),
    /// Read the clipboard and paste it into `channel`.
    Paste { channel: String, bracketed: bool },
}

/// The bytes a paste sends: bracketed when the remote end asked for it
/// (the end marker is removed from the text so it cannot end the paste
/// early), with line breaks as carriage returns, at most [`MAX_PASTE`].
#[must_use]
pub fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    let mut cut = text.len().min(MAX_PASTE);
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    let text = text[..cut].replace("\r\n", "\r").replace('\n', "\r");
    if bracketed {
        let inner = text.replace("\x1b[201~", "");
        let mut bytes = b"\x1b[200~".to_vec();
        bytes.extend_from_slice(inner.as_bytes());
        bytes.extend_from_slice(b"\x1b[201~");
        bytes
    } else {
        text.into_bytes()
    }
}

/// Writes a paste to the hub in pieces it accepts.
pub fn paste(hub: &TerminalHub, channel: &str, text: &str, bracketed: bool) {
    for chunk in paste_bytes(text, bracketed).chunks(32 * 1024) {
        if !hub.write(channel, chunk) {
            return;
        }
    }
}

/// Columns and rows that fit `size` with cells of `cell`.
#[must_use]
pub fn grid_size(size: Size, cell: Size) -> (usize, usize) {
    if cell.width <= 0.0 || cell.height <= 0.0 {
        return (1, 1);
    }
    let cols = ((size.width / cell.width) + 0.001).floor().max(1.0) as usize;
    let rows = ((size.height / cell.height) + 0.001).floor().max(1.0) as usize;
    (cols.min(MAX_COLS), rows.min(MAX_ROWS))
}

#[derive(Default)]
struct Notes {
    /// `Some(None)` resets the title.
    title: Option<Option<String>>,
    bell: bool,
    exited: Option<Option<i64>>,
}

/// Receives what the emulator asks for, on the producer's thread.
struct Listener {
    channel: String,
    hub: &'static TerminalHub,
    notes: Arc<Mutex<Notes>>,
    window: Arc<Mutex<(WindowSize, [Rgb; 3])>>,
}

impl EventListener for Listener {
    fn send_event(&self, event: TermEvent) {
        match event {
            TermEvent::PtyWrite(text) => {
                self.hub.write(&self.channel, text.as_bytes());
            }
            TermEvent::Title(title) => {
                if let Ok(mut notes) = self.notes.lock() {
                    notes.title = Some(Some(title.chars().take(256).collect()));
                }
            }
            TermEvent::ResetTitle => {
                if let Ok(mut notes) = self.notes.lock() {
                    notes.title = Some(None);
                }
            }
            TermEvent::Bell => {
                if let Ok(mut notes) = self.notes.lock() {
                    notes.bell = true;
                }
            }
            TermEvent::TextAreaSizeRequest(format) => {
                if let Ok(window) = self.window.lock() {
                    let reply = format(window.0);
                    self.hub.write(&self.channel, reply.as_bytes());
                }
            }
            TermEvent::ColorRequest(index, format) => {
                let color = self.window.lock().ok().and_then(|window| match index {
                    256 => Some(window.1[0]),
                    257 => Some(window.1[1]),
                    258 => Some(window.1[2]),
                    _ => None,
                });
                if let Some(color) = color {
                    self.hub.write(&self.channel, format(color).as_bytes());
                }
            }
            // OSC 52 clipboard access is not granted to the remote end.
            _ => {}
        }
    }
}

struct Emulator {
    term: Term<Listener>,
    parser: Processor<StdSyncHandler>,
}

/// State shared with the subscriber callback.
struct Shared {
    emulator: Mutex<Emulator>,
    notes: Arc<Mutex<Notes>>,
    window: Arc<Mutex<(WindowSize, [Rgb; 3])>>,
    changed: AtomicBool,
    /// A notification is in the inbox and not handled yet.
    posted: AtomicBool,
}

/// The selection being dragged.
#[derive(Clone, Copy)]
struct Drag {
    id: NodeId,
    origin: GridPoint,
    moved: bool,
}

struct Instance {
    channel: String,
    shared: Option<Arc<Shared>>,
    subscription: Option<TerminalSubscription>,
    font: FontSpec,
    line_height: f32,
    cell: Size,
    baseline: f32,
    /// Where the grid starts: the padding.
    inset: Point,
    theme: Theme,
    scrollback: usize,
    /// The grid size of the emulator, and the last one the remote end heard.
    grid: (usize, usize),
    reported: (usize, usize),
    measure: bool,
    paint: bool,
    last_paint: u64,
    focused: bool,
    /// Wheel motion that did not make a whole line yet.
    wheel: f32,
    exited: bool,
}

impl Instance {
    fn new() -> Self {
        Self {
            channel: String::new(),
            shared: None,
            subscription: None,
            font: FontSpec::default(),
            line_height: 0.0,
            cell: Size::new(0.0, 0.0),
            baseline: 0.0,
            inset: Point::new(0.0, 0.0),
            theme: Theme::default(),
            scrollback: DEFAULT_SCROLLBACK,
            grid: (0, 0),
            reported: (0, 0),
            measure: true,
            paint: true,
            last_paint: 0,
            focused: false,
            wheel: 0.0,
            exited: false,
        }
    }
}

/// Posts "terminal `id` has news" to the UI thread.
pub type Notify = Arc<dyn Fn(NodeId) + Send + Sync>;

pub struct Terminals {
    hub: &'static TerminalHub,
    notify: Notify,
    nodes: IdMap<Instance>,
    drag: Option<Drag>,
    dirty: bool,
}

impl Terminals {
    #[must_use]
    pub fn new(hub: &'static TerminalHub, notify: Notify) -> Self {
        Self {
            hub,
            notify,
            nodes: IdMap::default(),
            drag: None,
            dirty: false,
        }
    }

    #[must_use]
    pub fn contains(&self, id: NodeId) -> bool {
        self.nodes.contains_key(&id)
    }

    /// The grid size of a terminal, for tests and the inspector.
    #[must_use]
    pub fn grid(&self, id: NodeId) -> Option<(usize, usize)> {
        self.nodes.get(&id).map(|instance| instance.grid)
    }

    pub fn on_change(&mut self, _ctx: &mut Ctx<'_>, change: Change) {
        match change {
            Change::Created(id, NodeKind::Terminal) => {
                self.nodes.insert(id, Instance::new());
                self.dirty = true;
            }
            Change::Removed(id, NodeKind::Terminal) => {
                self.nodes.remove(&id);
                if self.drag.is_some_and(|drag| drag.id == id) {
                    self.drag = None;
                }
            }
            Change::Updated(id, key) => {
                if let Some(instance) = self.nodes.get_mut(&id)
                    && !matches!(
                        key,
                        PropKey::OnTerminalResize
                            | PropKey::OnTerminalTitle
                            | PropKey::OnTerminalExit
                            | PropKey::OnTerminalBell
                    )
                {
                    instance.measure = true;
                    self.dirty = true;
                }
            }
            Change::Frame(id) => {
                if let Some(instance) = self.nodes.get_mut(&id) {
                    instance.measure = true;
                    self.dirty = true;
                }
            }
            _ => {}
        }
    }

    /// The emulator of `id` has news (posted by [`Notify`]).
    pub fn on_output(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        let Some(instance) = self.nodes.get_mut(&id) else {
            return;
        };
        let Some(shared) = instance.shared.clone() else {
            return;
        };
        shared.posted.store(false, Ordering::Release);
        let notes = shared
            .notes
            .lock()
            .map(|mut notes| std::mem::take(&mut *notes))
            .unwrap_or_default();
        if let Some(title) = notes.title {
            let payload = WireMap::new()
                .with("title", title.unwrap_or_default())
                .encode();
            ctx.emit_if(
                id,
                PropKey::OnTerminalTitle,
                event_kind::TERMINAL_TITLE,
                payload,
            );
        }
        if notes.bell {
            ctx.emit_if(
                id,
                PropKey::OnTerminalBell,
                event_kind::TERMINAL_BELL,
                WireMap::new().encode(),
            );
        }
        if let Some(status) = notes.exited
            && !instance.exited
        {
            instance.exited = true;
            instance.subscription = None;
            let payload = WireMap::new()
                .with("exitCode", status.unwrap_or(-1))
                .encode();
            ctx.emit_if(
                id,
                PropKey::OnTerminalExit,
                event_kind::TERMINAL_EXIT,
                payload,
            );
        }
        if shared.changed.swap(false, Ordering::AcqRel) {
            Self::schedule_paint(ctx, id, instance);
            self.dirty = true;
        }
    }

    fn schedule_paint(ctx: &mut Ctx<'_>, id: NodeId, instance: &mut Instance) {
        let due = instance.last_paint + FRAME_MS;
        if ctx.now_ms >= due {
            instance.paint = true;
        } else if !ctx.timers.is_pending(Timer::TerminalFrame(id)) {
            ctx.timers.schedule(Timer::TerminalFrame(id), due);
        }
    }

    pub fn on_timer(&mut self, ctx: &mut Ctx<'_>, timer: Timer) {
        match timer {
            Timer::TerminalFrame(id) => {
                if let Some(instance) = self.nodes.get_mut(&id) {
                    instance.paint = true;
                    self.dirty = true;
                }
            }
            Timer::TerminalResize(id) => {
                if let Some(instance) = self.nodes.get_mut(&id) {
                    Self::report_size(ctx, self.hub, id, instance);
                }
            }
            _ => {}
        }
    }

    fn report_size(ctx: &mut Ctx<'_>, hub: &TerminalHub, id: NodeId, instance: &mut Instance) {
        if instance.reported == instance.grid || instance.channel.is_empty() {
            return;
        }
        instance.reported = instance.grid;
        let (cols, rows) = instance.grid;
        hub.resize(
            &instance.channel,
            u16::try_from(cols).unwrap_or(u16::MAX),
            u16::try_from(rows).unwrap_or(u16::MAX),
        );
        let payload = WireMap::new()
            .with("cols", cols)
            .with("rows", rows)
            .encode();
        ctx.emit_if(
            id,
            PropKey::OnTerminalResize,
            event_kind::TERMINAL_RESIZE,
            payload,
        );
    }

    pub fn on_focus(&mut self, _ctx: &mut Ctx<'_>, id: NodeId, focused: bool) {
        if let Some(instance) = self.nodes.get_mut(&id)
            && instance.focused != focused
        {
            instance.focused = focused;
            instance.paint = true;
            if let Some(shared) = &instance.shared
                && let Ok(mut emulator) = shared.emulator.lock()
            {
                emulator.term.is_focused = focused;
            }
            self.dirty = true;
        }
    }

    /// Measures, (re)binds and paints what needs it. Runs at the end of every
    /// settle; does nothing when nothing changed.
    pub fn evaluate(&mut self, ctx: &mut Ctx<'_>, measure: Measure<'_>) {
        if !std::mem::take(&mut self.dirty) {
            return;
        }
        let ids: Vec<NodeId> = self.nodes.keys().copied().collect();
        for id in ids {
            let Some(instance) = self.nodes.get_mut(&id) else {
                continue;
            };
            if instance.measure {
                instance.measure = false;
                self.configure(ctx, id, measure);
            }
            let Some(instance) = self.nodes.get_mut(&id) else {
                continue;
            };
            if instance.paint {
                if !ctx.scene.is_effectively_visible(id) {
                    // Painted when it shows again.
                    self.dirty = true;
                    continue;
                }
                instance.paint = false;
                instance.last_paint = ctx.now_ms;
                let commands = Self::paint(ctx, id, instance);
                ctx.commands.push(HostCommand::SetDrawing { id, commands });
            }
        }
    }

    /// Reads the properties: font, cell size, theme, scrollback, channel,
    /// and the grid size from the frame.
    fn configure(&mut self, ctx: &mut Ctx<'_>, id: NodeId, measure: Measure<'_>) {
        let Some(node) = ctx.scene.node(id) else {
            return;
        };
        let props = node.props();
        let frame = node.frame;
        let padding = props.padding();
        let scale = ctx.scene.environment.scale.max(1.0);
        let family = props
            .text(PropKey::FontFamily)
            .filter(|family| !family.is_empty())
            .unwrap_or(DEFAULT_FONT_FAMILY)
            .to_owned();
        let size = props
            .number(PropKey::FontSize)
            .filter(|size| size.is_finite() && *size >= 4.0)
            .unwrap_or(DEFAULT_FONT_SIZE)
            .min(96.0);
        let line_height = props
            .number(PropKey::LineHeight)
            .filter(|height| height.is_finite() && *height > 0.0)
            .unwrap_or(0.0);
        let channel = props.text_or_empty(PropKey::TerminalChannel).to_owned();
        let scrollback = props
            .integer_or(PropKey::TerminalScrollback, DEFAULT_SCROLLBACK as i64)
            .clamp(0, MAX_SCROLLBACK as i64) as usize;
        let mut theme = Theme::default();
        if let Some(color) = props.color(PropKey::BackgroundColor) {
            theme.background = color;
        }
        if let Some(color) = props.color(PropKey::TextColor) {
            theme.foreground = color;
        }
        if let Some(color) = props.color(PropKey::TerminalCursorColor) {
            theme.cursor = color;
        }
        if let Some(color) = props.color(PropKey::TerminalSelectionColor) {
            theme.selection = color;
        }
        if let Some(palette) = props.text(PropKey::TerminalPalette) {
            theme.palette = parse_palette(palette);
        }
        let font = FontSpec {
            family,
            size,
            weight: 400,
            italic: false,
            features: String::from("\"liga\" 0, \"calt\" 0"),
        };
        let Some(instance) = self.nodes.get_mut(&id) else {
            return;
        };
        if font != instance.font
            || line_height != instance.line_height
            || instance.cell.width <= 0.0
        {
            let (cell, baseline) = measure_cell(&font, line_height, scale, measure);
            instance.font = font;
            instance.line_height = line_height;
            instance.cell = cell;
            instance.baseline = baseline;
        }
        if theme != instance.theme {
            instance.theme = theme;
        }
        instance.inset = Point::new(padding.left, padding.top);
        let inner = Size::new(
            (frame.width - padding.left - padding.right).max(0.0),
            (frame.height - padding.top - padding.bottom).max(0.0),
        );
        let grid = grid_size(inner, instance.cell);
        if channel != instance.channel || instance.shared.is_none() {
            instance.subscription = None;
            instance.shared = None;
            instance.exited = false;
            instance.reported = (0, 0);
            instance.channel = channel;
            instance.scrollback = scrollback;
            instance.grid = grid;
            if !instance.channel.is_empty() {
                Self::bind(self.hub, &self.notify, id, instance);
            }
            Self::report_size(ctx, self.hub, id, instance);
        } else {
            if scrollback != instance.scrollback {
                instance.scrollback = scrollback;
                if let Some(shared) = &instance.shared
                    && let Ok(mut emulator) = shared.emulator.lock()
                {
                    emulator.term.set_options(config(scrollback));
                }
            }
            if grid != instance.grid {
                instance.grid = grid;
                if let Some(shared) = &instance.shared
                    && let Ok(mut emulator) = shared.emulator.lock()
                {
                    emulator.term.resize(TermSize::new(grid.0, grid.1));
                }
                ctx.timers
                    .schedule(Timer::TerminalResize(id), ctx.now_ms + RESIZE_DEBOUNCE_MS);
            }
        }
        if let Some(shared) = &instance.shared
            && let Ok(mut window) = shared.window.lock()
        {
            *window = window_info(instance);
        }
        instance.paint = true;
    }

    /// Creates the emulator and subscribes it to the hub.
    fn bind(hub: &'static TerminalHub, notify: &Notify, id: NodeId, instance: &mut Instance) {
        let notes = Arc::new(Mutex::new(Notes::default()));
        let window = Arc::new(Mutex::new(window_info(instance)));
        let listener = Listener {
            channel: instance.channel.clone(),
            hub,
            notes: Arc::clone(&notes),
            window: Arc::clone(&window),
        };
        let (cols, rows) = instance.grid;
        let mut term = Term::new(
            config(instance.scrollback),
            &TermSize::new(cols, rows),
            listener,
        );
        term.is_focused = instance.focused;
        let shared = Arc::new(Shared {
            emulator: Mutex::new(Emulator {
                term,
                parser: Processor::new(),
            }),
            notes,
            window,
            changed: AtomicBool::new(true),
            posted: AtomicBool::new(false),
        });
        let callback_shared = Arc::clone(&shared);
        let callback_notify = Arc::clone(notify);
        let post = move |shared: &Shared| {
            if !shared.posted.swap(true, Ordering::AcqRel) {
                callback_notify(id);
            }
        };
        // The backlog is fed under the emulator lock, so output that arrives
        // meanwhile waits for it and keeps its order.
        let Ok(mut emulator) = shared.emulator.lock() else {
            return;
        };
        let subscription = hub.subscribe(
            &instance.channel,
            Arc::new(move |event| match event {
                TerminalEvent::Output(bytes) => {
                    if let Ok(mut emulator) = callback_shared.emulator.lock() {
                        let Emulator { term, parser } = &mut *emulator;
                        parser.advance(term, bytes);
                    }
                    callback_shared.changed.store(true, Ordering::Release);
                    post(&callback_shared);
                }
                TerminalEvent::Exited(status) => {
                    if let Ok(mut notes) = callback_shared.notes.lock() {
                        notes.exited = Some(status);
                    }
                    post(&callback_shared);
                }
            }),
        );
        match subscription {
            Ok(mut subscription) => {
                let backlog = subscription.take_backlog();
                let Emulator { term, parser } = &mut *emulator;
                parser.advance(term, &backlog);
                instance.subscription = Some(subscription);
            }
            Err(_) => {
                // Unknown channel (closed, or not open yet): an empty screen.
                instance.exited = true;
            }
        }
        drop(emulator);
        instance.shared = Some(shared);
    }

    fn paint(ctx: &Ctx<'_>, id: NodeId, instance: &Instance) -> Vec<DrawCommand> {
        let Some(node) = ctx.scene.node(id) else {
            return Vec::new();
        };
        let frame = Rect::from_size(node.frame.size());
        let mut commands = vec![DrawCommand::Rect {
            rect: frame,
            radius: 0.0,
            color: instance.theme.background,
        }];
        let Some(shared) = &instance.shared else {
            return commands;
        };
        let Ok(emulator) = shared.emulator.lock() else {
            return commands;
        };
        let look = Look {
            origin: instance.inset,
            theme: &instance.theme,
            font: &instance.font,
            cell: instance.cell,
            baseline: instance.baseline,
            focused: instance.focused,
        };
        paint_grid(&emulator.term, &look, &mut commands);
        commands
    }

    fn hit(&self, ctx: &Ctx<'_>, position: Point) -> Option<NodeId> {
        let mut current = ctx.scene.hit_test(position)?;
        for _ in 0..512 {
            if self.nodes.contains_key(&current) {
                return Some(current);
            }
            let parent = ctx.scene.node(current)?.parent;
            if parent == 0 || parent == current {
                return None;
            }
            current = parent;
        }
        None
    }

    fn grid_point(
        ctx: &Ctx<'_>,
        id: NodeId,
        instance: &Instance,
        term: &Term<Listener>,
        position: Point,
    ) -> (GridPoint, Side) {
        let origin = ctx.scene.window_origin(id);
        let x = (position.x - origin.x - instance.inset.x).max(0.0);
        let y = (position.y - origin.y - instance.inset.y).max(0.0);
        let cols = term.columns().max(1);
        let rows = term.screen_lines().max(1);
        let column = ((x / instance.cell.width.max(1.0)) as usize).min(cols - 1);
        let row = ((y / instance.cell.height.max(1.0)) as usize).min(rows - 1);
        let within = x / instance.cell.width.max(1.0) - column as f32;
        let side = if within < 0.5 {
            Side::Left
        } else {
            Side::Right
        };
        let line = Line(row as i32 - term.grid().display_offset() as i32);
        (GridPoint::new(line, Column(column)), side)
    }

    /// Clicks focus and select; drags extend the selection.
    pub fn on_pointer(&mut self, ctx: &mut Ctx<'_>, pointer: PointerEvent) -> bool {
        if let Some(drag) = self.drag {
            let Some(instance) = self.nodes.get_mut(&drag.id) else {
                self.drag = None;
                return false;
            };
            let Some(shared) = instance.shared.clone() else {
                self.drag = None;
                return true;
            };
            match pointer.phase {
                PointerPhase::Move => {
                    if let Ok(mut emulator) = shared.emulator.lock() {
                        let (point, side) = Self::grid_point(
                            ctx,
                            drag.id,
                            instance,
                            &emulator.term,
                            pointer.position,
                        );
                        if let Some(selection) = emulator.term.selection.as_mut() {
                            selection.update(point, side);
                        }
                        if point != drag.origin {
                            self.drag = Some(Drag {
                                moved: true,
                                ..drag
                            });
                        }
                    }
                    instance.paint = true;
                    self.dirty = true;
                }
                PointerPhase::Up | PointerPhase::Cancel => {
                    self.drag = None;
                    if let Ok(mut emulator) = shared.emulator.lock()
                        && !drag.moved
                        && emulator
                            .term
                            .selection
                            .as_ref()
                            .is_some_and(|selection| selection.ty == SelectionType::Simple)
                    {
                        emulator.term.selection = None;
                    }
                    instance.paint = true;
                    self.dirty = true;
                }
                _ => {}
            }
            return true;
        }
        if pointer.phase != PointerPhase::Down {
            return false;
        }
        let Some(id) = self.hit(ctx, pointer.position) else {
            return false;
        };
        if !instance_enabled(ctx, id) {
            return false;
        }
        ctx.commands.push(HostCommand::Focus(Some(id)));
        if pointer.button != PointerButton::Primary {
            return true;
        }
        let Some(instance) = self.nodes.get_mut(&id) else {
            return false;
        };
        let Some(shared) = instance.shared.clone() else {
            return true;
        };
        if let Ok(mut emulator) = shared.emulator.lock() {
            let (point, side) =
                Self::grid_point(ctx, id, instance, &emulator.term, pointer.position);
            let kind = match pointer.click_count {
                0 | 1 => SelectionType::Simple,
                2 => SelectionType::Semantic,
                _ => SelectionType::Lines,
            };
            emulator.term.selection = Some(Selection::new(kind, point, side));
            self.drag = Some(Drag {
                id,
                origin: point,
                moved: kind != SelectionType::Simple,
            });
        }
        instance.paint = true;
        self.dirty = true;
        true
    }

    /// Scrolls the scrollback, or sends arrows to programs in the alternate
    /// screen (less, vim, htop).
    pub fn on_wheel(&mut self, ctx: &mut Ctx<'_>, position: Point, delta: Point) -> bool {
        let Some(id) = self.hit(ctx, position) else {
            return false;
        };
        let hub = self.hub;
        let Some(instance) = self.nodes.get_mut(&id) else {
            return false;
        };
        let Some(shared) = instance.shared.clone() else {
            return true;
        };
        let cell = instance.cell.height.max(1.0);
        instance.wheel += delta.y / cell;
        let lines = instance.wheel.trunc() as i32;
        if lines == 0 {
            return true;
        }
        instance.wheel -= lines as f32;
        let Ok(mut emulator) = shared.emulator.lock() else {
            return true;
        };
        let mode = *emulator.term.mode();
        if mode.contains(TermMode::ALT_SCREEN) && mode.contains(TermMode::ALTERNATE_SCROLL) {
            drop(emulator);
            let key = if lines > 0 { "ArrowDown" } else { "ArrowUp" };
            let application = mode.contains(TermMode::APP_CURSOR);
            if let Some(bytes) = encode_key(key, Modifiers::default(), application) {
                let count = lines.unsigned_abs().min(64) as usize;
                hub.write(&instance.channel, &bytes.repeat(count));
            }
            return true;
        }
        // Positive y scrolls the content up: towards the bottom.
        emulator.term.scroll_display(Scroll::Delta(-lines));
        drop(emulator);
        instance.paint = true;
        self.dirty = true;
        let _ = ctx;
        true
    }

    /// A key while `focused` has keyboard focus.
    pub fn on_key(
        &mut self,
        _ctx: &mut Ctx<'_>,
        focused: Option<NodeId>,
        key: &KeyEvent,
    ) -> KeyOutcome {
        let Some(id) = focused.filter(|id| self.nodes.contains_key(id)) else {
            return KeyOutcome::Ignored;
        };
        if !key.pressed {
            return KeyOutcome::Handled;
        }
        let hub = self.hub;
        let Some(instance) = self.nodes.get_mut(&id) else {
            return KeyOutcome::Ignored;
        };
        let Some(shared) = instance.shared.clone() else {
            return KeyOutcome::Handled;
        };
        let Ok(mut emulator) = shared.emulator.lock() else {
            return KeyOutcome::Handled;
        };
        let modifiers = key.modifiers;
        let shortcut = if cfg!(target_os = "macos") {
            modifiers.meta && !modifiers.control && !modifiers.alt
        } else {
            modifiers.control && modifiers.shift && !modifiers.alt && !modifiers.meta
        };
        if shortcut && key.key.eq_ignore_ascii_case("c") {
            return match emulator.term.selection_to_string() {
                Some(text) if !text.is_empty() => KeyOutcome::Copy(text),
                _ => KeyOutcome::Handled,
            };
        }
        if shortcut && key.key.eq_ignore_ascii_case("v") {
            return KeyOutcome::Paste {
                channel: instance.channel.clone(),
                bracketed: emulator.term.mode().contains(TermMode::BRACKETED_PASTE),
            };
        }
        if modifiers.shift
            && !modifiers.control
            && matches!(key.key.as_str(), "PageUp" | "PageDown")
        {
            let scroll = if key.key == "PageUp" {
                Scroll::PageUp
            } else {
                Scroll::PageDown
            };
            emulator.term.scroll_display(scroll);
            drop(emulator);
            instance.paint = true;
            self.dirty = true;
            return KeyOutcome::Handled;
        }
        let application = emulator.term.mode().contains(TermMode::APP_CURSOR);
        let Some(bytes) = encode_key(&key.key, modifiers, application) else {
            return if modifiers.meta {
                KeyOutcome::Ignored
            } else {
                KeyOutcome::Handled
            };
        };
        // Typing returns to the bottom and drops the selection.
        let mut repaint = false;
        if emulator.term.grid().display_offset() != 0 {
            emulator.term.scroll_display(Scroll::Bottom);
            repaint = true;
        }
        if emulator.term.selection.take().is_some() {
            repaint = true;
        }
        drop(emulator);
        hub.write(&instance.channel, &bytes);
        if repaint {
            instance.paint = true;
            self.dirty = true;
        }
        KeyOutcome::Handled
    }
}

fn instance_enabled(ctx: &Ctx<'_>, id: NodeId) -> bool {
    ctx.scene
        .node(id)
        .is_some_and(|node| node.props().flag_or(PropKey::Enabled, true))
}

fn config(scrollback: usize) -> Config {
    Config {
        scrolling_history: scrollback.min(MAX_SCROLLBACK),
        ..Config::default()
    }
}

fn window_info(instance: &Instance) -> (WindowSize, [Rgb; 3]) {
    let clamp = |value: f32| value.clamp(0.0, f32::from(u16::MAX)) as u16;
    (
        WindowSize {
            num_lines: u16::try_from(instance.grid.1).unwrap_or(u16::MAX),
            num_cols: u16::try_from(instance.grid.0).unwrap_or(u16::MAX),
            cell_width: clamp(instance.cell.width),
            cell_height: clamp(instance.cell.height),
        },
        [
            to_rgb(instance.theme.foreground),
            to_rgb(instance.theme.background),
            to_rgb(instance.theme.cursor),
        ],
    )
}

/// The cell of the monospace font: the advance of `M` and the line height
/// (the font's own, or `line_height` as a multiple when ≤ 4, in logical
/// units above), snapped to device pixels; and the baseline in the cell.
fn measure_cell(
    font: &FontSpec,
    line_height: f32,
    scale: f32,
    measure: Measure<'_>,
) -> (Size, f32) {
    const SAMPLE: usize = 64;
    let block = TextBlock {
        runs: vec![TextRun {
            text: "M".repeat(SAMPLE),
            font: font.clone(),
            color: None,
            background: None,
            underline: false,
            strikethrough: false,
            letter_spacing: 0.0,
            press: None,
        }],
        ..TextBlock::default()
    };
    let metrics = measure(&block, f32::MAX);
    let width = if metrics.width > 0.0 {
        metrics.width / SAMPLE as f32
    } else {
        font.size * 0.6
    };
    let natural = if metrics.height > 0.0 {
        metrics.height / metrics.line_count.max(1) as f32
    } else {
        font.size * 1.25
    };
    let height = if line_height <= 0.0 {
        natural
    } else if line_height <= 4.0 {
        natural * line_height
    } else {
        line_height
    };
    let height = ((height * scale).round() / scale).max(1.0);
    let first = if metrics.first_baseline > 0.0 {
        metrics.first_baseline
    } else {
        font.size
    };
    let baseline = first + (height - natural) / 2.0;
    (Size::new(width, height), baseline)
}

/// A run of text being collected on a row.
struct Run {
    text: String,
    column: usize,
    color: Color,
    bold: bool,
    italic: bool,
}

fn flush_run(
    run: &mut Option<Run>,
    x: f32,
    row_y: f32,
    cell: Size,
    baseline: f32,
    font: &FontSpec,
    commands: &mut Vec<DrawCommand>,
) {
    let Some(run) = run.take() else {
        return;
    };
    let text = run.text.trim_end();
    if text.is_empty() {
        return;
    }
    commands.push(DrawCommand::Text {
        text: text.to_owned(),
        baseline: Point::new(x + run.column as f32 * cell.width, row_y + baseline),
        font: FontSpec {
            weight: if run.bold { 700 } else { 400 },
            italic: run.italic,
            ..font.clone()
        },
        color: run.color,
        align: TextAlign::Start,
    });
}

/// How a grid is painted.
#[derive(Clone, Copy)]
struct Look<'a> {
    /// Where the grid starts in the view (the padding).
    origin: Point,
    theme: &'a Theme,
    font: &'a FontSpec,
    cell: Size,
    /// The baseline inside a cell.
    baseline: f32,
    focused: bool,
}

/// Paints the visible part of the grid: background runs, text runs,
/// decorations, the selection and the cursor.
fn paint_grid<T>(term: &Term<T>, look: &Look<'_>, commands: &mut Vec<DrawCommand>) {
    let Look {
        origin,
        theme,
        font,
        cell,
        baseline,
        focused,
    } = *look;
    let grid = term.grid();
    let offset = grid.display_offset() as i32;
    let rows = term.screen_lines();
    let cols = term.columns();
    let palette = Palette {
        theme,
        colors: term.colors(),
    };
    let selection = term
        .selection
        .as_ref()
        .and_then(|selection| selection.to_range(term));
    let cursor = grid.cursor.point;
    let show_cursor = term.mode().contains(TermMode::SHOW_CURSOR)
        && term.cursor_style().shape != CursorShape::Hidden;
    for row in 0..rows {
        let line = Line(row as i32 - offset);
        let y = origin.y + row as f32 * cell.height;
        // Backgrounds, merged into runs of equal colour.
        let mut background: Option<(usize, Color)> = None;
        let flush_background =
            |from: usize, to: usize, color: Color, commands: &mut Vec<DrawCommand>| {
                if color != theme.background {
                    commands.push(DrawCommand::Rect {
                        rect: Rect::new(
                            origin.x + from as f32 * cell.width,
                            y,
                            (to - from) as f32 * cell.width,
                            cell.height,
                        ),
                        radius: 0.0,
                        color,
                    });
                }
            };
        let mut colors = Vec::with_capacity(cols);
        for column in 0..cols {
            let cell_ref = &grid[line][Column(column)];
            let (foreground, mut back) = palette.cell(cell_ref);
            if selection
                .as_ref()
                .is_some_and(|range| range.contains(GridPoint::new(line, Column(column))))
            {
                back = theme.selection;
            }
            colors.push(foreground);
            match background {
                Some((start, color)) if color == back => {
                    let _ = start;
                }
                Some((start, color)) => {
                    flush_background(start, column, color, commands);
                    background = Some((column, back));
                }
                None => background = Some((column, back)),
            }
        }
        if let Some((start, color)) = background {
            flush_background(start, cols, color, commands);
        }
        // Text runs.
        let mut run: Option<Run> = None;
        let mut column = 0;
        while column < cols {
            let cell_ref = &grid[line][Column(column)];
            let flags = cell_ref.flags;
            if flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                flush_run(&mut run, origin.x, y, cell, baseline, font, commands);
                column += 1;
                continue;
            }
            let color = colors[column];
            let bold = flags.contains(Flags::BOLD);
            let italic = flags.contains(Flags::ITALIC);
            let character = cell_ref.c;
            let simple = character.is_ascii() && cell_ref.zerowidth().is_none();
            if simple {
                let same = run.as_ref().is_some_and(|run| {
                    run.color == color && run.bold == bold && run.italic == italic
                });
                if !same {
                    flush_run(&mut run, origin.x, y, cell, baseline, font, commands);
                    if character != ' ' {
                        run = Some(Run {
                            text: String::new(),
                            column,
                            color,
                            bold,
                            italic,
                        });
                    }
                }
                if let Some(run) = run.as_mut() {
                    run.text.push(if character.is_ascii_control() {
                        ' '
                    } else {
                        character
                    });
                }
            } else {
                // Its own command at its own cell: a fallback font cannot
                // move the rest of the row.
                flush_run(&mut run, origin.x, y, cell, baseline, font, commands);
                let mut text = String::from(character);
                if let Some(extra) = cell_ref.zerowidth() {
                    text.extend(extra.iter());
                }
                let mut single = Some(Run {
                    text,
                    column,
                    color,
                    bold,
                    italic,
                });
                flush_run(&mut single, origin.x, y, cell, baseline, font, commands);
            }
            if flags.intersects(Flags::ALL_UNDERLINES) || flags.contains(Flags::STRIKEOUT) {
                let width = if flags.contains(Flags::WIDE_CHAR) {
                    2.0
                } else {
                    1.0
                } * cell.width;
                let x = origin.x + column as f32 * cell.width;
                let thickness = (font.size / 14.0).max(1.0);
                if flags.intersects(Flags::ALL_UNDERLINES) {
                    let underline_y = y + baseline + thickness * 1.5;
                    commands.push(DrawCommand::Rect {
                        rect: Rect::new(x, underline_y, width, thickness),
                        radius: 0.0,
                        color,
                    });
                }
                if flags.contains(Flags::STRIKEOUT) {
                    commands.push(DrawCommand::Rect {
                        rect: Rect::new(x, y + cell.height / 2.0, width, thickness),
                        radius: 0.0,
                        color,
                    });
                }
            }
            column += 1;
        }
        flush_run(&mut run, origin.x, y, cell, baseline, font, commands);
    }
    // The cursor, when its line is on screen.
    let cursor_row = cursor.line.0 + offset;
    if show_cursor && (0..rows as i32).contains(&cursor_row) {
        let column = cursor.column.0.min(cols.saturating_sub(1));
        let x = origin.x + column as f32 * cell.width;
        let y = origin.y + cursor_row as f32 * cell.height;
        let cell_ref = &grid[cursor.line][Column(column)];
        let wide = cell_ref.flags.contains(Flags::WIDE_CHAR);
        let width = cell.width * if wide { 2.0 } else { 1.0 };
        let thin = (cell.width / 8.0).max(1.0);
        let shape = term.cursor_style().shape;
        if !focused || shape == CursorShape::HollowBlock {
            for rect in [
                Rect::new(x, y, width, thin),
                Rect::new(x, y + cell.height - thin, width, thin),
                Rect::new(x, y, thin, cell.height),
                Rect::new(x + width - thin, y, thin, cell.height),
            ] {
                commands.push(DrawCommand::Rect {
                    rect,
                    radius: 0.0,
                    color: theme.cursor,
                });
            }
        } else {
            match shape {
                CursorShape::Underline => commands.push(DrawCommand::Rect {
                    rect: Rect::new(x, y + cell.height - thin * 2.0, width, thin * 2.0),
                    radius: 0.0,
                    color: theme.cursor,
                }),
                CursorShape::Beam => commands.push(DrawCommand::Rect {
                    rect: Rect::new(x, y, thin * 2.0, cell.height),
                    radius: 0.0,
                    color: theme.cursor,
                }),
                _ => {
                    commands.push(DrawCommand::Rect {
                        rect: Rect::new(x, y, width, cell.height),
                        radius: 0.0,
                        color: theme.cursor,
                    });
                    let character = cell_ref.c;
                    if character != ' ' && !cell_ref.flags.contains(Flags::HIDDEN) {
                        let mut run = Some(Run {
                            text: character.to_string(),
                            column,
                            color: theme.background,
                            bold: cell_ref.flags.contains(Flags::BOLD),
                            italic: cell_ref.flags.contains(Flags::ITALIC),
                        });
                        flush_run(&mut run, origin.x, y, cell, baseline, font, commands);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::event::VoidListener;

    fn keys(key: &str, modifiers: Modifiers) -> Vec<u8> {
        encode_key(key, modifiers, false).unwrap()
    }

    const NONE: Modifiers = Modifiers {
        shift: false,
        control: false,
        alt: false,
        meta: false,
    };

    #[test]
    fn keys_encode_like_xterm() {
        assert_eq!(keys("a", NONE), b"a");
        assert_eq!(keys("ç", NONE), "ç".as_bytes());
        assert_eq!(keys(" ", NONE), b" ");
        assert_eq!(keys("Enter", NONE), b"\r");
        assert_eq!(keys("Backspace", NONE), b"\x7f");
        assert_eq!(keys("Tab", NONE), b"\t");
        assert_eq!(
            keys(
                "Tab",
                Modifiers {
                    shift: true,
                    ..NONE
                }
            ),
            b"\x1b[Z"
        );
        assert_eq!(keys("Escape", NONE), b"\x1b");
        assert_eq!(keys("ArrowUp", NONE), b"\x1b[A");
        assert_eq!(encode_key("ArrowUp", NONE, true).unwrap(), b"\x1bOA");
        assert_eq!(
            keys(
                "ArrowLeft",
                Modifiers {
                    control: true,
                    ..NONE
                }
            ),
            b"\x1b[1;5D"
        );
        assert_eq!(keys("Home", NONE), b"\x1b[H");
        assert_eq!(keys("End", NONE), b"\x1b[F");
        assert_eq!(keys("PageUp", NONE), b"\x1b[5~");
        assert_eq!(
            keys(
                "Delete",
                Modifiers {
                    shift: true,
                    ..NONE
                }
            ),
            b"\x1b[3;2~"
        );
        assert_eq!(keys("F1", NONE), b"\x1bOP");
        assert_eq!(keys("F4", NONE), b"\x1bOS");
        assert_eq!(keys("F5", NONE), b"\x1b[15~");
        assert_eq!(keys("F12", NONE), b"\x1b[24~");
        assert_eq!(
            keys(
                "F2",
                Modifiers {
                    shift: true,
                    ..NONE
                }
            ),
            b"\x1b[1;2Q"
        );
        assert_eq!(
            keys(
                "c",
                Modifiers {
                    control: true,
                    ..NONE
                }
            ),
            b"\x03"
        );
        assert_eq!(
            keys(
                "C",
                Modifiers {
                    control: true,
                    ..NONE
                }
            ),
            b"\x03"
        );
        assert_eq!(
            keys(
                "[",
                Modifiers {
                    control: true,
                    ..NONE
                }
            ),
            b"\x1b"
        );
        assert_eq!(
            keys(
                " ",
                Modifiers {
                    control: true,
                    ..NONE
                }
            ),
            b"\x00"
        );
        assert_eq!(keys("b", Modifiers { alt: true, ..NONE }), b"\x1bb");
        assert_eq!(
            keys("Backspace", Modifiers { alt: true, ..NONE }),
            b"\x1b\x7f"
        );
        assert_eq!(encode_key("Shift", NONE, false), None);
        assert_eq!(encode_key("Dead_acute", NONE, false), None);
        assert_eq!(
            encode_key("c", Modifiers { meta: true, ..NONE }, false),
            None
        );
    }

    #[test]
    fn pastes_are_bracketed_and_bounded() {
        assert_eq!(paste_bytes("a\nb\r\nc", false), b"a\rb\rc");
        assert_eq!(
            paste_bytes("x\x1b[201~y", true),
            b"\x1b[200~xy\x1b[201~".to_vec()
        );
        let long = "é".repeat(MAX_PASTE);
        assert!(paste_bytes(&long, false).len() <= MAX_PASTE);
        assert!(std::str::from_utf8(&paste_bytes(&long, false)).is_ok());
    }

    #[test]
    fn grid_size_fits_whole_cells() {
        assert_eq!(
            grid_size(Size::new(800.0, 480.0), Size::new(8.0, 16.0)),
            (100, 30)
        );
        assert_eq!(
            grid_size(Size::new(803.9, 495.0), Size::new(8.0, 16.0)),
            (100, 30)
        );
        assert_eq!(grid_size(Size::new(3.0, 3.0), Size::new(8.0, 16.0)), (1, 1));
        assert_eq!(
            grid_size(Size::new(1e9, 1e9), Size::new(8.0, 16.0)),
            (MAX_COLS, MAX_ROWS)
        );
    }

    #[test]
    fn palettes_parse_and_fall_back() {
        let palette = parse_palette("#000000, #f00, bogus,#80112233");
        assert_eq!(palette[0], Color::from_argb(0xFF000000));
        assert_eq!(palette[1], Color::from_argb(0xFFFF0000));
        assert_eq!(palette[2], Theme::default().palette[2]);
        assert_eq!(palette[3], Color::from_argb(0x80112233));
        assert_eq!(indexed(16), Color::rgba(0, 0, 0, 255));
        assert_eq!(indexed(231), Color::rgba(255, 255, 255, 255));
        assert_eq!(indexed(232), Color::rgba(8, 8, 8, 255));
    }

    fn term(cols: usize, rows: usize, bytes: &[u8]) -> Term<VoidListener> {
        let mut term = Term::new(config(100), &TermSize::new(cols, rows), VoidListener);
        let mut parser: Processor<StdSyncHandler> = Processor::new();
        parser.advance(&mut term, bytes);
        term
    }

    fn paint(term: &Term<VoidListener>, focused: bool) -> Vec<DrawCommand> {
        let mut commands = Vec::new();
        let theme = Theme::default();
        let font = FontSpec::default();
        let look = Look {
            origin: Point::new(0.0, 0.0),
            theme: &theme,
            font: &font,
            cell: Size::new(8.0, 16.0),
            baseline: 12.0,
            focused,
        };
        paint_grid(term, &look, &mut commands);
        commands
    }

    fn texts(commands: &[DrawCommand]) -> Vec<(String, f32, f32, Color, u16)> {
        commands
            .iter()
            .filter_map(|command| match command {
                DrawCommand::Text {
                    text,
                    baseline,
                    color,
                    font,
                    ..
                } => Some((text.clone(), baseline.x, baseline.y, *color, font.weight)),
                _ => None,
            })
            .collect()
    }

    fn rects(commands: &[DrawCommand]) -> Vec<(Rect, Color)> {
        commands
            .iter()
            .filter_map(|command| match command {
                DrawCommand::Rect { rect, color, .. } => Some((*rect, *color)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn text_is_painted_in_style_runs_per_row() {
        // "ab" plain, "cd" red, then bold "ef"; a second row "xy".
        let term = term(20, 3, b"ab\x1b[31mcd\x1b[0m \x1b[1mef\x1b[0m\r\nxy");
        let theme = Theme::default();
        let runs = texts(&paint(&term, false));
        assert_eq!(runs[0].0, "ab");
        assert_eq!(runs[0].3, theme.foreground);
        assert_eq!((runs[1].0.as_str(), runs[1].1), ("cd", 16.0));
        assert_eq!(runs[1].3, theme.palette[1]);
        assert_eq!(
            (runs[2].0.as_str(), runs[2].1, runs[2].4),
            ("ef", 40.0, 700)
        );
        assert_eq!((runs[3].0.as_str(), runs[3].2), ("xy", 16.0 + 12.0));
        assert_eq!(runs.len(), 4);
    }

    #[test]
    fn backgrounds_merge_into_runs_and_wide_characters_keep_their_cell() {
        let term = term(
            20,
            2,
            b"\x1b[44m    \x1b[0m  \x1b[41m  \x1b[0m\r\n\xe6\xbc\xa2x",
        );
        let theme = Theme::default();
        let backgrounds: Vec<(Rect, Color)> = rects(&paint(&term, false))
            .into_iter()
            .filter(|(_, color)| *color == theme.palette[4] || *color == theme.palette[1])
            .collect();
        assert_eq!(
            backgrounds,
            vec![
                (Rect::new(0.0, 0.0, 32.0, 16.0), theme.palette[4]),
                (Rect::new(48.0, 0.0, 16.0, 16.0), theme.palette[1]),
            ]
        );
        let runs = texts(&paint(&term, false));
        let wide = runs.iter().find(|run| run.0 == "漢").unwrap();
        assert_eq!(wide.1, 0.0);
        let after = runs.iter().find(|run| run.0 == "x").unwrap();
        assert_eq!(
            after.1, 16.0,
            "the character after a wide one starts two cells later"
        );
    }

    #[test]
    fn selection_paints_its_cells_and_copies_its_text() {
        let mut term = term(20, 2, b"hello world");
        let mut selection = Selection::new(
            SelectionType::Simple,
            GridPoint::new(Line(0), Column(6)),
            Side::Left,
        );
        selection.update(GridPoint::new(Line(0), Column(10)), Side::Right);
        term.selection = Some(selection);
        assert_eq!(term.selection_to_string().as_deref(), Some("world"));
        let theme = Theme::default();
        let selected: Vec<Rect> = rects(&paint(&term, false))
            .into_iter()
            .filter(|(_, color)| *color == theme.selection)
            .map(|(rect, _)| rect)
            .collect();
        assert_eq!(selected, vec![Rect::new(48.0, 0.0, 40.0, 16.0)]);
        term.selection = Some(Selection::new(
            SelectionType::Semantic,
            GridPoint::new(Line(0), Column(1)),
            Side::Left,
        ));
        assert_eq!(term.selection_to_string().as_deref(), Some("hello"));
    }

    #[test]
    fn the_cursor_is_solid_when_focused_and_hollow_otherwise() {
        let term = term(10, 2, b"ab");
        let theme = Theme::default();
        let cursor = |focused| -> Vec<Rect> {
            rects(&paint(&term, focused))
                .into_iter()
                .filter(|(_, color)| *color == theme.cursor)
                .map(|(rect, _)| rect)
                .collect()
        };
        assert_eq!(cursor(true), vec![Rect::new(16.0, 0.0, 8.0, 16.0)]);
        assert_eq!(cursor(false).len(), 4);
        let hidden = self::term(10, 2, b"ab\x1b[?25l");
        assert!(
            rects(&paint(&hidden, true))
                .iter()
                .all(|(_, color)| *color != theme.cursor)
        );
    }

    #[test]
    fn resizing_keeps_the_content_and_the_scrollback_is_bounded() {
        let mut term = term(10, 3, b"one\r\ntwo\r\nthree");
        term.resize(TermSize::new(20, 5));
        assert_eq!(term.columns(), 20);
        assert_eq!(term.screen_lines(), 5);
        let text: String = texts(&paint(&term, false))
            .into_iter()
            .map(|run| run.0)
            .collect();
        assert!(text.contains("one") && text.contains("three"), "{text}");
        let mut long = self::term(10, 3, &b"x\r\n".repeat(1000));
        assert!(long.grid().history_size() <= 100);
        long.scroll_display(Scroll::Delta(5));
        assert_eq!(long.grid().display_offset(), 5);
    }
}

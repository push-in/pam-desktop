//! What the user sees when PHP reports an error.
//!
//! In development a non-fatal error shows a toast at the bottom of the
//! window and a fatal one a full panel with the message, the source around
//! the failing line and the call stack. In production nothing technical is
//! shown: a fatal error reloads the application a few times and then offers
//! a "try again" screen.
//!
//! The overlay is host chrome drawn with synthetic nodes, so it works even
//! when PHP never produced a frame.

use std::collections::HashSet;

use pam_desktop_wire::{NodeKind, PropKey, PropValue};

use crate::backend::NodeId;
use crate::chrome::{Builder, Measure, TextStyle};
use crate::context::Ctx;
use crate::geometry::{Rect, Size};

const MAX_QUEUE: usize = 50;
const MAX_MESSAGE_CHARS: usize = 12_000;
const MAX_FRAMES: usize = 128;
/// Automatic reloads of a production application before the fallback.
const RECOVERY_ATTEMPTS: u32 = 3;
const HEADER_HEIGHT: f32 = 52.0;
const FOOTER_HEIGHT: f32 = 64.0;
const MARGIN: f32 = 20.0;

/// Commands of the overlay's own buttons, carried by `Action::Chrome`.
pub mod command {
    pub const OPEN: u16 = 1;
    pub const DISMISS_ALL: u16 = 2;
    pub const MINIMIZE: u16 = 3;
    pub const DISMISS: u16 = 4;
    pub const COPY: u16 = 5;
    pub const RELOAD: u16 = 6;
    pub const PREVIOUS: u16 = 7;
    pub const NEXT: u16 = 8;
    pub const TOGGLE_FRAMES: u16 = 9;
}

/// What the runtime must do after an overlay command.
#[derive(Debug, PartialEq, Eq)]
pub enum Effect {
    None,
    /// Restart the PHP worker.
    Reload,
    /// Put this text on the clipboard.
    Copy(String),
}

#[derive(Clone, Debug, PartialEq)]
struct Frame {
    file: Option<String>,
    line: i64,
    call: String,
    app: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct Snippet {
    start: i64,
    line: i64,
    lines: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
struct Report {
    kind: String,
    message: String,
    file: String,
    line: i64,
    phase: String,
    fatal: bool,
    frames: Vec<Frame>,
    app_frame: Option<usize>,
    snippet: Option<Snippet>,
    fingerprint: String,
    count: u32,
}

impl Report {
    /// Parses a `PAMERR1\n<json>` report; anything else is a fatal native error.
    fn parse(raw: &str) -> Self {
        let plain = |message: &str| Self {
            kind: String::from("NativeRuntimeError"),
            message: message.chars().take(MAX_MESSAGE_CHARS).collect(),
            file: String::from("Native runtime"),
            line: 0,
            phase: String::from("native"),
            fatal: true,
            frames: Vec::new(),
            app_frame: None,
            snippet: None,
            fingerprint: message.chars().take(256).collect(),
            count: 1,
        };
        let Some(json) = raw.strip_prefix("PAMERR1\n") else {
            return plain(raw);
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
            return plain(json);
        };
        let text = |key: &str| {
            value
                .get(key)
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let phase = text("phase");
        let frames: Vec<Frame> = value
            .get("frames")
            .and_then(serde_json::Value::as_array)
            .map(|frames| {
                frames
                    .iter()
                    .take(MAX_FRAMES)
                    .map(|frame| Frame {
                        file: frame
                            .get("file")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_owned),
                        line: frame
                            .get("line")
                            .and_then(serde_json::Value::as_i64)
                            .unwrap_or(0),
                        call: frame
                            .get("call")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        app: frame.get("kind").and_then(serde_json::Value::as_str) == Some("app"),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let snippet = value
            .get("snippet")
            .filter(|snippet| snippet.is_object())
            .map(|snippet| Snippet {
                start: snippet
                    .get("start")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or(1),
                line: snippet
                    .get("line")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or(0),
                lines: snippet
                    .get("lines")
                    .and_then(serde_json::Value::as_array)
                    .map(|lines| {
                        lines
                            .iter()
                            .take(16)
                            .map(|line| line.as_str().unwrap_or_default().to_owned())
                            .collect()
                    })
                    .unwrap_or_default(),
            });
        let message: String = text("message").chars().take(MAX_MESSAGE_CHARS).collect();
        let fingerprint = Some(text("fingerprint"))
            .filter(|fingerprint| !fingerprint.is_empty())
            .unwrap_or_else(|| format!("{}|{}|{}", text("type"), text("file"), message));
        Self {
            kind: text("type"),
            message,
            file: text("file"),
            line: value
                .get("line")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(0),
            // Version 1 reports carry no phase and count as fatal.
            fatal: value
                .get("fatal")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(phase.is_empty()),
            phase,
            app_frame: value
                .get("appFrame")
                .and_then(serde_json::Value::as_u64)
                .map(|index| index as usize)
                .filter(|index| *index < frames.len()),
            frames,
            snippet,
            fingerprint,
            count: 1,
        }
    }

    fn short_kind(&self) -> &str {
        self.kind.rsplit('\\').next().unwrap_or(&self.kind)
    }

    fn phase_label(&self) -> &'static str {
        match self.phase.as_str() {
            "boot" => "Boot error",
            "render" => "Render error",
            "event" => "Event handler error",
            "module" => "Module callback error",
            "promise" => "Unhandled promise rejection",
            "hot-reload" => "Hot reload error",
            "native" => "Native runtime error",
            _ => "Uncaught error",
        }
    }

    /// Where the application's own code failed.
    fn location(&self) -> String {
        match self.app_frame.and_then(|index| self.frames.get(index)) {
            Some(Frame {
                file: Some(file),
                line,
                ..
            }) => format!("{file}:{line}"),
            _ if self.line > 0 => format!("{}:{}", self.file, self.line),
            _ => self.file.clone(),
        }
    }

    /// The plain-text report the Copy button puts on the clipboard.
    fn copy_text(&self) -> String {
        let mut text = format!(
            "{}: {}\n  at {} ({}{})\n",
            self.kind,
            self.message,
            self.location(),
            self.phase,
            if self.fatal { ", fatal" } else { "" }
        );
        if let Some(snippet) = &self.snippet {
            text.push('\n');
            for (offset, line) in snippet.lines.iter().enumerate() {
                let number = snippet.start + offset as i64;
                text.push_str(&format!(
                    "{} {number:>5} | {line}\n",
                    if number == snippet.line { ">" } else { " " }
                ));
            }
        }
        if !self.frames.is_empty() {
            text.push('\n');
            for (index, frame) in self.frames.iter().enumerate() {
                text.push_str(&format!(
                    "#{index} {}({}): {}\n",
                    frame.file.as_deref().unwrap_or("[internal]"),
                    frame.line,
                    frame.call
                ));
            }
        }
        text
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum View {
    Hidden,
    Toast,
    Panel,
    /// Production: the application could not recover by itself.
    Fallback,
}

struct Palette {
    background: i64,
    surface: i64,
    text: i64,
    muted: i64,
    border: i64,
    accent: i64,
    accent_text: i64,
    highlight: i64,
    code: i64,
}

fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            background: 0xFF12_1214,
            surface: 0xFF1C_1C20,
            text: 0xFFF4_F4F5,
            muted: 0xFFA1_A1AA,
            border: 0xFF34_343A,
            accent: 0xFFB3_261E,
            accent_text: 0xFFFF_8A80,
            highlight: 0xFF4A_1C1A,
            code: 0xFF0B_0B0D,
        }
    } else {
        Palette {
            background: 0xFFFF_FFFF,
            surface: 0xFFF4_F4F5,
            text: 0xFF18_181B,
            muted: 0xFF52_525B,
            border: 0xFFE4_E4E7,
            accent: 0xFFB3_261E,
            accent_text: 0xFFB3_261E,
            highlight: 0xFFFC_E1DE,
            code: 0xFFF7_F7F8,
        }
    }
}

pub struct ErrorOverlay {
    /// Development shows details; production recovers silently.
    development: bool,
    queue: Vec<Report>,
    /// Fingerprints dismissed since the last reload.
    suppressed: HashSet<String>,
    current: usize,
    view: View,
    /// Framework frames of the call stack are listed.
    expanded: bool,
    roots: Vec<NodeId>,
    /// Reloads tried since the last committed frame (production).
    attempts: u32,
    portuguese: bool,
}

impl ErrorOverlay {
    #[must_use]
    pub fn new(development: bool, locale: &str) -> Self {
        Self {
            development,
            queue: Vec::new(),
            suppressed: HashSet::new(),
            current: 0,
            view: View::Hidden,
            expanded: false,
            roots: Vec::new(),
            attempts: 0,
            portuguese: locale.to_ascii_lowercase().starts_with("pt"),
        }
    }

    #[must_use]
    pub fn is_visible(&self) -> bool {
        self.view != View::Hidden
    }

    /// The overlay covers the window and takes the keyboard.
    #[must_use]
    pub fn is_blocking(&self) -> bool {
        matches!(self.view, View::Panel | View::Fallback)
    }

    /// Records an error report from PHP. Returns how long to wait before
    /// reloading the application by itself (production, fatal errors).
    pub fn report(&mut self, raw: &str) -> Option<u32> {
        let report = Report::parse(raw);
        if !self.development {
            if !report.fatal {
                return None;
            }
            if self.attempts < RECOVERY_ATTEMPTS {
                self.attempts += 1;
                // 250 ms, 500 ms, 1 s.
                return Some(250_u32 << (self.attempts - 1).min(3));
            }
            self.view = View::Fallback;
            return None;
        }
        if self.suppressed.contains(&report.fingerprint) {
            return None;
        }
        let fatal = report.fatal;
        match self
            .queue
            .iter()
            .position(|queued| queued.fingerprint == report.fingerprint)
        {
            Some(index) => {
                self.queue[index].count += 1;
                if fatal {
                    self.current = index;
                }
            }
            None => {
                if self.queue.len() == MAX_QUEUE {
                    self.queue.remove(0);
                    self.current = self.current.saturating_sub(1);
                }
                self.queue.push(report);
                if fatal || self.view == View::Hidden {
                    self.current = self.queue.len() - 1;
                }
            }
        }
        if fatal {
            self.view = View::Panel;
        } else if self.view == View::Hidden {
            self.view = View::Toast;
        }
        None
    }

    /// A frame of the application reached the screen: it recovered.
    pub fn frame_committed(&mut self) -> bool {
        self.attempts = 0;
        if self.view == View::Fallback {
            self.view = View::Hidden;
            return true;
        }
        false
    }

    /// The application reloads: nothing reported before applies any more.
    pub fn reset(&mut self) {
        self.queue.clear();
        self.suppressed.clear();
        self.current = 0;
        self.expanded = false;
        if self.view != View::Fallback {
            self.view = View::Hidden;
        }
    }

    /// Runs one of the overlay's buttons.
    pub fn run(&mut self, command: u16) -> Effect {
        match command {
            command::OPEN => self.view = View::Panel,
            command::MINIMIZE => {
                if self
                    .queue
                    .get(self.current)
                    .is_some_and(|report| !report.fatal)
                {
                    self.view = View::Toast;
                }
            }
            command::DISMISS_ALL => {
                for report in self.queue.drain(..) {
                    self.suppressed.insert(report.fingerprint);
                }
                self.current = 0;
                self.view = View::Hidden;
            }
            command::DISMISS => {
                if self.current < self.queue.len() {
                    let report = self.queue.remove(self.current);
                    self.suppressed.insert(report.fingerprint);
                }
                self.current = self.current.min(self.queue.len().saturating_sub(1));
                self.expanded = false;
                if self.queue.is_empty() {
                    self.view = View::Hidden;
                }
            }
            command::PREVIOUS => {
                self.current = self.current.saturating_sub(1);
                self.expanded = false;
            }
            command::NEXT => {
                self.current = (self.current + 1).min(self.queue.len().saturating_sub(1));
                self.expanded = false;
            }
            command::TOGGLE_FRAMES => self.expanded = !self.expanded,
            command::COPY => {
                return self
                    .queue
                    .get(self.current)
                    .map_or(Effect::None, |report| Effect::Copy(report.copy_text()));
            }
            command::RELOAD => {
                self.attempts = 0;
                return Effect::Reload;
            }
            _ => {}
        }
        Effect::None
    }

    /// Escape: the panel dismisses everything; the toast ignores it.
    pub fn escape(&mut self) -> bool {
        if self.view == View::Panel {
            self.run(command::DISMISS_ALL);
            return true;
        }
        false
    }

    /// Rebuilds the overlay for the current state and window size.
    pub fn rebuild(&mut self, ctx: &mut Ctx<'_>, measure: Measure<'_>, dark: bool) {
        for root in self.roots.drain(..) {
            ctx.scene.remove_synthetic(root);
        }
        let viewport = ctx.viewport;
        let colors = palette(dark);
        let mut builder = Builder::new(ctx, measure);
        let root = match self.view {
            View::Hidden => None,
            View::Toast => self.queue.get(self.current).map(|report| {
                Self::toast(&mut builder, report, self.queue.len(), viewport, &colors)
            }),
            View::Panel => self.queue.get(self.current).map(|report| {
                Self::panel(
                    &mut builder,
                    report,
                    self.current,
                    self.queue.len(),
                    self.expanded,
                    viewport,
                    &colors,
                )
            }),
            View::Fallback => Some(Self::fallback(
                &mut builder,
                self.portuguese,
                viewport,
                &colors,
            )),
        };
        self.roots.extend(root);
    }

    /// The overlay's top-level nodes, for the scene's host overlays.
    #[must_use]
    pub fn roots(&self) -> &[NodeId] {
        &self.roots
    }

    fn toast(
        builder: &mut Builder<'_, '_, '_>,
        report: &Report,
        queued: usize,
        viewport: Size,
        colors: &Palette,
    ) -> NodeId {
        let width = (viewport.width - 24.0).clamp(160.0, 560.0);
        let height = 76.0;
        let frame = Rect::new(
            (viewport.width - width) / 2.0,
            viewport.height - height - 12.0,
            width,
            height,
        );
        let root = builder.view(
            0,
            NodeKind::View,
            frame,
            &[
                (PropKey::BackgroundColor, PropValue::Integer(colors.surface)),
                (PropKey::BorderRadius, PropValue::Float(14.0)),
                (PropKey::BorderWidth, PropValue::Float(1.0)),
                (PropKey::BorderColor, PropValue::Integer(colors.border)),
                (
                    PropKey::BoxShadows,
                    PropValue::String("[[0,6,24,0,855638016,0]]".into()),
                ),
                (PropKey::Cursor, PropValue::String("pointer".into())),
            ],
            Some(command::OPEN),
        );
        let badge = Rect::new(frame.x + 14.0, frame.y + (height - 28.0) / 2.0, 28.0, 28.0);
        builder.view(
            root,
            NodeKind::Text,
            badge,
            &[
                (
                    PropKey::Text,
                    PropValue::String(if queued > 1 {
                        queued.to_string()
                    } else {
                        String::from("!")
                    }),
                ),
                (PropKey::FontSize, PropValue::Float(14.0)),
                (PropKey::FontWeight, PropValue::Integer(700)),
                (PropKey::TextColor, PropValue::Integer(0xFFFF_FFFF)),
                (PropKey::TextAlign, PropValue::Integer(2)),
                (PropKey::BackgroundColor, PropValue::Integer(colors.accent)),
                (PropKey::BorderRadius, PropValue::Float(14.0)),
                (PropKey::PointerEvents, PropValue::Integer(2)),
            ],
            None,
        );
        let text_x = badge.right() + 12.0;
        let text_width = (frame.right() - 52.0 - text_x).max(40.0);
        let title = if report.count > 1 {
            format!("{}  x{}", report.short_kind(), report.count)
        } else {
            report.short_kind().to_owned()
        };
        let title_height = builder.text(
            root,
            text_x,
            frame.y + 12.0,
            text_width,
            &title,
            &TextStyle::new(14.0, 700, colors.text).lines(1),
        );
        builder.text(
            root,
            text_x,
            frame.y + 12.0 + title_height + 2.0,
            text_width,
            &report.message,
            &TextStyle::new(13.0, 400, colors.muted).lines(2),
        );
        builder.button(
            root,
            Rect::new(
                frame.right() - 46.0,
                frame.y + (height - 36.0) / 2.0,
                36.0,
                36.0,
            ),
            "×",
            0,
            colors.muted,
            None,
            command::DISMISS_ALL,
        );
        root
    }

    #[allow(clippy::too_many_arguments)]
    fn panel(
        builder: &mut Builder<'_, '_, '_>,
        report: &Report,
        index: usize,
        queued: usize,
        expanded: bool,
        viewport: Size,
        colors: &Palette,
    ) -> NodeId {
        let (width, height) = (viewport.width, viewport.height);
        let root = builder.view(
            0,
            NodeKind::View,
            Rect::new(0.0, 0.0, width, height),
            &[(
                PropKey::BackgroundColor,
                PropValue::Integer(colors.background),
            )],
            None,
        );

        // Header.
        builder.view(
            root,
            NodeKind::View,
            Rect::new(0.0, 0.0, width, HEADER_HEIGHT),
            &[(PropKey::BackgroundColor, PropValue::Integer(colors.accent))],
            None,
        );
        builder.text(
            root,
            MARGIN,
            16.0,
            (width - 260.0).max(80.0),
            report.phase_label(),
            &TextStyle::new(15.0, 700, 0xFFFF_FFFF).lines(1),
        );
        let mut right = width - MARGIN;
        if !report.fatal {
            right -= 92.0;
            builder.button(
                root,
                Rect::new(right, 10.0, 92.0, 32.0),
                "Minimize",
                0x33FF_FFFF,
                0xFFFF_FFFF,
                None,
                command::MINIMIZE,
            );
            right -= 12.0;
        }
        if queued > 1 {
            right -= 32.0;
            builder.button(
                root,
                Rect::new(right, 10.0, 32.0, 32.0),
                "›",
                0x33FF_FFFF,
                0xFFFF_FFFF,
                None,
                command::NEXT,
            );
            right -= 56.0;
            builder.text(
                root,
                right,
                17.0,
                56.0,
                &format!("{} / {queued}", index + 1),
                &TextStyle::new(13.0, 600, 0xFFFF_FFFF).lines(1).centered(),
            );
            right -= 32.0;
            builder.button(
                root,
                Rect::new(right, 10.0, 32.0, 32.0),
                "‹",
                0x33FF_FFFF,
                0xFFFF_FFFF,
                None,
                command::PREVIOUS,
            );
        }

        // Body.
        let body_height = (height - HEADER_HEIGHT - FOOTER_HEIGHT).max(0.0);
        let body = builder.view(
            root,
            NodeKind::Scroll,
            Rect::new(0.0, HEADER_HEIGHT, width, body_height),
            &[],
            None,
        );
        let content_width = (width - MARGIN * 2.0).max(80.0);
        let mut y = HEADER_HEIGHT + MARGIN;
        let kind = if report.count > 1 {
            format!("{}  x{}", report.kind, report.count)
        } else {
            report.kind.clone()
        };
        y += builder.text(
            body,
            MARGIN,
            y,
            content_width,
            &kind,
            &TextStyle::new(13.0, 700, colors.accent_text),
        ) + 6.0;
        y += builder.text(
            body,
            MARGIN,
            y,
            content_width,
            &report.message,
            &TextStyle::new(20.0, 600, colors.text),
        ) + 10.0;
        y += builder.text(
            body,
            MARGIN,
            y,
            content_width,
            &report.location(),
            &TextStyle::new(13.0, 400, colors.muted).mono(),
        ) + 18.0;

        if let Some(snippet) = report
            .snippet
            .as_ref()
            .filter(|snippet| !snippet.lines.is_empty())
        {
            y += builder.text(
                body,
                MARGIN,
                y,
                content_width,
                "SOURCE",
                &TextStyle::new(11.0, 700, colors.muted),
            ) + 8.0;
            let top = y;
            let line_height = 20.0;
            let block = builder.view(
                body,
                NodeKind::View,
                Rect::new(
                    MARGIN,
                    top,
                    content_width,
                    snippet.lines.len() as f32 * line_height + 16.0,
                ),
                &[
                    (PropKey::BackgroundColor, PropValue::Integer(colors.code)),
                    (PropKey::BorderRadius, PropValue::Float(10.0)),
                    (PropKey::Overflow, PropValue::Integer(2)),
                ],
                None,
            );
            y += 8.0;
            for (offset, line) in snippet.lines.iter().enumerate() {
                let number = snippet.start + offset as i64;
                let failing = number == snippet.line;
                if failing {
                    builder.view(
                        block,
                        NodeKind::View,
                        Rect::new(MARGIN, y, content_width, line_height),
                        &[(
                            PropKey::BackgroundColor,
                            PropValue::Integer(colors.highlight),
                        )],
                        None,
                    );
                }
                let color = if failing { colors.text } else { colors.muted };
                builder.text(
                    block,
                    MARGIN + 12.0,
                    y + 2.0,
                    content_width - 24.0,
                    &format!("{number:>4}  {line}"),
                    &TextStyle::new(12.5, if failing { 600 } else { 400 }, color)
                        .mono()
                        .lines(1),
                );
                y += line_height;
            }
            y += 8.0 + 18.0;
        }

        if !report.frames.is_empty() {
            y += builder.text(
                body,
                MARGIN,
                y,
                content_width,
                "CALL STACK",
                &TextStyle::new(11.0, 700, colors.muted),
            ) + 8.0;
            let mut index = 0;
            while index < report.frames.len() {
                let frame = &report.frames[index];
                if frame.app || expanded {
                    let tinted = report.app_frame == Some(index);
                    let top = y;
                    let row = builder.view(
                        body,
                        NodeKind::View,
                        Rect::new(MARGIN, top, content_width, 44.0),
                        &[
                            (
                                PropKey::BackgroundColor,
                                PropValue::Integer(if tinted { colors.highlight } else { 0 }),
                            ),
                            (PropKey::BorderRadius, PropValue::Float(8.0)),
                        ],
                        None,
                    );
                    let call_height = builder.text(
                        row,
                        MARGIN + 10.0,
                        top + 6.0,
                        content_width - 20.0,
                        &frame.call,
                        &TextStyle::new(
                            13.0,
                            if frame.app { 600 } else { 400 },
                            if frame.app { colors.text } else { colors.muted },
                        )
                        .mono()
                        .lines(1),
                    );
                    let place = format!(
                        "{}:{}",
                        frame.file.as_deref().unwrap_or("[internal]"),
                        frame.line
                    );
                    let place_height = builder.text(
                        row,
                        MARGIN + 10.0,
                        top + 6.0 + call_height + 1.0,
                        content_width - 20.0,
                        &place,
                        &TextStyle::new(12.0, 400, colors.muted).mono().lines(1),
                    );
                    let row_height = 12.0 + call_height + 1.0 + place_height;
                    builder.set_frame(row, Rect::new(MARGIN, top, content_width, row_height));
                    y += row_height + 2.0;
                    index += 1;
                    continue;
                }
                // Consecutive frames outside the application collapse into one row.
                let hidden = report.frames[index..]
                    .iter()
                    .take_while(|frame| !frame.app)
                    .count();
                builder.button(
                    body,
                    Rect::new(MARGIN, y, content_width.min(260.0), 30.0),
                    &format!(
                        "▸ {hidden} framework frame{}",
                        if hidden == 1 { "" } else { "s" }
                    ),
                    colors.surface,
                    colors.muted,
                    None,
                    command::TOGGLE_FRAMES,
                );
                y += 34.0;
                index += hidden;
            }
            if expanded && report.frames.iter().any(|frame| !frame.app) {
                builder.button(
                    body,
                    Rect::new(MARGIN, y, content_width.min(260.0), 30.0),
                    "▾ Hide framework frames",
                    colors.surface,
                    colors.muted,
                    None,
                    command::TOGGLE_FRAMES,
                );
                y += 34.0;
            }
        }
        // Bottom padding of the scrolled content.
        builder.view(
            body,
            NodeKind::View,
            Rect::new(0.0, y, 1.0, MARGIN),
            &[],
            None,
        );

        // Footer.
        let footer_top = height - FOOTER_HEIGHT;
        builder.view(
            root,
            NodeKind::View,
            Rect::new(0.0, footer_top, width, FOOTER_HEIGHT),
            &[
                (
                    PropKey::BackgroundColor,
                    PropValue::Integer(colors.background),
                ),
                (PropKey::BorderTopWidth, PropValue::Float(1.0)),
                (PropKey::BorderColor, PropValue::Integer(colors.border)),
            ],
            None,
        );
        let button_top = footer_top + 14.0;
        builder.button(
            root,
            Rect::new(MARGIN, button_top, 96.0, 36.0),
            "Dismiss",
            colors.surface,
            colors.text,
            Some(colors.border),
            command::DISMISS,
        );
        builder.button(
            root,
            Rect::new(MARGIN + 106.0, button_top, 84.0, 36.0),
            "Copy",
            colors.surface,
            colors.text,
            Some(colors.border),
            command::COPY,
        );
        builder.button(
            root,
            Rect::new(width - MARGIN - 104.0, button_top, 104.0, 36.0),
            "Reload",
            colors.accent,
            0xFFFF_FFFF,
            None,
            command::RELOAD,
        );
        root
    }

    fn fallback(
        builder: &mut Builder<'_, '_, '_>,
        portuguese: bool,
        viewport: Size,
        colors: &Palette,
    ) -> NodeId {
        let (title, message, action) = if portuguese {
            (
                "Algo deu errado",
                "O app encontrou um problema inesperado. Tente novamente para continuar.",
                "Tentar novamente",
            )
        } else {
            (
                "Something went wrong",
                "The app hit an unexpected problem. Try again to continue.",
                "Try again",
            )
        };
        let (width, height) = (viewport.width, viewport.height);
        let root = builder.view(
            0,
            NodeKind::View,
            Rect::new(0.0, 0.0, width, height),
            &[(
                PropKey::BackgroundColor,
                PropValue::Integer(colors.background),
            )],
            None,
        );
        let column = (width - 64.0).clamp(160.0, 420.0);
        let left = (width - column) / 2.0;
        let mut y = (height / 2.0 - 110.0).max(24.0);
        builder.view(
            root,
            NodeKind::Text,
            Rect::new((width - 56.0) / 2.0, y, 56.0, 56.0),
            &[
                (PropKey::Text, PropValue::String("!".into())),
                (PropKey::FontSize, PropValue::Float(28.0)),
                (PropKey::FontWeight, PropValue::Integer(700)),
                (PropKey::TextColor, PropValue::Integer(0xFFFF_FFFF)),
                (PropKey::TextAlign, PropValue::Integer(2)),
                (PropKey::BackgroundColor, PropValue::Integer(colors.accent)),
                (PropKey::BorderRadius, PropValue::Float(28.0)),
            ],
            None,
        );
        y += 76.0;
        y += builder.text(
            root,
            left,
            y,
            column,
            title,
            &TextStyle::new(20.0, 700, colors.text).centered(),
        ) + 8.0;
        y += builder.text(
            root,
            left,
            y,
            column,
            message,
            &TextStyle::new(14.0, 400, colors.muted).centered(),
        ) + 24.0;
        builder.button(
            root,
            Rect::new((width - 180.0) / 2.0, y, 180.0, 42.0),
            action,
            colors.accent,
            0xFFFF_FFFF,
            None,
            command::RELOAD,
        );
        root
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPORT: &str = "PAMERR1\n{\"version\":2,\"type\":\"App\\\\Broken\",\"message\":\"Boom\",\"file\":\"index.php\",\"line\":12,\"phase\":\"event\",\"fatal\":false,\"frames\":[{\"file\":\"vendor/x.php\",\"line\":3,\"call\":\"X::run()\",\"kind\":\"framework\"},{\"file\":\"index.php\",\"line\":12,\"call\":\"Home->press()\",\"kind\":\"app\"}],\"appFrame\":1,\"snippet\":{\"file\":\"index.php\",\"line\":12,\"start\":11,\"lines\":[\"a();\",\"b();\"]},\"fingerprint\":\"abc\"}";

    #[test]
    fn reports_are_parsed() {
        let report = Report::parse(REPORT);
        assert_eq!(
            (
                report.kind.as_str(),
                report.short_kind(),
                report.message.as_str()
            ),
            ("App\\Broken", "Broken", "Boom")
        );
        assert_eq!(
            (
                report.phase_label(),
                report.fatal,
                report.location().as_str()
            ),
            ("Event handler error", false, "index.php:12")
        );
        assert_eq!(report.frames.len(), 2);
        assert!(report.copy_text().contains(">    12 | b();"));
        let plain = Report::parse("it crashed");
        assert!(plain.fatal && plain.phase == "native" && plain.message == "it crashed");
    }

    #[test]
    fn development_queues_counts_and_suppresses() {
        let mut overlay = ErrorOverlay::new(true, "en-US");
        assert_eq!(overlay.report(REPORT), None);
        assert_eq!(overlay.view, View::Toast);
        overlay.report(REPORT);
        assert_eq!((overlay.queue.len(), overlay.queue[0].count), (1, 2));
        assert_eq!(overlay.run(command::OPEN), Effect::None);
        assert!(overlay.is_blocking());
        assert!(
            matches!(overlay.run(command::COPY), Effect::Copy(text) if text.starts_with("App\\Broken: Boom"))
        );
        overlay.run(command::DISMISS);
        assert!(!overlay.is_visible());
        // A dismissed error stays quiet until the next reload.
        overlay.report(REPORT);
        assert!(!overlay.is_visible());
        overlay.reset();
        overlay.report(&REPORT.replace("\"fatal\":false", "\"fatal\":true"));
        assert_eq!(overlay.view, View::Panel);
        assert_eq!(overlay.run(command::RELOAD), Effect::Reload);
    }

    #[test]
    fn production_recovers_then_falls_back() {
        let fatal = REPORT.replace("\"fatal\":false", "\"fatal\":true");
        let mut overlay = ErrorOverlay::new(false, "pt-BR");
        assert_eq!(overlay.report(REPORT), None);
        assert!(!overlay.is_visible());
        assert_eq!(overlay.report(&fatal), Some(250));
        assert_eq!(overlay.report(&fatal), Some(500));
        assert_eq!(overlay.report(&fatal), Some(1000));
        assert_eq!(overlay.report(&fatal), None);
        assert_eq!(overlay.view, View::Fallback);
        assert!(overlay.frame_committed());
        assert!(!overlay.is_visible());
        assert_eq!(overlay.report(&fatal), Some(250));
    }
}

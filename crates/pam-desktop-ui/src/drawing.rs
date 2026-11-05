//! Drawing surfaces: the vector `Canvas` (a list of solid primitives) and
//! the freehand `DrawingCanvas` (strokes over an image).
//!
//! Both end as a list of [`DrawCommand`]s in the view's logical coordinates;
//! the backend only rasterises it.

use pam_desktop_wire::{NodeKind, PropKey};

use crate::backend::{DrawCommand, NodeId, PointerButton, PointerEvent, PointerPhase};
use crate::color::Color;
use crate::context::{event_kind, Ctx, HostCommand};
use crate::geometry::{Point, Rect, Size};
use crate::scene::{Change, IdMap};

const MAX_COMMANDS: usize = 10_000;
const MAX_STROKES: usize = 256;
const MAX_POINTS: usize = 2048;
const MIN_NORMAL_WIDTH: f32 = 0.0005;
const MAX_NORMAL_WIDTH: f32 = 0.25;
/// Width of a stroke whose document carries none: 6 px on a 360 px image.
const DEFAULT_NORMAL_WIDTH: f32 = 6.0 / 360.0;
/// Pointer samples closer than this to the previous point are dropped.
const MIN_SAMPLE_DISTANCE: f32 = 1.25;
/// Events above the bridge limit never reach PHP.
const MAX_EVENT_BYTES: usize = 1024 * 1024;

fn number(value: Option<&serde_json::Value>) -> f32 {
    value.and_then(serde_json::Value::as_f64).filter(|number| number.is_finite()).map_or(0.0, |number| number as f32)
}

fn color(value: Option<&serde_json::Value>) -> Option<Color> {
    let value = value?;
    let bits = value.as_i64().or_else(|| value.as_u64().map(|bits| bits as i64)).or_else(|| value.as_f64().map(|bits| bits as i64))?;
    Some(Color::from_argb(bits as u32))
}

/// Decodes the `CanvasCommands` JSON: rectangles, rounded rectangles,
/// circles and lines. Anything malformed is skipped.
#[must_use]
pub fn parse_canvas(json: &str) -> Vec<DrawCommand> {
    let Ok(serde_json::Value::Array(entries)) = serde_json::from_str::<serde_json::Value>(json) else { return Vec::new() };
    let mut commands = Vec::with_capacity(entries.len().min(MAX_COMMANDS));
    for entry in entries.iter().take(MAX_COMMANDS) {
        let Some(object) = entry.as_object() else { continue };
        let field = |name: &str| number(object.get(name));
        let paint = color(object.get("color")).unwrap_or(Color::from_argb(0));
        let kind = object.get("kind").and_then(serde_json::Value::as_f64).map_or(0, |kind| kind as i64);
        let command = match kind {
            1 | 2 => {
                let rect = Rect::new(field("x"), field("y"), field("width"), field("height"));
                if rect.width <= 0.0 || rect.height <= 0.0 {
                    continue;
                }
                let radius = if kind == 2 { field("radius").clamp(0.0, rect.width.min(rect.height) / 2.0) } else { 0.0 };
                DrawCommand::Rect { rect, radius, color: paint }
            }
            3 => DrawCommand::Circle { center: Point::new(field("centerX"), field("centerY")), radius: field("radius").max(0.0), color: paint },
            4 => DrawCommand::Line {
                from: Point::new(field("startX"), field("startY")),
                to: Point::new(field("endX"), field("endY")),
                width: field("width").max(0.0),
                color: paint,
            },
            _ => continue,
        };
        commands.push(command);
    }
    commands
}

/// A committed stroke, normalised to the content rectangle (0..1).
#[derive(Clone, Debug, PartialEq)]
struct Stroke {
    color: u32,
    /// Fraction of the content width.
    width: f32,
    erase: bool,
    points: Vec<Point>,
}

/// Decodes a stroke document; malformed strokes are skipped.
fn decode_strokes(json: &str) -> Vec<Stroke> {
    let Ok(serde_json::Value::Object(root)) = serde_json::from_str::<serde_json::Value>(json) else { return Vec::new() };
    let Some(entries) = root.get("strokes").and_then(serde_json::Value::as_array) else { return Vec::new() };
    let mut strokes = Vec::new();
    for entry in entries.iter().take(MAX_STROKES) {
        let Some(values) = entry.get("points").and_then(serde_json::Value::as_array) else { continue };
        let Some(numbers) = values.iter().map(|value| value.as_f64().filter(|number| number.is_finite())).collect::<Option<Vec<f64>>>() else { continue };
        let points: Vec<Point> =
            numbers.as_chunks::<2>().0.iter().take(MAX_POINTS).map(|pair| Point::new((pair[0] as f32).clamp(0.0, 1.0), (pair[1] as f32).clamp(0.0, 1.0))).collect();
        if points.is_empty() {
            continue;
        }
        let width = entry.get("width").and_then(serde_json::Value::as_f64).map_or(DEFAULT_NORMAL_WIDTH, |width| width as f32);
        strokes.push(Stroke {
            color: color(entry.get("color")).map_or(0xFFFF_FFFF, Color::to_argb),
            width: if width.is_finite() { width.clamp(MIN_NORMAL_WIDTH, MAX_NORMAL_WIDTH) } else { DEFAULT_NORMAL_WIDTH },
            erase: entry.get("mode").and_then(serde_json::Value::as_i64) == Some(2),
            points,
        });
    }
    strokes
}

/// Shortest text of a normalised value: six decimals, no trailing zeros.
fn compact(value: f32) -> String {
    let text = format!("{value:.6}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text.is_empty() || text == "-0" { String::from("0") } else { text.to_owned() }
}

/// Encodes the document with ordered keys, the last 256 strokes at most.
fn encode_strokes(strokes: &[Stroke]) -> String {
    let mut json = String::from("{\"version\":1,\"strokes\":[");
    let first = strokes.len().saturating_sub(MAX_STROKES);
    for (index, stroke) in strokes[first..].iter().enumerate() {
        if index > 0 {
            json.push(',');
        }
        json.push_str(&format!("{{\"color\":{},\"width\":{},\"mode\":{},\"points\":[", stroke.color, compact(stroke.width), if stroke.erase { 2 } else { 1 }));
        for (position, point) in stroke.points.iter().enumerate() {
            if position > 0 {
                json.push(',');
            }
            json.push_str(&compact(point.x));
            json.push(',');
            json.push_str(&compact(point.y));
        }
        json.push_str("]}");
    }
    json.push_str("]}");
    json
}

/// The aspect-fit rectangle of an image of `natural` size inside `view`.
fn content_rect(view: Size, natural: Option<Size>) -> Rect {
    match natural.filter(|size| size.width > 0.0 && size.height > 0.0) {
        Some(natural) => {
            let scale = (view.width / natural.width).min(view.height / natural.height);
            let (width, height) = (natural.width * scale, natural.height * scale);
            Rect::new((view.width - width) / 2.0, (view.height - height) / 2.0, width, height)
        }
        None => Rect::from_size(view),
    }
}

#[derive(Default)]
struct Sketch {
    strokes: Vec<Stroke>,
    /// Points of the stroke being drawn, normalised.
    active: Option<Vec<Point>>,
    /// The document as last received or emitted, compared byte for byte.
    encoded: String,
    clear_token: i64,
    undo_token: i64,
    natural: Option<Size>,
    mounted: bool,
    dirty: bool,
}

#[derive(Default)]
pub struct Drawings {
    canvases: IdMap<bool>,
    sketches: IdMap<Sketch>,
    /// The sketch that owns the pointer until it is released.
    capture: Option<NodeId>,
    dirty: bool,
}

impl Drawings {
    pub fn on_change(&mut self, _ctx: &mut Ctx<'_>, change: Change) {
        match change {
            Change::Created(id, NodeKind::Canvas) => {
                self.canvases.insert(id, true);
                self.dirty = true;
            }
            Change::Created(id, NodeKind::DrawingCanvas) => {
                self.sketches.insert(id, Sketch { dirty: true, ..Sketch::default() });
                self.dirty = true;
            }
            Change::Removed(id, NodeKind::Canvas) => {
                self.canvases.remove(&id);
            }
            Change::Removed(id, NodeKind::DrawingCanvas) => {
                self.sketches.remove(&id);
                if self.capture == Some(id) {
                    self.capture = None;
                }
            }
            Change::Updated(id, PropKey::CanvasCommands) => {
                if let Some(dirty) = self.canvases.get_mut(&id) {
                    *dirty = true;
                    self.dirty = true;
                }
            }
            Change::Updated(id, _) | Change::Frame(id) => {
                if let Some(sketch) = self.sketches.get_mut(&id) {
                    sketch.dirty = true;
                    self.dirty = true;
                }
            }
            _ => {}
        }
    }

    pub fn after_commit(&mut self, ctx: &mut Ctx<'_>) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        for (id, dirty) in &mut self.canvases {
            if std::mem::take(dirty) {
                let commands = ctx.scene.node(*id).map(|node| parse_canvas(node.props().text_or_empty(PropKey::CanvasCommands))).unwrap_or_default();
                ctx.commands.push(HostCommand::SetDrawing { id: *id, commands });
            }
        }
        let dirty: Vec<NodeId> = self.sketches.iter().filter(|(_, sketch)| sketch.dirty).map(|(id, _)| *id).collect();
        for id in dirty {
            self.sync(ctx, id);
        }
    }

    /// Applies the properties of a sketch: the document and the clear and undo tokens.
    fn sync(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        let Some(node) = ctx.scene.node(id) else { return };
        let props = node.props();
        let value = props.text_or_empty(PropKey::Value).to_owned();
        let (clear, undo) = (props.integer_or(PropKey::DrawingClearRequest, 0), props.integer_or(PropKey::DrawingUndoRequest, 0));
        let Some(sketch) = self.sketches.get_mut(&id) else { return };
        sketch.dirty = false;
        if value != sketch.encoded {
            sketch.strokes = decode_strokes(&value);
            sketch.active = None;
            if self.capture == Some(id) {
                self.capture = None;
            }
            sketch.encoded = value;
        }
        let mut changed = false;
        if !sketch.mounted {
            // Tokens present at mount are the baseline: a remount must not
            // repeat a clear the application asked for long ago.
            sketch.mounted = true;
            sketch.clear_token = clear;
            sketch.undo_token = undo;
        } else {
            if clear != sketch.clear_token {
                sketch.clear_token = clear;
                if clear > 0 && (!sketch.strokes.is_empty() || sketch.active.is_some()) {
                    sketch.strokes.clear();
                    sketch.active = None;
                    changed = true;
                }
            }
            if undo != sketch.undo_token {
                sketch.undo_token = undo;
                if undo > 0 && sketch.strokes.pop().is_some() {
                    changed = true;
                }
            }
        }
        if changed {
            self.emit(ctx, id);
        }
        self.paint(ctx, id);
    }

    /// The image under a sketch was decoded (or went away): strokes follow
    /// its aspect-fit rectangle.
    pub fn set_natural_size(&mut self, ctx: &mut Ctx<'_>, id: NodeId, natural: Option<Size>) {
        if let Some(sketch) = self.sketches.get_mut(&id)
            && sketch.natural != natural {
                sketch.natural = natural;
                self.paint(ctx, id);
            }
    }

    fn emit(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        let Some(sketch) = self.sketches.get_mut(&id) else { return };
        sketch.encoded = encode_strokes(&sketch.strokes);
        if sketch.encoded.len() > MAX_EVENT_BYTES {
            eprintln!("[pam-desktop] drawing {id} exceeds the one MiB event limit; the change was not reported");
            return;
        }
        let payload = sketch.encoded.clone().into_bytes();
        ctx.emit_if(id, PropKey::OnChange, event_kind::CHANGE, payload);
    }

    fn content(&self, ctx: &Ctx<'_>, id: NodeId) -> Option<Rect> {
        let size = ctx.scene.node(id)?.frame.size();
        Some(content_rect(size, self.sketches.get(&id)?.natural))
    }

    fn brush(ctx: &Ctx<'_>, id: NodeId, content: Rect) -> (u32, f32, bool) {
        let Some(node) = ctx.scene.node(id) else { return (0xFFFF_FFFF, DEFAULT_NORMAL_WIDTH, false) };
        let props = node.props();
        let color = props.integer(PropKey::DrawingColor).map_or(0xFFFF_FFFF, |color| color as u32);
        let width = props.number_or(PropKey::DrawingWidth, 6.0).clamp(1.0, 64.0);
        let normal = (width / content.width.max(1.0)).clamp(MIN_NORMAL_WIDTH, MAX_NORMAL_WIDTH);
        (color, normal, props.integer_or(PropKey::DrawingMode, 1) == 2)
    }

    /// Rebuilds the draw list of a sketch.
    fn paint(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        let Some(content) = self.content(ctx, id) else { return };
        let Some(sketch) = self.sketches.get(&id) else { return };
        let to_view = |point: &Point| Point::new(content.x + point.x * content.width, content.y + point.y * content.height);
        let mut commands = Vec::with_capacity(sketch.strokes.len() + 3);
        commands.push(DrawCommand::PushGroup { clip: content });
        for stroke in &sketch.strokes {
            commands.push(DrawCommand::Stroke {
                points: stroke.points.iter().map(to_view).collect(),
                width: (stroke.width * content.width).max(1.0),
                color: Color::from_argb(stroke.color),
                erase: stroke.erase,
            });
        }
        if let Some(active) = &sketch.active {
            let (color, width, erase) = Self::brush(ctx, id, content);
            commands.push(DrawCommand::Stroke { points: active.iter().map(to_view).collect(), width: (width * content.width).max(1.0), color: Color::from_argb(color), erase });
        }
        commands.push(DrawCommand::PopGroup);
        ctx.commands.push(HostCommand::SetDrawing { id, commands });
    }

    /// Feeds a pointer event to the sketches. `true` means a sketch consumed
    /// it and nothing else should react.
    pub fn on_pointer(&mut self, ctx: &mut Ctx<'_>, pointer: PointerEvent) -> bool {
        if let Some(id) = self.capture {
            let Some(content) = self.content(ctx, id) else {
                self.capture = None;
                return false;
            };
            let origin = ctx.scene.window_origin(id);
            let local = Point::new(pointer.position.x - origin.x, pointer.position.y - origin.y);
            match pointer.phase {
                PointerPhase::Move => {
                    self.sample(id, content, local);
                    self.paint(ctx, id);
                }
                PointerPhase::Up if pointer.button == PointerButton::Primary => {
                    self.capture = None;
                    self.sample(id, content, local);
                    let (color, width, erase) = Self::brush(ctx, id, content);
                    if let Some(sketch) = self.sketches.get_mut(&id)
                        && let Some(points) = sketch.active.take() {
                            sketch.strokes.push(Stroke { color, width, erase, points });
                            if sketch.strokes.len() > MAX_STROKES {
                                sketch.strokes.remove(0);
                            }
                        }
                    self.emit(ctx, id);
                    self.paint(ctx, id);
                }
                // Leaving the window keeps the stroke: the button is still held.
                PointerPhase::Cancel => {
                    self.cancel(ctx);
                }
                _ => {}
            }
            return true;
        }
        if pointer.phase != PointerPhase::Down || pointer.button != PointerButton::Primary {
            return false;
        }
        let Some(id) = ctx.scene.hit_test(pointer.position) else { return false };
        if !self.sketches.contains_key(&id) || !ctx.scene.node(id).is_some_and(|node| node.props().flag_or(PropKey::Enabled, true)) {
            return false;
        }
        let Some(content) = self.content(ctx, id) else { return false };
        let origin = ctx.scene.window_origin(id);
        let local = Point::new(pointer.position.x - origin.x, pointer.position.y - origin.y);
        let inside = local.x >= content.x && local.x <= content.right() && local.y >= content.y && local.y <= content.bottom();
        if !inside {
            return false;
        }
        if let Some(sketch) = self.sketches.get_mut(&id) {
            sketch.active = Some(Vec::new());
        }
        self.capture = Some(id);
        self.sample(id, content, local);
        self.paint(ctx, id);
        true
    }

    /// Drops the stroke in progress (Escape, lost pointer).
    pub fn cancel(&mut self, ctx: &mut Ctx<'_>) -> bool {
        let Some(id) = self.capture.take() else { return false };
        if let Some(sketch) = self.sketches.get_mut(&id) {
            sketch.active = None;
        }
        self.paint(ctx, id);
        true
    }

    /// Appends a pointer sample to the active stroke, unless it is too close
    /// to the previous one.
    fn sample(&mut self, id: NodeId, content: Rect, local: Point) {
        let Some(active) = self.sketches.get_mut(&id).and_then(|sketch| sketch.active.as_mut()) else { return };
        if active.len() >= MAX_POINTS {
            return;
        }
        let normal = Point::new(((local.x - content.x) / content.width.max(1.0)).clamp(0.0, 1.0), ((local.y - content.y) / content.height.max(1.0)).clamp(0.0, 1.0));
        if let Some(last) = active.last() {
            let (dx, dy) = ((normal.x - last.x) * content.width, (normal.y - last.y) * content.height);
            if dx.hypot(dy) < MIN_SAMPLE_DISTANCE {
                return;
            }
        }
        active.push(normal);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canvas_commands_decode_the_four_primitives() {
        let commands = parse_canvas(
            r#"[{"kind":1,"x":1,"y":2,"width":3,"height":4,"color":4278190335},
                {"kind":2,"x":0,"y":0,"width":10,"height":4,"radius":9,"color":4294901760},
                {"kind":3,"centerX":5,"centerY":6,"radius":-2,"color":1},
                {"kind":4,"startX":1,"startY":1,"endX":9,"endY":1,"width":2,"color":4294967295},
                {"kind":9}, 7, {"kind":1,"x":0,"y":0,"width":-1,"height":4}]"#,
        );
        assert_eq!(
            commands,
            vec![
                DrawCommand::Rect { rect: Rect::new(1.0, 2.0, 3.0, 4.0), radius: 0.0, color: Color::from_argb(0xFF00_00FF) },
                DrawCommand::Rect { rect: Rect::new(0.0, 0.0, 10.0, 4.0), radius: 2.0, color: Color::from_argb(0xFFFF_0000) },
                DrawCommand::Circle { center: Point::new(5.0, 6.0), radius: 0.0, color: Color::from_argb(1) },
                DrawCommand::Line { from: Point::new(1.0, 1.0), to: Point::new(9.0, 1.0), width: 2.0, color: Color::from_argb(0xFFFF_FFFF) },
            ]
        );
        assert!(parse_canvas("{}").is_empty());
        assert!(parse_canvas("not json").is_empty());
    }

    #[test]
    fn stroke_documents_round_trip() {
        let strokes = decode_strokes(r#"{"strokes":[{"mode":2,"points":[0.25,0.5,2,-1,0.1],"width":4.9E-4,"color":4278255360},{"points":["x",1]},{"points":[]}],"version":1}"#);
        assert_eq!(strokes, vec![Stroke { color: 0xFF00_FF00, width: MIN_NORMAL_WIDTH, erase: true, points: vec![Point::new(0.25, 0.5), Point::new(1.0, 0.0)] }]);
        let encoded = encode_strokes(&strokes);
        assert_eq!(encoded, r#"{"version":1,"strokes":[{"color":4278255360,"width":0.0005,"mode":2,"points":[0.25,0.5,1,0]}]}"#);
        assert_eq!(decode_strokes(&encoded), strokes);
        assert_eq!(encode_strokes(&[]), r#"{"version":1,"strokes":[]}"#);
        assert!(decode_strokes("").is_empty());
    }

    #[test]
    fn the_content_rectangle_fits_the_image() {
        assert_eq!(content_rect(Size::new(200.0, 100.0), Some(Size::new(50.0, 50.0))), Rect::new(50.0, 0.0, 100.0, 100.0));
        assert_eq!(content_rect(Size::new(200.0, 100.0), None), Rect::new(0.0, 0.0, 200.0, 100.0));
        assert_eq!(content_rect(Size::new(100.0, 100.0), Some(Size::new(0.0, 10.0))), Rect::new(0.0, 0.0, 100.0, 100.0));
    }
}

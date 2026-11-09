//! `PamView`: the widget behind every box, text and image node.
//!
//! It positions its children at the frames computed by the layout engine and
//! paints its own decoration and text. It never measures its children: the
//! engine already did.

use std::cell::RefCell;
use std::collections::HashMap;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, graphene, gsk, pango};
use pam_desktop_ui::backend::{
    DrawCommand, ImageContent, ImageFit, TextContent, TextGeometry, VerticalAlign,
};
use pam_desktop_ui::color::Color;
use pam_desktop_ui::geometry::{Affine, Insets, Rect, Size};
use pam_desktop_ui::style::Decoration;
use pam_desktop_ui::text::SpacerMap;

use crate::{paint, text};

/// Where a parent places a child widget.
#[derive(Clone, Copy, Debug)]
pub struct ChildLayout {
    pub frame: Rect,
    pub transform: Affine,
    pub pointer_targetable: bool,
    /// The child takes exactly its frame width, even below its GTK minimum,
    /// and clips what does not fit (a field with a long placeholder).
    pub clip_width: bool,
}

thread_local! {
    /// Engine frames of every node widget, keyed by widget address. Widgets of
    /// any class (boxes, scrolled windows, entries) are positioned from here.
    static LAYOUTS: RefCell<HashMap<usize, ChildLayout>> = RefCell::new(HashMap::new());
}

fn key(widget: &impl IsA<gtk::Widget>) -> usize {
    widget.as_ref().as_ptr() as usize
}

pub fn layout_of(widget: &impl IsA<gtk::Widget>) -> Option<ChildLayout> {
    LAYOUTS.with(|layouts| layouts.borrow().get(&key(widget)).copied())
}

pub fn set_layout(widget: &impl IsA<gtk::Widget>, update: impl FnOnce(&mut ChildLayout)) {
    LAYOUTS.with(|layouts| {
        let mut layouts = layouts.borrow_mut();
        let layout = layouts.entry(key(widget)).or_insert(ChildLayout {
            frame: Rect::ZERO,
            transform: Affine::IDENTITY,
            pointer_targetable: true,
            clip_width: false,
        });
        update(layout);
    });
}

pub fn forget_layout(widget: &impl IsA<gtk::Widget>) {
    LAYOUTS.with(|layouts| {
        layouts.borrow_mut().remove(&key(widget));
    });
}

pub struct TextState {
    pub content: TextContent,
    pub layout: pango::Layout,
    /// Width the layout was last wrapped to.
    pub wrapped_to: f32,
    /// Selected range in UTF-16 units.
    pub selection: Option<(u32, u32)>,
    /// Paragraph offsets around the inline box edges of the layout text.
    pub spacers: SpacerMap,
    /// Inline box fragments for the current wrap width (layout coordinates).
    pub boxes: Vec<(Rect, Decoration)>,
    /// Vertical offset that puts the baselines where Chromium does.
    pub baseline_shift: f32,
    /// The paragraph rasterized with Chromium's text gamma.
    pub raster: crate::glyphs::Raster,
    /// Exact minus snapped top of the first line box (-0.5..0.5): Chromium
    /// snaps every line box's top on its own, so lines a fractional
    /// line-height apart land on the pixel of their exact top.
    pub line_phase: f32,
}

impl TextState {
    /// Wraps the layout to the view and returns where its top-left corner
    /// sits inside a view of `size` (snapped) laid out at `geometry` (exact):
    /// the lines are centred in the exact height and snap to whole pixels
    /// from the exact top.
    fn place(&mut self, size: Size, geometry: Option<TextGeometry>) -> (f32, f32) {
        let insets = self.content.insets;
        // The engine's exact width: the snapped frame can be a fraction wider
        // or narrower and wrap the lines differently.
        let available = geometry.map_or((size.width - insets.horizontal()).max(0.0), |exact| {
            exact.wrap_width
        });
        let (offset_y, height) =
            geometry.map_or((0.0, size.height), |exact| (exact.offset_y, exact.height));
        if self.wrapped_to != available {
            let width = if text::nowrap(&self.content.block) {
                f32::INFINITY
            } else {
                available
            };
            text::set_width(&self.layout, width);
            self.wrapped_to = available;
            self.boxes = text::box_fragments(&self.layout, &self.content.block);
        }
        let free = height - insets.vertical() - text::height(&self.layout);
        let top = insets.top
            + match self.content.vertical {
                VerticalAlign::Top => 0.0,
                // An inline box's line box overflows its content area by
                // Blink's floored half-leading above.
                VerticalAlign::Center if self.content.inline => (free / 2.0).ceil(),
                VerticalAlign::Center => (free / 2.0).max(0.0),
                VerticalAlign::Bottom => free.max(0.0),
            };
        let exact = offset_y + top;
        let snapped = snap_text_top(exact);
        self.line_phase = exact - snapped;
        (
            insets.left + text::alignment_shift(&self.content.block),
            snapped + self.baseline_shift,
        )
    }
}

pub struct ImageState {
    pub content: ImageContent,
    pub texture: gdk::Texture,
}

#[derive(Default)]
pub struct ViewState {
    pub pointer_passthrough: bool,
    pub image: Option<ImageState>,
    /// Painted above the image and below the text and the children.
    pub drawing: Vec<DrawCommand>,
    /// Exact logical size from the engine (allocations are whole pixels).
    pub size: Size,
    /// Exact geometry from the engine, when known.
    pub text_geometry: Option<TextGeometry>,
    pub decoration: Decoration,
    pub text: Option<TextState>,
    /// Reported as the widget's size request (scroll content).
    pub content_size: Option<Size>,
    /// Called when GTK gives the widget a new size (window host).
    pub on_resize: Option<Box<dyn Fn(f32, f32)>>,
    pub blur: f32,
}

thread_local! {
    /// Views with `backdrop-filter` (weakly held), so an ancestor knows to
    /// keep what it paints behind them.
    static BACKDROP_VIEWS: RefCell<Vec<glib::WeakRef<PamView>>> = const { RefCell::new(Vec::new()) };
    /// While a subtree holding a backdrop view is painted: each ancestor
    /// level and the render node of everything it painted before that subtree.
    static BEHIND: RefCell<Vec<(gtk::Widget, gsk::RenderNode)>> = const { RefCell::new(Vec::new()) };
}

fn backdrop_inside(widget: &gtk::Widget) -> bool {
    BACKDROP_VIEWS.with(|views| {
        let mut views = views.borrow_mut();
        views.retain(|view| view.upgrade().is_some());
        views
            .iter()
            .filter_map(glib::WeakRef::upgrade)
            .any(|view| view.is_ancestor(widget))
    })
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct PamView {
        pub state: RefCell<ViewState>,
        /// Content of a scroll view: textures of its rows (software rendering).
        pub rows: RefCell<Option<crate::raster::RowCache>>,
        /// The backdrop composited on the CPU (`cpu_backdrop`).
        pub backdrop: RefCell<CpuBackdrop>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PamView {
        const NAME: &'static str = "PamView";
        type Type = super::PamView;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for PamView {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for PamView {
        fn unmap(&self) {
            self.parent_unmap();
            // Hidden text keeps no paragraph texture.
            if let Some(text) = self.state.borrow_mut().text.as_mut() {
                text.raster.clear();
            }
        }

        fn contains(&self, x: f64, y: f64) -> bool {
            !self.state.borrow().pointer_passthrough && self.parent_contains(x, y)
        }

        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::ConstantSize
        }

        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let extent = self.state.borrow().content_size.map_or(0, |size| {
                let value = if orientation == gtk::Orientation::Horizontal {
                    size.width
                } else {
                    size.height
                };
                // Snapped like Chromium's scrollHeight (rounded, not ceiled):
                // 1231.11 scrolls to 431 past an 800 viewport, not 432.
                value.round().max(0.0) as i32
            });
            (extent, extent, -1, -1)
        }

        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            let widget = self.obj();
            {
                let state = self.state.borrow();
                if let Some(on_resize) = &state.on_resize {
                    on_resize(width as f32, height as f32);
                }
            }
            let mut child = widget.first_child();
            while let Some(current) = child {
                // Popovers (context menus) are their own surfaces: they are
                // presented, not allocated inside this view.
                if let Some(popover) = current.downcast_ref::<gtk::Popover>() {
                    popover.present();
                    child = current.next_sibling();
                    continue;
                }
                match layout_of(&current) {
                    Some(layout) => {
                        let frame = layout.frame;
                        // A field's control (GtkText) starts at the exact left
                        // edge, like Skia's subpixel glyph positions in a
                        // browser field; the view itself sits on whole pixels.
                        // Only a text-bearing view (a field) has a geometry.
                        let exact_x = self
                            .state
                            .borrow()
                            .text_geometry
                            .map_or(0.0, |exact| exact.offset_x);
                        let mut transform = gsk::Transform::new()
                            .translate(&graphene::Point::new(frame.x + exact_x, frame.y));
                        if !layout.transform.is_identity() {
                            let affine = layout.transform;
                            if affine.is_translation() {
                                transform = transform
                                    .translate(&graphene::Point::new(affine.tx, affine.ty));
                            } else {
                                transform = transform.matrix(&graphene::Matrix::from_2d(
                                    f64::from(affine.a),
                                    f64::from(affine.b),
                                    f64::from(affine.c),
                                    f64::from(affine.d),
                                    f64::from(affine.tx),
                                    f64::from(affine.ty),
                                ));
                            }
                        }
                        // Native children keep at least their minimum size,
                        // unless they clip their width to the frame.
                        let (minimum_width, _, _, _) =
                            current.measure(gtk::Orientation::Horizontal, -1);
                        let allocated_width = if layout.clip_width {
                            frame.width.ceil() as i32
                        } else {
                            (frame.width.ceil() as i32).max(minimum_width)
                        };
                        let (minimum_height, _, _, _) =
                            current.measure(gtk::Orientation::Vertical, allocated_width);
                        let allocated_height = (frame.height.ceil() as i32).max(minimum_height);
                        current.allocate(allocated_width, allocated_height, -1, Some(transform));
                    }
                    None => current.allocate(width, height, -1, None),
                }
                child = current.next_sibling();
            }
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            paint::note_renderer(&*widget);
            let mut state = self.state.borrow_mut();
            let size = if state.size.is_empty() {
                Size::new(widget.width() as f32, widget.height() as f32)
            } else {
                state.size
            };
            let blur = state.blur;
            if blur > 0.0 {
                // CSS blur(r) is a Gaussian with standard deviation r; GSK's radius is 2σ.
                snapshot.push_blur(f64::from(blur) * 2.0);
            }
            let backdrop_blur = state.decoration.backdrop_blur;
            // A translucent fill over a frosted backdrop is composited into
            // the blurred backdrop itself (Chromium's 8-bit blend).
            let precomposited = pam_desktop_ui::backdrop::backdrop_fill(&state.decoration);
            if backdrop_blur > 0.0 {
                paint_backdrop(&widget, snapshot, size, &state, precomposited);
            }
            let cleared;
            let decoration = if precomposited.is_some() || state.decoration.fill_composited {
                cleared = Decoration {
                    background: Color::TRANSPARENT,
                    ..state.decoration.clone()
                };
                &cleared
            } else {
                &state.decoration
            };
            let plain = decoration.is_plain();
            if !plain {
                match flattened_background(widget.upcast_ref(), decoration, size) {
                    Some((background, pieces)) if pieces.is_empty() => {
                        let decoration = Decoration {
                            background,
                            ..state.decoration.clone()
                        };
                        paint::paint_background(snapshot, size, &decoration);
                    }
                    Some((background, pieces)) => {
                        // Earlier siblings paint under part of the box: the
                        // translucent fill there, the composited one elsewhere.
                        paint::paint_background(snapshot, size, &state.decoration);
                        let flat = Decoration {
                            background,
                            radii: state.decoration.radii,
                            ..Decoration::default()
                        };
                        for piece in pieces {
                            snapshot.push_clip(&graphene::Rect::new(
                                piece.x,
                                piece.y,
                                piece.width,
                                piece.height,
                            ));
                            paint::paint_background(snapshot, size, &flat);
                            snapshot.pop();
                        }
                    }
                    None => paint::paint_background(snapshot, size, decoration),
                }
                paint::paint_border(snapshot, size, decoration);
            }
            let clip = state.decoration.clip;
            if clip {
                let [left, top, right, bottom] = state.decoration.border.widths;
                let padding_box = paint::rounded(
                    size,
                    state.decoration.radii,
                    Insets::new(left, top, right, bottom),
                );
                snapshot.push_rounded_clip(&padding_box);
            }
            if let Some(image) = &state.image {
                paint_image(snapshot, size, &state.decoration, image);
            }
            if !state.drawing.is_empty() && !size.is_empty() {
                // Cairo clips the drawing to the bounds given here.
                let context =
                    snapshot.append_cairo(&graphene::Rect::new(0.0, 0.0, size.width, size.height));
                paint::paint_drawing(
                    &context,
                    &state.drawing,
                    1.0 / f64::from(widget.scale_factor().max(1)),
                );
            }
            let geometry = state.text_geometry;
            let offset_x = geometry.map_or(0.0, |exact| exact.offset_x);
            if let Some(text_state) = state.text.as_mut() {
                let (left, top) = text_state.place(size, geometry);
                let left = left + offset_x;
                let color = crate::compat::widget_color(&*widget);
                snapshot.save();
                snapshot.translate(&graphene::Point::new(left, top));
                for (rect, decoration) in &text_state.boxes {
                    snapshot.save();
                    snapshot.translate(&graphene::Point::new(rect.x, rect.y));
                    let size = Size::new(rect.width, rect.height);
                    paint::paint_background(snapshot, size, decoration);
                    paint::paint_border(snapshot, size, decoration);
                    snapshot.restore();
                }
                if let Some(shadow) = text_state.content.block.shadow {
                    snapshot.push_shadow(&[gsk::Shadow::new(
                        paint::rgba(shadow.color),
                        shadow.offset.x,
                        shadow.offset.y,
                        shadow.radius,
                    )]);
                    append_text(&widget, snapshot, text_state, (left, top), &color);
                    snapshot.pop();
                } else {
                    append_text(&widget, snapshot, text_state, (left, top), &color);
                }
                snapshot.restore();
            }
            drop(state);
            let keeps_behind = backdrop_inside(widget.upcast_ref());
            let cached = !keeps_behind
                && self
                    .rows
                    .borrow_mut()
                    .as_mut()
                    .is_some_and(|rows| rows.snapshot_children(widget.upcast_ref(), snapshot));
            let mut behind: Vec<gsk::RenderNode> = Vec::new();
            let mut child = if cached { None } else { widget.first_child() };
            while let Some(current) = child {
                if keeps_behind && backdrop_inside(&current) {
                    // Everything painted before this subtree is what its
                    // backdrop view frosts; the subtree itself is behind a
                    // later one (a dialog stacked over another dialog).
                    let level = widget.clone().upcast::<gtk::Widget>();
                    let node = gsk::ContainerNode::new(&behind).upcast();
                    BEHIND.with(|stack| stack.borrow_mut().push((level, node)));
                    let recorder = gtk::Snapshot::new();
                    widget.snapshot_child(&current, &recorder);
                    BEHIND.with(|stack| stack.borrow_mut().pop());
                    if let Some(node) = recorder.to_node() {
                        snapshot.append_node(&node);
                        behind.push(node);
                    }
                } else if keeps_behind {
                    let recorder = gtk::Snapshot::new();
                    widget.snapshot_child(&current, &recorder);
                    if let Some(node) = recorder.to_node() {
                        snapshot.append_node(&node);
                        behind.push(node);
                    }
                } else {
                    widget.snapshot_child(&current, snapshot);
                }
                child = current.next_sibling();
            }
            if clip {
                snapshot.pop();
            }
            if blur > 0.0 {
                snapshot.pop();
            }
        }
    }
}

/// Appends the paragraph at the snapshot origin, which is `origin` in the
/// view: rasterized like Chromium when the view sits on the window with a
/// plain translation, otherwise as GSK glyphs.
fn append_text(
    widget: &PamView,
    snapshot: &gtk::Snapshot,
    text_state: &mut TextState,
    origin: (f32, f32),
    color: &gdk::RGBA,
) {
    let window_origin = widget.root().and_then(|root| {
        let transform = widget.compute_transform(root.upcast_ref::<gtk::Widget>())?;
        let (xx, yx, xy, yy, x0, y0) = transform.to_2d()?;
        let eps = 1e-6;
        ((xx - 1.0).abs() < eps && (yy - 1.0).abs() < eps && yx.abs() < eps && xy.abs() < eps)
            .then(|| graphene::Point::new(x0 as f32 + origin.0, y0 as f32 + origin.1))
    });
    let phase = f64::from(text_state.line_phase);
    let drawn = window_origin.is_some_and(|point| {
        text_state.raster.append(
            widget.upcast_ref(),
            snapshot,
            &text_state.layout,
            color,
            (f64::from(point.x()), f64::from(point.y())),
            phase,
            widget.scale_factor(),
        )
    });
    if !drawn {
        // GSK rounds each glyph baseline to the pixel grid: offsetting the
        // paragraph by the first line's phase (plus a 1/256 bias so ties go
        // down the page) keeps line 1 on its snapped pixel and puts later
        // lines on the pixel of their exact top, as Blink snaps line boxes.
        let shift = text_state.line_phase + 1.0 / 256.0;
        snapshot.save();
        snapshot.translate(&graphene::Point::new(0.0, shift));
        snapshot.append_layout(&text_state.layout, color);
        snapshot.restore();
    }
}

/// What a CPU backdrop is composited from, besides the pixels behind.
#[derive(Clone, Debug, PartialEq)]
pub struct BackdropRequest {
    fill: Color,
    blur: f32,
    boxes: Vec<pam_desktop_ui::style::BackdropBox>,
    size: Size,
    scale: i32,
}

/// A backdrop view's last CPU composite and the job refreshing it.
#[derive(Default)]
pub struct CpuBackdrop {
    /// What `texture` was made from: its request and a digest of the
    /// pixels behind.
    made: Option<(BackdropRequest, u64)>,
    texture: Option<gdk::Texture>,
    /// The latest request and what was painted behind it, for the job.
    wanted: Option<(BackdropRequest, Vec<(gtk::Widget, gsk::RenderNode)>)>,
    pending: bool,
}

/// Chromium's backdrop, exactly (`pam_desktop_ui::backdrop::composite`:
/// Skia's box blur, the fill and the dialog's shadow and fills blended in
/// 8 bits): the last composite matching `request`, if any. Each call (a
/// snapshot of the view) also schedules a job, shortly after, that renders
/// what is behind and composites it when those pixels or the request
/// changed; the view is then redrawn with it.
fn cpu_backdrop(
    widget: &PamView,
    request: &BackdropRequest,
    layers: &[(gtk::Widget, gsk::RenderNode)],
) -> Option<gdk::Texture> {
    let mut cache = widget.imp().backdrop.borrow_mut();
    cache.wanted = Some((request.clone(), layers.to_vec()));
    if !cache.pending {
        cache.pending = true;
        schedule_cpu_backdrop(widget);
    }
    cache
        .made
        .as_ref()
        .filter(|(made, _)| made == request)
        .and(cache.texture.clone())
}

fn refresh_cpu_backdrop(widget: &PamView) {
    let wanted = widget.imp().backdrop.borrow_mut().wanted.take();
    let done = |widget: &PamView| widget.imp().backdrop.borrow_mut().pending = false;
    let Some((request, layers)) = wanted else {
        return done(widget);
    };
    let scale = request.scale.max(1);
    let width = (request.size.width * scale as f32).ceil() as usize;
    let height = (request.size.height * scale as f32).ceil() as usize;
    let renderer = widget.native().and_then(|native| native.renderer());
    let snapshot = gtk::Snapshot::new();
    snapshot.scale(scale as f32, scale as f32);
    for (level, node) in &layers {
        let Some(origin) = level.compute_point(widget, &graphene::Point::new(0.0, 0.0)) else {
            continue;
        };
        snapshot.save();
        snapshot.translate(&origin);
        snapshot.append_node(node);
        snapshot.restore();
    }
    let (Some(renderer), Some(node)) = (renderer, snapshot.to_node()) else {
        return done(widget);
    };
    if width == 0 || height == 0 {
        return done(widget);
    }
    let texture = renderer.render_texture(
        &node,
        Some(&graphene::Rect::new(0.0, 0.0, width as f32, height as f32)),
    );
    let mut pixels = vec![0_u8; width * height * 4];
    // GDK's default layout: B, G, R, A, premultiplied (opaque here).
    texture.download(&mut pixels, width * 4);
    let digest = {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        pixels.hash(&mut hasher);
        hasher.finish()
    };
    let unchanged = widget
        .imp()
        .backdrop
        .borrow()
        .made
        .as_ref()
        .is_some_and(|(made, made_digest)| *made == request && *made_digest == digest);
    if unchanged {
        return done(widget);
    }
    let bgr = |color: Color| Color::rgba(color.b, color.g, color.r, color.a);
    let fill = bgr(request.fill);
    let boxes: Vec<_> = request
        .boxes
        .iter()
        .cloned()
        .map(|mut painted| {
            if let Some(shadow) = painted.shadow.as_mut() {
                shadow.color = bgr(shadow.color);
            }
            painted.fill = painted.fill.map(bgr);
            painted
        })
        .collect();
    let sigma = request.blur * scale as f32;
    // On this thread: a worker's allocator arena would keep its buffers.
    pam_desktop_ui::backdrop::composite(
        &mut pixels,
        width,
        height,
        width * 4,
        sigma,
        Some(fill),
        &boxes,
        scale as f32,
    );
    let texture = gdk::MemoryTexture::new(
        width as i32,
        height as i32,
        gdk::MemoryFormat::B8g8r8a8Premultiplied,
        &glib::Bytes::from_owned(pixels),
        width * 4,
    );
    let again = {
        let mut cache = widget.imp().backdrop.borrow_mut();
        cache.made = Some((request, digest));
        cache.texture = Some(texture.upcast());
        cache.pending = false;
        cache.wanted.is_some()
    };
    widget.queue_draw();
    if again {
        refresh_cpu_backdrop_later(widget);
    }
}

fn refresh_cpu_backdrop_later(widget: &PamView) {
    let mut cache = widget.imp().backdrop.borrow_mut();
    if cache.pending {
        return;
    }
    cache.pending = true;
    drop(cache);
    schedule_cpu_backdrop(widget);
}

/// Runs the job once the view stops changing for a moment: a dialog that
/// mounts in several updates is composited once.
fn schedule_cpu_backdrop(widget: &PamView) {
    let weak = widget.downgrade();
    glib::timeout_add_local_once(std::time::Duration::from_millis(80), move || {
        if let Some(widget) = weak.upgrade() {
            refresh_cpu_backdrop(&widget);
        }
    });
}

/// GSK colour matrix for [`skia_over`] of `fill` over any opaque colour:
/// `fill premultiplied + floor(dst × (256 − α) / 256)`. The matrix scales
/// the destination; the offset adds the premultiplied fill minus half a
/// level (plus 1/512, below the 1/256 steps of `dst × (256 − α) / 256`), so
/// the 8-bit store's round-to-nearest yields the floor. Alpha becomes 1: the
/// backdrop of a page is opaque.
fn skia_over_matrix(fill: Color) -> (graphene::Matrix, graphene::Vec4) {
    let alpha = u32::from(fill.a);
    let scale = (256 - alpha) as f32 / 256.0;
    let offset = |channel: u8| {
        let premultiplied = (u32::from(channel) * alpha + 127) / 255;
        (premultiplied as f32 - 0.5 + 1.0 / 512.0) / 255.0
    };
    let mut matrix = [0.0; 16];
    matrix[0] = scale;
    matrix[5] = scale;
    matrix[10] = scale;
    (
        graphene::Matrix::from_float(matrix),
        graphene::Vec4::new(offset(fill.r), offset(fill.g), offset(fill.b), 1.0),
    )
}

/// Paints what the ancestors drew behind this view, blurred and clipped to
/// the view, like CSS `backdrop-filter: blur()`. With `fill`, the view's
/// translucent background is composited into it in one colour-matrix pass
/// (Chromium blends each layer in 8 bits; GSK's float blend is up to a level
/// brighter, which adds up across stacked masks).
fn paint_backdrop(
    widget: &PamView,
    snapshot: &gtk::Snapshot,
    size: Size,
    state: &ViewState,
    fill: Option<Color>,
) {
    let layers = BEHIND.with(|stack| stack.borrow().clone());
    if layers.is_empty() || size.is_empty() {
        if let Some(fill) = fill {
            // Nothing recorded behind: the plain fill.
            let decoration = Decoration {
                background: fill,
                radii: state.decoration.radii,
                ..Decoration::default()
            };
            paint::paint_background(snapshot, size, &decoration);
        }
        return;
    }
    let append_layers = |snapshot: &gtk::Snapshot| {
        for (level, node) in &layers {
            let Some(origin) = level.compute_point(widget, &graphene::Point::new(0.0, 0.0)) else {
                continue;
            };
            snapshot.save();
            snapshot.translate(&origin);
            snapshot.append_node(node);
            snapshot.restore();
        }
    };
    let outline = paint::rounded(size, state.decoration.radii, Insets::ZERO);
    // The exact CPU composite only under a dialog (the topmost one).
    if state.decoration.backdrop_boxes.is_empty() {
        let mut cache = widget.imp().backdrop.borrow_mut();
        if cache.texture.is_some() {
            *cache = CpuBackdrop {
                pending: cache.pending,
                ..CpuBackdrop::default()
            };
        }
    }
    if let Some(fill) = fill.filter(|_| !state.decoration.backdrop_boxes.is_empty()) {
        let request = BackdropRequest {
            fill,
            blur: state.decoration.backdrop_blur,
            boxes: state.decoration.backdrop_boxes.clone(),
            size,
            scale: widget.scale_factor(),
        };
        if let Some(texture) = cpu_backdrop(widget, &request, &layers) {
            snapshot.push_rounded_clip(&outline);
            snapshot.append_texture(&texture, &paint::bounds(size));
            snapshot.pop();
            return;
        }
    }
    snapshot.push_rounded_clip(&outline);
    if let Some(fill) = fill {
        let (matrix, offset) = skia_over_matrix(fill);
        snapshot.push_color_matrix(&matrix, &offset);
        // GSK's blur fades out at the edges of what it blurs: the sharp
        // backdrop under it keeps the composited input opaque there.
        append_layers(snapshot);
    }
    // CSS blur(r) is a Gaussian with standard deviation r; GSK's radius is 2σ.
    snapshot.push_blur(f64::from(state.decoration.backdrop_blur) * 2.0);
    append_layers(snapshot);
    snapshot.pop();
    if fill.is_some() {
        snapshot.pop();
        // Until the CPU composite is ready: the boxes' shadows and fills,
        // in floats.
        for painted in &state.decoration.backdrop_boxes {
            let decoration = Decoration {
                background: painted.fill.unwrap_or(Color::TRANSPARENT),
                shadows: painted.shadow.into_iter().collect(),
                radii: painted.radii,
                ..Decoration::default()
            };
            if let Some(clip) = painted.clip {
                snapshot.push_clip(&graphene::Rect::new(
                    clip.x,
                    clip.y,
                    clip.width,
                    clip.height,
                ));
            }
            snapshot.save();
            snapshot.translate(&graphene::Point::new(painted.rect.x, painted.rect.y));
            paint::paint_background(snapshot, painted.rect.size(), &decoration);
            snapshot.restore();
            if painted.clip.is_some() {
                snapshot.pop();
            }
        }
    }
    snapshot.pop();
}

fn paint_image(snapshot: &gtk::Snapshot, size: Size, decoration: &Decoration, image: &ImageState) {
    let content = &image.content;
    let insets = content.insets;
    let area = Rect::new(
        insets.left,
        insets.top,
        (size.width - insets.horizontal()).max(0.0),
        (size.height - insets.vertical()).max(0.0),
    );
    let natural = Size::new(image.texture.width() as f32, image.texture.height() as f32);
    if area.is_empty() || natural.is_empty() || content.opacity <= 0.0 {
        return;
    }
    let scale = match content.fit {
        ImageFit::Cover => (area.width / natural.width).max(area.height / natural.height),
        ImageFit::Contain => (area.width / natural.width).min(area.height / natural.height),
        _ => 1.0,
    };
    let target = match content.fit {
        ImageFit::Fill => area,
        ImageFit::Repeat => Rect::new(area.x, area.y, natural.width, natural.height),
        _ => {
            let (width, height) = (natural.width * scale, natural.height * scale);
            Rect::new(
                area.x + (area.width - width) / 2.0,
                area.y + (area.height - height) / 2.0,
                width,
                height,
            )
        }
    };
    let bounds = graphene::Rect::new(target.x, target.y, target.width, target.height);
    let area_rect = graphene::Rect::new(area.x, area.y, area.width, area.height);

    // Images never paint outside their (rounded) box.
    let clip = paint::rounded(size, decoration.radii, insets);
    snapshot.push_rounded_clip(&clip);
    if content.opacity < 1.0 {
        snapshot.push_opacity(f64::from(content.opacity));
    }
    let draw = |snapshot: &gtk::Snapshot| {
        if content.fit == ImageFit::Repeat {
            snapshot.push_repeat(&area_rect, Some(&bounds));
            crate::compat::append_scaled_texture(
                snapshot,
                &image.texture,
                gsk::ScalingFilter::Linear,
                &bounds,
            );
            snapshot.pop();
        } else {
            crate::compat::append_scaled_texture(
                snapshot,
                &image.texture,
                gsk::ScalingFilter::Trilinear,
                &bounds,
            );
        }
    };
    match content.tint {
        Some(tint) if crate::compat::push_alpha_mask(snapshot) => {
            draw(snapshot);
            snapshot.pop();
            snapshot.append_color(&paint::rgba(tint), &area_rect);
            snapshot.pop();
        }
        // Without masks (GTK before 4.10), the same: the tint color with the
        // alpha of the image.
        Some(tint) => {
            let [red, green, blue, alpha] = tint.to_f32();
            let mut matrix = [0.0; 16];
            matrix[15] = alpha;
            snapshot.push_color_matrix(
                &graphene::Matrix::from_float(matrix),
                &graphene::Vec4::new(red, green, blue, 0.0),
            );
            draw(snapshot);
            snapshot.pop();
        }
        None => draw(snapshot),
    }
    if let Some(overlay) = content.overlay {
        snapshot.append_color(&paint::rgba(overlay), &area_rect);
    }
    if content.opacity < 1.0 {
        snapshot.pop();
    }
    snapshot.pop();
}

glib::wrapper! {
    pub struct PamView(ObjectSubclass<imp::PamView>)
        @extends gtk::Widget,
        @implements gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for PamView {
    fn default() -> Self {
        Self::new()
    }
}

impl PamView {
    #[must_use]
    pub fn new() -> Self {
        glib::Object::new()
    }

    pub fn with_state<T>(&self, call: impl FnOnce(&mut ViewState) -> T) -> T {
        call(&mut self.imp().state.borrow_mut())
    }

    pub fn set_text_geometry(&self, geometry: TextGeometry) {
        let changed = self.with_state(|state| {
            let changed = state.text_geometry != Some(geometry);
            state.text_geometry = Some(geometry);
            changed
        });
        if changed {
            self.queue_draw();
            if self.first_child().is_some() {
                self.queue_allocate();
            }
        }
    }

    pub fn set_size(&self, size: Size) {
        let changed = self.with_state(|state| {
            let changed = state.size != size;
            state.size = size;
            changed
        });
        if changed {
            self.queue_draw();
        }
    }

    pub fn set_decoration(&self, decoration: &Decoration) {
        self.with_state(|state| {
            state.blur = decoration.blur;
            state.decoration = decoration.clone();
        });
        let weak = self.downgrade();
        BACKDROP_VIEWS.with(|views| {
            let mut views = views.borrow_mut();
            views.retain(|view| view.upgrade().is_some_and(|view| &view != self));
            if decoration.backdrop_blur > 0.0 {
                views.push(weak);
            }
        });
        self.queue_draw();
    }

    pub fn set_text(&self, content: Option<&TextContent>) {
        let next = content.map(|content| {
            let layout = self.create_pango_layout(None);
            text::use_subpixel_positions(&layout);
            text::configure(&layout, &content.block);
            let baseline_shift = text::baseline_shift(&layout, &content.block);
            TextState {
                content: content.clone(),
                layout,
                wrapped_to: f32::NAN,
                selection: None,
                spacers: content.block.spacer_map(),
                boxes: Vec::new(),
                baseline_shift,
                raster: crate::glyphs::Raster::default(),
                line_phase: 0.0,
            }
        });
        self.with_state(|state| state.text = next);
        self.queue_draw();
    }

    /// Paints a UTF-16 range of the text as selected.
    pub fn set_text_selection(&self, range: Option<(u32, u32)>) {
        let changed = self.with_state(|state| match state.text.as_mut() {
            Some(text) if text.selection != range => {
                text.selection = range;
                text::set_selection(&text.layout, &text.content.block, range);
                true
            }
            _ => false,
        });
        if changed {
            self.queue_draw();
        }
    }

    /// UTF-16 offset of the character boundary nearest to a point of the view.
    pub fn text_index_at(&self, x: f32, y: f32) -> Option<u32> {
        let fallback = Size::new(self.width() as f32, self.height() as f32);
        self.with_state(|state| {
            let size = if state.size.is_empty() {
                fallback
            } else {
                state.size
            };
            let geometry = state.text_geometry;
            let offset_x = geometry.map_or(0.0, |exact| exact.offset_x);
            let text = state.text.as_mut()?;
            let (left, top) = text.place(size, geometry);
            let left = left + offset_x;
            Some(text::index_at(
                &text.layout,
                &text.spacers,
                x - left,
                y - top,
            ))
        })
    }

    pub fn set_image(&self, image: Option<ImageState>) {
        self.with_state(|state| state.image = image);
        self.queue_draw();
    }

    pub fn set_drawing(&self, commands: &[DrawCommand]) {
        self.with_state(|state| {
            state.drawing.clear();
            state.drawing.extend_from_slice(commands);
        });
        self.queue_draw();
    }

    /// Makes this view the content of `scrolled`: with GTK's software
    /// renderer, its rows are drawn from textures while it scrolls (see
    /// `raster`).
    pub fn cache_rows(&self, scrolled: &gtk::ScrolledWindow) {
        self.imp()
            .rows
            .replace(Some(crate::raster::RowCache::default()));
        for adjustment in [scrolled.hadjustment(), scrolled.vadjustment()] {
            let weak = self.downgrade();
            adjustment.connect_value_changed(move |_| {
                if let Some(view) = weak.upgrade() {
                    crate::raster::keep_pace(view.upcast_ref());
                    view.scrolled();
                }
            });
            // A viewport that changes size while rows are cached shows
            // other rows: the content is snapshotted again.
            let weak = self.downgrade();
            adjustment.connect_changed(move |_| {
                if let Some(view) = weak.upgrade()
                    && view
                        .imp()
                        .rows
                        .borrow()
                        .as_ref()
                        .is_some_and(crate::raster::RowCache::active)
                {
                    view.queue_draw();
                }
            });
        }
    }

    /// The scroll view this view is the content of moved: with GTK's
    /// software renderer its rows are drawn from images until a second
    /// after the scroll ends, then directly again and the images go.
    fn scrolled(&self) {
        if !crate::raster::software_rendered(self) {
            return;
        }
        let arm = {
            let mut rows = self.imp().rows.borrow_mut();
            let Some(rows) = rows.as_mut() else {
                return;
            };
            rows.scrolled()
        };
        // Rows are placed on whole pixels and rasterized as they come on
        // screen: the content is snapshotted again at each step.
        self.queue_draw();
        if arm {
            self.release_rows(crate::raster::ROWS_KEPT);
        }
    }

    fn release_rows(&self, delay: std::time::Duration) {
        let weak = self.downgrade();
        glib::timeout_add_local_once(delay, move || {
            let Some(view) = weak.upgrade() else {
                return;
            };
            let remaining = view
                .imp()
                .rows
                .borrow_mut()
                .as_mut()
                .and_then(crate::raster::RowCache::release);
            match remaining {
                Some(remaining) => view.release_rows(remaining),
                None => view.queue_draw(),
            }
        });
    }

    pub fn set_content_size(&self, size: Option<Size>) {
        let changed = self.with_state(|state| {
            let changed = state.content_size != size;
            state.content_size = size;
            changed
        });
        if changed {
            self.queue_resize();
        }
    }
}

/// The whole pixel Chromium draws a line box's text from: its top rounded
/// half up (down the page), with or without a `will-change` layer. Measured
/// in Electron 37 for every 1/64 fraction (units/probe: 4 fonts x 2 line
/// heights x 2 layer modes x 64 tops, baseline = round-half-up(top) + const).
fn snap_text_top(top: f32) -> f32 {
    (top + 0.5).floor()
}

/// Registry check run from the GTK test thread (GTK allows one thread).
#[cfg(test)]
pub(crate) fn assert_backdrop_registry() {
    use pam_desktop_ui::style::Decoration;
    let root = PamView::new();
    let layer = PamView::new();
    let backdrop = PamView::new();
    layer.set_parent(&root);
    backdrop.set_parent(&layer);
    assert!(!backdrop_inside(root.upcast_ref()));
    let decoration = Decoration {
        backdrop_blur: 8.0,
        ..Decoration::default()
    };
    backdrop.set_decoration(&decoration);
    assert!(backdrop_inside(root.upcast_ref()));
    assert!(backdrop_inside(layer.upcast_ref()));
    backdrop.set_decoration(&Decoration::default());
    assert!(!backdrop_inside(root.upcast_ref()));
    backdrop.unparent();
    layer.unparent();
}

/// Chromium's 8-bit blend of a translucent fill (`rgba(29,41,61,.4)` over
/// `#0C1117` is `#131A25`; GSK blends in floats and lands one level off per
/// channel). Where only a known opaque colour lies under a translucent box,
/// it is painted already composited.
pub use pam_desktop_ui::backdrop::skia_over;

/// At most this many clip rectangles for a partly flattened fill; more
/// overlaps paint the plain translucent fill.
const MAX_FLAT_PIECES: usize = 8;

/// The opaque fill to paint instead of a translucent background, when what
/// lies under the box is one known opaque colour: the parent's (or an
/// ancestor's through transparent boxes) plain fill, with no border or
/// rounded corner of those ancestors under the box. Earlier siblings (and
/// their outset shadows) that paint under part of the box leave that part
/// translucent: the second value lists the box-local rectangles to paint
/// flattened (empty: the whole box).
fn flattened_background(
    widget: &gtk::Widget,
    decoration: &Decoration,
    size: Size,
) -> Option<(Color, Vec<Rect>)> {
    let background = decoration.background;
    if background.a == 0
        || background.a == 255
        || !decoration.gradients.is_empty()
        || decoration.backdrop_blur > 0.0
        || decoration.blur > 0.0
        || widget.opacity() < 1.0
    {
        return None;
    }
    let target = Rect::new(0.0, 0.0, size.width, size.height);
    let mut blocked = Vec::new();
    let under = opaque_under(widget, target, Some(target), &mut blocked, 0)?;
    let pieces = if blocked.is_empty() {
        Vec::new()
    } else {
        let pieces = subtract(target, &blocked);
        if pieces.is_empty() || pieces.len() > MAX_FLAT_PIECES {
            return None;
        }
        pieces
    };
    Some((skia_over(background, under), pieces))
}

/// `area` minus the union of `holes`, as disjoint rectangles; the holes are
/// first grown to whole pixels so no seam is left at their edges.
fn subtract(area: Rect, holes: &[Rect]) -> Vec<Rect> {
    let mut pieces = vec![area];
    for hole in holes {
        let x = hole.x.floor();
        let y = hole.y.floor();
        let hole = Rect::new(x, y, hole.right().ceil() - x, hole.bottom().ceil() - y);
        let mut next = Vec::with_capacity(pieces.len() + 3);
        for piece in pieces {
            let Some(cut) = piece.intersect(hole) else {
                next.push(piece);
                continue;
            };
            // Bands above and below the hole, then its left and right sides.
            let bands = [
                Rect::new(piece.x, piece.y, piece.width, cut.y - piece.y),
                Rect::new(
                    piece.x,
                    cut.bottom(),
                    piece.width,
                    piece.bottom() - cut.bottom(),
                ),
                Rect::new(piece.x, cut.y, cut.x - piece.x, cut.height),
                Rect::new(cut.right(), cut.y, piece.right() - cut.right(), cut.height),
            ];
            next.extend(
                bands
                    .into_iter()
                    .filter(|band| band.width > 0.0 && band.height > 0.0),
            );
        }
        pieces = next;
        if pieces.len() > MAX_FLAT_PIECES * 2 {
            break;
        }
    }
    pieces
}

/// The opaque colour under `rect` (widget-local). `target` is the box being
/// flattened in the same coordinates: earlier siblings painting inside it are
/// pushed to `blocked` (box-local) instead of failing; with no target (lost
/// in a scroller) any overlap fails.
fn opaque_under(
    widget: &gtk::Widget,
    rect: Rect,
    target: Option<Rect>,
    blocked: &mut Vec<Rect>,
    depth: u32,
) -> Option<Color> {
    if depth > 32 {
        return None;
    }
    // Scroll content sits in a viewport of a scrolled window: what lies under
    // it is what lies under the whole scrolled window.
    if let Some(wrapper) = widget
        .parent()
        .filter(|parent| parent.is::<gtk::Viewport>() || parent.is::<gtk::ScrolledWindow>())
    {
        let scrolled = if wrapper.is::<gtk::Viewport>() {
            wrapper
                .parent()
                .filter(|outer| outer.is::<gtk::ScrolledWindow>())?
        } else {
            wrapper
        };
        let full = Rect::new(0.0, 0.0, scrolled.width() as f32, scrolled.height() as f32);
        return opaque_under(&scrolled, full, None, blocked, depth + 1);
    }
    let layout = layout_of(widget)?;
    if !layout.transform.is_identity() {
        return None;
    }
    let rect = Rect::new(
        rect.x + layout.frame.x,
        rect.y + layout.frame.y,
        rect.width,
        rect.height,
    );
    // The flattened box in the parent's coordinates.
    let target = target.map(|target| {
        Rect::new(
            target.x + layout.frame.x,
            target.y + layout.frame.y,
            target.width,
            target.height,
        )
    });
    let parent = widget.parent()?.downcast::<PamView>().ok()?;
    let overlaps = |other: Rect| {
        other.x < rect.right()
            && rect.x < other.right()
            && other.y < rect.bottom()
            && rect.y < other.bottom()
    };
    let mut child = parent.first_child();
    while let Some(current) = child {
        if &current == widget {
            break;
        }
        if current.is_visible() {
            let frame = layout_of(&current)?.frame;
            let painted = current.downcast_ref::<PamView>().map_or(frame, |view| {
                view.with_state(|state| {
                    state
                        .decoration
                        .shadows
                        .iter()
                        .filter(|shadow| !shadow.inset)
                        .fold(frame, |area, shadow| {
                            let reach = shadow.bounds(frame);
                            let x = area.x.min(reach.x);
                            let y = area.y.min(reach.y);
                            Rect::new(
                                x,
                                y,
                                area.right().max(reach.right()) - x,
                                area.bottom().max(reach.bottom()) - y,
                            )
                        })
                })
            });
            if overlaps(painted) {
                // Only what it paints inside the box matters; that part keeps
                // the translucent fill (box-local coordinates).
                let inside =
                    target.and_then(|target| painted.intersect(target).map(|cut| (target, cut)));
                match (target, inside) {
                    (None, _) => return None,
                    (Some(_), None) => {}
                    (Some(_), Some((target, cut))) => {
                        if blocked.len() >= MAX_FLAT_PIECES {
                            return None;
                        }
                        blocked.push(Rect::new(
                            cut.x - target.x,
                            cut.y - target.y,
                            cut.width,
                            cut.height,
                        ));
                    }
                }
            }
        }
        child = current.next_sibling();
    }
    let (decoration, size, plain_content) = parent.with_state(|state| {
        let size = if state.size.is_empty() {
            Size::new(parent.width() as f32, parent.height() as f32)
        } else {
            state.size
        };
        (
            state.decoration.clone(),
            size,
            state.image.is_none() && state.drawing.is_empty() && state.text.is_none(),
        )
    });
    if !plain_content
        || !decoration.gradients.is_empty()
        || decoration.backdrop_blur > 0.0
        || decoration.blur > 0.0
        || decoration.shadows.iter().any(|shadow| shadow.inset)
    {
        return None;
    }
    // Inside the border and clear of the rounded corners.
    let [left, top, right, bottom] = decoration.border.widths;
    let radius = decoration
        .radii
        .top_left
        .max(decoration.radii.top_right)
        .max(decoration.radii.bottom_left)
        .max(decoration.radii.bottom_right);
    let inside = rect.x >= left
        && rect.y >= top
        && rect.right() <= size.width - right
        && rect.bottom() <= size.height - bottom;
    let clear_of_corners = radius <= 0.0
        || ((rect.x >= radius || rect.y >= radius)
            && (rect.right() <= size.width - radius || rect.y >= radius)
            && (rect.x >= radius || rect.bottom() <= size.height - radius)
            && (rect.right() <= size.width - radius || rect.bottom() <= size.height - radius));
    if !inside || !clear_of_corners {
        return None;
    }
    match decoration.background.a {
        255 => Some(decoration.background),
        0 => {
            if parent.opacity() < 1.0 {
                return None;
            }
            opaque_under(parent.upcast_ref(), rect, target, blocked, depth + 1)
        }
        _ => {
            if parent.opacity() < 1.0 {
                return None;
            }
            opaque_under(
                parent.upcast_ref(),
                Rect::new(0.0, 0.0, size.width, size.height),
                target,
                blocked,
                depth + 1,
            )
            .map(|under| skia_over(decoration.background, under))
        }
    }
}

#[cfg(test)]
mod blend_tests {
    use super::*;

    #[test]
    fn text_tops_snap_half_up_like_chromium() {
        // Electron 37: a top at k/64 draws from round-half-up(top) for all k,
        // the .5 tie included (790.5 -> 791).
        for k in 0..64_u8 {
            let top = 790.0 + f32::from(k) / 64.0;
            let chromium = if k >= 32 { 791.0 } else { 790.0 };
            assert_eq!(snap_text_top(top), chromium, "top {top}");
        }
        assert_eq!(snap_text_top(-0.5), 0.0);
    }

    #[test]
    fn flattening_skips_what_earlier_siblings_paint() {
        let area = Rect::new(0.0, 0.0, 100.0, 50.0);
        // A shadow reaching 10.4px into the top of the box: rows 0..11 stay translucent.
        let pieces = subtract(area, &[Rect::new(-20.0, -30.0, 140.0, 40.4)]);
        assert_eq!(pieces, vec![Rect::new(0.0, 11.0, 100.0, 39.0)]);
        // A hole in the middle leaves four disjoint bands covering the rest.
        let pieces = subtract(area, &[Rect::new(40.0, 10.0, 20.0, 20.0)]);
        assert_eq!(pieces.len(), 4);
        let covered: f32 = pieces.iter().map(|piece| piece.width * piece.height).sum();
        assert!((covered - (5000.0 - 400.0)).abs() < 1e-3);
        assert!(subtract(area, &[Rect::new(-1.0, -1.0, 102.0, 52.0)]).is_empty());
    }

    #[test]
    fn backdrop_matrix_rounds_to_the_skia_blend() {
        // Electron 37 (harness, Linux and macOS): a slate-950/70 mask with
        // blur(8px) over v reads (1,4,17) + floor(v × 77 / 256); stacked
        // masks repeat it on the previous 8-bit result.
        for fill in [
            Color::rgba(2, 6, 24, 179),
            Color::rgba(0, 0, 0, 184),
            Color::rgba(255, 128, 7, 1),
            Color::rgba(250, 250, 250, 254),
        ] {
            let (matrix, offset) = skia_over_matrix(fill);
            let scale = matrix.value(0, 0);
            let offsets = [offset.x(), offset.y(), offset.z()];
            for under in 0..=255_u8 {
                let expected = skia_over(fill, Color::rgba(under, under, under, 255));
                let expected = [expected.r, expected.g, expected.b];
                for (channel, offset) in offsets.iter().enumerate() {
                    let stored = (((f32::from(under) / 255.0) * scale + offset) * 255.0)
                        .round()
                        .clamp(0.0, 255.0) as u8;
                    assert_eq!(stored, expected[channel], "{fill:?} over {under}");
                }
            }
            assert_eq!(offset.w(), 1.0);
        }
        assert_eq!(
            skia_over(Color::rgba(2, 6, 24, 179), Color::rgba(0, 0, 0, 255)),
            Color::rgba(1, 4, 17, 255)
        );
    }

    #[test]
    fn translucent_fills_composite_like_skia() {
        let page = Color {
            r: 0x0C,
            g: 0x11,
            b: 0x17,
            a: 255,
        };
        // rgba(29,41,61,.4) over the page, then rgba(15,23,43,.5) over that:
        // the Electron captures read #131A25 and #111928.
        let card = skia_over(
            Color {
                r: 29,
                g: 41,
                b: 61,
                a: 102,
            },
            page,
        );
        assert_eq!((card.r, card.g, card.b), (0x13, 0x1A, 0x25));
        let field = skia_over(
            Color {
                r: 15,
                g: 23,
                b: 43,
                a: 128,
            },
            card,
        );
        assert_eq!((field.r, field.g, field.b), (0x11, 0x19, 0x28));
    }
}

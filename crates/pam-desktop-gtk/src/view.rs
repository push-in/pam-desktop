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
use pam_desktop_ui::backend::{DrawCommand, ImageContent, ImageFit, TextContent, VerticalAlign};
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
}

impl TextState {
    /// Wraps the layout to the view and returns where its top-left corner
    /// sits inside a view of `size`.
    fn place(&mut self, size: Size) -> (f32, f32) {
        let insets = self.content.insets;
        let available = (size.width - insets.horizontal()).max(0.0);
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
        let free = size.height - insets.vertical() - text::height(&self.layout);
        let top = insets.top
            + match self.content.vertical {
                VerticalAlign::Top => 0.0,
                VerticalAlign::Center => (free / 2.0).max(0.0),
                VerticalAlign::Bottom => free.max(0.0),
            };
        (insets.left, top.round())
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
                value.ceil().max(0.0) as i32
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
                        let mut transform = gsk::Transform::new()
                            .translate(&graphene::Point::new(frame.x, frame.y));
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
                        // Native children keep at least their minimum size.
                        let (minimum_width, _, _, _) =
                            current.measure(gtk::Orientation::Horizontal, -1);
                        let allocated_width = (frame.width.ceil() as i32).max(minimum_width);
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
            let mut state = self.state.borrow_mut();
            let size = if state.size.is_empty() {
                Size::new(widget.width() as f32, widget.height() as f32)
            } else {
                state.size
            };
            let blur = state.blur;
            if blur > 0.0 {
                snapshot.push_blur(f64::from(blur));
            }
            let backdrop_blur = state.decoration.backdrop_blur;
            if backdrop_blur > 0.0 {
                paint_backdrop(&widget, snapshot, size, &state);
            }
            let plain = state.decoration.is_plain();
            if !plain {
                paint::paint_background(snapshot, size, &state.decoration);
                paint::paint_border(snapshot, size, &state.decoration);
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
            if let Some(text_state) = state.text.as_mut() {
                let (left, top) = text_state.place(size);
                let color = widget.color();
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
                    snapshot.append_layout(&text_state.layout, &color);
                    snapshot.pop();
                } else {
                    snapshot.append_layout(&text_state.layout, &color);
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
                    // backdrop view frosts.
                    let level = widget.clone().upcast::<gtk::Widget>();
                    let node = gsk::ContainerNode::new(&behind).upcast();
                    BEHIND.with(|stack| stack.borrow_mut().push((level, node)));
                    widget.snapshot_child(&current, snapshot);
                    BEHIND.with(|stack| stack.borrow_mut().pop());
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

/// Paints what the ancestors drew behind this view, blurred and clipped to
/// the view, like CSS `backdrop-filter: blur()`.
fn paint_backdrop(widget: &PamView, snapshot: &gtk::Snapshot, size: Size, state: &ViewState) {
    let layers = BEHIND.with(|stack| stack.borrow().clone());
    if layers.is_empty() || size.is_empty() {
        return;
    }
    let outline = paint::rounded(size, state.decoration.radii, Insets::ZERO);
    snapshot.push_rounded_clip(&outline);
    // CSS blur(r) is a Gaussian with standard deviation r; GSK's radius is 2σ.
    snapshot.push_blur(f64::from(state.decoration.backdrop_blur) * 2.0);
    for (level, node) in &layers {
        let Some(origin) = level.compute_point(widget, &graphene::Point::new(0.0, 0.0)) else {
            continue;
        };
        snapshot.save();
        snapshot.translate(&origin);
        snapshot.append_node(node);
        snapshot.restore();
    }
    snapshot.pop();
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
            snapshot.append_scaled_texture(&image.texture, gsk::ScalingFilter::Linear, &bounds);
            snapshot.pop();
        } else {
            snapshot.append_scaled_texture(&image.texture, gsk::ScalingFilter::Trilinear, &bounds);
        }
    };
    match content.tint {
        Some(tint) => {
            snapshot.push_mask(gsk::MaskMode::Alpha);
            draw(snapshot);
            snapshot.pop();
            snapshot.append_color(&paint::rgba(tint), &area_rect);
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
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
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
            TextState {
                content: content.clone(),
                layout,
                wrapped_to: f32::NAN,
                selection: None,
                spacers: content.block.spacer_map(),
                boxes: Vec::new(),
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
            let text = state.text.as_mut()?;
            let (left, top) = text.place(size);
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

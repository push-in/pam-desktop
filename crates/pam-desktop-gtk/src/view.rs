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
use pam_desktop_ui::backend::{ImageContent, ImageFit, TextContent, VerticalAlign};
use pam_desktop_ui::geometry::{Affine, Insets, Rect, Size};
use pam_desktop_ui::style::Decoration;

use crate::{paint, text};

/// Where a parent places a child widget.
#[derive(Clone, Copy, Debug)]
pub struct ChildLayout {
    pub frame: Rect,
    pub transform: Affine,
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
        let layout = layouts.entry(key(widget)).or_insert(ChildLayout { frame: Rect::ZERO, transform: Affine::IDENTITY });
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
}

pub struct ImageState {
    pub content: ImageContent,
    pub texture: gdk::Texture,
}

#[derive(Default)]
pub struct ViewState {
    pub image: Option<ImageState>,
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

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct PamView {
        pub state: RefCell<ViewState>,
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
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::ConstantSize
        }

        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let extent = self.state.borrow().content_size.map_or(0, |size| {
                let value = if orientation == gtk::Orientation::Horizontal { size.width } else { size.height };
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
                        let mut transform = gsk::Transform::new().translate(&graphene::Point::new(frame.x, frame.y));
                        if !layout.transform.is_identity() {
                            let affine = layout.transform;
                            if affine.is_translation() {
                                transform = transform.translate(&graphene::Point::new(affine.tx, affine.ty));
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
                        let (minimum_width, _, _, _) = current.measure(gtk::Orientation::Horizontal, -1);
                        let allocated_width = (frame.width.ceil() as i32).max(minimum_width);
                        let (minimum_height, _, _, _) = current.measure(gtk::Orientation::Vertical, allocated_width);
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
            let plain = state.decoration.is_plain();
            if !plain {
                paint::paint_background(snapshot, size, &state.decoration);
                paint::paint_border(snapshot, size, &state.decoration);
            }
            let clip = state.decoration.clip;
            if clip {
                let [left, top, right, bottom] = state.decoration.border.widths;
                let padding_box = paint::rounded(size, state.decoration.radii, Insets::new(left, top, right, bottom));
                snapshot.push_rounded_clip(&padding_box);
            }
            if let Some(image) = &state.image {
                paint_image(snapshot, size, &state.decoration, image);
            }
            if let Some(text_state) = state.text.as_mut() {
                let insets = text_state.content.insets;
                let available = (size.width - insets.horizontal()).max(0.0);
                if text_state.wrapped_to != available {
                    text::set_width(&text_state.layout, available);
                    text_state.wrapped_to = available;
                }
                let text_height = text::height(&text_state.layout);
                let free = size.height - insets.vertical() - text_height;
                let top = insets.top
                    + match text_state.content.vertical {
                        VerticalAlign::Top => 0.0,
                        VerticalAlign::Center => (free / 2.0).max(0.0),
                        VerticalAlign::Bottom => free.max(0.0),
                    };
                let color = widget.color();
                snapshot.save();
                snapshot.translate(&graphene::Point::new(insets.left, top.round()));
                if let Some(shadow) = text_state.content.block.shadow {
                    snapshot.push_shadow(&[gsk::Shadow::new(paint::rgba(shadow.color), shadow.offset.x, shadow.offset.y, shadow.radius)]);
                    snapshot.append_layout(&text_state.layout, &color);
                    snapshot.pop();
                } else {
                    snapshot.append_layout(&text_state.layout, &color);
                }
                snapshot.restore();
            }
            drop(state);
            let mut child = widget.first_child();
            while let Some(current) = child {
                widget.snapshot_child(&current, snapshot);
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
            Rect::new(area.x + (area.width - width) / 2.0, area.y + (area.height - height) / 2.0, width, height)
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
        self.queue_draw();
    }

    pub fn set_text(&self, content: Option<&TextContent>) {
        let next = content.map(|content| {
            let layout = self.create_pango_layout(None);
            text::configure(&layout, &content.block);
            TextState { content: content.clone(), layout, wrapped_to: f32::NAN }
        });
        self.with_state(|state| state.text = next);
        self.queue_draw();
    }

    pub fn set_image(&self, image: Option<ImageState>) {
        self.with_state(|state| state.image = image);
        self.queue_draw();
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

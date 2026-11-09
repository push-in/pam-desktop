//! Draws a window: the retained tree is walked back to front on the
//! Direct2D device context and the swap chain presents the result.
//!
//! A frame is only rendered when something changed (`WindowEntry::damage`),
//! and at most once per `WM_PAINT`.

use pam_desktop_ui::backend::{NodeId, ViewClass, WindowId};
use pam_desktop_ui::geometry::{Insets, Point, Rect, Size};
use windows::Win32::Foundation::{D2DERR_RECREATE_TARGET, RECT};
use windows::Win32::Graphics::Direct2D::Common::{D2D_RECT_F, D2D1_COMPOSITE_MODE_SOURCE_OVER};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT,
    D2D1_INTERPOLATION_MODE_LINEAR, D2D1_ROUNDED_RECT, ID2D1DeviceContext, ID2D1Image,
};
use windows::Win32::Graphics::Dxgi::{
    DXGI_ERROR_DEVICE_REMOVED, DXGI_ERROR_DEVICE_RESET, DXGI_PRESENT, DXGI_PRESENT_PARAMETERS,
};
use windows::core::Interface;
use windows_numerics::Matrix3x2;

use crate::app::{State, Theme};
use crate::gfx::{Graphics, Surface};
use crate::images::Images;
use crate::paint::{self, INFINITE, PaintCache, PaintEnv, ShadowCache};
use crate::scene::Scene;
use crate::text::{TextEngine, text_origin};
use crate::util::{color_f, pixel_rect, rect_f, vec2};
use crate::window::Damage;

struct Painter<'a> {
    scene: &'a mut Scene,
    graphics: &'a mut Graphics,
    text: &'a TextEngine,
    images: &'a Images,
    shadows: &'a mut ShadowCache,
    theme: Theme,
    scale: f32,
    /// Nesting depth of offscreen passes.
    depth: usize,
    /// Reused for selection and background rectangles.
    rects: Vec<Rect>,
    /// Something painted needs the next vertical blank (a playing video).
    wants_tick: bool,
    /// Layers open on the window's context: what is painted inside one is
    /// not on the target until the layer is popped.
    layers: usize,
    /// A backdrop was blurred in this frame.
    blurred: bool,
    /// The view whose translucent background its backdrop just painted.
    filled: Option<NodeId>,
    /// Flushing the frame before a backdrop was copied failed: the frame
    /// is lost, as if `EndDraw` had failed with this error.
    failed: Option<windows::core::Error>,
}

impl Painter<'_> {
    /// Paints a view with its subtree. `visible` is what clipping ancestors
    /// leave to see, in the coordinates of the view's parent content.
    fn paint_view(
        &mut self,
        context: &ID2D1DeviceContext,
        id: NodeId,
        level: usize,
        visible: Option<Rect>,
    ) {
        let Some(view) = self.scene.views.get(&id) else {
            return;
        };
        if !view.layer.visible || view.layer.opacity <= 0.001 || level > 512 {
            return;
        }
        let opacity = view.layer.opacity.min(1.0);
        let filtered = view.decoration.blur > 0.0 || view.decoration.color_matrix.is_some();
        let frosted = view.decoration.backdrop_blur > 0.0;
        let placed = view.to_parent();
        // The visible area in the view's own coordinates (unknown when the
        // transform cannot be inverted: then nothing is culled below).
        let visible =
            visible.and_then(|visible| placed.invert().map(|inverse| inverse.bounds(visible)));
        let saved = paint::concat_transform(context, &placed);
        if frosted && !filtered {
            self.paint_backdrop(context, id, opacity);
        }
        if filtered {
            self.paint_filtered(context, id, level, opacity);
        } else if opacity < 0.999 {
            // The layer only needs to cover what the subtree can paint.
            let bounds = self.scene.subtree_bounds(id, 0, true).outset(8.0);
            paint::push_layer(context, rect_f(bounds), None, opacity);
            self.layers += 1;
            self.paint_content(context, id, level, visible);
            self.layers -= 1;
            // SAFETY: pops the layer pushed above.
            unsafe { context.PopLayer() };
        } else {
            self.paint_content(context, id, level, visible);
        }
        // SAFETY: restores the transform saved above.
        unsafe { context.SetTransform(&saved) };
    }

    /// `backdropBlur`: what is already painted behind the view is copied,
    /// blurred and drawn back inside the view's outline, before the view
    /// paints itself. The copy needs the pixels painted so far: only the
    /// window's own target has them, and not those still inside an open
    /// layer. Anywhere else, and when Direct2D refuses a step, the view is
    /// painted without the blur.
    fn paint_backdrop(&mut self, context: &ID2D1DeviceContext, id: NodeId, opacity: f32) {
        if self.depth != 0
            || self.layers != 0
            || self.graphics.backdrop_failed
            || !backdrop_blur_enabled()
        {
            return;
        }
        let factory = self.graphics.factory.clone();
        let generation = self.graphics.generation;
        let Some(view) = self.scene.views.get_mut(&id) else {
            return;
        };
        let size = view.frame.size();
        // An opaque background would cover the blur.
        if size.is_empty() || view.decoration.background.is_opaque() {
            return;
        }
        let Some(device) = self.graphics.device.as_mut() else {
            return;
        };
        let cache = view
            .cache
            .get_or_insert_with(|| Box::new(PaintCache::default()));
        // Direct2D batches: what was drawn so far must reach the target
        // before it is copied.
        // SAFETY: plain call inside the context's BeginDraw.
        if let Err(error) = unsafe { context.Flush(None, None) } {
            self.failed.get_or_insert(error);
            // The frame is lost, but it was lost to the blur: counting it
            // as blurred lets the window paint again without backdrop
            // blurs, instead of failing on the same flush every frame
            // (Direct2D implementations without `Flush` never show a
            // modal with a backdrop blur otherwise).
            self.blurred = true;
            return;
        }
        // CSS `blur(r)`: a Gaussian with standard deviation r.
        let sigma = view.decoration.backdrop_blur;
        let fill = pam_desktop_ui::backdrop::backdrop_fill(&view.decoration);
        let Some((image, origin)) =
            paint::backdrop_image(context, device, cache, generation, size, sigma, fill)
        else {
            return;
        };
        if fill.is_some() {
            self.filled = Some(id);
        }
        let outline = paint::rounded(&factory, size, view.decoration.radii, Insets::ZERO);
        let clip = paint::push_clip(context, &factory, &outline);
        if opacity < 0.999 {
            paint::push_layer(context, rect_f(Rect::from_size(size)), None, opacity);
        }
        let offset = vec2(origin.x, origin.y);
        // SAFETY: COM calls with live objects inside the context's
        // BeginDraw; the transform is restored and the layer pushed above
        // is popped.
        unsafe {
            let mut transform = Matrix3x2::identity();
            context.GetTransform(&mut transform);
            // The image is in the target's own coordinates.
            context.SetTransform(&Matrix3x2::identity());
            context.DrawImage(
                &image,
                Some(std::ptr::from_ref(&offset)),
                None,
                D2D1_INTERPOLATION_MODE_LINEAR,
                D2D1_COMPOSITE_MODE_SOURCE_OVER,
            );
            context.SetTransform(&transform);
            if opacity < 0.999 {
                context.PopLayer();
            }
        }
        paint::pop_clip(context, clip);
        self.blurred = true;
    }

    /// Blur and color matrix apply to the view with its children: the
    /// subtree is recorded and drawn through the effects.
    fn paint_filtered(
        &mut self,
        context: &ID2D1DeviceContext,
        id: NodeId,
        level: usize,
        opacity: f32,
    ) {
        let Some(view) = self.scene.views.get(&id) else {
            return;
        };
        let (blur, color_matrix) = (view.decoration.blur, view.decoration.color_matrix);
        let dpi = 96.0 * self.scale;
        let Some(device) = self.graphics.device.as_mut() else {
            return;
        };
        let Ok(recorder) = device.offscreen(self.depth) else {
            return;
        };
        // SAFETY: COM calls on live objects. The recorder belongs to this
        // nesting depth, so it is not inside another BeginDraw.
        unsafe {
            let Ok(list) = recorder.CreateCommandList() else {
                return;
            };
            let mut transform = Matrix3x2::identity();
            context.GetTransform(&mut transform);
            recorder.SetDpi(dpi, dpi);
            recorder.SetTarget(&list);
            recorder.BeginDraw();
            recorder.SetTransform(&transform);
            self.depth += 1;
            // Filters spread paint: the whole subtree is recorded.
            self.paint_content(&recorder, id, level, None);
            self.depth -= 1;
            let drawn = recorder.EndDraw(None, None);
            recorder.SetTarget(None::<&ID2D1Image>);
            if drawn.is_err() || list.Close().is_err() {
                return;
            }
            let Ok(recorded) = list.cast::<ID2D1Image>() else {
                return;
            };
            let Some(image) = paint::filter_image(context, &recorded, blur, color_matrix.as_ref())
            else {
                return;
            };
            // The recording already holds window coordinates.
            context.SetTransform(&Matrix3x2::identity());
            if opacity < 0.999 {
                paint::push_layer(context, INFINITE, None, opacity);
            }
            context.DrawImage(
                &image,
                None,
                None,
                D2D1_INTERPOLATION_MODE_LINEAR,
                D2D1_COMPOSITE_MODE_SOURCE_OVER,
            );
            if opacity < 0.999 {
                context.PopLayer();
            }
            context.SetTransform(&transform);
        }
    }

    fn paint_content(
        &mut self,
        context: &ID2D1DeviceContext,
        id: NodeId,
        level: usize,
        visible: Option<Rect>,
    ) {
        let factory = self.graphics.factory.clone();
        let generation = self.graphics.generation;
        let Some(view) = self.scene.views.get_mut(&id) else {
            return;
        };
        let size = view.frame.size();
        let class = view.class;
        // Its backdrop already composited the translucent background.
        let cleared;
        let decoration = if self.filled == Some(id) {
            cleared = pam_desktop_ui::style::Decoration {
                background: pam_desktop_ui::color::Color::TRANSPARENT,
                ..view.decoration.clone()
            };
            &cleared
        } else {
            &view.decoration
        };

        if !decoration.is_plain() {
            let cache = view
                .cache
                .get_or_insert_with(|| Box::new(PaintCache::default()));
            let mut env = PaintEnv {
                graphics: &mut *self.graphics,
                shadows: &mut *self.shadows,
                scale: self.scale,
                depth: self.depth,
            };
            paint::paint_decoration(context, &mut env, cache, size, decoration);
        }

        // Children, images and text never leave a clipping box.
        let clip = if view.decoration.clip {
            let [left, top, right, bottom] = view.decoration.border.widths;
            let padding_box = paint::rounded(
                &factory,
                size,
                view.decoration.radii,
                Insets::new(left, top, right, bottom),
            );
            Some(paint::push_clip(context, &factory, &padding_box))
        } else if class == ViewClass::Scroll {
            Some(paint::push_clip(
                context,
                &factory,
                &paint::Shape::Rect(Rect::from_size(size)),
            ))
        } else {
            None
        };
        let layered = clip == Some(paint::Clip::Layer);
        if layered {
            self.layers += 1;
        }

        if let (Some(content), Some(device)) = (view.image.as_ref(), self.graphics.device.as_ref())
            && let Some(bitmap) = self.images.bitmap(content.key)
        {
            paint::paint_image(
                context,
                device,
                &factory,
                size,
                &view.decoration,
                content,
                bitmap,
            );
        }
        if let (Some(player), Some(device)) = (view.media.as_ref(), self.graphics.device.as_ref()) {
            // A player on screen pulls frames; one that is not painted does not.
            self.wants_tick |= player.paint_video(context, device, size);
        }

        if !view.drawing.is_empty() && !size.is_empty() {
            let hairline = 1.0 / self.scale.max(0.1);
            if paint::drawing_needs_surface(&view.drawing) {
                let cache = view
                    .cache
                    .get_or_insert_with(|| Box::new(PaintCache::default()));
                let fresh =
                    cache
                        .drawing
                        .as_ref()
                        .is_some_and(|(_, made_for, scale, made_generation)| {
                            *made_for == size
                                && *scale == self.scale
                                && *made_generation == generation
                        });
                if !fresh {
                    cache.drawing = None;
                    // A surface larger than the device allows is rasterized
                    // at a lower resolution instead of not at all.
                    // SAFETY: plain getter on the live context.
                    let limit =
                        unsafe { context.GetMaximumBitmapSize() }.clamp(1024, 16_384) as f32;
                    let raster = self.scale.min(limit / size.width).min(limit / size.height);
                    let dpi = 96.0 * raster;
                    let (width, height) = (
                        (size.width * raster).ceil() as u32,
                        (size.height * raster).ceil() as u32,
                    );
                    let surface = self
                        .graphics
                        .device
                        .as_mut()
                        .and_then(|device| device.offscreen(self.depth).ok());
                    let bitmap =
                        self.graphics.device.as_ref().and_then(|device| {
                            device.create_target_bitmap(width, height, dpi).ok()
                        });
                    if let (Some(surface), Some(bitmap), Some(device)) =
                        (surface, bitmap, self.graphics.device.as_ref())
                    {
                        // SAFETY: COM calls on live objects; the offscreen
                        // context of this depth is idle.
                        let drawn = unsafe {
                            surface.SetDpi(dpi, dpi);
                            surface.SetTarget(&bitmap);
                            surface.BeginDraw();
                            surface.SetTransform(&Matrix3x2::identity());
                            surface.Clear(None);
                            paint::paint_drawing(
                                &surface,
                                device,
                                self.graphics,
                                self.text,
                                &view.drawing,
                                hairline,
                                true,
                            );
                            let drawn = surface.EndDraw(None, None);
                            surface.SetTarget(None::<&ID2D1Image>);
                            drawn
                        };
                        if drawn.is_ok() {
                            cache.drawing = Some((bitmap, size, self.scale, generation));
                        }
                    }
                }
                if let Some((bitmap, ..)) = &cache.drawing {
                    // SAFETY: COM call with live objects inside BeginDraw.
                    unsafe {
                        context.DrawBitmap(
                            bitmap,
                            Some(&rect_f(Rect::from_size(size))),
                            1.0,
                            D2D1_INTERPOLATION_MODE_LINEAR,
                            None,
                            None,
                        )
                    };
                }
            } else if let Some(device) = self.graphics.device.as_ref() {
                // SAFETY: the clip is popped right after the drawing.
                unsafe {
                    context.PushAxisAlignedClip(
                        &rect_f(Rect::from_size(size)),
                        D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                    );
                    paint::paint_drawing(
                        context,
                        device,
                        self.graphics,
                        self.text,
                        &view.drawing,
                        hairline,
                        false,
                    );
                    context.PopAxisAlignedClip();
                }
            }
        }

        self.paint_text(context, id, size);

        // Children, in content coordinates under a scroll view.
        let Some(view) = self.scene.views.get_mut(&id) else {
            return;
        };
        let offset = view.content_offset();
        let children = std::mem::take(&mut view.children);
        // A clipping view narrows what its descendants can show.
        let (visible, hidden) = match (clip.is_some(), visible) {
            (true, Some(visible)) => match visible.intersect(Rect::from_size(size)) {
                Some(seen) => (Some(seen), false),
                None => (None, true),
            },
            (true, None) => (Some(Rect::from_size(size)), false),
            (false, visible) => (visible, false),
        };
        let visible = visible.map(|visible| visible.translate(offset.x, offset.y));
        if !children.is_empty() && !hidden {
            let saved = (offset != Point::ZERO).then(|| {
                paint::concat_transform(
                    context,
                    &pam_desktop_ui::geometry::Affine::translation(-offset.x, -offset.y),
                )
            });
            for child in &children {
                if let Some(visible) = visible {
                    let Some(child_view) = self.scene.views.get(child) else {
                        continue;
                    };
                    let placed = child_view.to_parent();
                    // The cheap test first: most children either show their
                    // own box or are far away with everything they hold.
                    if !placed.bounds(child_view.bounds()).intersects(visible)
                        && !placed
                            .bounds(self.scene.subtree_bounds(*child, 0, false))
                            .intersects(visible)
                    {
                        continue;
                    }
                }
                self.paint_view(context, *child, level + 1, visible);
            }
            if let Some(saved) = saved {
                // SAFETY: restores the transform saved above.
                unsafe { context.SetTransform(&saved) };
            }
        }
        if let Some(view) = self.scene.views.get_mut(&id) {
            // Nothing can have changed the list while painting.
            view.children = children;
        }

        self.paint_scrollbars(context, id, size);
        if let (Some(view), Some(device)) =
            (self.scene.views.get(&id), self.graphics.device.as_ref())
            && let Some(player) = view.media.as_ref()
        {
            player.paint_controls(context, device, self.graphics, self.text, size);
        }
        if let Some(clip) = clip {
            paint::pop_clip(context, clip);
        }
        if layered {
            self.layers -= 1;
        }

        // Keyboard focus ring, outside the box.
        if self.scene.focus == Some(id)
            && self.scene.focus_visible
            && class == ViewClass::Box
            && let (Some(view), Some(device)) =
                (self.scene.views.get(&id), self.graphics.device.as_ref())
        {
            let ring = paint::rounded(&factory, size, view.decoration.radii, Insets::all(-3.0));
            ring.stroke(context, device.solid(self.theme.accent), 2.0, None);
        }
    }

    fn paint_text(&mut self, context: &ID2D1DeviceContext, id: NodeId, size: Size) {
        let generation = self.graphics.generation;
        let dpi = 96.0 * self.scale;
        let Some(view) = self.scene.views.get_mut(&id) else {
            return;
        };
        let Some(state) = view.text.as_mut() else {
            return;
        };
        let insets = state.content.insets;
        let available = (size.width - insets.horizontal()).max(0.0);
        if state.layout.is_none() || state.wrapped_to != available {
            state.layout = self
                .text
                .build(&self.graphics.dwrite, &state.content.block, available);
            state.wrapped_to = available;
            state.colored_for = 0;
        }
        let Some(layout) = state.layout.as_ref() else {
            return;
        };
        if !layout.boxes.is_empty() {
            // Inline boxes (`<kbd>`, padded spans) sit behind the glyphs.
            let origin = text_origin(
                size,
                insets,
                state.content.vertical,
                state.content.inline,
                layout.height,
            );
            let mut env = PaintEnv {
                graphics: &mut *self.graphics,
                shadows: &mut *self.shadows,
                scale: self.scale,
                depth: self.depth,
            };
            let mut cache = PaintCache::default();
            let mut transform = Matrix3x2::identity();
            // SAFETY: COM calls on the live context inside its BeginDraw; the
            // transform and the clip are restored before returning.
            unsafe {
                context.GetTransform(&mut transform);
                if layout.truncated {
                    let visible = D2D_RECT_F {
                        left: -1.0e6,
                        top: origin.y - 1.0,
                        right: 1.0e6,
                        bottom: origin.y + layout.height + 1.0,
                    };
                    context.PushAxisAlignedClip(&visible, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
                }
                for fragment in &layout.boxes {
                    let placed = Matrix3x2::translation(
                        origin.x + fragment.rect.x,
                        origin.y + fragment.rect.y,
                    ) * transform;
                    context.SetTransform(&placed);
                    cache.invalidate();
                    paint::paint_decoration(
                        context,
                        &mut env,
                        &mut cache,
                        Size::new(fragment.rect.width, fragment.rect.height),
                        &fragment.decoration,
                    );
                }
                context.SetTransform(&transform);
                if layout.truncated {
                    context.PopAxisAlignedClip();
                }
            }
        }
        let Some(device) = self.graphics.device.as_mut() else {
            return;
        };
        if state.colored_for != generation {
            layout.apply_colors(
                &state.content.block,
                device,
                state
                    .selection
                    .map(|range| (range, self.theme.highlight_text)),
            );
            state.colored_for = generation;
        }
        let origin = text_origin(
            size,
            insets,
            state.content.vertical,
            state.content.inline,
            layout.height,
        );
        // SAFETY: COM calls with live objects inside the context's BeginDraw;
        // the clip pushed for truncated text is popped below.
        unsafe {
            if layout.truncated {
                // Lines beyond the limit (and anything painted for them) stay out.
                let visible = D2D_RECT_F {
                    left: -1.0e6,
                    top: origin.y - 1.0,
                    right: 1.0e6,
                    bottom: origin.y + layout.height + 1.0,
                };
                context.PushAxisAlignedClip(&visible, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            }
            // Run backgrounds and the selection sit behind the glyphs.
            for (run, range) in state.content.block.runs.iter().zip(&layout.ranges) {
                let Some(background) = run.background.filter(|color| !color.is_transparent())
                else {
                    continue;
                };
                layout.range_rects(
                    range.startPosition,
                    range.startPosition + range.length,
                    &mut self.rects,
                );
                for rect in &self.rects {
                    context.FillRectangle(
                        &rect_f(rect.translate(origin.x, origin.y)),
                        device.solid(background),
                    );
                }
            }
            if let Some((start, end)) = state.selection {
                layout.range_rects(
                    layout.visible_index(start, true),
                    layout.visible_index(end, false),
                    &mut self.rects,
                );
                for rect in &self.rects {
                    context.FillRectangle(
                        &rect_f(rect.translate(origin.x, origin.y)),
                        device.solid(self.theme.highlight),
                    );
                }
            }
            if let Some(shadow) = state
                .content
                .block
                .shadow
                .filter(|shadow| !shadow.color.is_transparent())
            {
                let text_layout = layout.layout.clone();
                let text_color = self.theme.text;
                let image = paint::text_shadow_image(
                    device,
                    context,
                    self.depth,
                    dpi,
                    shadow.radius,
                    shadow.color,
                    |recorder, device| {
                        recorder.DrawTextLayout(
                            vec2(origin.x, origin.y),
                            &text_layout,
                            device.solid(text_color),
                            D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT,
                        );
                    },
                );
                if let Some(image) = image {
                    let offset = vec2(shadow.offset.x, shadow.offset.y);
                    context.DrawImage(
                        &image,
                        Some(&offset),
                        None,
                        D2D1_INTERPOLATION_MODE_LINEAR,
                        D2D1_COMPOSITE_MODE_SOURCE_OVER,
                    );
                }
            }
            context.DrawTextLayout(
                vec2(origin.x, origin.y),
                &layout.layout,
                device.solid(self.theme.text),
                D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT,
            );
            if layout.truncated {
                context.PopAxisAlignedClip();
            }
        }
    }

    fn paint_scrollbars(&mut self, context: &ID2D1DeviceContext, id: NodeId, size: Size) {
        let Some(scroll) = self
            .scene
            .views
            .get(&id)
            .and_then(|view| view.scroll.as_ref())
        else {
            return;
        };
        // A styled bar in a reserved gutter is a classic bar: always shown.
        let opacity =
            if scroll.config.reserved_gutter && scroll.config.scrollbar_thickness.is_some() {
                1.0
            } else {
                scroll.bar_opacity
            };
        if opacity <= 0.0 {
            return;
        }
        let Some(device) = self.graphics.device.as_ref() else {
            return;
        };
        // A neutral thumb that reads on light and dark content alike.
        let base = if self.theme.dark {
            pam_desktop_ui::color::Color::rgba(255, 255, 255, 140)
        } else {
            pam_desktop_ui::color::Color::rgba(0, 0, 0, 115)
        };
        // CSS `scrollbar-color` names the thumb.
        let base = scroll.config.scrollbar_color.unwrap_or(base);
        let color = base.with_alpha(opacity);
        for vertical in [true, false] {
            let Some((_, thumb)) = scroll.bar(size, vertical) else {
                continue;
            };
            let radius = thumb.width.min(thumb.height) / 2.0;
            // SAFETY: COM call with live objects inside BeginDraw.
            unsafe {
                context.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: rect_f(thumb),
                        radiusX: radius,
                        radiusY: radius,
                    },
                    device.solid(color),
                );
            }
        }
    }
}

/// What happened to a frame.
pub enum Outcome {
    Presented,
    /// Nothing to draw on (no device, minimized, zero size).
    Skipped,
    /// The device was lost: resources were dropped and every window is
    /// dirty again.
    DeviceLost,
}

/// The swap chain could not be created, resized or wrapped. A removed
/// device is the usual reason: everything is rebuilt. Otherwise a new swap
/// chain is made for the next frame that something asks for.
fn surface_failed(state: &mut State, window: WindowId) -> Outcome {
    if state
        .graphics
        .device
        .as_ref()
        .is_some_and(crate::gfx::Device::is_removed)
    {
        state.device_lost();
        return Outcome::DeviceLost;
    }
    if let Some(entry) = state.scene.windows.get_mut(&window) {
        entry.surface = None;
        entry.dirty = Damage::All;
    }
    if let Some(device) = &state.graphics.device {
        // A window can only have one flip-model swap chain: the old one
        // must really be gone (Direct3D defers destruction to a flush)
        // before the next frame makes a new one.
        // SAFETY: plain calls on the live immediate context.
        unsafe {
            device.d3d_context.ClearState();
            device.d3d_context.Flush();
        }
    }
    Outcome::Skipped
}

/// Renders and presents one window.
pub fn render_window(state: &mut State, window: WindowId) -> Outcome {
    let Some(entry) = state.scene.windows.get_mut(&window) else {
        return Outcome::Skipped;
    };
    // The damage is only forgotten once a frame was presented: a frame
    // that could not be drawn is retried with the next change.
    let damage = entry.dirty;
    let (width, height) = entry.pixel_size;
    if width == 0 || height == 0 {
        return Outcome::Skipped;
    }
    if !state.graphics.ensure_device() {
        return Outcome::Skipped;
    }
    let generation = state.graphics.generation;
    if entry
        .surface
        .as_ref()
        .is_some_and(|surface| surface.generation != generation)
    {
        entry.surface = None;
    }
    if entry.surface.is_none() {
        entry.surface = Surface::new(&state.graphics, entry.hwnd, width, height).ok();
    }
    if entry.surface.is_none() {
        return surface_failed(state, window);
    }
    let dpi = entry.dpi as f32;
    let scale = entry.scale();
    let size = entry.size;
    let (root, overlays) = (entry.root, entry.overlays.clone());
    let Some(surface) = entry.surface.as_mut() else {
        return Outcome::Skipped;
    };
    if surface.resize(width, height).is_err() {
        return surface_failed(state, window);
    }
    let Some(device) = state.graphics.device.as_ref() else {
        return Outcome::Skipped;
    };
    let Ok(target) = surface.target(device, dpi) else {
        return surface_failed(state, window);
    };
    let context = device.context.clone();
    state.shadows.begin_frame(generation);
    let theme = state.theme;

    // SAFETY: COM calls on live objects; BeginDraw/EndDraw are paired and
    // the target is detached before presenting.
    let (drawn, wants_tick, blurred) = unsafe {
        context.SetTarget(&target);
        context.SetDpi(dpi, dpi);
        context.BeginDraw();
        context.SetTransform(&Matrix3x2::identity());
        context.Clear(Some(&color_f(theme.window)));
        let mut painter = Painter {
            scene: &mut state.scene,
            graphics: &mut state.graphics,
            text: &state.text,
            images: &state.images,
            shadows: &mut state.shadows,
            theme,
            scale,
            depth: 0,
            rects: Vec::new(),
            wants_tick: false,
            layers: 0,
            blurred: false,
            filled: None,
            failed: None,
        };
        let everything = Rect::from_size(size);
        for layer in root.iter().chain(overlays.iter()) {
            painter.paint_view(&context, *layer, 0, Some(everything));
        }
        let (wants_tick, blurred) = (painter.wants_tick, painter.blurred);
        let failed = painter.failed.take();
        let drawn = context.EndDraw(None, None);
        context.SetTarget(None::<&ID2D1Image>);
        (failed.map_or(drawn, Err), wants_tick, blurred)
    };
    if wants_tick {
        state.request_tick();
    }
    if blurred && let Some(device) = state.graphics.device.as_ref() {
        device.release_backdrop();
    }
    drop(target);
    if let Err(error) = drawn {
        if error.code() == D2DERR_RECREATE_TARGET {
            state.device_lost();
            return Outcome::DeviceLost;
        }
        // Any other failure leaves the previous frame on screen; the next
        // change draws everything again.
        if let Some(entry) = state.scene.windows.get_mut(&window) {
            entry.dirty = Damage::All;
            if blurred && !state.graphics.backdrop_failed {
                // A frame without the blur is better than no frame: this
                // one is drawn again without it, and so are the next ones.
                entry.damage(None);
            }
        }
        state.graphics.backdrop_failed |= blurred;
        return Outcome::Skipped;
    }

    let Some(entry) = state.scene.windows.get_mut(&window) else {
        return Outcome::Skipped;
    };
    let Some(surface) = entry.surface.as_mut() else {
        return Outcome::Skipped;
    };
    let full = RECT {
        left: 0,
        top: 0,
        right: width as i32,
        bottom: height as i32,
    };
    let region = match damage {
        // A blur spreads a change behind it beyond the changed region.
        Damage::Region(region) if surface.presented && partial_present_enabled() && !blurred => {
            let rect = pixel_rect(region, scale);
            let clipped = RECT {
                left: rect.left.max(0),
                top: rect.top.max(0),
                right: rect.right.min(full.right),
                bottom: rect.bottom.min(full.bottom),
            };
            (clipped.right > clipped.left && clipped.bottom > clipped.top).then_some(clipped)
        }
        _ => None,
    };
    // SAFETY: the dirty rectangle outlives the call.
    let presented = unsafe {
        match region {
            Some(mut rect) => {
                // The whole frame was drawn; the rectangle only tells the
                // compositor which part differs from the previous one.
                let parameters = DXGI_PRESENT_PARAMETERS {
                    DirtyRectsCount: 1,
                    pDirtyRects: &mut rect,
                    pScrollRect: std::ptr::null_mut(),
                    pScrollOffset: std::ptr::null_mut(),
                };
                surface.swap_chain.Present1(1, DXGI_PRESENT(0), &parameters)
            }
            None => surface.swap_chain.Present(1, DXGI_PRESENT(0)),
        }
    };
    if presented == DXGI_ERROR_DEVICE_REMOVED || presented == DXGI_ERROR_DEVICE_RESET {
        state.device_lost();
        return Outcome::DeviceLost;
    }
    surface.presented = presented.is_ok();
    if presented.is_ok() {
        entry.dirty = Damage::Clean;
    }
    Outcome::Presented
}

/// `PAM_DESKTOP_FULL_PRESENT=1` presents every frame whole (diagnostics).
fn partial_present_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("PAM_DESKTOP_FULL_PRESENT").is_none())
}

/// `PAM_DESKTOP_BACKDROP_BLUR=0` paints `backdropBlur` views without the
/// blur (diagnostics).
fn backdrop_blur_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var_os("PAM_DESKTOP_BACKDROP_BLUR").is_none_or(|value| value != "0")
    })
}

/// Largest side of a view snapshot, in pixels.
const SNAPSHOT_LIMIT: f32 = 640.0;

/// Renders a view with its children into premultiplied BGRA pixels (top
/// row first), for the image that follows the pointer during a drag.
pub fn snapshot_view(state: &mut State, id: NodeId) -> Option<(Vec<u8>, u32, u32)> {
    let window = state.scene.window_of(id)?;
    let scale = state.scene.windows.get(&window)?.scale();
    let size = state.scene.views.get(&id)?.frame.size();
    let (width, height) = (
        (size.width * scale).ceil().min(SNAPSHOT_LIMIT),
        (size.height * scale).ceil().min(SNAPSHOT_LIMIT),
    );
    if width < 1.0 || height < 1.0 || !state.graphics.ensure_device() {
        return None;
    }
    let (width, height) = (width as u32, height as u32);
    let dpi = 96.0 * scale;
    let generation = state.graphics.generation;
    let device = state.graphics.device.as_mut()?;
    let target = device.create_target_bitmap(width, height, dpi).ok()?;
    let context = device.offscreen(0).ok()?;
    state.shadows.begin_frame(generation);
    let theme = state.theme;
    // SAFETY: COM calls on live objects. No frame is being drawn (this runs
    // from a deferred task), so the offscreen context is idle; the mapped
    // bits are only read between Map and Unmap, within the reported pitch.
    unsafe {
        context.SetDpi(dpi, dpi);
        context.SetTarget(&target);
        context.BeginDraw();
        context.SetTransform(&Matrix3x2::identity());
        context.Clear(None);
        let mut painter = Painter {
            scene: &mut state.scene,
            graphics: &mut state.graphics,
            text: &state.text,
            images: &state.images,
            shadows: &mut state.shadows,
            theme,
            scale,
            depth: 1,
            rects: Vec::new(),
            wants_tick: false,
            layers: 0,
            blurred: false,
            filled: None,
            failed: None,
        };
        painter.paint_content(&context, id, 0, None);
        let drawn = context.EndDraw(None, None);
        context.SetTarget(None::<&ID2D1Image>);
        drawn.ok()?;
        let properties = windows::Win32::Graphics::Direct2D::D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: windows::Win32::Graphics::Direct2D::Common::D2D1_PIXEL_FORMAT {
                format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode:
                    windows::Win32::Graphics::Direct2D::Common::D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: dpi,
            dpiY: dpi,
            bitmapOptions: windows::Win32::Graphics::Direct2D::D2D1_BITMAP_OPTIONS_CPU_READ
                | windows::Win32::Graphics::Direct2D::D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
            colorContext: std::mem::ManuallyDrop::new(None),
        };
        let readable = context
            .CreateBitmap(
                windows::Win32::Graphics::Direct2D::Common::D2D_SIZE_U { width, height },
                None,
                0,
                &properties,
            )
            .ok()?;
        readable.CopyFromBitmap(None, &target, None).ok()?;
        let mapped = readable
            .Map(windows::Win32::Graphics::Direct2D::D2D1_MAP_OPTIONS_READ)
            .ok()?;
        let row = (width * 4) as usize;
        let mut pixels = vec![0_u8; row * height as usize];
        if !mapped.bits.is_null() && mapped.pitch as usize >= row {
            for line in 0..height as usize {
                let source =
                    std::slice::from_raw_parts(mapped.bits.add(line * mapped.pitch as usize), row);
                pixels[line * row..(line + 1) * row].copy_from_slice(source);
            }
        }
        let _ = readable.Unmap();
        Some((pixels, width, height))
    }
}

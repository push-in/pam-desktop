//! Secondary windows: a `Modal` node marked as a window surface is shown as
//! the root of a native window of its own instead of over the main one.
//!
//! The engine lays the node out against the size of that window, so the
//! application describes the window's content like any other tree.

use pam_desktop_wire::{NodeKind, PropKey};

use crate::backend::{MAIN_WINDOW, NodeId, WindowConfig, WindowId};
use crate::context::{Ctx, HostCommand, event_kind};
use crate::geometry::Size;
use crate::scene::{Change, IdMap};

struct Surface {
    window: WindowId,
    open: bool,
    /// Content size of the window, as last known.
    size: Size,
    config: Option<WindowConfig>,
    dirty: bool,
}

pub struct Windows {
    surfaces: IdMap<Surface>,
    next_window: WindowId,
    /// Surface sizes the engine must learn: node and its size (`None` when closed).
    resized: Vec<(NodeId, Option<Size>)>,
    dirty: bool,
}

impl Default for Windows {
    fn default() -> Self {
        Self {
            surfaces: IdMap::default(),
            next_window: MAIN_WINDOW + 1,
            resized: Vec::new(),
            dirty: false,
        }
    }
}

impl Windows {
    pub fn on_change(&mut self, ctx: &mut Ctx<'_>, change: Change) {
        match change {
            Change::Created(id, NodeKind::Modal) => {
                if ctx
                    .scene
                    .node(id)
                    .is_some_and(|node| node.handles(PropKey::WindowSurface))
                {
                    let window = self.next_window;
                    self.next_window += 1;
                    self.surfaces.insert(
                        id,
                        Surface {
                            window,
                            open: false,
                            size: Size::ZERO,
                            config: None,
                            dirty: true,
                        },
                    );
                    self.dirty = true;
                }
            }
            Change::Removed(id, NodeKind::Modal) => {
                if let Some(surface) = self.surfaces.remove(&id) {
                    if surface.open {
                        ctx.commands.push(HostCommand::CloseWindow(surface.window));
                    }
                    self.resized.push((id, None));
                    self.publish(ctx);
                }
            }
            Change::Updated(id, _) => {
                if let Some(surface) = self.surfaces.get_mut(&id) {
                    surface.dirty = true;
                    self.dirty = true;
                }
            }
            _ => {}
        }
    }

    fn publish(&self, ctx: &mut Ctx<'_>) {
        ctx.scene.set_window_roots(
            self.surfaces
                .iter()
                .filter(|(_, surface)| surface.open)
                .map(|(id, _)| *id)
                .collect(),
        );
    }

    pub fn after_commit(&mut self, ctx: &mut Ctx<'_>) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let dirty: Vec<NodeId> = self
            .surfaces
            .iter()
            .filter(|(_, surface)| surface.dirty)
            .map(|(id, _)| *id)
            .collect();
        for id in dirty {
            let Some(node) = ctx.scene.node(id) else {
                continue;
            };
            let props = node.props();
            let visible = props.flag_or(PropKey::Visible, true);
            let length =
                |key: PropKey, default: f32| props.number_or(key, default).clamp(1.0, 16_384.0);
            let minimum = match (
                props.number(PropKey::WindowMinWidth),
                props.number(PropKey::WindowMinHeight),
            ) {
                (Some(width), Some(height)) if width >= 1.0 && height >= 1.0 => {
                    Some(Size::new(width, height))
                }
                _ => None,
            };
            let config = WindowConfig {
                title: props
                    .text_or_empty(PropKey::WindowTitle)
                    .chars()
                    .take(512)
                    .collect(),
                size: Size::new(
                    length(PropKey::WindowWidth, 640.0),
                    length(PropKey::WindowHeight, 480.0),
                ),
                min_size: minimum,
                resizable: props.flag_or(PropKey::WindowResizable, true),
                decorated: true,
                parent: Some(MAIN_WINDOW),
                modal: props.flag_or(PropKey::WindowModal, false),
            };
            let Some(surface) = self.surfaces.get_mut(&id) else {
                continue;
            };
            surface.dirty = false;
            if !visible {
                if std::mem::take(&mut surface.open) {
                    ctx.commands.push(HostCommand::CloseWindow(surface.window));
                    surface.config = None;
                    self.resized.push((id, None));
                }
                continue;
            }
            if surface.config.as_ref() != Some(&config) {
                // Opening, or a new title, size or policy for an open window.
                let resized =
                    surface.config.as_ref().map(|previous| previous.size) != Some(config.size);
                ctx.commands.push(HostCommand::OpenWindow {
                    window: surface.window,
                    config: Box::new(config.clone()),
                });
                if resized {
                    surface.size = config.size;
                    self.resized.push((id, Some(config.size)));
                }
                surface.config = Some(config);
            }
            if !std::mem::replace(&mut surface.open, true) {
                ctx.commands.push(HostCommand::SetWindowRoot {
                    window: surface.window,
                    root: Some(id),
                });
            }
        }
        self.publish(ctx);
    }

    #[must_use]
    pub fn has_resized(&self) -> bool {
        !self.resized.is_empty()
    }

    /// Surface sizes that changed since the last call, for the engine.
    pub fn take_resized(&mut self) -> Vec<(NodeId, Option<Size>)> {
        std::mem::take(&mut self.resized)
    }

    /// The node shown in `window`, when it is one of ours.
    #[must_use]
    pub fn root_of(&self, window: WindowId) -> Option<NodeId> {
        self.surfaces
            .iter()
            .find(|(_, surface)| surface.open && surface.window == window)
            .map(|(id, _)| *id)
    }

    /// The user resized a secondary window.
    pub fn on_resized(&mut self, window: WindowId, size: Size) {
        if let Some((id, surface)) = self
            .surfaces
            .iter_mut()
            .find(|(_, surface)| surface.window == window)
            && surface.size != size
        {
            surface.size = size;
            self.resized.push((*id, Some(size)));
        }
    }

    /// The user asked to close a secondary window: the application decides.
    pub fn on_close_requested(&mut self, ctx: &mut Ctx<'_>, window: WindowId) {
        if let Some(id) = self.root_of(window) {
            ctx.emit_if(
                id,
                PropKey::OnModalRequestClose,
                event_kind::MODAL_REQUEST_CLOSE,
                Vec::new(),
            );
        }
    }
}

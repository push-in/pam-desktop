//! `TabHost`: one scene per tab and a bar drawn by the host.
//!
//! The bar is made of ordinary scene nodes, so it paints, hit-tests and takes
//! keyboard focus like the rest of the tree on every backend. The engine
//! already lays the scenes out in the area the bar leaves free.

use pam_desktop_engine::{TAB_BAR_BOTTOM_HEIGHT, TAB_BAR_RAIL_WIDTH, TAB_BAR_TOP_HEIGHT};
use pam_desktop_wire::{NodeKind, PropKey, PropValue};

use crate::backend::NodeId;
use crate::context::{event_kind, Ctx};
use crate::geometry::Rect;
use crate::props::PropMap;
use crate::scene::{Action, Change, IdMap, LayerOverride, LayerSlot};

const POSITION_TOP: i64 = 2;
const POSITION_RAIL: i64 = 3;
const RAIL_ITEM_HEIGHT: f32 = 64.0;

#[derive(Clone, Debug, PartialEq)]
struct Item {
    label: String,
    badge: Option<String>,
}

#[derive(Debug, Default)]
struct Host {
    bar: Option<NodeId>,
    cells: Vec<NodeId>,
    items: Vec<Item>,
    position: i64,
    /// 1-based, like the wire.
    selected: u32,
    rebuild: bool,
    restyle: bool,
    relayout: bool,
    sync: bool,
}

#[derive(Default)]
pub struct Tabs {
    hosts: IdMap<Host>,
    dirty: bool,
}

/// `[{"name": …, "label": …, "badge": …}, …]`
fn parse_items(json: &str) -> Vec<Item> {
    let Ok(serde_json::Value::Array(entries)) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    entries
        .iter()
        .take(32)
        .map(|entry| Item {
            label: entry.get("label").and_then(serde_json::Value::as_str).unwrap_or("").chars().take(64).collect(),
            badge: entry
                .get("badge")
                .and_then(serde_json::Value::as_str)
                .filter(|badge| !badge.is_empty())
                .map(|badge| badge.chars().take(12).collect()),
        })
        .collect()
}

fn scenes(ctx: &Ctx<'_>, host: NodeId) -> Vec<NodeId> {
    ctx.scene
        .ordered_children(host)
        .into_iter()
        .filter(|child| ctx.scene.node(*child).is_some_and(|node| !node.is_synthetic()))
        .collect()
}

impl Tabs {
    pub fn on_change(&mut self, _ctx: &mut Ctx<'_>, change: Change) {
        match change {
            Change::Created(id, NodeKind::TabHost) => {
                self.hosts.insert(id, Host { rebuild: true, sync: true, ..Host::default() });
                self.dirty = true;
            }
            Change::Removed(id, NodeKind::TabHost) => {
                self.hosts.remove(&id);
            }
            Change::Updated(id, key) => {
                let Some(host) = self.hosts.get_mut(&id) else { return };
                match key {
                    PropKey::TabItems | PropKey::TabPosition => host.rebuild = true,
                    PropKey::TabSelectedIndex => {
                        host.restyle = true;
                        host.sync = true;
                    }
                    PropKey::TabActiveColor
                    | PropKey::TabInactiveColor
                    | PropKey::TabBackgroundColor
                    | PropKey::TabIndicatorColor => host.restyle = true,
                    _ => return,
                }
                self.dirty = true;
            }
            Change::Children(id) => {
                if let Some(host) = self.hosts.get_mut(&id) {
                    host.sync = true;
                    self.dirty = true;
                }
            }
            Change::Frame(id) => {
                if let Some(host) = self.hosts.get_mut(&id) {
                    host.relayout = true;
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
        let ids: Vec<NodeId> = self.hosts.keys().copied().collect();
        for id in ids {
            let Some(host) = self.hosts.get_mut(&id) else { continue };
            let (rebuild, restyle, relayout, sync) = (
                std::mem::take(&mut host.rebuild),
                std::mem::take(&mut host.restyle),
                std::mem::take(&mut host.relayout),
                std::mem::take(&mut host.sync),
            );
            if rebuild {
                self.rebuild(ctx, id);
            } else {
                if restyle {
                    self.read_selection(ctx, id);
                    self.restyle(ctx, id);
                }
                if relayout {
                    self.layout(ctx, id);
                }
            }
            if rebuild || sync {
                self.sync(ctx, id);
            }
        }
    }

    fn read_selection(&mut self, ctx: &Ctx<'_>, id: NodeId) {
        let selected = ctx.scene.node(id).map_or(1, |node| node.props().integer_or(PropKey::TabSelectedIndex, 1));
        if let Some(host) = self.hosts.get_mut(&id) {
            host.selected = selected.clamp(1, 32) as u32;
        }
    }

    /// Recreates the bar and its cells (items or position changed).
    fn rebuild(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        let Some(node) = ctx.scene.node(id) else { return };
        let props = node.props();
        let items = parse_items(props.text_or_empty(PropKey::TabItems));
        let position = props.integer_or(PropKey::TabPosition, 1);
        let Some(host) = self.hosts.get_mut(&id) else { return };
        if let Some(bar) = host.bar.take() {
            ctx.scene.remove_synthetic(bar);
        }
        host.cells.clear();
        host.items = items;
        host.position = position;
        if host.items.is_empty() {
            return;
        }
        let bar = ctx.scene.add_synthetic(id, NodeKind::Row, PropMap::new(), Rect::ZERO, 0, None);
        ctx.scene.set_prop(bar, PropKey::AccessibilityRole, Some(PropValue::Integer(26)));
        host.bar = Some(bar);
        for index in 0..host.items.len() {
            let cell = ctx.scene.add_synthetic(
                bar,
                NodeKind::Button,
                PropMap::new(),
                Rect::ZERO,
                index as u32,
                Some(Action::TabSelect { host: id, index: index as u32 + 1 }),
            );
            host.cells.push(cell);
        }
        self.read_selection(ctx, id);
        self.restyle(ctx, id);
        self.layout(ctx, id);
    }

    fn restyle(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        let Some(node) = ctx.scene.node(id) else { return };
        let props = node.props();
        let active = props.integer_or(PropKey::TabActiveColor, 0xFF00_0000);
        let inactive = props.integer_or(PropKey::TabInactiveColor, 0xFF77_7777);
        let background = props.integer_or(PropKey::TabBackgroundColor, 0xFFFF_FFFF);
        let indicator = props.integer_or(PropKey::TabIndicatorColor, 0xFF00_0000);
        let Some(host) = self.hosts.get(&id) else { return };
        let Some(bar) = host.bar else { return };
        let top = host.position == POSITION_TOP;
        let rail = host.position == POSITION_RAIL;

        ctx.scene.set_prop(bar, PropKey::BackgroundColor, Some(PropValue::Integer(background)));
        // A hairline separates the bar from the scenes.
        let separator = PropValue::Integer(0x1F00_0000);
        let edge = if top {
            PropKey::BorderBottomWidth
        } else if rail {
            PropKey::BorderRightWidth
        } else {
            PropKey::BorderTopWidth
        };
        ctx.scene.set_prop(bar, edge, Some(PropValue::Integer(1)));
        ctx.scene.set_prop(bar, PropKey::BorderColor, Some(separator));

        for (index, cell) in host.cells.iter().enumerate() {
            let item = &host.items[index];
            let selected = index as u32 + 1 == host.selected;
            let label = match &item.badge {
                Some(badge) => format!("{}\n{badge}", item.label),
                None => item.label.clone(),
            };
            ctx.scene.set_prop(*cell, PropKey::Text, Some(PropValue::String(label)));
            ctx.scene.set_prop(*cell, PropKey::FontSize, Some(PropValue::Integer(if top { 14 } else { 12 })));
            ctx.scene.set_prop(*cell, PropKey::FontWeight, Some(PropValue::Integer(if selected { 600 } else { 500 })));
            ctx.scene.set_prop(*cell, PropKey::NumberOfLines, Some(PropValue::Integer(2)));
            ctx.scene.set_prop(*cell, PropKey::TextAlign, Some(PropValue::Integer(2)));
            ctx.scene.set_prop(*cell, PropKey::TextColor, Some(PropValue::Integer(if selected { active } else { inactive })));
            ctx.scene.set_prop(*cell, PropKey::AccessibilityRole, Some(PropValue::Integer(25)));
            ctx.scene.set_prop(*cell, PropKey::AccessibilitySelected, Some(PropValue::Boolean(selected)));
            // The selected tab of a top strip carries the indicator line.
            let indicator_edge = if rail { PropKey::BorderLeftWidth } else { PropKey::BorderBottomWidth };
            let shows_indicator = selected && (top || rail);
            ctx.scene.set_prop(*cell, indicator_edge, shows_indicator.then_some(PropValue::Integer(2)));
            ctx.scene.set_prop(*cell, PropKey::BorderColor, shows_indicator.then_some(PropValue::Integer(indicator)));
        }
    }

    /// Positions the bar and its cells inside the host frame.
    fn layout(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        let Some(frame) = ctx.scene.node(id).map(|node| node.frame) else { return };
        let Some(host) = self.hosts.get(&id) else { return };
        let Some(bar) = host.bar else { return };
        let count = host.cells.len().max(1) as f32;
        let bar_frame = match host.position {
            POSITION_TOP => Rect::new(frame.x, frame.y, frame.width, TAB_BAR_TOP_HEIGHT.min(frame.height)),
            POSITION_RAIL => Rect::new(frame.x, frame.y, TAB_BAR_RAIL_WIDTH.min(frame.width), frame.height),
            _ => {
                let height = TAB_BAR_BOTTOM_HEIGHT.min(frame.height);
                Rect::new(frame.x, frame.bottom() - height, frame.width, height)
            }
        };
        ctx.scene.set_synthetic_frame(bar, bar_frame);
        for (index, cell) in host.cells.iter().enumerate() {
            let cell_frame = if host.position == POSITION_RAIL {
                Rect::new(bar_frame.x, bar_frame.y + index as f32 * RAIL_ITEM_HEIGHT, bar_frame.width, RAIL_ITEM_HEIGHT)
            } else {
                let width = bar_frame.width / count;
                Rect::new(bar_frame.x + index as f32 * width, bar_frame.y, width, bar_frame.height)
            };
            ctx.scene.set_synthetic_frame(*cell, cell_frame);
        }
    }

    /// Only the selected scene is visible; the others stay mounted.
    fn sync(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        self.read_selection(ctx, id);
        let Some(host) = self.hosts.get(&id) else { return };
        for (index, scene) in scenes(ctx, id).into_iter().enumerate() {
            let visible = index as u32 + 1 == host.selected;
            ctx.scene.set_layer_override(
                scene,
                LayerSlot::Presenter,
                if visible { LayerOverride::NONE } else { LayerOverride::HIDDEN },
            );
        }
    }

    /// The user picked a tab: switch at once, then tell PHP (also for a
    /// reselect, which pops that tab's stack).
    pub fn select(&mut self, ctx: &mut Ctx<'_>, id: NodeId, index: u32) {
        let Some(host) = self.hosts.get_mut(&id) else { return };
        if index == 0 || index as usize > host.items.len() {
            return;
        }
        host.selected = index;
        ctx.scene.set_prop(id, PropKey::TabSelectedIndex, Some(PropValue::Integer(i64::from(index))));
        // `set_prop` is reported as a change of the host; apply it now.
        self.restyle(ctx, id);
        self.sync(ctx, id);
        ctx.emit_if(id, PropKey::OnChange, event_kind::CHANGE, index.to_string().into_bytes());
    }

    #[must_use]
    pub fn selected(&self, id: NodeId) -> Option<u32> {
        self.hosts.get(&id).map(|host| host.selected)
    }
}

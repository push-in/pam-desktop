//! The header a `NavigationHost` asks the host to draw: a bar over the top
//! of the routes with the title of the active route, a back button when
//! there is somewhere to go back to, and an optional search field.

use pam_desktop_wire::{NodeKind, PropKey, PropValue};

use crate::backend::NodeId;
use crate::context::Ctx;
use crate::geometry::Rect;
use crate::props::PropMap;
use crate::scene::{Action, Change, IdMap};

const HEIGHT: f32 = 56.0;
const BACK_WIDTH: f32 = 44.0;

#[derive(Default)]
struct Chrome {
    bar: Option<NodeId>,
    search: Option<NodeId>,
    /// Everything the bar was built from; a different key rebuilds it.
    signature: String,
    dirty: bool,
}

#[derive(Default)]
pub struct Headers {
    hosts: IdMap<Chrome>,
    dirty: bool,
}

impl Headers {
    pub fn on_change(&mut self, _ctx: &mut Ctx<'_>, change: Change) {
        match change {
            Change::Created(id, NodeKind::NavigationHost) => {
                self.hosts.insert(
                    id,
                    Chrome {
                        dirty: true,
                        ..Chrome::default()
                    },
                );
                self.dirty = true;
            }
            // The bar is a child of the host and goes away with it.
            Change::Removed(id, NodeKind::NavigationHost) => {
                self.hosts.remove(&id);
            }
            Change::Updated(id, _) | Change::Frame(id) => {
                if let Some(chrome) = self.hosts.get_mut(&id) {
                    chrome.dirty = true;
                    self.dirty = true;
                }
            }
            _ => {}
        }
    }

    /// The host whose header owns the search field `input`.
    #[must_use]
    pub fn search_host(&self, input: NodeId) -> Option<NodeId> {
        self.hosts
            .iter()
            .find(|(_, chrome)| chrome.search == Some(input))
            .map(|(host, _)| *host)
    }

    pub fn after_commit(&mut self, ctx: &mut Ctx<'_>, dark: bool) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let dirty: Vec<NodeId> = self
            .hosts
            .iter()
            .filter(|(_, chrome)| chrome.dirty)
            .map(|(id, _)| *id)
            .collect();
        for id in dirty {
            self.sync(ctx, id, dark);
        }
    }

    fn sync(&mut self, ctx: &mut Ctx<'_>, id: NodeId, dark: bool) {
        let Some(node) = ctx.scene.node(id) else {
            return;
        };
        let props = node.props();
        let frame = node.frame;
        let shown = props.flag_or(PropKey::NavigationHeaderShown, false);
        let title = props.text_or_empty(PropKey::NavigationTitle).to_owned();
        let transparent = props.flag_or(PropKey::NavigationHeaderTransparent, false);
        let background = props
            .integer(PropKey::NavigationHeaderBackgroundColor)
            .unwrap_or(if dark { 0xFF1C_1C1E } else { 0xFFFF_FFFF });
        let tint = props
            .integer(PropKey::NavigationHeaderTintColor)
            .unwrap_or(if dark { 0xFFFF_FFFF } else { 0xFF00_0000 });
        let shadow = props.flag_or(PropKey::NavigationHeaderShadowVisible, true) && !transparent;
        let search = props.flag_or(PropKey::NavigationHeaderSearchEnabled, false);
        let placeholder = props
            .text(PropKey::NavigationHeaderSearchPlaceholder)
            .unwrap_or_else(|| crate::i18n::text(crate::i18n::Text::Search))
            .to_owned();
        // PHP disables the gesture on the first route: nothing to go back to.
        let back = props.flag_or(PropKey::NavigationGestureEnabled, true)
            && node
                .children
                .iter()
                .filter(|child| {
                    ctx.scene
                        .node(**child)
                        .is_some_and(|child| !child.is_synthetic())
                })
                .count()
                >= 2;
        let signature = format!(
            "{shown}|{title}|{transparent}|{background}|{tint}|{shadow}|{search}|{placeholder}|{back}|{}|{}|{}",
            frame.x, frame.y, frame.width
        );
        let Some(chrome) = self.hosts.get_mut(&id) else {
            return;
        };
        chrome.dirty = false;
        if chrome.signature == signature {
            return;
        }
        chrome.signature = signature;
        if let Some(bar) = chrome.bar.take() {
            ctx.scene.remove_synthetic(bar);
        }
        chrome.search = None;
        if !shown {
            return;
        }
        let map = |entries: &[(PropKey, PropValue)]| entries.iter().cloned().collect::<PropMap>();
        let mut bar_props = vec![(
            PropKey::BackgroundColor,
            PropValue::Integer(if transparent { 0 } else { background }),
        )];
        if shadow {
            bar_props.push((
                PropKey::BoxShadows,
                PropValue::String("[[0,1,4,0,520093696,0]]".into()),
            ));
        }
        let bar = ctx.scene.add_synthetic(
            id,
            NodeKind::View,
            map(&bar_props),
            Rect::new(frame.x, frame.y, frame.width, HEIGHT),
            1000,
            None,
        );
        let mut left = frame.x + 16.0;
        if back {
            let glyph = [
                (PropKey::Text, PropValue::String("‹".into())),
                (PropKey::FontSize, PropValue::Float(30.0)),
                (PropKey::TextColor, PropValue::Integer(tint)),
                (PropKey::TextAlign, PropValue::Integer(2)),
                (PropKey::Cursor, PropValue::String("pointer".into())),
                (PropKey::AccessibilityRole, PropValue::Integer(2)),
                (
                    PropKey::AccessibilityLabel,
                    PropValue::String("Back".into()),
                ),
            ];
            ctx.scene.add_synthetic(
                bar,
                NodeKind::Text,
                map(&glyph),
                Rect::new(frame.x + 4.0, frame.y, BACK_WIDTH, HEIGHT),
                1,
                Some(Action::NavigationBack { host: id }),
            );
            left = frame.x + 4.0 + BACK_WIDTH + 4.0;
        }
        let right = frame.x + frame.width - 16.0;
        if search {
            let input = [
                (PropKey::Placeholder, PropValue::String(placeholder)),
                (PropKey::FontSize, PropValue::Float(15.0)),
                (PropKey::TextColor, PropValue::Integer(tint)),
                (PropKey::PaddingLeft, PropValue::Float(12.0)),
                (PropKey::PaddingRight, PropValue::Float(12.0)),
                (PropKey::BorderRadius, PropValue::Float(10.0)),
                (
                    PropKey::BackgroundColor,
                    PropValue::Integer(if dark { 0x24FF_FFFF } else { 0x14_000000 }),
                ),
                (PropKey::OnChange, PropValue::Boolean(true)),
            ];
            chrome.search = Some(ctx.scene.add_synthetic(
                bar,
                NodeKind::Input,
                map(&input),
                Rect::new(left, frame.y + 8.0, (right - left).max(40.0), HEIGHT - 16.0),
                2,
                None,
            ));
        } else {
            let label = [
                (PropKey::Text, PropValue::String(title)),
                (PropKey::FontSize, PropValue::Float(17.0)),
                (PropKey::FontWeight, PropValue::Integer(600)),
                (PropKey::TextColor, PropValue::Integer(tint)),
                (PropKey::NumberOfLines, PropValue::Integer(1)),
                (PropKey::AccessibilityRole, PropValue::Integer(10)),
                (PropKey::PointerEvents, PropValue::Integer(2)),
            ];
            ctx.scene.add_synthetic(
                bar,
                NodeKind::Text,
                map(&label),
                Rect::new(left, frame.y, (right - left).max(40.0), HEIGHT),
                2,
                None,
            );
        }
        chrome.bar = Some(bar);
    }
}

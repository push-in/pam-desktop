//! What the core's controllers work with: the scene, the animation clock and
//! a queue of events for PHP. Controllers never touch a backend; everything
//! they do becomes scene state that the next flush pushes to it.

use pam_desktop_wire::PropKey;

use crate::animator::{Animator, Timers};
use crate::backend::NodeId;
use crate::geometry::{Point, Size};
use crate::scene::Scene;

/// Event kinds of the wire protocol (`Pam\Desktop\EventKind`).
pub mod event_kind {
    pub const PRESS: u16 = 1;
    pub const CHANGE: u16 = 2;
    pub const BACK: u16 = 3;
    pub const LONG_PRESS: u16 = 5;
    pub const FOCUS: u16 = 6;
    pub const BLUR: u16 = 7;
    pub const SUBMIT: u16 = 8;
    pub const SCROLL: u16 = 9;
    pub const REFRESH: u16 = 10;
    pub const TOGGLE: u16 = 11;
    pub const END_REACHED: u16 = 12;
    pub const NATIVE: u16 = 15;
    pub const APP_STATE: u16 = 16;
    pub const DIMENSIONS: u16 = 17;
    pub const MEMORY_PRESSURE: u16 = 18;
    pub const IMAGE_LOAD_START: u16 = 19;
    pub const IMAGE_PROGRESS: u16 = 20;
    pub const IMAGE_LOAD: u16 = 21;
    pub const IMAGE_ERROR: u16 = 22;
    pub const IMAGE_LOAD_END: u16 = 23;
    pub const INPUT_END_EDITING: u16 = 24;
    pub const INPUT_SELECTION_CHANGE: u16 = 25;
    pub const INPUT_CONTENT_SIZE_CHANGE: u16 = 26;
    pub const INPUT_KEY_PRESS: u16 = 27;
    pub const PRESS_IN: u16 = 28;
    pub const PRESS_OUT: u16 = 29;
    pub const PRESS_MOVE: u16 = 30;
    pub const MODAL_REQUEST_CLOSE: u16 = 31;
    pub const MODAL_SHOW: u16 = 32;
    pub const MODAL_DISMISS: u16 = 33;
    pub const CLICK_OUTSIDE: u16 = 35;
    pub const INTERSECT: u16 = 36;
    pub const RESIZE: u16 = 38;
    pub const TOUCH_START: u16 = 39;
    pub const TOUCH_MOVE: u16 = 40;
    pub const TOUCH_END: u16 = 41;
    pub const BOTTOM_SHEET_CHANGE: u16 = 46;
    pub const BOTTOM_SHEET_DISMISS: u16 = 47;
    pub const WEBVIEW_LOAD: u16 = 48;
    pub const WEBVIEW_ERROR: u16 = 49;
    pub const WEBVIEW_MESSAGE: u16 = 50;
    pub const MEDIA_READY: u16 = 51;
    pub const MEDIA_PROGRESS: u16 = 52;
    pub const MEDIA_END: u16 = 53;
    pub const MEDIA_ERROR: u16 = 54;
    pub const DRAG_START: u16 = 55;
    pub const DRAG_END: u16 = 56;
    pub const DROP: u16 = 57;
    pub const MENU_ACTION: u16 = 58;
    pub const MEDIA_CACHE_HIT: u16 = 61;
    pub const MEDIA_CACHE_MISS: u16 = 62;
    pub const MEDIA_CACHE_PROGRESS: u16 = 63;
    pub const MEDIA_CACHE_READY: u16 = 64;
    pub const MEDIA_BUFFERING: u16 = 68;
    pub const MEDIA_LOAD_START: u16 = 69;
    pub const ANIMATION_COMPLETE: u16 = 60;
    pub const ACCESSIBILITY_ACTION: u16 = 65;
    pub const SPAN_PRESS: u16 = 66;
    pub const LAYOUT: u16 = 67;
    pub const DOUBLE_TAP: u16 = 70;
    pub const SCROLL_BEGIN_DRAG: u16 = 72;
    pub const SCROLL_END_DRAG: u16 = 73;
    pub const MOMENTUM_SCROLL_END: u16 = 74;
    pub const TEXT_LAYOUT: u16 = 75;
    pub const PAGE_CHANGE: u16 = 77;
    pub const ROUTE_COVERED: u16 = 78;
    pub const ROUTE_FOCUSED: u16 = 79;
    pub const LIST_WINDOW: u16 = 80;
    // Desktop table.
    pub const HOVER_IN: u16 = 2001;
    pub const HOVER_OUT: u16 = 2002;
    pub const KEY_DOWN: u16 = 2003;
    pub const KEY_UP: u16 = 2004;
    pub const FILES_DROP: u16 = 2005;
}

/// An event on its way to the PHP worker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutEvent {
    pub node: NodeId,
    pub kind: u16,
    pub payload: Vec<u8>,
}

/// Imperative requests the runtime forwards to the backend after the
/// controllers ran (they have no backend of their own).
#[derive(Clone, Debug, PartialEq)]
pub enum HostCommand {
    /// Moves keyboard focus to a node, or clears it.
    Focus(Option<NodeId>),
    ScrollTo { id: NodeId, offset: Point, animated: bool },
    InputConfig { id: NodeId, config: Box<crate::backend::InputConfig> },
    InputText { id: NodeId, text: String, selection: Option<(u32, u32)> },
    InputSelection { id: NodeId, start: u32, end: u32 },
    /// The press being handled grabbed a window drag region.
    BeginWindowDrag,
    /// Decodes an image; `max_dimension` bounds its longest side in pixels.
    LoadImage { key: crate::backend::ImageKey, source: crate::backend::ImageSource, max_dimension: Option<u32> },
    ReleaseImage(crate::backend::ImageKey),
    SetImage { id: NodeId, content: Option<crate::backend::ImageContent> },
    SetDrawing { id: NodeId, commands: Vec<crate::backend::DrawCommand> },
    SetTextSelection { id: NodeId, range: Option<(u32, u32)> },
    /// Starts a system drag of text from the press being handled.
    BeginTextDrag { source: NodeId, text: String },
    SetMedia { id: NodeId, state: Box<crate::backend::MediaState> },
    SetWebView { id: NodeId, state: Box<crate::backend::WebViewState> },
}

pub struct Ctx<'a> {
    pub scene: &'a mut Scene,
    pub animator: &'a mut Animator,
    pub timers: &'a mut Timers,
    pub events: &'a mut Vec<OutEvent>,
    pub commands: &'a mut Vec<HostCommand>,
    /// Monotonic milliseconds.
    pub now_ms: u64,
    /// Size of the window's content area.
    pub viewport: Size,
    pub reduced_motion: bool,
}

impl Ctx<'_> {
    pub fn emit(&mut self, node: NodeId, kind: u16, payload: Vec<u8>) {
        self.events.push(OutEvent { node, kind, payload });
    }

    /// Emits only when the node carries the handler marker property, like
    /// every host of the protocol does.
    pub fn emit_if(&mut self, node: NodeId, marker: PropKey, kind: u16, payload: Vec<u8>) -> bool {
        let handled = self.scene.node(node).is_some_and(|node| node.handles(marker));
        if handled {
            self.emit(node, kind, payload);
        }
        handled
    }
}

//! Native text entry: a child `EDIT` control over the node, single-line for
//! `ViewClass::Input` and multi-line for `ViewClass::TextArea`. The box
//! around it (background, border, radius) is painted by the view, so the
//! field looks as the application styled it while typing, selection, input
//! methods and accessibility stay native.
//!
//! The control is created when its view first reaches a window; until then
//! configuration, text and selection are kept here and applied on creation.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use pam_desktop_ui::backend::{
    BackendEvent, InputConfig, InputEvent, InputPurpose, NodeId, PointerButton, PointerPhase,
    VerticalAlign,
};
use pam_desktop_ui::color::Color;
use pam_desktop_ui::geometry::{Rect, Size};
use pam_desktop_ui::text::TextAlign;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, CreateRectRgn, CreateSolidBrush,
    DEFAULT_CHARSET, DT_CENTER, DT_EDITCONTROL, DT_LEFT, DT_NOPREFIX, DT_RIGHT, DT_SINGLELINE,
    DT_WORDBREAK, DeleteObject, DrawTextW, GetDC, GetTextMetricsW, HBRUSH, HDC, HFONT, HGDIOBJ,
    InvalidateRect, OUT_DEFAULT_PRECIS, ReleaseDC, SelectObject, SetBkColor, SetBkMode,
    SetTextColor, SetWindowRgn, TEXTMETRICW, TRANSPARENT,
};
use windows::Win32::UI::Controls::{
    EM_GETFIRSTVISIBLELINE, EM_GETLINECOUNT, EM_GETRECT, EM_GETSEL, EM_SCROLLCARET,
    EM_SETLIMITTEXT, EM_SETMARGINS, EM_SETPASSWORDCHAR, EM_SETREADONLY, EM_SETSEL, EM_UNDO,
    WM_MOUSELEAVE,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetFocus, SetFocus, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_BACK,
    VK_ESCAPE, VK_F1, VK_F24, VK_RETURN, VK_TAB,
};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, EC_LEFTMARGIN, EC_RIGHTMARGIN, EN_CHANGE, ES_AUTOHSCROLL,
    ES_AUTOVSCROLL, ES_CENTER, ES_LEFT, ES_MULTILINE, ES_RIGHT, ES_WANTRETURN, GA_ROOT,
    GetAncestor, GetClientRect, GetWindowTextLengthW, GetWindowTextW, HideCaret, IsWindowVisible,
    PostMessageW, SW_HIDE, SW_SHOWNA, SWP_NOACTIVATE, SWP_NOZORDER, SendMessageW, SetWindowPos,
    SetWindowTextW, ShowWindow, WINDOW_STYLE, WM_CHAR, WM_CONTEXTMENU, WM_KEYDOWN, WM_KEYUP,
    WM_KILLFOCUS, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE,
    WM_MOUSEWHEEL, WM_NCDESTROY, WM_PAINT, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SETFOCUS, WM_SETFONT,
    WM_SYSKEYDOWN, WS_CHILD, WS_CLIPSIBLINGS, WS_EX_NOPARENTNOTIFY,
};
use windows::core::{PCWSTR, w};

use crate::app::{App, State};
use crate::events;
use crate::scene::Placement;
use crate::util::{colorref, pixel_rect_nearest, wide};

windows::core::link!("msctf.dll" "system" fn SetInputScope(hwnd: HWND, inputscope: i32) -> windows::core::HRESULT);

/// The bullet shown for each character of a secure field.
const PASSWORD_CHARACTER: usize = 0x25CF;
const SUBCLASS_ID: usize = 0x5041_4D31;

thread_local! {
    /// Control handle to node, for the subclass procedure.
    static CONTROLS: RefCell<HashMap<isize, NodeId>> = RefCell::new(HashMap::new());
}

fn node_of(hwnd: HWND) -> Option<NodeId> {
    CONTROLS.with(|controls| controls.borrow().get(&(hwnd.0 as isize)).copied())
}

/// GDI objects of a control, released with it.
struct Resources {
    font: HFONT,
    /// Height of one text line in pixels.
    line_height: i32,
    /// Scale and font the GDI font was made for.
    made_for: (f32, pam_desktop_ui::text::FontSpec),
    brush: HBRUSH,
    brush_color: COLORREF,
}

impl Drop for Resources {
    fn drop(&mut self) {
        // SAFETY: the objects were created by this host and are no longer
        // selected anywhere once the control is gone or re-fonted.
        unsafe {
            if !self.font.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(self.font.0));
            }
            if !self.brush.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(self.brush.0));
            }
        }
    }
}

pub struct InputHost {
    multiline: bool,
    pub hwnd: Option<HWND>,
    /// The top-level window the control was created in.
    parent: HWND,
    pub config: InputConfig,
    /// The text of the field (line breaks as `\n`).
    text: String,
    /// Selection in UTF-16 units of `text`.
    selection: (u32, u32),
    /// The host itself is changing the control: no events.
    updating: bool,
    /// Style bits that can only be set at creation.
    created_style: u32,
    resources: Option<Resources>,
    placement: Option<Placement>,
    /// Keyboard focus was asked before the control existed.
    focus_pending: bool,
    /// Line count last reported with `ContentSizeChanged`.
    lines: i32,
    /// Background and text colors the control last painted with.
    painted: Option<(COLORREF, COLORREF)>,
    /// The first half of a character outside the basic plane, waiting for
    /// the second `WM_CHAR`.
    high_surrogate: Option<u16>,
    /// A captured key must not also insert its translated WM_CHAR.
    /// `TrackMouseEvent` is armed for the control.
    tracking_leave: bool,
}

impl InputHost {
    pub fn new(multiline: bool) -> Self {
        Self {
            multiline,
            hwnd: None,
            parent: HWND::default(),
            config: InputConfig::default(),
            text: String::new(),
            selection: (0, 0),
            updating: false,
            created_style: 0,
            resources: None,
            placement: None,
            focus_pending: false,
            lines: 0,
            painted: None,
            high_surrogate: None,
            tracking_leave: false,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Forgets a focus request made before the control existed.
    pub fn cancel_focus(&mut self) {
        self.focus_pending = false;
    }

    fn creation_style(&self) -> u32 {
        let alignment = match self.config.align {
            TextAlign::Start | TextAlign::Justify => ES_LEFT,
            TextAlign::Center => ES_CENTER,
            TextAlign::End => ES_RIGHT,
        };
        // Password masking is switched at run time (EM_SETPASSWORDCHAR),
        // so it is not a reason to rebuild the control.
        let kind = if self.multiline {
            ES_MULTILINE | ES_AUTOVSCROLL | ES_WANTRETURN
        } else {
            ES_AUTOHSCROLL
        };
        (alignment | kind) as u32
    }
}

/// The control's text uses CR LF line breaks; the protocol uses LF.
fn to_control(text: &str, multiline: bool) -> Vec<u16> {
    if !multiline {
        return wide(text);
    }
    let mut units = Vec::with_capacity(text.len() + 8);
    let mut previous = 0_u16;
    for unit in text.encode_utf16() {
        if unit == u16::from(b'\n') && previous != u16::from(b'\r') {
            units.push(u16::from(b'\r'));
        }
        units.push(unit);
        previous = unit;
    }
    units.push(0);
    units
}

/// Control offset (UTF-16 units, CR LF breaks) to protocol offset (LF breaks).
fn from_control_offset(control_text: &[u16], offset: u32, multiline: bool) -> u32 {
    if !multiline {
        return offset;
    }
    let end = (offset as usize).min(control_text.len());
    let breaks = control_text[..end]
        .windows(2)
        .filter(|pair| pair[0] == u16::from(b'\r') && pair[1] == u16::from(b'\n'))
        .count();
    offset.saturating_sub(breaks as u32)
}

/// Protocol offset to control offset.
fn to_control_offset(text: &str, offset: u32, multiline: bool) -> u32 {
    if !multiline {
        return offset;
    }
    let breaks = text
        .encode_utf16()
        .take(offset as usize)
        .filter(|unit| *unit == u16::from(b'\n'))
        .count();
    offset + breaks as u32
}

fn read_text(hwnd: HWND) -> Vec<u16> {
    // SAFETY: the buffer holds the reported length plus the terminator.
    unsafe {
        let length = GetWindowTextLengthW(hwnd).max(0) as usize;
        let mut buffer = vec![0_u16; length + 1];
        let copied = GetWindowTextW(hwnd, &mut buffer).max(0) as usize;
        buffer.truncate(copied);
        buffer
    }
}

fn read_selection(hwnd: HWND) -> (u32, u32) {
    let (mut start, mut end) = (0_u32, 0_u32);
    // SAFETY: EM_GETSEL writes two DWORDs through the pointers passed.
    unsafe {
        SendMessageW(
            hwnd,
            EM_GETSEL,
            Some(WPARAM(std::ptr::from_mut(&mut start) as usize)),
            Some(LPARAM(std::ptr::from_mut(&mut end) as isize)),
        );
    }
    (start, end)
}

fn protocol_text(control_text: &[u16], multiline: bool) -> String {
    let text = String::from_utf16_lossy(control_text);
    if multiline {
        text.replace("\r\n", "\n")
    } else {
        text
    }
}

fn send(hwnd: HWND, message: u32, wparam: usize, lparam: isize) -> LRESULT {
    // SAFETY: plain message send to a live control of this thread.
    unsafe { SendMessageW(hwnd, message, Some(WPARAM(wparam)), Some(LPARAM(lparam))) }
}

/// The color behind the control: the nearest opaque background up the tree.
fn background_of(state: &State, id: NodeId) -> Color {
    let mut current = Some(id);
    while let Some(view) = current.and_then(|id| state.scene.views.get(&id)) {
        if view.decoration.background.is_opaque() {
            return view.decoration.background;
        }
        current = view.parent;
    }
    state.theme.window
}

fn make_font(family: &str, config: &InputConfig, scale: f32) -> HFONT {
    let face = wide(family);
    let height = -((config.font.size * scale).round() as i32).max(1);
    // SAFETY: the face name is NUL-terminated and outlives the call.
    unsafe {
        CreateFontW(
            height,
            0,
            0,
            0,
            i32::from(config.font.weight.clamp(1, 1000)),
            u32::from(config.font.italic),
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            0,
            PCWSTR(face.as_ptr()),
        )
    }
}

fn line_height_of(hwnd: HWND, font: HFONT) -> i32 {
    // SAFETY: the DC is released after the font is selected back out.
    unsafe {
        let dc = GetDC(Some(hwnd));
        let previous = SelectObject(dc, HGDIOBJ(font.0));
        let mut metrics = TEXTMETRICW::default();
        let _ = GetTextMetricsW(dc, &mut metrics);
        SelectObject(dc, previous);
        ReleaseDC(Some(hwnd), dc);
        metrics.tmHeight.max(1)
    }
}

/// What must be done to a control, computed under the state borrow and
/// carried out after it.
struct Plan {
    hwnd: Option<HWND>,
    parent: HWND,
    create: Option<u32>,
    /// A stale control to destroy (its creation style changed).
    replace: Option<HWND>,
    config: InputConfig,
    multiline: bool,
    family: String,
    text: String,
    selection: (u32, u32),
    font_stale: bool,
    focus: bool,
    /// The colors around the control changed since it last painted.
    repaint: bool,
    placement: Placement,
    previous: Option<Placement>,
}

/// Creates the control when needed and moves it over its node.
pub fn place(app: &Rc<App>, id: NodeId, placement: &Placement) {
    let plan = app
        .with_state(|state| {
            let parent = state.scene.windows.get(&placement.window)?.hwnd;
            // SAFETY: plain query; it sends no message.
            let window_shown = unsafe { IsWindowVisible(parent) }.as_bool();
            let wanted_colors = (
                colorref(background_of(state, id)),
                colorref(
                    state
                        .scene
                        .views
                        .get(&id)?
                        .input
                        .as_ref()?
                        .config
                        .color
                        .unwrap_or(state.theme.text),
                ),
            );
            let view = state.scene.views.get_mut(&id)?;
            let host = view.input.as_mut()?;
            let family = state.text.family(&host.config.font);
            let style = host.creation_style();
            let stale = host
                .hwnd
                .filter(|_| host.created_style != style || host.parent != parent);
            if stale.is_some() {
                host.hwnd = None;
                host.resources = None;
            }
            let font_stale = host.resources.as_ref().is_none_or(|resources| {
                resources.made_for != (placement.scale, host.config.font.clone())
            });
            let previous = host.placement.replace(*placement);
            Some(Plan {
                hwnd: host.hwnd,
                parent,
                create: host.hwnd.is_none().then_some(style),
                replace: stale,
                config: host.config.clone(),
                multiline: host.multiline,
                family,
                text: host.text.clone(),
                selection: host.selection,
                font_stale,
                // A window that is not shown yet would take the focus away
                // again when it is activated: the request waits for it.
                focus: window_shown && std::mem::take(&mut host.focus_pending),
                repaint: host.painted.is_some_and(|painted| painted != wanted_colors),
                placement: *placement,
                previous,
            })
        })
        .flatten();
    let Some(mut plan) = plan else { return };
    if let Some(stale) = plan.replace {
        // The replacement keeps the keyboard focus of the control it replaces.
        // SAFETY: plain query.
        plan.focus |= unsafe { GetFocus() } == stale;
        destroy_control(stale);
    }
    let created = plan.create.is_some();
    if let Some(style) = plan.create {
        // SAFETY: standard creation of a child EDIT control; the subclass
        // procedure is a valid function for the life of the control.
        let hwnd = unsafe {
            // No WM_PARENTNOTIFY: the parent has no use for it.
            let Ok(hwnd) = CreateWindowExW(
                WS_EX_NOPARENTNOTIFY,
                w!("EDIT"),
                PCWSTR::null(),
                WS_CHILD | WS_CLIPSIBLINGS | WINDOW_STYLE(style),
                0,
                0,
                0,
                0,
                Some(plan.parent),
                None,
                Some(app.instance),
                None,
            ) else {
                return;
            };
            CONTROLS.with(|controls| controls.borrow_mut().insert(hwnd.0 as isize, id));
            let _ = SetWindowSubclass(hwnd, Some(edit_proc), SUBCLASS_ID, 0);
            hwnd
        };
        plan.hwnd = Some(hwnd);
        app.with_state(|state| {
            if let Some(host) = state
                .scene
                .views
                .get_mut(&id)
                .and_then(|view| view.input.as_mut())
            {
                host.hwnd = Some(hwnd);
                host.parent = plan.parent;
                host.created_style = style;
                host.updating = true;
            }
        });
        apply_config(hwnd, &plan.config, plan.multiline);
        let text = to_control(&plan.text, plan.multiline);
        // SAFETY: the text is NUL-terminated and outlives the call.
        unsafe {
            let _ = SetWindowTextW(hwnd, PCWSTR(text.as_ptr()));
        }
        let (start, end) = plan.selection;
        send(
            hwnd,
            EM_SETSEL,
            to_control_offset(&plan.text, start, plan.multiline) as usize,
            to_control_offset(&plan.text, end, plan.multiline) as isize,
        );
        app.with_state(|state| {
            if let Some(host) = state
                .scene
                .views
                .get_mut(&id)
                .and_then(|view| view.input.as_mut())
            {
                host.updating = false;
                // The change notification of the initial text moved it.
                host.selection = plan.selection;
            }
        });
        crate::accessibility::control_created(app, id, hwnd);
    }
    let Some(hwnd) = plan.hwnd else { return };
    let mut line_height = app
        .with_state(|state| {
            state
                .scene
                .views
                .get(&id)?
                .input
                .as_ref()?
                .resources
                .as_ref()
                .map(|resources| resources.line_height)
        })
        .flatten();
    if plan.font_stale || created || line_height.is_none() {
        let font = make_font(&plan.family, &plan.config, plan.placement.scale);
        send(hwnd, WM_SETFONT, font.0 as usize, 1);
        // The control adds margins of its own when the font changes.
        send(
            hwnd,
            EM_SETMARGINS,
            (EC_LEFTMARGIN | EC_RIGHTMARGIN) as usize,
            0,
        );
        let height = line_height_of(hwnd, font);
        line_height = Some(height);
        app.with_state(|state| {
            if let Some(host) = state
                .scene
                .views
                .get_mut(&id)
                .and_then(|view| view.input.as_mut())
            {
                let (brush, brush_color) = host
                    .resources
                    .as_mut()
                    .map_or((HBRUSH::default(), COLORREF(0)), |resources| {
                        (std::mem::take(&mut resources.brush), resources.brush_color)
                    });
                host.resources = Some(Resources {
                    font,
                    line_height: height,
                    made_for: (plan.placement.scale, plan.config.font.clone()),
                    brush,
                    brush_color,
                });
            }
        });
    }
    if plan.previous != Some(plan.placement) || created || plan.font_stale {
        move_control(
            hwnd,
            &plan.placement,
            &plan.config,
            plan.multiline,
            line_height.unwrap_or(16),
        );
        // A new width or font wraps the text differently.
        report_content_size(app, id, hwnd);
    }
    if plan.repaint {
        // The box behind the control changed color (focus or hover style,
        // appearance): the control paints its background again.
        // SAFETY: invalidating a live control.
        unsafe {
            let _ = InvalidateRect(Some(hwnd), None, true);
        }
    }
    if plan.focus {
        // SAFETY: the control is live; no state borrow is held.
        unsafe {
            let _ = SetFocus(Some(hwnd));
        }
    }
}

fn move_control(
    hwnd: HWND,
    placement: &Placement,
    config: &InputConfig,
    multiline: bool,
    line_height: i32,
) {
    let scale = placement.scale;
    let content = placement.rect.inset_by(config.insets);
    let content = Rect::new(
        content.x,
        content.y,
        content.width.max(0.0),
        content.height.max(0.0),
    );
    let mut rect = pixel_rect_nearest(content, scale);
    if !multiline {
        // A single-line control has no vertical alignment of its own: it is
        // exactly one line tall and placed inside the box.
        let free = (rect.bottom - rect.top - line_height).max(0);
        let top = rect.top
            + match config.vertical {
                VerticalAlign::Top => 0,
                VerticalAlign::Center => free / 2,
                VerticalAlign::Bottom => free,
            };
        rect.bottom = (top + line_height).min(rect.bottom.max(top + 1));
        rect.top = top;
    }
    let (width, height) = (
        (rect.right - rect.left).max(0),
        (rect.bottom - rect.top).max(0),
    );
    let visible = placement.visible && width > 0 && height > 0;
    // SAFETY: the control is live; these calls dispatch messages and no
    // state borrow is held. The region passed to SetWindowRgn is owned by
    // the system afterwards.
    unsafe {
        if !visible {
            let _ = ShowWindow(hwnd, SW_HIDE);
            return;
        }
        let _ = SetWindowPos(
            hwnd,
            None,
            rect.left,
            rect.top,
            width,
            height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        match placement.clip {
            Some(clip) => {
                let clip = pixel_rect_nearest(clip, scale);
                let region = CreateRectRgn(
                    clip.left - rect.left,
                    clip.top - rect.top,
                    clip.right - rect.left,
                    clip.bottom - rect.top,
                );
                SetWindowRgn(hwnd, Some(region), true);
            }
            None => {
                SetWindowRgn(hwnd, None, true);
            }
        }
        let _ = ShowWindow(hwnd, SW_SHOWNA);
    }
}

fn apply_config(hwnd: HWND, config: &InputConfig, multiline: bool) {
    if !multiline {
        send(
            hwnd,
            EM_SETPASSWORDCHAR,
            if config.secure { PASSWORD_CHARACTER } else { 0 },
            0,
        );
    }
    // Zero restores the control's own (very large) limit.
    send(
        hwnd,
        EM_SETLIMITTEXT,
        config.max_length.map_or(0, |length| length as usize),
        0,
    );
    send(hwnd, EM_SETREADONLY, usize::from(!config.editable), 0);
    let scope = match config.purpose {
        _ if config.secure => 31,
        InputPurpose::Text => 0,
        InputPurpose::Url => 1,
        InputPurpose::Email => 5,
        InputPurpose::Number => 28,
        InputPurpose::Decimal => 29,
        InputPurpose::Phone => 32,
        InputPurpose::Search => 50,
    };
    // SAFETY: the control is live. The input scope only hints the touch
    // keyboard; failing to set it changes nothing else.
    unsafe {
        let _ = SetInputScope(hwnd, scope);
        if !config.enabled && GetFocus() == hwnd {
            // A disabled window that keeps the focus swallows the keyboard.
            let _ = SetFocus(Some(GetAncestor(hwnd, GA_ROOT)));
        }
        let _ = EnableWindow(hwnd, config.enabled);
        let _ = InvalidateRect(Some(hwnd), None, true);
    }
}

/// Hides the control of a view that is no longer in a window. `parent` is
/// the window being synchronized: controls of other windows are left alone.
pub fn detach(app: &Rc<App>, id: NodeId, parent: HWND) {
    let hidden = app
        .with_state(|state| {
            let host = state.scene.views.get_mut(&id)?.input.as_mut()?;
            if host.parent != parent {
                return None;
            }
            // Forgetting the placement makes the control be placed again
            // when the view returns.
            host.placement.take()?;
            host.hwnd
        })
        .flatten();
    if let Some(hwnd) = hidden {
        // SAFETY: hiding a live control; no state borrow is held.
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
}

/// `Backend::set_input_config`.
pub fn configure(app: &Rc<App>, id: NodeId, config: &InputConfig) {
    let known = app
        .with_state(|state| {
            let view = state.scene.views.get_mut(&id)?;
            let host = view.input.as_mut()?;
            host.config = config.clone();
            // Font, insets or alignment may move or rebuild the control.
            host.placement = None;
            let (hwnd, multiline) = (host.hwnd, host.multiline);
            if let Some(window) = state.scene.window_of(id)
                && let Some(entry) = state.scene.windows.get_mut(&window)
            {
                entry.natives_dirty = true;
                entry.damage(None);
            }
            Some((hwnd, multiline))
        })
        .flatten();
    if let Some((Some(hwnd), multiline)) = known {
        apply_config(hwnd, config, multiline);
    }
}

/// `Backend::set_input_text`.
pub fn set_text(app: &Rc<App>, id: NodeId, text: &str, selection: Option<(u32, u32)>) {
    let end = crate::util::utf16_len(text);
    let selection = selection.unwrap_or((end, end));
    let known = app
        .with_state(|state| {
            let host = state.scene.views.get_mut(&id)?.input.as_mut()?;
            let changed = host.text != text;
            let was_empty = host.text.is_empty();
            if changed {
                host.text = text.to_owned();
            }
            host.selection = selection;
            let hwnd = host.hwnd?;
            host.updating = true;
            Some((hwnd, host.multiline, changed, was_empty != text.is_empty()))
        })
        .flatten();
    let Some((hwnd, multiline, changed, emptiness_changed)) = known else {
        return;
    };
    if changed {
        let units = to_control(text, multiline);
        // SAFETY: the text is NUL-terminated and outlives the call.
        unsafe {
            let _ = SetWindowTextW(hwnd, PCWSTR(units.as_ptr()));
        }
    }
    send(
        hwnd,
        EM_SETSEL,
        to_control_offset(text, selection.0, multiline) as usize,
        to_control_offset(text, selection.1, multiline) as isize,
    );
    send(hwnd, EM_SCROLLCARET, 0, 0);
    if emptiness_changed {
        // The placeholder appears or goes.
        // SAFETY: invalidating a live control.
        unsafe {
            let _ = InvalidateRect(Some(hwnd), None, true);
        }
    }
    app.with_state(|state| {
        if let Some(host) = state
            .scene
            .views
            .get_mut(&id)
            .and_then(|view| view.input.as_mut())
        {
            host.updating = false;
            // What the change notification recorded was the caret before
            // the selection was applied.
            host.selection = selection;
        }
    });
    // A multi-line control sends no change notification for a text set
    // by code: its height is reported here.
    report_content_size(app, id, hwnd);
}

/// `Backend::set_input_selection`.
pub fn set_selection(app: &Rc<App>, id: NodeId, start: u32, end: u32) {
    let known = app
        .with_state(|state| {
            let host = state.scene.views.get_mut(&id)?.input.as_mut()?;
            host.selection = (start, end);
            let hwnd = host.hwnd?;
            host.updating = true;
            let (from, to) = (
                to_control_offset(&host.text, start, host.multiline),
                to_control_offset(&host.text, end, host.multiline),
            );
            Some((hwnd, from, to))
        })
        .flatten();
    let Some((hwnd, from, to)) = known else {
        return;
    };
    send(hwnd, EM_SETSEL, from as usize, to as isize);
    app.with_state(|state| {
        if let Some(host) = state
            .scene
            .views
            .get_mut(&id)
            .and_then(|view| view.input.as_mut())
        {
            host.updating = false;
        }
    });
}

/// Gives keyboard focus to the control of `id` (now, or once it exists).
pub fn focus(app: &Rc<App>, id: NodeId) -> bool {
    let known = app
        .with_state(|state| {
            let host = state.scene.views.get_mut(&id)?.input.as_mut()?;
            if host.hwnd.is_none() {
                host.focus_pending = true;
            }
            Some(host.hwnd)
        })
        .flatten();
    match known {
        Some(Some(hwnd)) => {
            // SAFETY: the control is live; no state borrow is held.
            unsafe {
                let _ = SetFocus(Some(hwnd));
            }
            true
        }
        Some(None) => true,
        None => false,
    }
}

/// Sends an edit command (`WM_COPY`, `WM_PASTE`…) to the focused control.
pub fn edit_command(message: u32) {
    // SAFETY: plain message send to whichever window has the focus on this thread.
    unsafe {
        let focused = GetFocus();
        if !focused.is_invalid() {
            if message == EM_UNDO {
                SendMessageW(focused, EM_UNDO, Some(WPARAM(0)), Some(LPARAM(0)));
            } else if message == EM_SETSEL {
                SendMessageW(focused, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
            } else {
                SendMessageW(focused, message, Some(WPARAM(0)), Some(LPARAM(0)));
            }
        }
    }
}

fn destroy_control(hwnd: HWND) {
    CONTROLS.with(|controls| controls.borrow_mut().remove(&(hwnd.0 as isize)));
    // SAFETY: destroying a child control of this thread; no state borrow is held.
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
}

/// Releases the control of a destroyed view. The window is hidden at once
/// and destroyed from the message loop: it may be inside its own procedure.
pub fn release(app: &Rc<App>, host: Box<InputHost>) {
    let Some(hwnd) = host.hwnd else { return };
    CONTROLS.with(|controls| controls.borrow_mut().remove(&(hwnd.0 as isize)));
    // SAFETY: hiding a live control; no state borrow is held.
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
    app.defer(Box::new(move |_| {
        // The GDI objects go after the control that used them.
        destroy_control(hwnd);
        drop(host);
    }));
}

/// The top-level window died and took its child controls with it.
pub fn parent_destroyed(host: &mut InputHost, parent: HWND) {
    if host.parent == parent {
        if let Some(hwnd) = host.hwnd.take() {
            CONTROLS.with(|controls| controls.borrow_mut().remove(&(hwnd.0 as isize)));
        }
        host.placement = None;
        host.resources = None;
    }
}

/// Reports the size of the laid out text of a multi-line control when
/// its line count changed (controls that grow with their content).
fn report_content_size(app: &Rc<App>, id: NodeId, hwnd: HWND) {
    let lines = send(hwnd, EM_GETLINECOUNT, 0, 0).0 as i32;
    let size = app
        .with_state(|state| {
            let host = state.scene.views.get_mut(&id)?.input.as_mut()?;
            if !host.multiline || lines == host.lines {
                return None;
            }
            host.lines = lines;
            let placement = host.placement?;
            let line_height = host
                .resources
                .as_ref()
                .map_or(16, |resources| resources.line_height);
            let width = (placement.rect.width - host.config.insets.horizontal()).max(0.0);
            Some(Size::new(
                width,
                lines.max(1) as f32 * line_height as f32 / placement.scale,
            ))
        })
        .flatten();
    if let Some(size) = size {
        app.emit_later(BackendEvent::Input {
            id,
            event: InputEvent::ContentSizeChanged { size },
        });
    }
}

/// `WM_COMMAND` notification from a control (sent to its parent).
pub fn notification(app: &Rc<App>, hwnd: HWND, code: u32) {
    if code != EN_CHANGE {
        return;
    }
    let Some(id) = node_of(hwnd) else { return };
    let control_text = read_text(hwnd);
    let (_, caret) = read_selection(hwnd);
    let report = app
        .with_state(|state| {
            let host = state.scene.views.get_mut(&id)?.input.as_mut()?;
            let text = protocol_text(&control_text, host.multiline);
            let caret = from_control_offset(&control_text, caret, host.multiline);
            let emptiness_changed = host.text.is_empty() != text.is_empty();
            host.text.clone_from(&text);
            if !host.updating {
                // A change made by code keeps the selection it asked for.
                host.selection = (caret, caret);
            }
            Some((host.updating, text, caret, emptiness_changed))
        })
        .flatten();
    let Some((updating, text, caret, emptiness_changed)) = report else {
        return;
    };
    if emptiness_changed {
        // SAFETY: invalidating a live control.
        unsafe {
            let _ = InvalidateRect(Some(hwnd), None, true);
        }
    }
    report_content_size(app, id, hwnd);
    if !updating {
        app.emit_later(BackendEvent::Input {
            id,
            event: InputEvent::Edited {
                text,
                selection: (caret, caret),
            },
        });
    }
}

/// `WM_CTLCOLOREDIT` / `WM_CTLCOLORSTATIC`: the colors the control paints with.
pub fn control_colors(app: &Rc<App>, hwnd: HWND, wparam: WPARAM) -> Option<LRESULT> {
    let id = node_of(hwnd)?;
    let dc = HDC(wparam.0 as *mut _);
    app.with_state(|state| {
        let background = background_of(state, id);
        let default_text = state.theme.text;
        let host = state.scene.views.get_mut(&id)?.input.as_mut()?;
        let text = host.config.color.unwrap_or(default_text);
        let resources = host.resources.as_mut()?;
        let wanted = colorref(background);
        // SAFETY: GDI calls on the DC the control passed for this message;
        // the brush is owned by the host and outlives the paint.
        unsafe {
            if resources.brush.is_invalid() || resources.brush_color != wanted {
                if !resources.brush.is_invalid() {
                    let _ = DeleteObject(HGDIOBJ(resources.brush.0));
                }
                resources.brush = CreateSolidBrush(wanted);
                resources.brush_color = wanted;
            }
            SetTextColor(dc, colorref(text));
            SetBkColor(dc, wanted);
        }
        let brush = resources.brush;
        host.painted = Some((wanted, colorref(text)));
        Some(LRESULT(brush.0 as isize))
    })
    .flatten()
}

fn paint_placeholder(app: &Rc<App>, hwnd: HWND, id: NodeId) {
    let wanted = app
        .with_state(|state| {
            let background = background_of(state, id);
            let default_text = state.theme.text;
            let host = state.scene.views.get(&id)?.input.as_ref()?;
            if !host.text.is_empty() || host.config.placeholder.is_empty() {
                return None;
            }
            // Without an authored color the hint is a dimmed version of the text.
            let color = host.config.placeholder_color.unwrap_or_else(|| {
                background.lerp(host.config.color.unwrap_or(default_text), 0.45)
            });
            Some((
                host.config.placeholder.clone(),
                color,
                host.config.align,
                host.multiline,
                host.resources.as_ref()?.font,
            ))
        })
        .flatten();
    let Some((placeholder, color, align, multiline, font)) = wanted else {
        return;
    };
    let mut text: Vec<u16> = placeholder.encode_utf16().collect();
    // SAFETY: GDI calls on a DC of the live control, released before returning.
    unsafe {
        let mut rect = RECT::default();
        SendMessageW(
            hwnd,
            EM_GETRECT,
            Some(WPARAM(0)),
            Some(LPARAM(std::ptr::from_mut(&mut rect) as isize)),
        );
        if rect.right <= rect.left {
            let _ = GetClientRect(hwnd, &mut rect);
        }
        let dc = GetDC(Some(hwnd));
        let previous = SelectObject(dc, HGDIOBJ(font.0));
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, colorref(color));
        let alignment = match align {
            TextAlign::Start | TextAlign::Justify => DT_LEFT,
            TextAlign::Center => DT_CENTER,
            TextAlign::End => DT_RIGHT,
        };
        let layout = if multiline {
            DT_WORDBREAK | DT_EDITCONTROL
        } else {
            DT_SINGLELINE
        };
        DrawTextW(dc, &mut text, &mut rect, DT_NOPREFIX | alignment | layout);
        SelectObject(dc, previous);
        ReleaseDC(Some(hwnd), dc);
    }
}

/// Reports a changed selection after a message that may have moved it.
fn check_selection(app: &Rc<App>, hwnd: HWND, id: NodeId) {
    let (start, end) = read_selection(hwnd);
    let multiline = app
        .with_state(|state| {
            state
                .scene
                .views
                .get(&id)
                .and_then(|view| view.input.as_ref())
                .is_some_and(|host| host.multiline)
        })
        .unwrap_or(false);
    // Offsets of a multi-line control count CR LF line breaks.
    let (start, end) = if multiline {
        let control_text = read_text(hwnd);
        (
            from_control_offset(&control_text, start, true),
            from_control_offset(&control_text, end, true),
        )
    } else {
        (start, end)
    };
    let changed = app
        .with_state(|state| {
            let host = state.scene.views.get_mut(&id)?.input.as_mut()?;
            if host.selection == (start, end) {
                return None;
            }
            host.selection = (start, end);
            (!host.updating).then_some((start, end))
        })
        .flatten();
    if let Some((start, end)) = changed {
        app.emit_later(BackendEvent::Input {
            id,
            event: InputEvent::SelectionChanged { start, end },
        });
    }
}

/// An empty control shows its placeholder, which the control knows
/// nothing about: whenever it may have drawn over it, it paints again.
fn refresh_placeholder(app: &Rc<App>, hwnd: HWND, id: NodeId) {
    let shows = app
        .with_state(|state| {
            state
                .scene
                .views
                .get(&id)
                .and_then(|view| view.input.as_ref())
                .is_some_and(|host| host.text.is_empty() && !host.config.placeholder.is_empty())
        })
        .unwrap_or(false);
    if shows {
        // SAFETY: invalidating a live control.
        unsafe {
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
    }
}

/// Reports a focus change of the control and returns what the control
/// does on focus: (select everything, hide the caret).
fn focus_changed(app: &Rc<App>, id: NodeId, focused: bool) -> (bool, bool) {
    crate::focus::native_changed(app, id, focused);
    app.with_state(|state| {
        state
            .scene
            .views
            .get(&id)
            .and_then(|view| view.input.as_ref())
            .map_or((false, false), |host| {
                (host.config.select_on_focus, host.config.caret_hidden)
            })
    })
    .unwrap_or((false, false))
}

/// Editing chords the control implements itself.
fn is_edit_chord(key: u32, control: bool, shift: bool) -> bool {
    const HOME: u32 = 0x24;
    const END: u32 = 0x23;
    const LEFT: u32 = 0x25;
    const DOWN: u32 = 0x28;
    const INSERT: u32 = 0x2D;
    const DELETE: u32 = 0x2E;
    if control {
        return matches!(
            key,
            0x41 | 0x43 | 0x56 | 0x58 | 0x59 | 0x5A | HOME | END | INSERT | DELETE
        ) || (LEFT..=DOWN).contains(&key)
            || key == u32::from(VK_BACK.0);
    }
    shift && matches!(key, INSERT | DELETE)
}

/// Subclass procedure of every input control.
unsafe extern "system" fn edit_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _subclass: usize,
    _data: usize,
) -> LRESULT {
    let handled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        edit_message(hwnd, message, wparam, lparam)
    }));
    match handled {
        Ok(Some(result)) => result,
        // SAFETY: forwarding the unmodified message down the subclass chain.
        _ => unsafe { DefSubclassProc(hwnd, message, wparam, lparam) },
    }
}

fn default(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: called from the subclass procedure of `hwnd` with its own message.
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

fn edit_message(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    if message == WM_NCDESTROY {
        CONTROLS.with(|controls| controls.borrow_mut().remove(&(hwnd.0 as isize)));
        // SAFETY: removing the subclass installed at creation.
        unsafe {
            let _ = RemoveWindowSubclass(hwnd, Some(edit_proc), SUBCLASS_ID);
        }
        return None;
    }
    let app = App::current()?;
    let id = node_of(hwnd)?;
    match message {
        WM_SETFOCUS => {
            let (select_all, caret_hidden) = focus_changed(&app, id, true);
            let result = default(hwnd, message, wparam, lparam);
            refresh_placeholder(&app, hwnd, id);
            // SAFETY: the control is live. Selecting is posted so it runs
            // after the click that focused the control placed the caret.
            unsafe {
                if caret_hidden {
                    let _ = HideCaret(Some(hwnd));
                }
                if select_all {
                    let _ = PostMessageW(Some(hwnd), EM_SETSEL, WPARAM(0), LPARAM(-1));
                }
            }
            Some(result)
        }
        WM_KILLFOCUS => {
            focus_changed(&app, id, false);
            let result = default(hwnd, message, wparam, lparam);
            refresh_placeholder(&app, hwnd, id);
            Some(result)
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            let key = wparam.0 as u32;
            let modifiers = events::modifiers();
            let (multiline, submit_on_enter) = app
                .with_state(|state| {
                    state
                        .scene
                        .views
                        .get(&id)?
                        .input
                        .as_ref()
                        .map(|host| (host.multiline, host.config.submit_on_enter))
                })
                .flatten()?;
            let chord = modifiers.control || modifiers.alt || modifiers.meta;
            if key == u32::from(VK_TAB.0) && !chord {
                // Tab moves the focus; an EDIT would insert it or ignore it.
                // SAFETY: plain query on a live control.
                let root = unsafe { GetAncestor(hwnd, GA_ROOT) };
                if let Some(window) = app
                    .with_state(|state| {
                        state
                            .scene
                            .windows
                            .iter()
                            .find(|(_, entry)| entry.hwnd == root)
                            .map(|(window, _)| *window)
                    })
                    .flatten()
                {
                    app.with_state(|state| state.scene.focus_visible = true);
                    crate::focus::advance(&app, window, !modifiers.shift);
                }
                return Some(LRESULT(0));
            }
            if key == u32::from(VK_RETURN.0) && !chord {
                app.emit_later(BackendEvent::Input {
                    id,
                    event: InputEvent::KeyPressed {
                        key: "Enter".into(),
                    },
                });
                if !multiline || (submit_on_enter && !modifiers.shift) {
                    app.emit_later(BackendEvent::Input {
                        id,
                        event: InputEvent::Submitted,
                    });
                    return Some(LRESULT(0));
                }
            } else if key == u32::from(VK_BACK.0) && !chord {
                app.emit_later(BackendEvent::Input {
                    id,
                    event: InputEvent::KeyPressed {
                        key: "Backspace".into(),
                    },
                });
            }
            // Keys the control has no use for go to the window: Escape,
            // function keys and chords that are not editing commands.
            let unused = key == u32::from(VK_ESCAPE.0)
                || (u32::from(VK_F1.0)..=u32::from(VK_F24.0)).contains(&key)
                || (chord
                    && !events::is_modifier(key)
                    && !is_edit_chord(key, modifiers.control && !modifiers.alt, modifiers.shift));
            if unused {
                crate::window::key_from_child(&app, hwnd, true, wparam, lparam);
                if message == WM_SYSKEYDOWN {
                    // Alt chords keep their system meaning (menus, Alt+F4).
                    return None;
                }
                return Some(LRESULT(0));
            }
            if modifiers.control && !modifiers.alt && key == 0x41 {
                // Select all, also where the control does not implement it.
                send(hwnd, EM_SETSEL, 0, -1);
                check_selection(&app, hwnd, id);
                return Some(LRESULT(0));
            }
            let result = default(hwnd, message, wparam, lparam);
            check_selection(&app, hwnd, id);
            Some(result)
        }
        WM_KEYUP => {
            let key = wparam.0 as u32;
            if key == u32::from(VK_ESCAPE.0)
                || (u32::from(VK_F1.0)..=u32::from(VK_F24.0)).contains(&key)
            {
                crate::window::key_from_child(&app, hwnd, false, wparam, lparam);
            }
            let result = default(hwnd, message, wparam, lparam);
            check_selection(&app, hwnd, id);
            Some(result)
        }
        WM_CHAR => {
            let unit = wparam.0 as u32;
            let (multiline, submit_on_enter) = app
                .with_state(|state| {
                    state
                        .scene
                        .views
                        .get(&id)?
                        .input
                        .as_ref()
                        .map(|host| (host.multiline, host.config.submit_on_enter))
                })
                .flatten()?;
            let modifiers = events::modifiers();
            match unit {
                // Tab and Escape were handled as keys; a single-line control
                // would only beep at them and at Enter.
                0x09 | 0x1B => return Some(LRESULT(0)),
                0x0D if !multiline || (submit_on_enter && !modifiers.shift) => {
                    return Some(LRESULT(0));
                }
                // Control+Enter (a line feed) and Control+Backspace (DEL)
                // were reported as keys; the control would insert them.
                0x0A | 0x7F => return Some(LRESULT(0)),
                // Control characters of chords that were reported as keys
                // (copy, paste, cut, undo and redo are the control's own).
                0x01..=0x1F
                    if unit != 0x08
                        && unit != 0x0D
                        && !matches!(unit, 0x03 | 0x16 | 0x18 | 0x1A | 0x19) =>
                {
                    return Some(LRESULT(0));
                }
                _ => {}
            }
            if unit >= 0x20 {
                // A character outside the basic plane arrives as two
                // messages, one per surrogate half: the pair is one key.
                let unit = unit as u16;
                let typed = app
                    .with_state(|state| {
                        let host = state.scene.views.get_mut(&id)?.input.as_mut()?;
                        if (0xD800..0xDC00).contains(&unit) {
                            host.high_surrogate = Some(unit);
                            return None;
                        }
                        let units = match host.high_surrogate.take() {
                            Some(high) if (0xDC00..0xE000).contains(&unit) => vec![high, unit],
                            _ => vec![unit],
                        };
                        String::from_utf16(&units).ok()
                    })
                    .flatten();
                if let Some(key) = typed {
                    app.emit_later(BackendEvent::Input {
                        id,
                        event: InputEvent::KeyPressed { key },
                    });
                }
            }
            let result = default(hwnd, message, wparam, lparam);
            check_selection(&app, hwnd, id);
            refresh_placeholder(&app, hwnd, id);
            Some(result)
        }
        WM_LBUTTONDOWN | WM_LBUTTONUP | WM_LBUTTONDBLCLK | WM_RBUTTONDOWN | WM_RBUTTONUP
        | WM_MOUSEMOVE => {
            // The core keeps hover and press state for the node under the control.
            let (phase, button) = match message {
                WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => (PointerPhase::Down, PointerButton::Primary),
                WM_LBUTTONUP => (PointerPhase::Up, PointerButton::Primary),
                WM_RBUTTONDOWN => (PointerPhase::Down, PointerButton::Secondary),
                WM_RBUTTONUP => (PointerPhase::Up, PointerButton::Secondary),
                _ => (PointerPhase::Move, PointerButton::Primary),
            };
            // The control's own context menu answers the secondary button.
            if button == PointerButton::Primary {
                crate::window::pointer_from_child(&app, hwnd, phase, button, lparam);
            }
            if message == WM_MOUSEMOVE {
                // The window's own leave tracking ended when the pointer
                // entered the control: the control watches for the exit.
                let arm = app
                    .with_state(|state| {
                        state
                            .scene
                            .views
                            .get_mut(&id)
                            .and_then(|view| view.input.as_mut())
                            .is_some_and(|host| !std::mem::replace(&mut host.tracking_leave, true))
                    })
                    .unwrap_or(false);
                if arm {
                    let mut track = TRACKMOUSEEVENT {
                        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    // SAFETY: the structure is a live local with its size set.
                    unsafe {
                        let _ = TrackMouseEvent(&mut track);
                    }
                }
            }
            let result = default(hwnd, message, wparam, lparam);
            if message != WM_MOUSEMOVE || wparam.0 & 0x0001 != 0 {
                check_selection(&app, hwnd, id);
                refresh_placeholder(&app, hwnd, id);
            }
            Some(result)
        }
        WM_MOUSELEAVE => {
            app.with_state(|state| {
                if let Some(host) = state
                    .scene
                    .views
                    .get_mut(&id)
                    .and_then(|view| view.input.as_mut())
                {
                    host.tracking_leave = false;
                }
            });
            crate::window::pointer_left_child(&app, hwnd);
            None
        }
        WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
            let multiline = app
                .with_state(|state| {
                    state
                        .scene
                        .views
                        .get(&id)?
                        .input
                        .as_ref()
                        .map(|host| host.multiline)
                })
                .flatten()?;
            // A field that does not scroll (a single line, or an editor
            // already at its end) leaves the wheel to the scroll view
            // around it.
            let before = send(hwnd, EM_GETFIRSTVISIBLELINE, 0, 0);
            let result = if multiline {
                default(hwnd, message, wparam, lparam)
            } else {
                LRESULT(0)
            };
            if !multiline || send(hwnd, EM_GETFIRSTVISIBLELINE, 0, 0) == before {
                // SAFETY: posting the same wheel message to the top-level window.
                unsafe {
                    let _ = PostMessageW(Some(GetAncestor(hwnd, GA_ROOT)), message, wparam, lparam);
                }
            }
            Some(result)
        }
        WM_CONTEXTMENU => {
            let hidden = app
                .with_state(|state| {
                    state
                        .scene
                        .views
                        .get(&id)?
                        .input
                        .as_ref()
                        .map(|host| host.config.context_menu_hidden)
                })
                .flatten()?;
            hidden.then_some(LRESULT(0))
        }
        WM_PAINT => {
            let result = default(hwnd, message, wparam, lparam);
            paint_placeholder(&app, hwnd, id);
            Some(result)
        }
        _ => None,
    }
}

//! Top-level windows: one `HWND` per `WindowId`, per-monitor DPI aware,
//! rendered as a whole with Direct2D. The window procedure translates raw
//! input into `BackendEvent`s; it never interprets the scene beyond what
//! scrolling, cursors and tooltips need.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::Rc;

use pam_desktop_ui::backend::{
    BackendEvent, GesturePhase, KeyEvent, NodeId, PointerButton, PointerEvent, PointerPhase, WindowConfig, WindowId, WindowMetrics,
};
use pam_desktop_ui::geometry::{Point, Rect, Size};
use pam_desktop_ui::shell::{Cursor, WindowCommand, WindowState};
use windows::core::{w, PCWSTR, PWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    ClientToScreen, GetMonitorInfoW, InvalidateRect, MonitorFromWindow, ScreenToClient, ValidateRect, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::SystemServices::{
    APPCOMMAND_BROWSER_BACKWARD, GC_PAN, GC_PAN_WITH_GUTTER, GC_PAN_WITH_INERTIA, GC_PAN_WITH_SINGLE_FINGER_HORIZONTALLY, GC_PAN_WITH_SINGLE_FINGER_VERTICALLY,
    GC_ROTATE,
};
use windows::Win32::UI::Controls::{
    TOOLTIPS_CLASSW, TTM_ADDTOOLW, TTM_NEWTOOLRECTW, TTM_POP, TTM_RELAYEVENT, TTM_SETMAXTIPWIDTH, TTM_UPDATETIPTEXTW, TTS_ALWAYSTIP, TTS_NOPREFIX,
    TTTOOLINFOW, WM_MOUSELEAVE,
};
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetCapture, GetDoubleClickTime, GetFocus, ReleaseCapture, SetCapture, SetFocus, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
};
use windows::Win32::UI::Input::Touch::{CloseGestureInfoHandle, GetGestureInfo, SetGestureConfig, GESTURECONFIG, GESTUREINFO, GID_PAN, GID_ROTATE, GID_ZOOM, HGESTUREINFO};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetCursorPos, GetMessageTime, GetSystemMetrics, GetWindowLongPtrW,
    GetWindowPlacement, GetWindowRect, IsChild, IsIconic, IsWindow, IsZoomed, LoadCursorW, PostQuitMessage, RegisterClassExW,
    SendMessageW, SetCursor, SetForegroundWindow, SetWindowDisplayAffinity, SetWindowLongPtrW, SetWindowPlacement, SetWindowPos, SetWindowTextW, ShowWindow,
    SystemParametersInfoW, WindowFromPoint, CREATESTRUCTW, CW_USEDEFAULT, GF_BEGIN, GF_END, GWLP_USERDATA, GWL_STYLE, HTBOTTOM, HTBOTTOMLEFT,
    HTBOTTOMRIGHT, HTCLIENT, HTLEFT, HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT, HWND_NOTOPMOST, HWND_TOP, HWND_TOPMOST, IDC_ARROW, IDC_CROSS,
    IDC_HAND, IDC_IBEAM, IDC_NO, IDC_SIZEALL, IDC_SIZENS, IDC_SIZEWE, IDC_WAIT, MINMAXINFO, MSG, NCCALCSIZE_PARAMS,
    SIZE_MINIMIZED, SM_CXDOUBLECLK, SM_CXFRAME, SM_CXPADDEDBORDER, SM_CYDOUBLECLK, SM_CYFRAME, SPI_GETWHEELSCROLLLINES, SWP_FRAMECHANGED,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE, SW_MAXIMIZE, SW_MINIMIZE, SW_RESTORE, SW_SHOW,
    SW_SHOWMAXIMIZED, SW_SHOWNORMAL, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, WA_INACTIVE, WDA_EXCLUDEFROMCAPTURE, WDA_MONITOR, WDA_NONE, WHEEL_DELTA, WINDOWPLACEMENT, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_ACTIVATE, WM_APPCOMMAND, WM_CAPTURECHANGED, WM_CHAR, WM_CLOSE, WM_COMMAND, WM_CTLCOLOREDIT, WM_CTLCOLORSTATIC,
    WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND, WM_GESTURE, WM_GESTURENOTIFY, WM_GETMINMAXINFO, WM_GETOBJECT, WM_KEYDOWN, WM_KEYUP,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCCALCSIZE, WM_NCCREATE,
    WM_NCHITTEST, WM_NCLBUTTONDOWN, WM_NOTIFY, WM_PAINT, WM_PARENTNOTIFY, WM_MOVE, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SETCURSOR, WM_SETFOCUS, WM_SETTINGCHANGE, WM_SIZE,
    WM_SYSCHAR, WM_SYSKEYDOWN, WM_SYSKEYUP, WM_XBUTTONUP, WNDCLASSEXW, WS_CLIPCHILDREN, WS_MAXIMIZEBOX,
    WS_OVERLAPPEDWINDOW, WS_POPUP, WS_THICKFRAME, XBUTTON1, HTCAPTION, WM_NCACTIVATE,
};

use crate::app::{App, State};
use crate::events;
use crate::gfx::Surface;
use crate::scroll::{BarDrag, LINE_EXTENT};
use crate::util::{from_wide, hiword, loword, lparam_point, pixel_rect, wide};

const WINDOW_CLASS: PCWSTR = w!("PamDesktopWindow");
/// `WHEEL_PAGESCROLL`: the wheel scrolls by pages.
const PAGE_SCROLL: u32 = u32::MAX;

/// What must be presented again.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Damage {
    Clean,
    Region(Rect),
    All,
}

#[derive(Clone, Copy)]
struct Click {
    time: i32,
    x: i32,
    y: i32,
    button: PointerButton,
    count: u8,
}

/// Style and placement to return to when leaving full screen.
pub struct SavedPlacement {
    style: isize,
    placement: WINDOWPLACEMENT,
}

pub struct WindowEntry {
    pub hwnd: HWND,
    pub surface: Option<Surface>,
    pub dpi: u32,
    /// Client area in logical units.
    pub size: Size,
    pub pixel_size: (u32, u32),
    pub root: Option<NodeId>,
    /// Back to front.
    pub overlays: Vec<NodeId>,
    /// The window was shown (its first root arrived).
    pub presented: bool,
    pub dirty: Damage,
    /// Child windows may need to move.
    pub natives_dirty: bool,
    pub decorated: bool,
    pub resizable: bool,
    pub min_size: Option<Size>,
    /// The window disabled while this one is open.
    pub modal_owner: Option<HWND>,
    pub fullscreen: Option<SavedPlacement>,
    pub active: bool,
    /// The state last reported to the core.
    pub state: WindowState,
    /// Deepest view under the pointer.
    pub hover: Option<NodeId>,
    pub cursor: Cursor,
    /// The view whose tooltip is armed.
    pub tooltip_node: Option<NodeId>,
    pub tooltip: HWND,
    pub pointer: Point,
    tracking_leave: bool,
    /// Bit per pressed button.
    buttons: u8,
    last_click: Option<Click>,
    press_count: u8,
    pub bar_drag: Option<BarDrag>,
    /// Reused by hit tests.
    pub path: Vec<NodeId>,
    /// The child that had keyboard focus when the window was deactivated.
    focus_restore: Option<HWND>,
    /// Finger distance at the start of a zoom gesture.
    gesture_start: f64,
    /// Where the fingers were at the previous step of a pan gesture.
    pan_from: Option<(i32, i32)>,
    /// The UI Automation root of the window.
    pub automation: Option<windows::Win32::UI::Accessibility::IRawElementProviderSimple>,
}

impl WindowEntry {
    pub fn scale(&self) -> f32 {
        self.dpi as f32 / 96.0
    }

    /// Marks a region (or everything) for the next frame and asks for a paint.
    pub fn damage(&mut self, region: Option<Rect>) {
        self.dirty = match (self.dirty, region) {
            (Damage::All, _) | (_, None) => Damage::All,
            (Damage::Clean, Some(region)) => Damage::Region(region),
            (Damage::Region(known), Some(region)) => Damage::Region(known.union(region)),
        };
        // SAFETY: invalidating only queues a paint; it sends no message.
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }
}

fn window_style(resizable: bool) -> WINDOW_STYLE {
    let mut style = WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN;
    if !resizable {
        style &= !(WS_THICKFRAME | WS_MAXIMIZEBOX);
    }
    style
}

pub fn register_class(app: &App) {
    let class = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(window_proc),
        hInstance: app.instance,
        // SAFETY: loading a stock system cursor.
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default(),
        // SAFETY: the first icon resource of the executable, when it has one.
        hIcon: unsafe { windows::Win32::UI::WindowsAndMessaging::LoadIconW(Some(app.instance), PCWSTR(1 as _)) }.unwrap_or_default(),
        lpszClassName: WINDOW_CLASS,
        ..Default::default()
    };
    // SAFETY: the class structure is fully initialized; a second
    // registration fails harmlessly.
    unsafe { RegisterClassExW(&class) };
}

/// Outer window size in pixels for a client area of `size` logical units.
fn outer_size(size: Size, style: WINDOW_STYLE, menu: bool, decorated: bool, dpi: u32) -> (i32, i32) {
    let scale = dpi as f32 / 96.0;
    let mut rect = RECT { left: 0, top: 0, right: (size.width * scale).round() as i32, bottom: (size.height * scale).round() as i32 };
    if decorated {
        // SAFETY: the rectangle is a live local.
        unsafe {
            let _ = AdjustWindowRectExForDpi(&mut rect, style, menu, WINDOW_EX_STYLE(0), dpi);
        }
    }
    (rect.right - rect.left, rect.bottom - rect.top)
}

fn client_size(hwnd: HWND) -> (u32, u32) {
    let mut rect = RECT::default();
    // SAFETY: the rectangle is a live local.
    unsafe {
        let _ = GetClientRect(hwnd, &mut rect);
    }
    ((rect.right - rect.left).max(0) as u32, (rect.bottom - rect.top).max(0) as u32)
}

fn has_menu(hwnd: HWND) -> bool {
    // SAFETY: plain query on a window handle.
    unsafe { !windows::Win32::UI::WindowsAndMessaging::GetMenu(hwnd).is_invalid() }
}

/// Creates (or reconfigures) a window. Runs outside any state borrow.
pub fn open(app: &Rc<App>, window: WindowId, config: &WindowConfig) {
    let title = wide(&config.title);
    let existing = app.with_state(|state| state.scene.windows.get(&window).map(|entry| entry.hwnd)).flatten();
    if let Some(hwnd) = existing {
        // SAFETY: `hwnd` is a live window of this thread; the title is NUL-terminated.
        unsafe {
            let _ = SetWindowTextW(hwnd, PCWSTR(title.as_ptr()));
        }
        command(app, window, &WindowCommand::SetResizable(config.resizable));
        command(app, window, &WindowCommand::SetMinSize(config.min_size));
        // The main window keeps the size the user gave it.
        if window != pam_desktop_ui::backend::MAIN_WINDOW {
            command(app, window, &WindowCommand::SetSize(config.size));
        }
        return;
    }
    let owner = config.parent.and_then(|parent| app.with_state(|state| state.scene.windows.get(&parent).map(|entry| entry.hwnd)).flatten());
    let style = window_style(config.resizable);
    // SAFETY: the class is registered, the strings are NUL-terminated and
    // the creation parameter is an integer, not a pointer that is read.
    let created = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            WINDOW_CLASS,
            PCWSTR(title.as_ptr()),
            style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            owner,
            None,
            Some(app.instance),
            Some(window as usize as *const _),
        )
    };
    let Ok(hwnd) = created else { return };
    // SAFETY: `hwnd` was just created on this thread.
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
    let tooltip = create_tooltip(app, hwnd);
    let entry = WindowEntry {
        hwnd,
        surface: None,
        dpi,
        size: config.size,
        pixel_size: (0, 0),
        root: None,
        overlays: Vec::new(),
        presented: false,
        dirty: Damage::All,
        natives_dirty: false,
        decorated: config.decorated,
        resizable: config.resizable,
        min_size: config.min_size,
        modal_owner: owner.filter(|_| config.modal),
        fullscreen: None,
        active: false,
        state: WindowState::default(),
        hover: None,
        cursor: Cursor::Default,
        tooltip_node: None,
        tooltip,
        pointer: Point::ZERO,
        tracking_leave: false,
        buttons: 0,
        last_click: None,
        press_count: 1,
        bar_drag: None,
        path: Vec::new(),
        focus_restore: None,
        gesture_start: 1.0,
        pan_from: None,
        automation: None,
    };
    let opened = app.with_state(|state| {
        state.scene.windows.insert(window, entry);
        (state.dark(), state.content_protected)
    });
    let (dark, protected) = opened.unwrap_or((false, false));
    if protected {
        protect_content(hwnd, true);
    }
    // Now that the entry exists the frame is computed by our own rules
    // (custom frame, minimum size), at the size the application asked for.
    let (width, height) = outer_size(config.size, style, false, config.decorated, dpi);
    // SAFETY: `hwnd` is live; these calls dispatch messages, and no state borrow is held.
    unsafe {
        let _ = SetWindowPos(hwnd, None, 0, 0, width, height, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
    }
    if owner.is_some() {
        command(app, window, &WindowCommand::Center);
    }
    let pixel_size = client_size(hwnd);
    app.with_state(|state| {
        if let Some(entry) = state.scene.windows.get_mut(&window) {
            let scale = entry.scale();
            entry.pixel_size = pixel_size;
            entry.size = Size::new(pixel_size.0 as f32 / scale, pixel_size.1 as f32 / scale);
        }
    });
    crate::shell::apply_title_bar_theme(hwnd, dark);
    crate::dragdrop::register(app, hwnd, window);
}

fn create_tooltip(app: &App, owner: HWND) -> HWND {
    // SAFETY: standard creation of a tooltip control owned by `owner`; the
    // tool structure is live for the calls that read it.
    unsafe {
        let Ok(tooltip) = CreateWindowExW(
            windows::Win32::UI::WindowsAndMessaging::WS_EX_TOPMOST,
            TOOLTIPS_CLASSW,
            PCWSTR::null(),
            WS_POPUP | WINDOW_STYLE(TTS_NOPREFIX | TTS_ALWAYSTIP),
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            Some(owner),
            None,
            Some(app.instance),
            None,
        ) else {
            return HWND::default();
        };
        let mut empty = [0_u16; 1];
        let tool = tool_info(owner, RECT::default(), PWSTR(empty.as_mut_ptr()));
        SendMessageW(tooltip, TTM_ADDTOOLW, Some(WPARAM(0)), Some(LPARAM(std::ptr::from_ref(&tool) as isize)));
        // A maximum width makes the control wrap long text and honor line breaks.
        SendMessageW(tooltip, TTM_SETMAXTIPWIDTH, Some(WPARAM(0)), Some(LPARAM(480)));
        tooltip
    }
}

fn tool_info(owner: HWND, rect: RECT, text: PWSTR) -> TTTOOLINFOW {
    TTTOOLINFOW {
        // The size without the trailing reserved pointer is accepted by
        // every version of the common controls.
        cbSize: (size_of::<TTTOOLINFOW>() - size_of::<*mut core::ffi::c_void>()) as u32,
        hwnd: owner,
        uId: 1,
        rect,
        lpszText: text,
        ..Default::default()
    }
}

/// Points the window's single tooltip tool at a view (`None` disarms it).
fn update_tooltip(tooltip: HWND, owner: HWND, target: Option<(&str, RECT)>) {
    if tooltip.is_invalid() {
        return;
    }
    let mut text = wide(target.map_or("", |(text, _)| text));
    let rect = target.map_or(RECT::default(), |(_, rect)| rect);
    let tool = tool_info(owner, rect, PWSTR(text.as_mut_ptr()));
    let tool_pointer = LPARAM(std::ptr::from_ref(&tool) as isize);
    // SAFETY: the tool structure and its text outlive the synchronous sends.
    unsafe {
        SendMessageW(tooltip, TTM_POP, Some(WPARAM(0)), Some(LPARAM(0)));
        SendMessageW(tooltip, TTM_UPDATETIPTEXTW, Some(WPARAM(0)), Some(tool_pointer));
        SendMessageW(tooltip, TTM_NEWTOOLRECTW, Some(WPARAM(0)), Some(tool_pointer));
    }
}

/// Lets the tooltip control see the mouse message so it runs its delays.
fn relay_to_tooltip(tooltip: HWND, hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) {
    if tooltip.is_invalid() {
        return;
    }
    let relayed = MSG { hwnd, message, wParam: wparam, lParam: lparam, ..Default::default() };
    // SAFETY: the message structure outlives the synchronous send.
    unsafe {
        SendMessageW(tooltip, TTM_RELAYEVENT, Some(WPARAM(0)), Some(LPARAM(std::ptr::from_ref(&relayed) as isize)));
    }
}

fn protect_content(hwnd: HWND, enabled: bool) -> bool {
    // SAFETY: plain calls on a live window of this process.
    unsafe {
        if !enabled {
            return SetWindowDisplayAffinity(hwnd, WDA_NONE).is_ok();
        }
        // Excluded from captures where the system can (Windows 10 2004),
        // shown as a black rectangle before that.
        SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE).is_ok() || SetWindowDisplayAffinity(hwnd, WDA_MONITOR).is_ok()
    }
}

/// `Backend::set_content_protection`: applies to every window, now and
/// when new ones open. Answers whether the system enforces it.
pub fn set_content_protection(app: &Rc<App>, enabled: bool) -> bool {
    let windows: Vec<HWND> = app
        .with_state(|state| {
            state.content_protected = enabled;
            state.scene.windows.values().map(|entry| entry.hwnd).collect()
        })
        .unwrap_or_default();
    let mut enforced = !windows.is_empty();
    for hwnd in windows {
        enforced &= protect_content(hwnd, enabled);
    }
    enabled && enforced
}

/// Destroys a window. Runs outside any state borrow.
pub fn close(app: &Rc<App>, window: WindowId) {
    let found = app.with_state(|state| state.scene.windows.get(&window).map(|entry| (entry.hwnd, entry.modal_owner))).flatten();
    let Some((hwnd, modal_owner)) = found else { return };
    // SAFETY: handles of live windows of this thread. The owner is enabled
    // before the dialog goes away so activation returns to it.
    unsafe {
        if let Some(owner) = modal_owner {
            let _ = EnableWindow(owner, true);
        }
        let _ = DestroyWindow(hwnd);
    }
}

pub fn metrics(app: &App, window: WindowId) -> WindowMetrics {
    let known = app.with_state(|state| {
        state.scene.windows.get(&window).map(|entry| (entry.hwnd, entry.size, entry.scale(), state.system_dark, state.reduced_motion))
    });
    let Some(Some((hwnd, size, scale, dark, reduced_motion))) = known else { return WindowMetrics::default() };
    WindowMetrics { size, scale, font_scale: crate::shell::text_scale(), dark, refresh_rate: crate::shell::refresh_rate(hwnd), reduced_motion }
}

/// Shows the window once its first root is attached. The first frame is
/// drawn before the window appears, from the message loop: by then the
/// drain that attached the root has also placed every view.
pub fn present(app: &Rc<App>, window: WindowId) {
    let first = app.with_state(|state| {
        let entry = state.scene.windows.get_mut(&window)?;
        if entry.presented || entry.root.is_none() {
            return None;
        }
        entry.presented = true;
        Some(())
    });
    if first != Some(Some(())) {
        return;
    }
    app.defer(Box::new(move |app| {
        let Some((hwnd, modal_owner)) = app.with_state(|state| state.scene.windows.get(&window).map(|entry| (entry.hwnd, entry.modal_owner))).flatten() else {
            return;
        };
        paint(app, hwnd, window);
        // SAFETY: live windows of this thread; no state borrow is held while
        // the calls dispatch messages.
        unsafe {
            if let Some(owner) = modal_owner {
                let _ = EnableWindow(owner, false);
            }
            let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
        }
        // Controls created for the first frame take a pending focus now
        // that the window is visible and active.
        app.with_state(|state| {
            state.scene.damage_all(window);
            if let Some(entry) = state.scene.windows.get_mut(&window) {
                entry.natives_dirty = true;
            }
            state.resume_frames();
        });
    }));
}

fn current_state(entry: &WindowEntry) -> WindowState {
    // SAFETY: plain queries on a live window.
    let (maximized, minimized) = unsafe { (IsZoomed(entry.hwnd).as_bool(), IsIconic(entry.hwnd).as_bool()) };
    WindowState { maximized, minimized, full_screen: entry.fullscreen.is_some(), focused: entry.active }
}

pub fn state_of(app: &App, window: WindowId) -> WindowState {
    app.with_state(|state| state.scene.windows.get(&window).map(current_state)).flatten().unwrap_or_default()
}

/// Reports the window state when it differs from the last report.
fn report_state(app: &App, window: WindowId) {
    let changed = app
        .with_state(|state| {
            let entry = state.scene.windows.get_mut(&window)?;
            let now = current_state(entry);
            let differs = (now.maximized, now.minimized, now.full_screen) != (entry.state.maximized, entry.state.minimized, entry.state.full_screen);
            entry.state = now;
            differs.then_some(now)
        })
        .flatten();
    if let Some(state) = changed {
        app.emit(BackendEvent::WindowStateChanged { window, state });
    }
}

fn monitor_rects(hwnd: HWND) -> Option<(RECT, RECT)> {
    let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    // SAFETY: the info structure is a live local with its size set.
    unsafe { GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut info) }.as_bool().then_some((info.rcMonitor, info.rcWork))
}

fn set_fullscreen(app: &Rc<App>, window: WindowId, hwnd: HWND, enabled: bool) {
    let saved = app
        .with_state(|state| {
            let entry = state.scene.windows.get_mut(&window)?;
            if entry.fullscreen.is_some() == enabled {
                return None;
            }
            if enabled {
                let mut placement = WINDOWPLACEMENT { length: size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
                // SAFETY: plain queries; the structure is a live local.
                let style = unsafe {
                    let _ = GetWindowPlacement(hwnd, &mut placement);
                    GetWindowLongPtrW(hwnd, GWL_STYLE)
                };
                entry.fullscreen = Some(SavedPlacement { style, placement });
                Some(None)
            } else {
                Some(entry.fullscreen.take())
            }
        })
        .flatten();
    let Some(saved) = saved else { return };
    // SAFETY: `hwnd` is a live window of this thread; style and position
    // changes dispatch messages, and no state borrow is held.
    unsafe {
        match saved {
            // Entering: a borderless window covering its monitor.
            None => {
                // A maximized window keeps the taskbar on top: the saved
                // placement brings the maximized state back on exit.
                if IsZoomed(hwnd).as_bool() {
                    let _ = ShowWindow(hwnd, SW_RESTORE);
                }
                let Some((monitor, _)) = monitor_rects(hwnd) else { return };
                let style = GetWindowLongPtrW(hwnd, GWL_STYLE) & !(WS_OVERLAPPEDWINDOW.0 as isize);
                SetWindowLongPtrW(hwnd, GWL_STYLE, style | WS_POPUP.0 as isize);
                let _ = SetWindowPos(
                    hwnd,
                    Some(HWND_TOP),
                    monitor.left,
                    monitor.top,
                    monitor.right - monitor.left,
                    monitor.bottom - monitor.top,
                    SWP_NOOWNERZORDER | SWP_FRAMECHANGED,
                );
            }
            Some(saved) => {
                SetWindowLongPtrW(hwnd, GWL_STYLE, saved.style);
                let _ = SetWindowPlacement(hwnd, &saved.placement);
                let _ = SetWindowPos(hwnd, None, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOOWNERZORDER | SWP_FRAMECHANGED);
            }
        }
    }
    report_state(app, window);
}

/// Runs a window command. Runs outside any state borrow.
pub fn command(app: &Rc<App>, window: WindowId, command: &WindowCommand) {
    let found = app
        .with_state(|state| {
            state.scene.windows.get(&window).map(|entry| (entry.hwnd, entry.dpi, entry.decorated, entry.resizable, entry.fullscreen.is_some(), entry.modal_owner))
        })
        .flatten();
    let Some((hwnd, dpi, decorated, resizable, fullscreen, modal_owner)) = found else { return };
    // SAFETY: `hwnd` is a live window of this thread; every call below may
    // dispatch messages, and no state borrow is held.
    unsafe {
        match command {
            WindowCommand::SetTitle(title) => {
                let title = wide(title);
                let _ = SetWindowTextW(hwnd, PCWSTR(title.as_ptr()));
            }
            WindowCommand::SetSize(size) => {
                let (width, height) = outer_size(*size, window_style(resizable), has_menu(hwnd), decorated, dpi);
                let _ = SetWindowPos(hwnd, None, 0, 0, width, height, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
            }
            WindowCommand::SetMinSize(size) => {
                app.with_state(|state| {
                    if let Some(entry) = state.scene.windows.get_mut(&window) {
                        entry.min_size = *size;
                    }
                });
            }
            WindowCommand::SetResizable(wanted) => {
                app.with_state(|state| {
                    if let Some(entry) = state.scene.windows.get_mut(&window) {
                        entry.resizable = *wanted;
                    }
                });
                if !fullscreen {
                    let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
                    let sizing = (WS_THICKFRAME | WS_MAXIMIZEBOX).0 as isize;
                    let next = if *wanted { style | sizing } else { style & !sizing };
                    if next != style {
                        SetWindowLongPtrW(hwnd, GWL_STYLE, next);
                        let _ = SetWindowPos(hwnd, None, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
                    }
                }
            }
            WindowCommand::Minimize => {
                let _ = ShowWindow(hwnd, SW_MINIMIZE);
            }
            WindowCommand::Maximize => {
                let _ = ShowWindow(hwnd, SW_MAXIMIZE);
            }
            WindowCommand::Restore => {
                if fullscreen {
                    set_fullscreen(app, window, hwnd, false);
                } else {
                    let _ = ShowWindow(hwnd, SW_RESTORE);
                }
            }
            WindowCommand::SetFullScreen(enabled) => set_fullscreen(app, window, hwnd, *enabled),
            WindowCommand::Focus => {
                if IsIconic(hwnd).as_bool() {
                    let _ = ShowWindow(hwnd, SW_RESTORE);
                } else {
                    let _ = ShowWindow(hwnd, SW_SHOW);
                }
                let _ = SetForegroundWindow(hwnd);
            }
            WindowCommand::Center => {
                let mut rect = RECT::default();
                if GetWindowRect(hwnd, &mut rect).is_ok() {
                    if let Some((_, work)) = monitor_rects(hwnd) {
                        let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
                        let x = work.left + ((work.right - work.left) - width) / 2;
                        let y = work.top + ((work.bottom - work.top) - height) / 2;
                        let _ = SetWindowPos(hwnd, None, x, y.max(work.top), 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
                    }
                }
            }
            WindowCommand::SetAlwaysOnTop(enabled) => {
                let after = if *enabled { HWND_TOPMOST } else { HWND_NOTOPMOST };
                let _ = SetWindowPos(hwnd, Some(after), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            }
            WindowCommand::Show => {
                if let Some(owner) = modal_owner {
                    let _ = EnableWindow(owner, false);
                }
                let _ = ShowWindow(hwnd, SW_SHOW);
                app.with_state(State::resume_frames);
            }
            WindowCommand::Hide => {
                // A hidden dialog must not keep its owner disabled.
                if let Some(owner) = modal_owner {
                    let _ = EnableWindow(owner, true);
                }
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
            WindowCommand::Close => close(app, window),
            WindowCommand::SetProgress(progress) => crate::shell::taskbar_progress(hwnd, *progress),
        }
    }
}

/// Starts moving the window with the mouse button that is down.
pub fn begin_drag(app: &Rc<App>, window: WindowId) {
    app.defer(Box::new(move |app| {
        let Some(hwnd) = app.with_state(|state| state.scene.windows.get(&window).map(|entry| entry.hwnd)).flatten() else { return };
        // SAFETY: `hwnd` is a live window of this thread. Releasing the
        // capture and sending the caption press is how a client area starts
        // the system's move loop; no state borrow is held.
        unsafe {
            let mut cursor = POINT::default();
            let _ = GetCursorPos(&mut cursor);
            let _ = ReleaseCapture();
            let position = ((cursor.y as u16 as u32) << 16) | (cursor.x as u16 as u32);
            SendMessageW(hwnd, WM_NCLBUTTONDOWN, Some(WPARAM(HTCAPTION as usize)), Some(LPARAM(position as isize)));
        }
    }));
}

pub fn system_cursor(cursor: Cursor) -> PCWSTR {
    match cursor {
        Cursor::Default => IDC_ARROW,
        Cursor::Pointer | Cursor::Grab => IDC_HAND,
        Cursor::Text => IDC_IBEAM,
        Cursor::Grabbing | Cursor::Move => IDC_SIZEALL,
        Cursor::NotAllowed => IDC_NO,
        Cursor::Crosshair => IDC_CROSS,
        Cursor::Wait => IDC_WAIT,
        Cursor::ResizeHorizontal => IDC_SIZEWE,
        Cursor::ResizeVertical => IDC_SIZENS,
    }
}

fn apply_cursor(cursor: Cursor) {
    // SAFETY: loading and selecting a stock system cursor.
    unsafe {
        if let Ok(handle) = LoadCursorW(None, system_cursor(cursor)) {
            SetCursor(Some(handle));
        }
    }
}

/// What the window procedure does after the state borrow of a pointer message.
#[derive(Default)]
struct PointerPlan {
    event: Option<PointerEvent>,
    capture: bool,
    release: bool,
    track: bool,
    cursor: Option<Cursor>,
    /// The armed tooltip changed: its text and rectangle, or nothing.
    tooltip: Option<Option<(String, RECT)>>,
    tooltip_window: HWND,
}

fn button_bit(button: PointerButton) -> u8 {
    match button {
        PointerButton::Primary => 1,
        PointerButton::Secondary => 2,
        PointerButton::Middle => 4,
    }
}

/// Updates hover state (cursor, tooltip, scrollbars) for a pointer position.
fn hover(state: &mut State, window: WindowId, position: Point, plan: &mut PointerPlan) {
    let Some(entry) = state.scene.windows.get_mut(&window) else { return };
    let mut path = std::mem::take(&mut entry.path);
    state.scene.hit_path(window, position, &mut path);
    state.scrolling().hover(&path, position);
    let cursor = path.iter().rev().filter_map(|id| state.scene.views.get(id)).map(|view| view.cursor).find(|cursor| *cursor != Cursor::Default).unwrap_or_default();
    let tooltip_node = path.iter().rev().copied().find(|id| state.scene.views.get(id).is_some_and(|view| view.tooltip.is_some()));
    let tooltip = tooltip_node.and_then(|id| {
        let text = state.scene.views.get(&id)?.tooltip.clone()?;
        let (_, transform) = state.scene.world_transform(id)?;
        let scale = state.scene.windows.get(&window)?.scale();
        Some((text, pixel_rect(transform.bounds(state.scene.views.get(&id)?.bounds()), scale)))
    });
    let hovered = path.last().copied();
    if let Some(entry) = state.scene.windows.get_mut(&window) {
        entry.path = path;
        entry.hover = hovered;
        if entry.cursor != cursor {
            entry.cursor = cursor;
            plan.cursor = Some(cursor);
        }
        if entry.tooltip_node != tooltip_node {
            entry.tooltip_node = tooltip_node;
            plan.tooltip = Some(tooltip);
        }
        plan.tooltip_window = entry.tooltip;
    }
}

fn pointer(app: &Rc<App>, hwnd: HWND, window: WindowId, message: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    let (phase, button) = match message {
        WM_MOUSEMOVE => (PointerPhase::Move, PointerButton::Primary),
        WM_LBUTTONDOWN => (PointerPhase::Down, PointerButton::Primary),
        WM_LBUTTONUP => (PointerPhase::Up, PointerButton::Primary),
        WM_RBUTTONDOWN => (PointerPhase::Down, PointerButton::Secondary),
        WM_RBUTTONUP => (PointerPhase::Up, PointerButton::Secondary),
        WM_MBUTTONDOWN => (PointerPhase::Down, PointerButton::Middle),
        _ => (PointerPhase::Up, PointerButton::Middle),
    };
    let (x, y) = lparam_point(lparam);
    // SAFETY: plain queries without preconditions.
    let (time, double_time, double_x, double_y) =
        unsafe { (GetMessageTime(), GetDoubleClickTime() as i32, GetSystemMetrics(SM_CXDOUBLECLK), GetSystemMetrics(SM_CYDOUBLECLK)) };
    let modifiers = events::modifiers();
    let plan = app.with_state(|state| {
        let mut plan = PointerPlan::default();
        let now = state.now_ms();
        let entry = state.scene.windows.get_mut(&window)?;
        let scale = entry.scale();
        let position = Point::new(x as f32 / scale, y as f32 / scale);
        entry.pointer = position;
        if !entry.tracking_leave {
            entry.tracking_leave = true;
            plan.track = true;
        }
        // A grabbed scrollbar thumb owns the pointer until the button is released.
        if let Some(drag) = entry.bar_drag {
            match phase {
                PointerPhase::Up if button == PointerButton::Primary => {
                    entry.bar_drag = None;
                    entry.buttons &= !button_bit(button);
                    plan.release = entry.buttons == 0;
                    if let Some(scroll) = state.scene.views.get_mut(&drag.id).and_then(|view| view.scroll.as_mut()) {
                        scroll.bar_held = false;
                    }
                }
                PointerPhase::Move => state.scrolling().drag(drag, position),
                _ => {}
            }
            state.schedule_scroll_work();
            return Some(plan);
        }
        if phase == PointerPhase::Down {
            let count = match entry.last_click {
                Some(last)
                    if last.button == button
                        && time.wrapping_sub(last.time) <= double_time
                        && (x - last.x).abs() <= double_x / 2
                        && (y - last.y).abs() <= double_y / 2 =>
                {
                    last.count.saturating_add(1)
                }
                _ => 1,
            };
            entry.last_click = Some(Click { time, x, y, button, count });
            entry.press_count = count;
            plan.capture = entry.buttons == 0;
            entry.buttons |= button_bit(button);
        } else if phase == PointerPhase::Up {
            entry.buttons &= !button_bit(button);
            plan.release = entry.buttons == 0;
        }
        let click_count = if entry.buttons != 0 || phase == PointerPhase::Up { entry.press_count } else { 1 };
        hover(state, window, position, &mut plan);
        if phase == PointerPhase::Down {
            // The pointer takes over from the keyboard: the focus ring goes.
            if state.scene.focus_visible {
                state.scene.focus_visible = false;
                if let Some(focus) = state.scene.focus {
                    state.scene.damage_view(focus);
                }
            }
            if button == PointerButton::Primary {
                let path = state.scene.windows.get_mut(&window).map(|entry| std::mem::take(&mut entry.path)).unwrap_or_default();
                let grabbed = state.scrolling().press(&path, position);
                let consumed = grabbed.is_some() || crate::media::press(state, &path, position);
                if let Some(entry) = state.scene.windows.get_mut(&window) {
                    entry.path = path;
                    entry.bar_drag = grabbed;
                }
                if consumed {
                    state.schedule_scroll_work();
                    return Some(plan);
                }
            }
        } else if phase == PointerPhase::Move {
            crate::media::hover(state, window, position);
        }
        state.schedule_scroll_work();
        plan.event = Some(PointerEvent { window, phase, position, button, modifiers, click_count, time_ms: now });
        Some(plan)
    });
    let Some(Some(plan)) = plan else { return None };
    // SAFETY: `hwnd` is the live window receiving this message; capture and
    // tracking calls may dispatch messages, and no state borrow is held.
    unsafe {
        if plan.track {
            let mut track = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
            let _ = TrackMouseEvent(&mut track);
        }
        if plan.capture {
            SetCapture(hwnd);
        }
        if plan.release && GetCapture() == hwnd {
            let _ = ReleaseCapture();
        }
    }
    if let Some(cursor) = plan.cursor {
        apply_cursor(cursor);
    }
    if let Some(tooltip) = &plan.tooltip {
        update_tooltip(plan.tooltip_window, hwnd, tooltip.as_ref().map(|(text, rect)| (text.as_str(), *rect)));
    }
    relay_to_tooltip(plan.tooltip_window, hwnd, message, wparam, lparam);
    if let Some(event) = plan.event {
        app.emit(BackendEvent::Pointer(event));
    }
    Some(LRESULT(0))
}

/// A pointer event seen by a child control (text inputs), reported in the
/// window's coordinates so hover and press states keep working over it.
pub fn pointer_from_child(app: &Rc<App>, child: HWND, phase: PointerPhase, button: PointerButton, lparam: LPARAM) {
    let (x, y) = lparam_point(lparam);
    let mut point = POINT { x, y };
    // SAFETY: plain coordinate conversions on live windows.
    let root = unsafe {
        let _ = ClientToScreen(child, &mut point);
        let root = windows::Win32::UI::WindowsAndMessaging::GetAncestor(child, windows::Win32::UI::WindowsAndMessaging::GA_ROOT);
        let _ = ScreenToClient(root, &mut point);
        root
    };
    let modifiers = events::modifiers();
    let event = app
        .with_state(|state| {
            let now = state.now_ms();
            let (window, entry) = state.scene.windows.iter_mut().find(|(_, entry)| entry.hwnd == root)?;
            let scale = entry.scale();
            let position = Point::new(point.x as f32 / scale, point.y as f32 / scale);
            entry.pointer = position;
            Some(PointerEvent { window: *window, phase, position, button, modifiers, click_count: 1, time_ms: now })
        })
        .flatten();
    if let Some(event) = event {
        app.emit_later(BackendEvent::Pointer(event));
    }
}

/// The pointer left `hwnd` or one of its child controls. It only left
/// the window when it is over neither.
fn pointer_left(app: &Rc<App>, hwnd: HWND, window: WindowId, from_child: bool) {
    let mut cursor = POINT::default();
    // SAFETY: plain queries; the point is a live local.
    let still_inside = unsafe {
        let _ = GetCursorPos(&mut cursor);
        let under = WindowFromPoint(cursor);
        under == hwnd || IsChild(hwnd, under).as_bool()
    };
    let plan = app.with_state(|state| {
        let now = state.now_ms();
        let entry = state.scene.windows.get_mut(&window)?;
        if !from_child {
            entry.tracking_leave = false;
        }
        if still_inside {
            return None;
        }
        entry.hover = None;
        entry.cursor = Cursor::Default;
        let had_tooltip = entry.tooltip_node.take().is_some();
        let (tooltip, position) = (entry.tooltip, entry.pointer);
        state.scrolling().hover(&[], position);
        crate::media::hover(state, window, Point::new(-1.0, -1.0));
        state.schedule_scroll_work();
        let event = PointerEvent {
            window,
            phase: PointerPhase::Leave,
            position,
            button: PointerButton::Primary,
            modifiers: events::modifiers(),
            click_count: 1,
            time_ms: now,
        };
        Some((event, had_tooltip.then_some(tooltip)))
    });
    if let Some(Some((event, tooltip))) = plan {
        if let Some(tooltip) = tooltip {
            update_tooltip(tooltip, hwnd, None);
        }
        app.emit(BackendEvent::Pointer(event));
    }
}

/// The pointer left a child control (text inputs track it themselves).
pub fn pointer_left_child(app: &Rc<App>, child: HWND) {
    // SAFETY: plain query on a live window.
    let root = unsafe { windows::Win32::UI::WindowsAndMessaging::GetAncestor(child, windows::Win32::UI::WindowsAndMessaging::GA_ROOT) };
    let window = app.with_state(|state| state.scene.windows.iter().find(|(_, entry)| entry.hwnd == root).map(|(window, _)| *window)).flatten();
    if let Some(window) = window {
        pointer_left(app, root, window, true);
        app.post_pump_if_pending();
    }
}

fn wheel(app: &Rc<App>, hwnd: HWND, window: WindowId, horizontal: bool, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    let rotation = f32::from(hiword(wparam.0) as i16);
    let (screen_x, screen_y) = lparam_point(lparam);
    let mut point = POINT { x: screen_x, y: screen_y };
    let mut lines = 3_u32;
    // SAFETY: plain queries; the out pointers refer to live locals.
    unsafe {
        let _ = ScreenToClient(hwnd, &mut point);
        let _ = SystemParametersInfoW(SPI_GETWHEELSCROLLLINES, 0, Some(std::ptr::from_mut(&mut lines).cast()), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
    }
    let modifiers = events::modifiers();
    // A mouse sends whole notches; touchpads send small pixel-precise steps
    // that are already smooth.
    let notched = (rotation as i32) % (WHEEL_DELTA as i32) == 0;
    let event = app.with_state(|state| {
        let entry = state.scene.windows.get_mut(&window)?;
        let scale = entry.scale();
        let position = Point::new(point.x as f32 / scale, point.y as f32 / scale);
        let per_notch = if lines == PAGE_SCROLL { entry.size.height * 0.9 } else { lines.max(1) as f32 * LINE_EXTENT };
        let amount = rotation / WHEEL_DELTA as f32 * per_notch;
        // Forward rotation scrolls up (content moves down); a horizontal
        // wheel tilted right scrolls right. Shift turns the wheel sideways.
        let delta = if horizontal {
            Point::new(amount, 0.0)
        } else if modifiers.shift {
            Point::new(-amount, 0.0)
        } else {
            Point::new(0.0, -amount)
        };
        let mut path = std::mem::take(&mut entry.path);
        state.scene.hit_path(window, position, &mut path);
        // Control + wheel is the zoom convention (and what a touchpad pinch
        // sends): it is never scrolling.
        let consumed = !modifiers.control && state.scrolling().wheel(&path, delta, notched);
        if let Some(entry) = state.scene.windows.get_mut(&window) {
            entry.path = path;
        }
        state.schedule_scroll_work();
        (!consumed).then_some(BackendEvent::Wheel { window, position, delta, modifiers })
    });
    if let Some(Some(event)) = event {
        app.emit(event);
    }
    Some(LRESULT(0))
}

fn key(app: &Rc<App>, window: WindowId, pressed: bool, system: bool, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    let code = wparam.0 as u32;
    let bits = lparam.0 as u32;
    let scan_code = (bits >> 16) & 0xFF;
    let repeat = pressed && bits & (1 << 30) != 0;
    let name = events::key_name(code, scan_code);
    let modifiers = events::modifiers();
    if pressed && !events::is_modifier(code) {
        // Keyboard use shows the focus ring.
        app.with_state(|state| {
            if !state.scene.focus_visible {
                state.scene.focus_visible = true;
                if let Some(focus) = state.scene.focus {
                    state.scene.damage_view(focus);
                }
            }
        });
    }
    if pressed && name == "Tab" && !modifiers.control && !modifiers.alt && crate::focus::advance(app, window, !modifiers.shift) {
        return Some(LRESULT(0));
    }
    let scrolled = pressed
        && !modifiers.alt
        && app
            .with_state(|state| {
                let scrolled = state.scrolling().key(&name, modifiers.control);
                state.schedule_scroll_work();
                scrolled
            })
            .unwrap_or(false);
    if !scrolled {
        app.emit(BackendEvent::Key(KeyEvent { window, pressed, key: name, modifiers, repeat }));
    }
    // System keys keep their default handling (Alt+F4, the window menu).
    if system { None } else { Some(LRESULT(0)) }
}

/// A key a native control did not use, reported for its window.
pub fn key_from_child(app: &Rc<App>, child: HWND, pressed: bool, wparam: WPARAM, lparam: LPARAM) {
    // SAFETY: plain query on a live window.
    let root = unsafe { windows::Win32::UI::WindowsAndMessaging::GetAncestor(child, windows::Win32::UI::WindowsAndMessaging::GA_ROOT) };
    let Some(window) = app.with_state(|state| state.scene.windows.iter().find(|(_, entry)| entry.hwnd == root).map(|(window, _)| *window)).flatten() else {
        return;
    };
    let bits = lparam.0 as u32;
    let name = events::key_name(wparam.0 as u32, (bits >> 16) & 0xFF);
    let repeat = pressed && bits & (1 << 30) != 0;
    app.emit_later(BackendEvent::Key(KeyEvent { window, pressed, key: name, modifiers: events::modifiers(), repeat }));
}

fn gesture(app: &Rc<App>, hwnd: HWND, window: WindowId, lparam: LPARAM) -> Option<LRESULT> {
    let mut info = GESTUREINFO { cbSize: size_of::<GESTUREINFO>() as u32, ..Default::default() };
    let handle = HGESTUREINFO(lparam.0 as *mut _);
    // SAFETY: the handle comes from the WM_GESTURE being handled and the
    // structure is a live local with its size set.
    if unsafe { GetGestureInfo(handle, &mut info) }.is_err() {
        return None;
    }
    let zoom = info.dwID == GID_ZOOM.0;
    let rotate = info.dwID == GID_ROTATE.0;
    let pan = info.dwID == GID_PAN.0;
    if !zoom && !rotate && !pan {
        return None;
    }
    let mut point = POINT { x: i32::from(info.ptsLocation.x), y: i32::from(info.ptsLocation.y) };
    // SAFETY: plain conversion; the handle is closed exactly once because
    // the message is not passed on.
    unsafe {
        let _ = ScreenToClient(hwnd, &mut point);
        let _ = CloseGestureInfoHandle(handle);
    }
    let phase = if info.dwFlags & GF_BEGIN != 0 {
        GesturePhase::Begin
    } else if info.dwFlags & GF_END != 0 {
        GesturePhase::End
    } else {
        GesturePhase::Update
    };
    if pan {
        // A touch pan scrolls what is under the fingers, like the wheel
        // (the system keeps sending steps for the inertia).
        let modifiers = events::modifiers();
        let event = app.with_state(|state| {
            let entry = state.scene.windows.get_mut(&window)?;
            let scale = entry.scale();
            let previous = entry.pan_from.replace((point.x, point.y));
            if phase == GesturePhase::End {
                entry.pan_from = None;
            }
            let (from_x, from_y) = previous.filter(|_| phase != GesturePhase::Begin)?;
            // The content follows the fingers.
            let delta = Point::new((from_x - point.x) as f32 / scale, (from_y - point.y) as f32 / scale);
            if delta == Point::ZERO {
                return None;
            }
            let position = Point::new(point.x as f32 / scale, point.y as f32 / scale);
            let mut path = std::mem::take(&mut entry.path);
            state.scene.hit_path(window, position, &mut path);
            let consumed = state.scrolling().wheel(&path, delta, false);
            if let Some(entry) = state.scene.windows.get_mut(&window) {
                entry.path = path;
            }
            state.schedule_scroll_work();
            (!consumed).then_some(BackendEvent::Wheel { window, position, delta, modifiers })
        });
        if let Some(Some(event)) = event {
            app.emit(event);
        }
        return Some(LRESULT(0));
    }
    // The low 32 bits carry the distance between the fingers (zoom) or the
    // angle encoded in 16 bits (rotate).
    let argument = (info.ullArguments & 0xFFFF_FFFF) as f64;
    let event = app
        .with_state(|state| {
            let entry = state.scene.windows.get_mut(&window)?;
            let scale = entry.scale();
            let position = Point::new(point.x as f32 / scale, point.y as f32 / scale);
            if zoom {
                if phase == GesturePhase::Begin {
                    entry.gesture_start = argument.max(1.0);
                }
                let factor = (argument / entry.gesture_start) as f32;
                Some(BackendEvent::Magnify { window, position, phase, scale: if phase == GesturePhase::Begin { 1.0 } else { factor } })
            } else {
                // GID_ROTATE_ANGLE_FROM_ARGUMENT: counter-clockwise radians.
                // At the start the argument is the absolute angle of the
                // fingers; afterwards it is the change since the start.
                let angle = (argument / 65535.0) * 4.0 * std::f64::consts::PI - 2.0 * std::f64::consts::PI;
                Some(BackendEvent::Rotate { window, position, phase, radians: if phase == GesturePhase::Begin { 0.0 } else { -angle as f32 } })
            }
        })
        .flatten();
    if let Some(event) = event {
        app.emit(event);
    }
    Some(LRESULT(0))
}

/// Resize borders of a window without the system frame.
fn frameless_hit_test(hwnd: HWND, lparam: LPARAM, dpi: u32, resizable: bool) -> LRESULT {
    let (x, y) = lparam_point(lparam);
    let mut rect = RECT::default();
    // SAFETY: plain queries; the rectangle is a live local.
    let (border, maximized) = unsafe {
        let _ = GetWindowRect(hwnd, &mut rect);
        (GetSystemMetricsForDpi(SM_CXFRAME, dpi) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi), IsZoomed(hwnd).as_bool())
    };
    if !resizable || maximized {
        return LRESULT(HTCLIENT as isize);
    }
    let (left, right) = (x < rect.left + border, x >= rect.right - border);
    let (top, bottom) = (y < rect.top + border, y >= rect.bottom - border);
    let hit = match (left, right, top, bottom) {
        (true, _, true, _) => HTTOPLEFT,
        (_, true, true, _) => HTTOPRIGHT,
        (true, _, _, true) => HTBOTTOMLEFT,
        (_, true, _, true) => HTBOTTOMRIGHT,
        (true, ..) => HTLEFT,
        (_, true, ..) => HTRIGHT,
        (_, _, true, _) => HTTOP,
        (_, _, _, true) => HTBOTTOM,
        _ => HTCLIENT,
    };
    LRESULT(hit as isize)
}

fn paint(app: &Rc<App>, hwnd: HWND, window: WindowId) {
    // Validate first: a device loss during the frame invalidates again.
    // SAFETY: validating the window receiving WM_PAINT.
    unsafe {
        let _ = ValidateRect(Some(hwnd), None);
    }
    let needs_natives = app.with_state(|state| state.scene.windows.get_mut(&window).is_some_and(|entry| std::mem::take(&mut entry.natives_dirty)));
    match needs_natives {
        Some(true) => crate::native::sync(app, window),
        Some(false) => {}
        None => return app.note_missed_paint(),
    }
    let mut lost = false;
    let rendered = app.with_state(|state| {
        let Some(entry) = state.scene.windows.get(&window) else { return };
        let never_presented = entry.surface.as_ref().is_none_or(|surface| !surface.presented);
        // SAFETY: plain query on a live window.
        let minimized = unsafe { IsIconic(entry.hwnd) }.as_bool();
        if minimized || (entry.dirty == Damage::Clean && !never_presented) {
            return;
        }
        if matches!(crate::render::render_window(state, window), crate::render::Outcome::DeviceLost) {
            lost = true;
        }
    });
    if rendered.is_none() {
        app.note_missed_paint();
    }
    if lost {
        // Players reopen on the new device; the frame is painted again.
        crate::media::recover(app);
    }
}

fn resized(app: &Rc<App>, hwnd: HWND, window: WindowId, kind: u32) {
    if kind == SIZE_MINIMIZED {
        report_state(app, window);
        return;
    }
    let (width, height) = client_size(hwnd);
    let event = app
        .with_state(|state| {
            let entry = state.scene.windows.get_mut(&window)?;
            let scale = entry.scale();
            let size = Size::new(width as f32 / scale, height as f32 / scale);
            let changed = entry.pixel_size != (width, height);
            entry.pixel_size = (width, height);
            entry.size = size;
            state.scene.damage_all(window);
            // Back from minimized: animations run again.
            state.resume_frames();
            changed.then_some(BackendEvent::WindowResized { window, size, scale })
        })
        .flatten();
    if let Some(event) = event {
        app.emit(event);
    }
    report_state(app, window);
}

fn dpi_changed(app: &Rc<App>, hwnd: HWND, window: WindowId, wparam: WPARAM, lparam: LPARAM) {
    let dpi = u32::from(hiword(wparam.0)).max(96);
    app.with_state(|state| {
        if let Some(entry) = state.scene.windows.get_mut(&window) {
            entry.dpi = dpi;
            if let Some(surface) = &mut entry.surface {
                // The target bitmap carries the DPI: rebuild it.
                surface.target = None;
            }
        }
        state.scene.damage_all(window);
    });
    // SAFETY: lParam of WM_DPICHANGED points to the suggested rectangle for
    // the duration of the message; no state borrow is held while resizing.
    unsafe {
        let suggested = *(lparam.0 as *const RECT);
        let _ = SetWindowPos(
            hwnd,
            None,
            suggested.left,
            suggested.top,
            suggested.right - suggested.left,
            suggested.bottom - suggested.top,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
    let (width, height) = client_size(hwnd);
    let event = app
        .with_state(|state| {
            let entry = state.scene.windows.get_mut(&window)?;
            let scale = entry.scale();
            entry.pixel_size = (width, height);
            entry.size = Size::new(width as f32 / scale, height as f32 / scale);
            entry.natives_dirty = true;
            Some(BackendEvent::WindowResized { window, size: entry.size, scale })
        })
        .flatten();
    if let Some(event) = event {
        app.emit(event);
    }
}

fn destroyed(app: &Rc<App>, hwnd: HWND, window: WindowId) {
    crate::dragdrop::revoke(hwnd);
    let remaining = app.with_state(|state| {
        if let Some(entry) = state.scene.windows.remove(&window) {
            for layer in entry.root.iter().chain(entry.overlays.iter()) {
                if let Some(view) = state.scene.views.get_mut(layer) {
                    view.layer_of = None;
                }
            }
        }
        // Child controls died with the window.
        crate::native::parent_destroyed(state, hwnd);
        state.scene.windows.len()
    });
    crate::webview::close_retired();
    crate::accessibility::window_destroyed(hwnd);
    if remaining == Some(0) && !app.quitting.get() {
        // SAFETY: ends the message loop of this thread.
        unsafe { PostQuitMessage(0) };
    }
}

fn handle(app: &Rc<App>, hwnd: HWND, window: WindowId, message: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    match message {
        WM_PAINT => {
            paint(app, hwnd, window);
            Some(LRESULT(0))
        }
        // Direct2D covers the whole client area.
        WM_ERASEBKGND => Some(LRESULT(1)),
        WM_SIZE => {
            resized(app, hwnd, window, wparam.0 as u32);
            Some(LRESULT(0))
        }
        WM_DPICHANGED => {
            dpi_changed(app, hwnd, window, wparam, lparam);
            Some(LRESULT(0))
        }
        WM_MOVE => {
            crate::webview::window_moved(app, window);
            None
        }
        WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP | WM_RBUTTONDOWN | WM_RBUTTONUP | WM_MBUTTONDOWN | WM_MBUTTONUP => {
            pointer(app, hwnd, window, message, wparam, lparam)
        }
        WM_MOUSELEAVE => {
            pointer_left(app, hwnd, window, false);
            Some(LRESULT(0))
        }
        WM_XBUTTONUP => {
            // The "back" side button of a mouse.
            if hiword(wparam.0) == XBUTTON1 {
                app.emit(BackendEvent::BackRequested { window });
            }
            Some(LRESULT(1))
        }
        WM_APPCOMMAND => {
            let command = u32::from(hiword(lparam.0 as usize) & 0x0FFF);
            if command == APPCOMMAND_BROWSER_BACKWARD.0 {
                app.emit(BackendEvent::BackRequested { window });
                Some(LRESULT(1))
            } else {
                None
            }
        }
        WM_CAPTURECHANGED => {
            // Another window (or the system's move loop) took the mouse
            // while a button was down: the press is over.
            let cancelled = app
                .with_state(|state| {
                    let now = state.now_ms();
                    let entry = state.scene.windows.get_mut(&window)?;
                    if entry.buttons == 0 || lparam.0 == hwnd.0 as isize {
                        return None;
                    }
                    entry.buttons = 0;
                    let position = entry.pointer;
                    if let Some(drag) = entry.bar_drag.take() {
                        // The thumb is no longer held: its bar may fade.
                        if let Some(scroll) = state.scene.views.get_mut(&drag.id).and_then(|view| view.scroll.as_mut()) {
                            scroll.bar_held = false;
                        }
                        state.schedule_scroll_work();
                    }
                    Some(PointerEvent {
                        window,
                        phase: PointerPhase::Cancel,
                        position,
                        button: PointerButton::Primary,
                        modifiers: events::modifiers(),
                        click_count: 1,
                        time_ms: now,
                    })
                })
                .flatten();
            if let Some(event) = cancelled {
                app.emit(BackendEvent::Pointer(event));
            }
            Some(LRESULT(0))
        }
        WM_MOUSEWHEEL => wheel(app, hwnd, window, false, wparam, lparam),
        WM_MOUSEHWHEEL => wheel(app, hwnd, window, true, wparam, lparam),
        WM_KEYDOWN => key(app, window, true, false, wparam, lparam),
        WM_KEYUP => key(app, window, false, false, wparam, lparam),
        WM_SYSKEYDOWN => key(app, window, true, true, wparam, lparam),
        WM_SYSKEYUP => key(app, window, false, true, wparam, lparam),
        // Keys are reported from WM_KEYDOWN; the translated characters are not needed.
        WM_CHAR => Some(LRESULT(0)),
        WM_SYSCHAR => {
            // Without a menu bar there are no mnemonics: swallowing the
            // character avoids the default beep on Alt chords. Alt+Space
            // still opens the window menu.
            if has_menu(hwnd) || wparam.0 == 0x20 { None } else { Some(LRESULT(0)) }
        }
        WM_GESTURENOTIFY => {
            // Rotation is off by default, and so is panning sideways with
            // one finger; the gutter would lock a pan to its first axis.
            let pan = (GC_PAN | GC_PAN_WITH_SINGLE_FINGER_VERTICALLY | GC_PAN_WITH_SINGLE_FINGER_HORIZONTALLY | GC_PAN_WITH_INERTIA).0;
            let wanted = [
                GESTURECONFIG { dwID: GID_ROTATE, dwWant: GC_ROTATE.0, dwBlock: 0 },
                GESTURECONFIG { dwID: GID_PAN, dwWant: pan, dwBlock: GC_PAN_WITH_GUTTER.0 },
            ];
            // SAFETY: the configuration array is live for the call.
            unsafe {
                let _ = SetGestureConfig(hwnd, 0, &wanted, size_of::<GESTURECONFIG>() as u32);
            }
            None
        }
        WM_GESTURE => gesture(app, hwnd, window, lparam),
        WM_SETCURSOR => {
            if u32::from(loword(lparam.0 as usize)) == HTCLIENT && wparam.0 == hwnd.0 as usize {
                let cursor = app.with_state(|state| state.scene.windows.get(&window).map(|entry| entry.cursor)).flatten()?;
                apply_cursor(cursor);
                Some(LRESULT(1))
            } else {
                None
            }
        }
        WM_ACTIVATE => {
            let active = u32::from(loword(wparam.0)) != WA_INACTIVE;
            // SAFETY: plain queries on live windows.
            let focused_child = unsafe {
                let focused = GetFocus();
                (!active && focused != hwnd && IsChild(hwnd, focused).as_bool()).then_some(focused)
            };
            let tooltip = app.with_state(|state| {
                let entry = state.scene.windows.get_mut(&window)?;
                entry.active = active;
                entry.state.focused = active;
                if !active {
                    entry.focus_restore = focused_child;
                    entry.tooltip_node = None;
                }
                Some(entry.tooltip)
            });
            if !active {
                if let Some(Some(tooltip)) = tooltip {
                    update_tooltip(tooltip, hwnd, None);
                }
            }
            app.emit(BackendEvent::WindowFocusChanged { window, focused: active });
            None
        }
        WM_SETFOCUS => {
            // A plain window does not remember which child had the focus.
            let restore = app.with_state(|state| state.scene.windows.get_mut(&window).and_then(|entry| entry.focus_restore.take())).flatten();
            if let Some(child) = restore {
                // SAFETY: the handle is checked to still be a window; no state borrow is held.
                unsafe {
                    if IsWindow(Some(child)).as_bool() && IsChild(hwnd, child).as_bool() {
                        let _ = SetFocus(Some(child));
                    }
                }
            }
            Some(LRESULT(0))
        }
        WM_CLOSE => {
            app.emit(BackendEvent::WindowCloseRequested { window });
            Some(LRESULT(0))
        }
        WM_DESTROY => {
            destroyed(app, hwnd, window);
            Some(LRESULT(0))
        }
        WM_GETMINMAXINFO => {
            let minimum = app.with_state(|state| state.scene.windows.get(&window).and_then(|entry| entry.min_size.map(|size| (size, entry.dpi, entry.decorated, entry.resizable)))).flatten()?;
            let (size, dpi, decorated, resizable) = minimum;
            let (width, height) = outer_size(size, window_style(resizable), has_menu(hwnd), decorated, dpi);
            // SAFETY: lParam of WM_GETMINMAXINFO points to a MINMAXINFO for the duration of the message.
            unsafe {
                let info = &mut *(lparam.0 as *mut MINMAXINFO);
                info.ptMinTrackSize = POINT { x: width, y: height };
            }
            Some(LRESULT(0))
        }
        WM_NCCALCSIZE if wparam.0 != 0 => {
            let (decorated, dpi) = app.with_state(|state| state.scene.windows.get(&window).map(|entry| (entry.decorated || entry.fullscreen.is_some(), entry.dpi))).flatten()?;
            if decorated {
                return None;
            }
            // The client area is the whole window. A maximized window hangs
            // over the monitor by its frame: keep the client on screen.
            let mut placement = WINDOWPLACEMENT { length: size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
            // SAFETY: lParam of WM_NCCALCSIZE (wParam TRUE) points to
            // NCCALCSIZE_PARAMS for the duration of the message.
            unsafe {
                let _ = GetWindowPlacement(hwnd, &mut placement);
                if placement.showCmd == SW_SHOWMAXIMIZED.0 as u32 {
                    let parameters = &mut *(lparam.0 as *mut NCCALCSIZE_PARAMS);
                    let padding = GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi);
                    let (frame_x, frame_y) = (GetSystemMetricsForDpi(SM_CXFRAME, dpi) + padding, GetSystemMetricsForDpi(SM_CYFRAME, dpi) + padding);
                    parameters.rgrc[0].left += frame_x;
                    parameters.rgrc[0].right -= frame_x;
                    parameters.rgrc[0].top += frame_y;
                    parameters.rgrc[0].bottom -= frame_y;
                }
            }
            Some(LRESULT(0))
        }
        WM_NCACTIVATE => {
            let decorated = app.with_state(|state| state.scene.windows.get(&window).map(|entry| entry.decorated)).flatten()?;
            if decorated {
                return None;
            }
            // Without a frame there is no caption to repaint: -1 keeps the
            // default handler from drawing one over the client area.
            // SAFETY: forwarding the message with the documented lParam.
            Some(unsafe { DefWindowProcW(hwnd, message, wparam, LPARAM(-1)) })
        }
        WM_NCHITTEST => {
            let (decorated, dpi, resizable) =
                app.with_state(|state| state.scene.windows.get(&window).map(|entry| (entry.decorated || entry.fullscreen.is_some(), entry.dpi, entry.resizable))).flatten()?;
            if decorated { None } else { Some(frameless_hit_test(hwnd, lparam, dpi, resizable)) }
        }
        WM_COMMAND => {
            if lparam.0 == 0 {
                crate::shell::menu_command(app, window, u32::from(loword(wparam.0)));
            } else {
                crate::input::notification(app, HWND(lparam.0 as *mut _), u32::from(hiword(wparam.0)));
            }
            Some(LRESULT(0))
        }
        WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC => crate::input::control_colors(app, HWND(lparam.0 as *mut _), wparam),
        WM_SETTINGCHANGE => {
            if lparam.0 != 0 {
                // SAFETY: lParam of WM_SETTINGCHANGE is a NUL-terminated
                // string (or null, checked above) for the duration of the message.
                let area = unsafe { PCWSTR(lparam.0 as *const u16).as_wide().to_vec() };
                if from_wide(&area) == "ImmersiveColorSet" {
                    crate::shell::system_appearance_changed(app);
                }
            }
            None
        }
        WM_GETOBJECT => crate::accessibility::get_object(app, hwnd, window, wparam, lparam),
        _ => None,
    }
}

extern "system" fn window_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let handled = catch_unwind(AssertUnwindSafe(|| {
        if message == WM_NCCREATE {
            // SAFETY: lParam of WM_NCCREATE points to the CREATESTRUCTW of
            // the CreateWindowExW call; its parameter is the window id.
            unsafe {
                let create = &*(lparam.0 as *const CREATESTRUCTW);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
            }
            return None;
        }
        let app = App::current()?;
        // SAFETY: reading the user data slot set at creation.
        let window = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as WindowId;
        if window == 0 {
            return None;
        }
        let result = handle(&app, hwnd, window, message, wparam, lparam);
        // Messages a child control sends from inside its own procedure
        // (notifications, color queries while it paints, creation and
        // destruction notices): events are drained after it returns.
        let from_control = matches!(message, WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC | WM_PARENTNOTIFY | WM_NOTIFY) || (message == WM_COMMAND && lparam.0 != 0);
        if from_control {
            app.post_pump_if_pending();
        } else {
            app.pump();
        }
        result
    }));
    match handled {
        Ok(Some(result)) => result,
        // SAFETY: forwarding the unmodified message to the default procedure.
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

/// Destroys every window (application quit).
pub fn close_all(app: &Rc<App>) {
    let windows: Vec<WindowId> = app.with_state(|state| state.scene.windows.keys().copied().collect()).unwrap_or_default();
    for window in windows {
        close(app, window);
    }
}

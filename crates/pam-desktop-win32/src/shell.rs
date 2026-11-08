//! Desktop shell on Win32: the menu bar and its accelerators, context
//! menus, the notification area icon, file dialogs, notifications, alerts,
//! the clipboard, global hot keys and the system appearance.
//!
//! Everything modal (menus, dialogs) runs from deferred tasks, never from
//! inside a `Backend` method: the runtime is not on the stack while a
//! nested message loop spins.

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use pam_desktop_ui::backend::{AlertButtonStyle, AlertConfig, BackendEvent, MAIN_WINDOW, WindowId};
use pam_desktop_ui::geometry::Point;
use pam_desktop_ui::shell::{
    FileDialogConfig, FileDialogKind, MenuNode, MenuRole, NotificationConfig, Shortcut, TrayConfig,
    WindowCommand,
};
use windows::Data::Xml::Dom::XmlDocument;
use windows::Foundation::TypedEventHandler;
use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
use windows::Win32::Foundation::{HANDLE, HGLOBAL, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWINDOWATTRIBUTE, DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    ClientToScreen, CreateBitmap, DEVMODEW, DeleteObject, ENUM_CURRENT_SETTINGS,
    EnumDisplaySettingsW, GetMonitorInfoW, HGDIOBJ, MONITOR_DEFAULTTONEAREST, MONITORINFOEXW,
    MonitorFromWindow,
};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppBGRA, IWICBitmapSource, IWICImagingFactory,
    WICBitmapDitherTypeNone, WICBitmapInterpolationModeFant, WICBitmapPaletteTypeMedianCut,
    WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    SetClipboardData,
};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::Memory::{
    GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
};
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ,
    RRF_RT_REG_DWORD, RegCloseKey, RegCreateKeyExW, RegGetValueW, RegQueryValueExW, RegSetValueExW,
};
use windows::Win32::UI::Controls::{
    TASKDIALOG_BUTTON, TASKDIALOGCONFIG, TDF_ALLOW_DIALOG_CANCELLATION,
    TDF_POSITION_RELATIVE_TO_WINDOW,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, RegisterHotKey,
    UnregisterHotKey,
};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    ExtractIconExW, FOS_ALLOWMULTISELECT, FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, FileOpenDialog,
    FileSaveDialog, GetCurrentProcessExplicitAppUserModelID, IFileDialog, IFileOpenDialog,
    IShellItem, ITaskbarList3, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIIF_INFO,
    NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NIN_BALLOONHIDE, NIN_BALLOONTIMEOUT,
    NIN_BALLOONUSERCLICK, NIN_SELECT, NOTIFYICON_VERSION_4, NOTIFYICONDATAW,
    SHCreateItemFromParsingName, SIGDN_FILESYSPATH, Shell_NotifyIconW, TBPF_NOPROGRESS,
    TBPF_NORMAL, TaskbarList,
};
use windows::Win32::UI::WindowsAndMessaging::{
    ACCEL, AppendMenuW, CheckMenuItem, CreateAcceleratorTableW, CreateIconIndirect, CreateMenu,
    CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyAcceleratorTable, DestroyIcon,
    DestroyMenu, DestroyWindow, DrawMenuBar, FALT, FCONTROL, FSHIFT, FVIRTKEY, GetCursorPos,
    GetSystemMetrics, GetWindowTextW, HACCEL, HICON, HMENU, ICONINFO, IDCANCEL, IDI_APPLICATION,
    IDNO, IDOK, IDYES, IsZoomed, LoadIconW, MB_OK, MB_OKCANCEL, MB_YESNOCANCEL, MENU_ITEM_FLAGS,
    MF_BYCOMMAND, MF_CHECKED, MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING, MF_UNCHECKED, MSG,
    MessageBoxW, PostMessageW, RegisterClassExW, RegisterWindowMessageW, SM_CXSMICON, SM_CYSMICON,
    SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SetForegroundWindow, SetMenu,
    SystemParametersInfoW, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenuEx, TranslateAcceleratorW,
    WINDOW_EX_STYLE, WM_CONTEXTMENU, WM_COPY, WM_CUT, WM_KEYDOWN, WM_NULL, WM_PASTE, WM_SYSKEYDOWN,
    WNDCLASSEXW, WS_POPUP,
};
use windows::core::{BOOL, HRESULT, HSTRING, Interface, PCSTR, PCWSTR, w};

use crate::app::{App, Mail, Theme, WM_APP_TRAY};
use crate::events;
use crate::util::{from_wide, loword, wide};

const TRAY_CLASS: PCWSTR = w!("PamDesktopTray");
const TRAY_ICON_ID: u32 = 1;
/// The icon was activated with the keyboard (`NIN_SELECT | NINF_KEY`).
const NIN_KEYSELECT: u32 = NIN_SELECT | 0x1;
/// Toasts kept alive so their activation handlers still fire.
const TOAST_HISTORY: usize = 16;

/// What choosing a menu item does.
#[derive(Clone)]
enum Activation {
    /// Report the id to the application.
    Report(String),
    Role(MenuRole),
}

struct MenuItem {
    activation: Activation,
    checked: Option<bool>,
}

struct MenuBar {
    menu: HMENU,
    accelerators: Option<HACCEL>,
    /// Item `n` has command id `n + 1`.
    items: Vec<MenuItem>,
}

struct Tray {
    icon: HICON,
    /// The icon was made by us and must be destroyed.
    owns_icon: bool,
    tooltip: String,
    menu: Vec<MenuNode>,
    /// Shown only to carry a balloon notification.
    temporary: bool,
    /// The notification the balloon on screen stands for.
    balloon: String,
}

#[derive(Default)]
pub struct ShellState {
    menu_bar: Option<MenuBar>,
    tray: Option<Tray>,
    /// The hidden window notification area messages go to, made once.
    tray_window: Option<HWND>,
    /// Registered global hot keys: their id is the index plus one.
    hot_keys: Vec<String>,
    taskbar: Option<ITaskbarList3>,
    toasts: Vec<ToastNotification>,
    /// Broadcast when the taskbar is recreated (Explorer restarted).
    taskbar_created: u32,
}

// ---- appearance ----------------------------------------------------------

fn registry_dword(path: PCWSTR, value: PCWSTR) -> Option<u32> {
    let mut data = 0_u32;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: the out pointers refer to live locals of the sizes passed.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            path,
            value,
            RRF_RT_REG_DWORD,
            None,
            Some(std::ptr::from_mut(&mut data).cast()),
            Some(&mut size),
        )
    };
    status.is_ok().then_some(data)
}

/// Whether applications should use the dark appearance.
pub fn system_prefers_dark() -> bool {
    registry_dword(
        w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
        w!("AppsUseLightTheme"),
    ) == Some(0)
}

/// The accessibility text size (Settings > Accessibility > Text size).
pub fn text_scale() -> f32 {
    registry_dword(
        w!("Software\\Microsoft\\Accessibility"),
        w!("TextScaleFactor"),
    )
    .map_or(1.0, |percent| (percent as f32 / 100.0).clamp(0.5, 4.0))
}

/// Whether the user turned window animations off.
pub fn reduced_motion() -> bool {
    let mut enabled = BOOL(1);
    // SAFETY: the out pointer refers to a live BOOL.
    let read = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some(std::ptr::from_mut(&mut enabled).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    read.is_ok() && !enabled.as_bool()
}

/// Refresh rate of the monitor showing `hwnd`.
pub fn refresh_rate(hwnd: HWND) -> f32 {
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    let mut mode = DEVMODEW {
        dmSize: size_of::<DEVMODEW>() as u16,
        ..Default::default()
    };
    // SAFETY: both structures are live locals with their sizes set; the
    // device name is NUL-terminated by the system.
    let found = unsafe {
        GetMonitorInfoW(
            MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST),
            std::ptr::from_mut(&mut info).cast(),
        )
        .as_bool()
            && EnumDisplaySettingsW(
                PCWSTR(info.szDevice.as_ptr()),
                ENUM_CURRENT_SETTINGS,
                &mut mode,
            )
            .as_bool()
    };
    // 0 and 1 mean "the hardware default".
    if found && mode.dmDisplayFrequency > 1 {
        mode.dmDisplayFrequency as f32
    } else {
        60.0
    }
}

/// Dark or light title bar and frame.
pub fn apply_title_bar_theme(hwnd: HWND, dark: bool) {
    let value = BOOL::from(dark);
    // SAFETY: the attribute value is a live BOOL of the size passed.
    unsafe {
        if DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            std::ptr::from_ref(&value).cast(),
            size_of::<BOOL>() as u32,
        )
        .is_err()
        {
            // Windows 10 before 20H1 used attribute 19 for the same thing.
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWINDOWATTRIBUTE(19),
                std::ptr::from_ref(&value).cast(),
                size_of::<BOOL>() as u32,
            );
        }
    }
}

/// Applies the appearance in effect to every window.
fn refresh_appearance(app: &Rc<App>) {
    let windows = app.with_state(|state| {
        let dark = state.dark();
        state.theme = Theme::new(dark);
        // Text without a color of its own follows the theme.
        for view in state.scene.views.values_mut() {
            if let Some(text) = &mut view.text {
                text.colored_for = 0;
            }
        }
        let ids: Vec<WindowId> = state.scene.windows.keys().copied().collect();
        for id in &ids {
            state.scene.damage_all(*id);
        }
        (
            dark,
            state
                .scene
                .windows
                .values()
                .map(|entry| entry.hwnd)
                .collect::<Vec<_>>(),
        )
    });
    if let Some((dark, windows)) = windows {
        for hwnd in &windows {
            apply_title_bar_theme(*hwnd, dark);
        }
        crate::menu_theme::apply(app, dark, &windows);
    }
}

/// `Backend::set_appearance`.
pub fn set_appearance(app: &Rc<App>, dark: Option<bool>) {
    if app.with_state(|state| std::mem::replace(&mut state.forced_dark, dark) != dark) == Some(true)
    {
        refresh_appearance(app);
    }
}

/// A system setting changed: tells the core when it was the text scale or
/// the animation setting.
pub fn system_metrics_changed(app: &Rc<App>) {
    let (motion, scale) = (reduced_motion(), text_scale());
    let changed = app.with_state(|state| {
        let changed =
            state.reduced_motion != motion || (state.text_scale - scale).abs() > f32::EPSILON;
        state.reduced_motion = motion;
        state.text_scale = scale;
        changed
    });
    if changed == Some(true) {
        app.emit(BackendEvent::MetricsChanged);
    }
}

/// The system's light / dark setting may have changed.
pub fn system_appearance_changed(app: &Rc<App>) {
    let dark = system_prefers_dark();
    let changed = app.with_state(|state| {
        state.reduced_motion = reduced_motion();
        std::mem::replace(&mut state.system_dark, dark) != dark
    });
    if changed == Some(true) {
        refresh_appearance(app);
        app.emit(BackendEvent::AppearanceChanged { dark });
    }
}

// ---- menus ----------------------------------------------------------------

/// Roles that only exist on macOS have no item here.
fn role_exists(role: MenuRole) -> bool {
    !matches!(
        role,
        MenuRole::Hide | MenuRole::HideOthers | MenuRole::ShowAll
    )
}

/// A label as menu text: an ampersand would otherwise mark a mnemonic.
fn menu_label(label: &str) -> String {
    label.replace('&', "&&")
}

/// Builds a native menu. Items get command ids `items.len()` (1-based);
/// accelerators are collected when `accelerators` is given.
fn build_menu(
    nodes: &[MenuNode],
    popup: bool,
    items: &mut Vec<MenuItem>,
    accelerators: &mut Option<&mut Vec<ACCEL>>,
) -> Option<HMENU> {
    // SAFETY: menu handles created here are owned by the caller (or by
    // their parent menu once appended); strings outlive the calls.
    unsafe {
        let menu = if popup {
            CreatePopupMenu()
        } else {
            CreateMenu()
        }
        .ok()?;
        for node in nodes {
            match node {
                MenuNode::Separator => {
                    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
                }
                MenuNode::Submenu {
                    label,
                    enabled,
                    children,
                } => {
                    let Some(submenu) = build_menu(children, true, items, accelerators) else {
                        continue;
                    };
                    let label = wide(&menu_label(label));
                    let flags = MF_POPUP
                        | if *enabled {
                            MENU_ITEM_FLAGS(0)
                        } else {
                            MF_GRAYED
                        };
                    if AppendMenuW(menu, flags, submenu.0 as usize, PCWSTR(label.as_ptr())).is_err()
                    {
                        let _ = DestroyMenu(submenu);
                    }
                }
                MenuNode::Item {
                    id,
                    label,
                    accelerator,
                    enabled,
                    checked,
                    role,
                    ..
                } => {
                    if role.is_some_and(|role| !role_exists(role)) {
                        continue;
                    }
                    let activation = match role {
                        Some(role) => Activation::Role(*role),
                        None => Activation::Report(id.clone()),
                    };
                    items.push(MenuItem {
                        activation,
                        checked: *checked,
                    });
                    let command = items.len();
                    let mut text =
                        menu_label(&pam_desktop_ui::i18n::menu_label(label, *role, None));
                    if let Some(accelerator) = accelerator {
                        text.push('\t');
                        text.push_str(&events::accelerator_label(accelerator));
                        // The Windows key cannot be part of an accelerator table.
                        if let (Some(table), Some(key), false) = (
                            accelerators.as_deref_mut(),
                            events::virtual_key(&accelerator.key),
                            accelerator.modifiers.meta,
                        ) {
                            let mut flags = FVIRTKEY;
                            if accelerator.modifiers.control {
                                flags |= FCONTROL;
                            }
                            if accelerator.modifiers.alt {
                                flags |= FALT;
                            }
                            if accelerator.modifiers.shift {
                                flags |= FSHIFT;
                            }
                            if let Ok(command) = u16::try_from(command) {
                                table.push(ACCEL {
                                    fVirt: flags,
                                    key,
                                    cmd: command,
                                });
                            }
                        }
                    }
                    let text = wide(&text);
                    let mut flags = MF_STRING;
                    if !*enabled {
                        flags |= MF_GRAYED;
                    }
                    if *checked == Some(true) {
                        flags |= MF_CHECKED;
                    }
                    let _ = AppendMenuW(menu, flags, command, PCWSTR(text.as_ptr()));
                }
            }
        }
        Some(menu)
    }
}

fn main_window(app: &App) -> Option<(HWND, bool)> {
    app.with_state(|state| {
        state
            .scene
            .windows
            .get(&MAIN_WINDOW)
            .map(|entry| (entry.hwnd, entry.decorated))
    })
    .flatten()
}

/// `Backend::set_menu_bar`. Runs outside any state borrow.
pub fn set_menu_bar(app: &Rc<App>, menu: &[MenuNode]) {
    let mut built = None;
    if !menu.is_empty() {
        let mut items = Vec::new();
        let mut table: Vec<ACCEL> = Vec::new();
        if let Some(handle) = build_menu(menu, false, &mut items, &mut Some(&mut table)) {
            // SAFETY: the table slice is live for the call.
            let accelerators = if table.is_empty() {
                None
            } else {
                unsafe { CreateAcceleratorTableW(&table) }.ok()
            };
            built = Some(MenuBar {
                menu: handle,
                accelerators,
                items,
            });
        }
    }
    let attach = built.as_ref().map(|bar| bar.menu);
    let previous = app
        .with_state(|state| std::mem::replace(&mut state.shell.menu_bar, built))
        .flatten();
    // SAFETY: handles of live windows and menus of this thread. Changing
    // the menu resizes the client area, so no state borrow is held.
    unsafe {
        if let Some((hwnd, decorated)) = main_window(app) {
            // A window that draws its own frame has no place for a menu
            // bar; its accelerators still work.
            if decorated {
                let _ = SetMenu(hwnd, attach);
                let _ = DrawMenuBar(hwnd);
            }
        }
        if let Some(previous) = previous {
            let _ = DestroyMenu(previous.menu);
            if let Some(accelerators) = previous.accelerators {
                let _ = DestroyAcceleratorTable(accelerators);
            }
        }
    }
}

/// Lets the menu bar's accelerators see a message before it is dispatched.
pub fn translate_accelerator(app: &App, message: &MSG) -> bool {
    if message.message != WM_KEYDOWN && message.message != WM_SYSKEYDOWN {
        return false;
    }
    let Some(Some((accelerators, hwnd))) = app.with_state(|state| {
        Some((
            state.shell.menu_bar.as_ref()?.accelerators?,
            state.scene.windows.get(&MAIN_WINDOW)?.hwnd,
        ))
    }) else {
        return false;
    };
    let modifiers = events::modifiers();
    // SAFETY: plain queries and a translation with live handles; the
    // resulting WM_COMMAND is sent with no state borrow held.
    unsafe {
        // While a text control has the focus, plain keys are typing.
        let focused = windows::Win32::UI::Input::KeyboardAndMouse::GetFocus();
        let typing = focused != hwnd && !focused.is_invalid();
        let function_key = (0x70..=0x87).contains(&(message.wParam.0 as u32));
        if typing && !modifiers.control && !modifiers.alt && !function_key {
            return false;
        }
        TranslateAcceleratorW(hwnd, accelerators, message) != 0
    }
}

fn run_role(app: &Rc<App>, window: WindowId, role: MenuRole) {
    let hwnd = app
        .with_state(|state| state.scene.windows.get(&window).map(|entry| entry.hwnd))
        .flatten();
    match role {
        MenuRole::Quit | MenuRole::CloseWindow => app.emit(BackendEvent::WindowCloseRequested {
            window: MAIN_WINDOW,
        }),
        // The EDIT control has a single undo level: undoing again redoes.
        MenuRole::Undo | MenuRole::Redo => {
            crate::input::edit_command(windows::Win32::UI::Controls::EM_UNDO)
        }
        MenuRole::Cut => crate::input::edit_command(WM_CUT),
        MenuRole::Copy => crate::input::edit_command(WM_COPY),
        MenuRole::Paste => crate::input::edit_command(WM_PASTE),
        MenuRole::SelectAll => crate::input::edit_command(windows::Win32::UI::Controls::EM_SETSEL),
        MenuRole::Minimize => crate::window::command(app, window, &WindowCommand::Minimize),
        MenuRole::Zoom => {
            // SAFETY: plain query on a live window.
            let maximized = hwnd.is_some_and(|hwnd| unsafe { IsZoomed(hwnd) }.as_bool());
            crate::window::command(
                app,
                window,
                if maximized {
                    &WindowCommand::Restore
                } else {
                    &WindowCommand::Maximize
                },
            );
        }
        MenuRole::ToggleFullScreen => {
            let full_screen = crate::window::state_of(app, window).full_screen;
            crate::window::command(app, window, &WindowCommand::SetFullScreen(!full_screen));
        }
        // The application decides what these show.
        MenuRole::About => app.emit(BackendEvent::MenuAction {
            id: "role:about".into(),
        }),
        MenuRole::Preferences => app.emit(BackendEvent::MenuAction {
            id: "role:preferences".into(),
        }),
        MenuRole::Hide | MenuRole::HideOthers | MenuRole::ShowAll => {}
    }
}

/// `WM_COMMAND` from the menu bar or one of its accelerators.
pub fn menu_command(app: &Rc<App>, window: WindowId, command: u32) {
    let chosen = app
        .with_state(|state| {
            let bar = state.shell.menu_bar.as_mut()?;
            let item = bar.items.get_mut((command as usize).checked_sub(1)?)?;
            // Check items flip at once; the application confirms with its next menu.
            let toggled = item.checked.map(|checked| {
                item.checked = Some(!checked);
                !checked
            });
            Some((item.activation.clone(), bar.menu, toggled))
        })
        .flatten();
    let Some((activation, menu, toggled)) = chosen else {
        return;
    };
    if let Some(checked) = toggled {
        // SAFETY: the menu handle is live (owned by the shell state).
        unsafe {
            CheckMenuItem(
                menu,
                command,
                (MF_BYCOMMAND | if checked { MF_CHECKED } else { MF_UNCHECKED }).0,
            )
        };
    }
    match activation {
        Activation::Report(id) => app.emit(BackendEvent::MenuAction { id }),
        Activation::Role(role) => run_role(app, window, role),
    }
}

/// Shows a popup menu at a screen position and returns what was chosen.
fn track_menu(owner: HWND, menu: &[MenuNode], x: i32, y: i32) -> Option<Activation> {
    let mut items = Vec::new();
    let handle = build_menu(menu, true, &mut items, &mut None)?;
    // SAFETY: `owner` is a live window of this thread. This runs a modal
    // menu loop; it is only called from deferred tasks.
    let command = unsafe {
        // The owner must be the foreground window or the menu does not
        // close when the user clicks elsewhere.
        let _ = SetForegroundWindow(owner);
        let command = TrackPopupMenuEx(
            handle,
            (TPM_RETURNCMD | TPM_RIGHTBUTTON).0,
            x,
            y,
            owner,
            None,
        )
        .0;
        let _ = PostMessageW(Some(owner), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(handle);
        command
    };
    let index = usize::try_from(command).ok()?.checked_sub(1)?;
    items.into_iter().nth(index).map(|item| item.activation)
}

/// `Backend::popup_menu`.
pub fn popup_menu(
    app: &Rc<App>,
    window: WindowId,
    position: Point,
    menu: &[MenuNode],
    done: Box<dyn FnOnce(Option<String>)>,
) {
    let menu = menu.to_vec();
    app.defer(Box::new(move |app| {
        let target = app
            .with_state(|state| {
                state
                    .scene
                    .windows
                    .get(&window)
                    .map(|entry| (entry.hwnd, entry.scale()))
            })
            .flatten();
        let Some((hwnd, scale)) = target else {
            return done(None);
        };
        let mut point = POINT {
            x: (position.x * scale).round() as i32,
            y: (position.y * scale).round() as i32,
        };
        // SAFETY: plain coordinate conversion on a live window.
        unsafe {
            let _ = ClientToScreen(hwnd, &mut point);
        }
        match track_menu(hwnd, &menu, point.x, point.y) {
            Some(Activation::Report(id)) => done(Some(id)),
            Some(Activation::Role(role)) => {
                run_role(app, window, role);
                done(None);
            }
            None => done(None),
        }
        app.pump();
    }));
}

// ---- notification area ------------------------------------------------------

fn tray_data(window: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: window,
        uID: TRAY_ICON_ID,
        ..Default::default()
    }
}

fn copy_text(target: &mut [u16], text: &str) {
    let limit = target.len().saturating_sub(1);
    let mut units = text.encode_utf16().take(limit).collect::<Vec<u16>>();
    // Never cut a surrogate pair in half.
    if units
        .last()
        .is_some_and(|unit| (0xD800..0xDC00).contains(unit))
    {
        units.pop();
    }
    target[..units.len()].copy_from_slice(&units);
    target[units.len()] = 0;
}

/// The icon of the executable, or the generic application icon, and
/// whether it must be destroyed (an extracted icon is ours, a stock one is not).
fn application_icon() -> (HICON, bool) {
    let mut path = [0_u16; 1024];
    // SAFETY: the buffers are live for the calls; a null module is the executable.
    unsafe {
        let length =
            windows::Win32::System::LibraryLoader::GetModuleFileNameW(None, &mut path) as usize;
        if length > 0 && length < path.len() {
            let mut small = HICON::default();
            if ExtractIconExW(PCWSTR(path.as_ptr()), 0, None, Some(&mut small), 1) > 0
                && !small.is_invalid()
            {
                return (small, true);
            }
        }
        (LoadIconW(None, IDI_APPLICATION).unwrap_or_default(), false)
    }
}

/// Decodes an image file into a small icon.
fn icon_from_file(path: &std::path::Path) -> Option<HICON> {
    use std::os::windows::ffi::OsStrExt;
    let wide_path: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: COM and GDI calls on objects created here; the bitmaps are
    // deleted once the icon holds its own copies.
    unsafe {
        let (width, height) = (
            GetSystemMetrics(SM_CXSMICON).max(16) as u32,
            GetSystemMetrics(SM_CYSMICON).max(16) as u32,
        );
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
        let decoder = factory
            .CreateDecoderFromFilename(
                PCWSTR(wide_path.as_ptr()),
                None,
                windows::Win32::Foundation::GENERIC_READ,
                WICDecodeMetadataCacheOnDemand,
            )
            .ok()?;
        let frame: IWICBitmapSource = decoder.GetFrame(0).ok()?.cast().ok()?;
        let scaler = factory.CreateBitmapScaler().ok()?;
        scaler
            .Initialize(&frame, width, height, WICBitmapInterpolationModeFant)
            .ok()?;
        let converter = factory.CreateFormatConverter().ok()?;
        converter
            .Initialize(
                &scaler,
                &GUID_WICPixelFormat32bppBGRA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeMedianCut,
            )
            .ok()?;
        let mut pixels = vec![0_u8; (width * height * 4) as usize];
        converter
            .CopyPixels(std::ptr::null(), width * 4, &mut pixels)
            .ok()?;
        let color = CreateBitmap(
            width as i32,
            height as i32,
            1,
            32,
            Some(pixels.as_ptr().cast()),
        );
        // The alpha channel decides transparency; the mask is all zeros
        // (rows are padded to 16 bits).
        let mask_bits = vec![0_u8; (width.div_ceil(16) * 2 * height) as usize];
        let mask = CreateBitmap(
            width as i32,
            height as i32,
            1,
            1,
            Some(mask_bits.as_ptr().cast()),
        );
        let info = ICONINFO {
            fIcon: true.into(),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color,
        };
        let icon = CreateIconIndirect(&info).ok();
        let _ = DeleteObject(HGDIOBJ(color.0));
        let _ = DeleteObject(HGDIOBJ(mask.0));
        icon
    }
}

fn ensure_tray_window(app: &Rc<App>) -> Option<HWND> {
    if let Some(Some(window)) = app.with_state(|state| state.shell.tray_window) {
        return Some(window);
    }
    let class = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(tray_proc),
        hInstance: app.instance,
        lpszClassName: TRAY_CLASS,
        ..Default::default()
    };
    // SAFETY: standard registration and creation of a hidden top-level
    // window (not message-only: it must receive broadcasts and be able to
    // own a popup menu).
    let window = unsafe {
        RegisterClassExW(&class);
        let created = RegisterWindowMessageW(w!("TaskbarCreated"));
        app.with_state(|state| state.shell.taskbar_created = created);
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            TRAY_CLASS,
            PCWSTR::null(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(app.instance),
            None,
        )
        .ok()?
    };
    app.with_state(|state| state.shell.tray_window = Some(window));
    Some(window)
}

/// Adds or updates the icon in the notification area.
fn show_tray_icon(window: HWND, icon: HICON, tooltip: &str, add: bool) {
    let mut data = tray_data(window);
    data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
    data.uCallbackMessage = WM_APP_TRAY;
    data.hIcon = icon;
    copy_text(&mut data.szTip, tooltip);
    // SAFETY: the structure is fully initialized and live for the calls.
    unsafe {
        if add {
            let _ = Shell_NotifyIconW(NIM_ADD, &data);
            data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            let _ = Shell_NotifyIconW(NIM_SETVERSION, &data);
        } else {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
        }
    }
}

/// Removes the icon from the notification area and releases it.
fn remove_tray(window: HWND, tray: Tray) {
    // SAFETY: removing our own icon and releasing a handle we own.
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &tray_data(window));
        if tray.owns_icon && !tray.icon.is_invalid() {
            let _ = DestroyIcon(tray.icon);
        }
    }
}

/// `Backend::set_tray`. Runs outside any state borrow.
pub fn set_tray(app: &Rc<App>, config: Option<&TrayConfig>) {
    let Some(window) = (match config {
        Some(_) => ensure_tray_window(app),
        None => app.with_state(|state| state.shell.tray_window).flatten(),
    }) else {
        return;
    };
    let Some(config) = config else {
        if let Some(Some(tray)) = app.with_state(|state| state.shell.tray.take()) {
            remove_tray(window, tray);
        }
        return;
    };
    let (icon, owns_icon) = match config.icon.as_deref().and_then(icon_from_file) {
        Some(icon) => (icon, true),
        None => application_icon(),
    };
    let tray = Tray {
        icon,
        owns_icon,
        tooltip: config.tooltip.clone(),
        menu: config.menu.clone(),
        temporary: false,
        balloon: String::new(),
    };
    let previous = app
        .with_state(|state| state.shell.tray.replace(tray))
        .flatten();
    show_tray_icon(window, icon, &config.tooltip, previous.is_none());
    if let Some(previous) = previous {
        // SAFETY: the previous icon is no longer shown.
        unsafe {
            if previous.owns_icon && !previous.icon.is_invalid() {
                let _ = DestroyIcon(previous.icon);
            }
        }
    }
}

fn tray_event(app: &Rc<App>, hwnd: HWND, event: u32) {
    match event {
        // Version 4 reports a click as NIN_SELECT (after the raw button
        // messages, which are ignored so one click activates once).
        NIN_SELECT | NIN_KEYSELECT => {
            app.emit(BackendEvent::TrayActivated);
            app.pump();
        }
        WM_CONTEXTMENU => {
            let menu = app
                .with_state(|state| state.shell.tray.as_ref().map(|tray| tray.menu.clone()))
                .flatten()
                .unwrap_or_default();
            if menu.is_empty() {
                return;
            }
            app.defer(Box::new(move |app| {
                let mut cursor = POINT::default();
                // SAFETY: the point is a live local.
                unsafe {
                    let _ = GetCursorPos(&mut cursor);
                }
                match track_menu(hwnd, &menu, cursor.x, cursor.y) {
                    Some(Activation::Report(id)) => app.emit(BackendEvent::MenuAction { id }),
                    Some(Activation::Role(role)) => run_role(app, MAIN_WINDOW, role),
                    None => {}
                }
                app.pump();
            }));
        }
        NIN_BALLOONUSERCLICK => {
            let id = app
                .with_state(|state| {
                    state
                        .shell
                        .tray
                        .as_mut()
                        .map(|tray| std::mem::take(&mut tray.balloon))
                })
                .flatten()
                .unwrap_or_default();
            app.emit(BackendEvent::NotificationActivated { id });
            end_balloon(app);
            app.pump();
        }
        NIN_BALLOONTIMEOUT | NIN_BALLOONHIDE => end_balloon(app),
        _ => {}
    }
}

/// An icon that only existed to show a balloon goes away with it.
fn end_balloon(app: &Rc<App>) {
    let temporary = app
        .with_state(|state| {
            let window = state.shell.tray_window?;
            let tray = if state.shell.tray.as_ref().is_some_and(|tray| tray.temporary) {
                state.shell.tray.take()
            } else {
                None
            };
            tray.map(|tray| (window, tray))
        })
        .flatten();
    if let Some((window, tray)) = temporary {
        remove_tray(window, tray);
    }
}

extern "system" fn tray_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let handled = std::panic::catch_unwind(|| {
        let Some(app) = App::current() else {
            return false;
        };
        if message == WM_APP_TRAY {
            // Version 4 packs the event in the low word of lParam.
            tray_event(&app, hwnd, u32::from(loword(lparam.0 as usize)));
            return true;
        }
        let recreated = app.with_state(|state| {
            state.shell.taskbar_created != 0 && message == state.shell.taskbar_created
        }) == Some(true);
        if recreated {
            // Explorer restarted: the icon must be added again.
            let tray = app
                .with_state(|state| {
                    state
                        .shell
                        .tray
                        .as_ref()
                        .filter(|tray| !tray.temporary)
                        .map(|tray| (tray.icon, tray.tooltip.clone()))
                })
                .flatten();
            if let Some((icon, tooltip)) = tray {
                show_tray_icon(hwnd, icon, &tooltip, true);
            }
        }
        false
    });
    match handled {
        Ok(true) => LRESULT(0),
        // SAFETY: forwarding the unmodified message to the default procedure.
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

// ---- notifications -----------------------------------------------------------

fn escape_xml(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            control if control.is_control() && control != '\n' => {}
            other => escaped.push(other),
        }
    }
    escaped
}

/// The explicit AppUserModelID of the process, when it has one.
fn app_user_model_id() -> Option<String> {
    // SAFETY: the returned string is copied and then freed with the COM allocator.
    unsafe {
        let id = GetCurrentProcessExplicitAppUserModelID().ok()?;
        let text = id.to_string().ok();
        CoTaskMemFree(Some(id.0.cast()));
        text.filter(|text| !text.is_empty())
    }
}

/// Unpackaged applications show toasts once their AppUserModelID has a
/// display name in the registry (no Start menu shortcut needed).
fn register_toast_identity(app: &App, identity: &str) {
    let path = wide(&format!("Software\\Classes\\AppUserModelId\\{identity}"));
    let mut key = HKEY::default();
    // SAFETY: registry calls with NUL-terminated strings and live out pointers; the key is closed.
    unsafe {
        if RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(path.as_ptr()),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_READ | KEY_WRITE,
            None,
            &mut key,
            None,
        )
        .is_err()
        {
            return;
        }
        if RegQueryValueExW(key, w!("DisplayName"), None, None, None, None).is_err() {
            let mut title = [0_u16; 256];
            let length = main_window(app).map_or(0, |(hwnd, _)| {
                GetWindowTextW(hwnd, &mut title).max(0) as usize
            });
            let name = if length > 0 {
                from_wide(&title[..length])
            } else {
                identity.to_owned()
            };
            let value = wide(&name);
            let bytes = std::slice::from_raw_parts(value.as_ptr().cast::<u8>(), value.len() * 2);
            let _ = RegSetValueExW(key, w!("DisplayName"), None, REG_SZ, Some(bytes));
        }
        let _ = RegCloseKey(key);
    }
}

fn show_toast(app: &Rc<App>, notification: &NotificationConfig) -> windows::core::Result<()> {
    let identity = app_user_model_id()
        .ok_or_else(|| windows::core::Error::from(windows::Win32::Foundation::E_FAIL))?;
    register_toast_identity(app, &identity);
    let mut xml = format!(
        "<toast launch=\"{}\"><visual><binding template=\"ToastGeneric\"><text>{}</text>",
        escape_xml(&notification.id),
        escape_xml(&notification.title)
    );
    if !notification.body.is_empty() {
        xml.push_str(&format!("<text>{}</text>", escape_xml(&notification.body)));
    }
    xml.push_str("</binding></visual></toast>");
    let document = XmlDocument::new()?;
    document.LoadXml(&HSTRING::from(xml))?;
    let toast = ToastNotification::CreateToastNotification(&document)?;
    if !notification.id.is_empty() {
        // A newer notification with the same id replaces the older one.
        let tag: String = notification.id.chars().take(64).collect();
        let _ = toast.SetTag(&HSTRING::from(tag));
    }
    let mailbox = Arc::clone(&app.mailbox);
    let id = notification.id.clone();
    // The handler runs on a system thread: it only posts to the UI thread.
    toast.Activated(&TypedEventHandler::new(move |_, _| {
        mailbox.post(Mail::Notification(id.clone()));
        Ok(())
    }))?;
    ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(identity))?.Show(&toast)?;
    app.with_state(|state| {
        if state.shell.toasts.len() >= TOAST_HISTORY {
            state.shell.toasts.remove(0);
        }
        state.shell.toasts.push(toast);
    });
    Ok(())
}

/// A balloon from the notification area icon (added for the occasion when
/// the application has none).
fn show_balloon(app: &Rc<App>, notification: &NotificationConfig) {
    let Some(window) = ensure_tray_window(app) else {
        return;
    };
    let missing = app.with_state(|state| state.shell.tray.is_none()) == Some(true);
    if missing {
        let (icon, owns_icon) = application_icon();
        let tray = Tray {
            icon,
            owns_icon,
            tooltip: notification.title.clone(),
            menu: Vec::new(),
            temporary: true,
            balloon: String::new(),
        };
        app.with_state(|state| state.shell.tray = Some(tray));
        show_tray_icon(window, icon, &notification.title, true);
    }
    app.with_state(|state| {
        if let Some(tray) = state.shell.tray.as_mut() {
            tray.balloon.clone_from(&notification.id);
        }
    });
    let mut data = tray_data(window);
    data.uFlags = NIF_INFO;
    data.dwInfoFlags = NIIF_INFO;
    copy_text(&mut data.szInfoTitle, &notification.title);
    // An empty text removes the balloon instead of showing it.
    copy_text(
        &mut data.szInfo,
        if notification.body.is_empty() {
            " "
        } else {
            &notification.body
        },
    );
    // SAFETY: the structure is fully initialized and live for the call.
    unsafe {
        let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
    }
}

/// `Backend::notify`.
pub fn notify(app: &Rc<App>, notification: &NotificationConfig) {
    let notification = notification.clone();
    // Toast activation talks to another process: keep it out of the runtime.
    app.defer(Box::new(move |app| {
        if show_toast(app, &notification).is_err() {
            show_balloon(app, &notification);
        }
    }));
}

// ---- file dialogs --------------------------------------------------------------

fn shell_item_path(item: &IShellItem) -> Option<PathBuf> {
    // SAFETY: the returned string is copied and then freed with the COM allocator.
    unsafe {
        let name = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let path = PathBuf::from(std::ffi::OsString::from(name.to_string().ok()?));
        CoTaskMemFree(Some(name.0.cast()));
        Some(path)
    }
}

fn run_file_dialog(
    owner: Option<HWND>,
    config: &FileDialogConfig,
) -> windows::core::Result<Vec<PathBuf>> {
    let save = config.kind == FileDialogKind::Save;
    // SAFETY: COM calls on the dialog created here; every string and the
    // filter array outlive the calls that read them. `Show` runs a modal
    // loop; this is only called from deferred tasks.
    unsafe {
        let dialog: IFileDialog = CoCreateInstance(
            if save {
                &FileSaveDialog
            } else {
                &FileOpenDialog
            },
            None,
            CLSCTX_INPROC_SERVER,
        )?;
        let mut options = dialog.GetOptions()? | FOS_FORCEFILESYSTEM;
        match config.kind {
            FileDialogKind::OpenMultiple => options |= FOS_ALLOWMULTISELECT,
            FileDialogKind::SelectFolder => options |= FOS_PICKFOLDERS,
            FileDialogKind::Open | FileDialogKind::Save => {}
        }
        dialog.SetOptions(options)?;
        if !config.title.is_empty() {
            let _ = dialog.SetTitle(&HSTRING::from(config.title.as_str()));
        }
        if !config.default_name.is_empty() {
            let _ = dialog.SetFileName(&HSTRING::from(config.default_name.as_str()));
        }
        if let Some(directory) = &config.directory
            && let Ok(folder) = SHCreateItemFromParsingName::<_, _, IShellItem>(
                &HSTRING::from(directory.as_os_str()),
                None,
            )
        {
            let _ = dialog.SetFolder(&folder);
        }
        if !config.filters.is_empty() && config.kind != FileDialogKind::SelectFolder {
            let names: Vec<Vec<u16>> = config
                .filters
                .iter()
                .map(|filter| wide(&filter.name))
                .collect();
            let patterns: Vec<Vec<u16>> = config
                .filters
                .iter()
                .map(|filter| {
                    let pattern: Vec<String> = filter
                        .extensions
                        .iter()
                        .map(|extension| {
                            if extension == "*" {
                                String::from("*.*")
                            } else {
                                format!("*.{extension}")
                            }
                        })
                        .collect();
                    wide(&if pattern.is_empty() {
                        String::from("*.*")
                    } else {
                        pattern.join(";")
                    })
                })
                .collect();
            let specs: Vec<COMDLG_FILTERSPEC> = names
                .iter()
                .zip(&patterns)
                .map(|(name, pattern)| COMDLG_FILTERSPEC {
                    pszName: PCWSTR(name.as_ptr()),
                    pszSpec: PCWSTR(pattern.as_ptr()),
                })
                .collect();
            let _ = dialog.SetFileTypes(&specs);
            if save {
                // The first extension of the first filter completes a name typed without one.
                if let Some(extension) = config
                    .filters
                    .first()
                    .and_then(|filter| filter.extensions.first())
                    .filter(|extension| *extension != "*")
                {
                    let _ = dialog.SetDefaultExtension(&HSTRING::from(extension.as_str()));
                }
            }
        }
        // Cancelling is reported as an error: no paths.
        if dialog.Show(owner).is_err() {
            return Ok(Vec::new());
        }
        if config.kind == FileDialogKind::OpenMultiple {
            let results = dialog.cast::<IFileOpenDialog>()?.GetResults()?;
            let count = results.GetCount()?;
            return Ok((0..count)
                .filter_map(|index| results.GetItemAt(index).ok())
                .filter_map(|item| shell_item_path(&item))
                .collect());
        }
        Ok(dialog
            .GetResult()
            .ok()
            .and_then(|item| shell_item_path(&item))
            .into_iter()
            .collect())
    }
}

/// `Backend::pick_files`.
pub fn pick_files(
    app: &Rc<App>,
    window: WindowId,
    config: &FileDialogConfig,
    done: Box<dyn FnOnce(Vec<PathBuf>)>,
) {
    let config = config.clone();
    app.defer(Box::new(move |app| {
        let owner = app
            .with_state(|state| state.scene.windows.get(&window).map(|entry| entry.hwnd))
            .flatten();
        done(run_file_dialog(owner, &config).unwrap_or_default());
        app.pump();
    }));
}

// ---- alerts ----------------------------------------------------------------------

type TaskDialogIndirect =
    unsafe extern "system" fn(*const TASKDIALOGCONFIG, *mut i32, *mut i32, *mut BOOL) -> HRESULT;

/// `TaskDialogIndirect` only exists in version 6 of the common controls,
/// which a process gets through its manifest: it is looked up at run time
/// so an executable without that manifest still starts.
fn task_dialog() -> Option<TaskDialogIndirect> {
    // SAFETY: the library stays loaded for the life of the process; the
    // symbol, when present, has the documented signature.
    unsafe {
        let module = LoadLibraryW(w!("comctl32.dll")).ok()?;
        let symbol = GetProcAddress(module, PCSTR(c"TaskDialogIndirect".as_ptr().cast()))?;
        Some(std::mem::transmute::<
            unsafe extern "system" fn() -> isize,
            TaskDialogIndirect,
        >(symbol))
    }
}

/// Button ids start above the predefined ones (IDOK, IDCANCEL…).
const ALERT_BUTTON_BASE: i32 = 100;

fn run_alert(owner: Option<HWND>, alert: &AlertConfig) -> Option<usize> {
    let title = wide(&alert.title);
    let message = wide(&alert.message);
    let labels: Vec<Vec<u16>> = alert
        .buttons
        .iter()
        .take(3)
        .map(|button| wide(&button.label))
        .collect();
    let cancel = alert
        .buttons
        .iter()
        .take(3)
        .position(|button| button.style == AlertButtonStyle::Cancel);
    // The first button that is not "cancel" answers to Enter.
    let default = alert
        .buttons
        .iter()
        .take(3)
        .position(|button| button.style != AlertButtonStyle::Cancel)
        .unwrap_or(0);
    if let Some(show) = task_dialog() {
        let buttons: Vec<TASKDIALOG_BUTTON> = labels
            .iter()
            .enumerate()
            .map(|(index, label)| TASKDIALOG_BUTTON {
                // The cancel button gets IDCANCEL so Escape and the close box choose it.
                nButtonID: if Some(index) == cancel {
                    IDCANCEL.0
                } else {
                    ALERT_BUTTON_BASE + index as i32
                },
                pszButtonText: PCWSTR(label.as_ptr()),
            })
            .collect();
        let mut config = TASKDIALOGCONFIG {
            cbSize: size_of::<TASKDIALOGCONFIG>() as u32,
            ..Default::default()
        };
        config.hwndParent = owner.unwrap_or_default();
        config.dwFlags = TDF_POSITION_RELATIVE_TO_WINDOW
            | if cancel.is_some() || buttons.is_empty() {
                TDF_ALLOW_DIALOG_CANCELLATION
            } else {
                Default::default()
            };
        config.pszWindowTitle = PCWSTR(title.as_ptr());
        config.pszMainInstruction = PCWSTR(title.as_ptr());
        config.pszContent = PCWSTR(message.as_ptr());
        if buttons.is_empty() {
            config.dwCommonButtons = windows::Win32::UI::Controls::TDCBF_OK_BUTTON;
        } else {
            config.cButtons = buttons.len() as u32;
            config.pButtons = buttons.as_ptr();
            config.nDefaultButton = if Some(default) == cancel {
                IDCANCEL.0
            } else {
                ALERT_BUTTON_BASE + default as i32
            };
        }
        let mut pressed = 0_i32;
        // SAFETY: the configuration and everything it points to outlive the
        // call, which runs a modal loop (deferred task only).
        let shown = unsafe {
            show(
                &config,
                &mut pressed,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if shown.is_ok() {
            return match pressed {
                id if id == IDCANCEL.0 => cancel,
                id if id == IDOK.0 && buttons.is_empty() => Some(0),
                id if id >= ALERT_BUTTON_BASE => {
                    Some((id - ALERT_BUTTON_BASE) as usize).filter(|index| *index < buttons.len())
                }
                _ => None,
            };
        }
    }
    // Without version 6 of the common controls: a message box, whose
    // buttons cannot be relabeled.
    let style = match labels.len() {
        0 | 1 => MB_OK,
        2 => MB_OKCANCEL,
        _ => MB_YESNOCANCEL,
    };
    let text = wide(&if alert.message.is_empty() {
        alert.title.clone()
    } else {
        format!("{}\n\n{}", alert.title, alert.message)
    });
    // SAFETY: the strings are NUL-terminated and outlive the modal call.
    let pressed =
        unsafe { MessageBoxW(owner, PCWSTR(text.as_ptr()), PCWSTR(title.as_ptr()), style) };
    match (labels.len(), pressed) {
        (0 | 1, result) if result == IDOK => Some(0),
        (2, result) if result == IDOK => Some(0),
        (2, result) if result == IDCANCEL => Some(1),
        (_, result) if result == IDYES => Some(0),
        (_, result) if result == IDNO => Some(1),
        (3, result) if result == IDCANCEL => Some(2),
        _ => None,
    }
}

/// `Backend::show_alert`.
pub fn show_alert(
    app: &Rc<App>,
    window: WindowId,
    alert: &AlertConfig,
    done: Box<dyn FnOnce(Option<usize>)>,
) {
    let alert = alert.clone();
    app.defer(Box::new(move |app| {
        let owner = app
            .with_state(|state| state.scene.windows.get(&window).map(|entry| entry.hwnd))
            .flatten();
        done(run_alert(owner, &alert));
        app.pump();
    }));
}

// ---- clipboard ------------------------------------------------------------------

/// `Backend::clipboard_set_text`.
pub fn clipboard_set_text(app: &App, text: &str) {
    let units = wide(text);
    // SAFETY: the global block is sized for the text and unlocked before
    // being handed to the clipboard, which then owns it.
    unsafe {
        if OpenClipboard(Some(app.message_window)).is_err() {
            return;
        }
        let _ = EmptyClipboard();
        if let Ok(block) = GlobalAlloc(GMEM_MOVEABLE, units.len() * 2) {
            let target = GlobalLock(block).cast::<u16>();
            if target.is_null() {
                let _ = windows::Win32::Foundation::GlobalFree(Some(block));
            } else {
                std::ptr::copy_nonoverlapping(units.as_ptr(), target, units.len());
                let _ = GlobalUnlock(block);
                if SetClipboardData(u32::from(CF_UNICODETEXT.0), Some(HANDLE(block.0))).is_err() {
                    let _ = windows::Win32::Foundation::GlobalFree(Some(block));
                }
            }
        }
        let _ = CloseClipboard();
    }
}

fn clipboard_read(app: &App) -> Option<String> {
    // SAFETY: the clipboard block is only read while locked and while the
    // clipboard is open; its size bounds the read.
    unsafe {
        IsClipboardFormatAvailable(u32::from(CF_UNICODETEXT.0)).ok()?;
        OpenClipboard(Some(app.message_window)).ok()?;
        let text = GetClipboardData(u32::from(CF_UNICODETEXT.0))
            .ok()
            .and_then(|handle| {
                let block = HGLOBAL(handle.0);
                let source = GlobalLock(block).cast::<u16>();
                if source.is_null() {
                    return None;
                }
                let units = std::slice::from_raw_parts(source, GlobalSize(block) / 2);
                let text = from_wide(units);
                let _ = GlobalUnlock(block);
                Some(text)
            });
        let _ = CloseClipboard();
        text
    }
}

/// `Backend::clipboard_text`: `done` runs later, from the message loop.
pub fn clipboard_text(app: &Rc<App>, done: Box<dyn FnOnce(Option<String>)>) {
    app.defer(Box::new(move |app| {
        done(clipboard_read(app));
        app.pump();
    }));
}

// ---- global hot keys -------------------------------------------------------------

/// `Backend::set_shortcuts`: local shortcuts are matched by the core from
/// key events; global ones are registered with the system.
pub fn set_shortcuts(app: &Rc<App>, shortcuts: &[Shortcut]) {
    let previous = app
        .with_state(|state| std::mem::take(&mut state.shell.hot_keys))
        .unwrap_or_default();
    // SAFETY: hot keys are registered on our own message window.
    unsafe {
        for index in 0..previous.len() {
            let _ = UnregisterHotKey(Some(app.message_window), index as i32 + 1);
        }
    }
    let mut registered = Vec::new();
    for shortcut in shortcuts.iter().filter(|shortcut| shortcut.global) {
        let Some(key) = events::virtual_key(&shortcut.accelerator.key) else {
            continue;
        };
        let mut modifiers = MOD_NOREPEAT;
        let held = shortcut.accelerator.modifiers;
        for (flag, wanted) in [
            (MOD_CONTROL, held.control),
            (MOD_ALT, held.alt),
            (MOD_SHIFT, held.shift),
            (MOD_WIN, held.meta),
        ] {
            if wanted {
                modifiers |= flag;
            }
        }
        let id = registered.len() as i32 + 1;
        // SAFETY: as above. A combination another application owns fails
        // to register and is simply left out.
        if unsafe {
            RegisterHotKey(
                Some(app.message_window),
                id,
                HOT_KEY_MODIFIERS(modifiers.0),
                u32::from(key),
            )
        }
        .is_ok()
        {
            registered.push(shortcut.id.clone());
        }
    }
    app.with_state(|state| state.shell.hot_keys = registered);
}

/// `WM_HOTKEY` on the message window.
pub fn hot_key(app: &Rc<App>, id: i32) {
    let shortcut = app
        .with_state(|state| {
            usize::try_from(id - 1)
                .ok()
                .and_then(|index| state.shell.hot_keys.get(index).cloned())
        })
        .flatten();
    if let Some(id) = shortcut {
        app.emit(BackendEvent::Shortcut { id });
    }
}

// ---- taskbar ----------------------------------------------------------------------

/// Progress on the taskbar button (`None` removes it).
pub fn taskbar_progress(hwnd: HWND, progress: Option<f32>) {
    let Some(app) = App::current() else { return };
    let taskbar = app
        .with_state(|state| state.shell.taskbar.clone())
        .flatten()
        .or_else(|| {
            // SAFETY: standard in-process activation; HrInit must precede other calls.
            let created: Option<ITaskbarList3> = unsafe {
                CoCreateInstance::<_, ITaskbarList3>(&TaskbarList, None, CLSCTX_INPROC_SERVER)
                    .ok()
                    .filter(|taskbar| taskbar.HrInit().is_ok())
            };
            app.with_state(|state| state.shell.taskbar.clone_from(&created));
            created
        });
    let Some(taskbar) = taskbar else { return };
    // SAFETY: COM calls on the live taskbar object with a live window.
    unsafe {
        match progress {
            Some(progress) => {
                let _ = taskbar.SetProgressState(hwnd, TBPF_NORMAL);
                let _ = taskbar.SetProgressValue(
                    hwnd,
                    (progress.clamp(0.0, 1.0) * 1000.0).round() as u64,
                    1000,
                );
            }
            None => {
                let _ = taskbar.SetProgressState(hwnd, TBPF_NOPROGRESS);
            }
        }
    }
}

/// Releases shell resources at shutdown.
pub fn shutdown(app: &Rc<App>) {
    set_menu_bar(app, &[]);
    set_shortcuts(app, &[]);
    set_tray(app, None);
    if let Some(Some(window)) = app.with_state(|state| state.shell.tray_window.take()) {
        // SAFETY: destroying our own hidden window; no state borrow is held.
        unsafe {
            let _ = DestroyWindow(window);
        }
    }
}

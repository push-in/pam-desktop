//! Dark menus. Win32 has no public dark mode for them: popup and context
//! menus follow the application mode of uxtheme, set through entry points
//! exported by ordinal only, and the menu bar is drawn by the window from
//! the undocumented messages the theme engine sends for it. Open-source
//! Win32 applications have done both since Windows 10 1809.
//!
//! Nothing here runs while the appearance has only ever been light.

use std::sync::OnceLock;

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    CreateSolidBrush, DT_CENTER, DT_HIDEPREFIX, DT_SINGLELINE, DT_VCENTER, DeleteObject, FillRect,
    GetWindowDC, HDC, MapWindowPoints, RDW_FRAME, RDW_INVALIDATE, RedrawWindow, ReleaseDC,
};
use windows::Win32::System::LibraryLoader::{
    GetModuleHandleW, GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
};
use windows::Win32::UI::Controls::{
    CloseThemeData, DRAWITEMSTRUCT, DTT_TEXTCOLOR, DTTOPTS, DrawThemeTextEx, MBI_NORMAL,
    MENU_BARITEM, ODS_DISABLED, ODS_GRAYED, ODS_HOTLIGHT, ODS_NOACCEL, ODS_SELECTED, OpenThemeData,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClientRect, GetMenu, GetMenuBarInfo, GetMenuItemInfoW, GetWindowRect, HMENU, MENUBARINFO,
    MENUITEMINFOW, MIIM_STRING, OBJID_MENU,
};
use windows::core::{PCSTR, PWSTR, w};

use crate::app::App;

/// Sent to a window to draw the background of its menu bar.
pub const WM_UAHDRAWMENU: u32 = 0x0091;
/// Sent to a window to draw one item of its menu bar.
pub const WM_UAHDRAWMENUITEM: u32 = 0x0092;

/// The first build with the dark mode entry points (Windows 10 1809).
const FIRST_DARK_BUILD: u32 = 17_763;
/// From this build (Windows 10 1903) ordinal 135 takes a mode; before, it
/// took a boolean (`AllowDarkModeForApp`).
const FIRST_APP_MODE_BUILD: u32 = 18_362;
/// `PreferredAppMode::ForceDark`: dark whatever the system setting is,
/// since the application may choose its own appearance.
const APP_MODE_FORCE_DARK: i32 = 2;

// The colors of the system's own dark popup menus.
const BAR: COLORREF = COLORREF(0x002B_2B2B);
const LIT_ITEM: COLORREF = COLORREF(0x0041_4141);
const TEXT: COLORREF = COLORREF(0x00FF_FFFF);
const DISABLED_TEXT: COLORREF = COLORREF(0x006D_6D6D);

/// The dark mode entry points of uxtheme.
struct DarkMode {
    /// Ordinal 133, `AllowDarkModeForWindow`.
    allow_for_window: unsafe extern "system" fn(HWND, bool) -> bool,
    /// Ordinal 135, `SetPreferredAppMode`.
    set_app_mode: unsafe extern "system" fn(i32) -> i32,
    /// Ordinal 136, `FlushMenuThemes`.
    flush_menu_themes: unsafe extern "system" fn(),
    /// Ordinal 135 takes a mode, not a boolean.
    modes: bool,
}

/// The build number of Windows 10 or later; `None` on anything older.
fn windows_build() -> Option<u32> {
    /// `OSVERSIONINFOW`.
    #[repr(C)]
    struct Version {
        size: u32,
        major: u32,
        minor: u32,
        build: u32,
        platform: u32,
        service_pack: [u16; 128],
    }
    let mut version = Version {
        size: size_of::<Version>() as u32,
        major: 0,
        minor: 0,
        build: 0,
        platform: 0,
        service_pack: [0; 128],
    };
    // SAFETY: ntdll is loaded in every process. `RtlGetVersion` has the
    // documented signature transmuted to and fills a structure whose size
    // is set; unlike `GetVersionExW` it does not depend on the manifest.
    let status = unsafe {
        let ntdll = GetModuleHandleW(w!("ntdll.dll")).ok()?;
        let symbol = GetProcAddress(ntdll, PCSTR(c"RtlGetVersion".as_ptr().cast()))?;
        let get_version = std::mem::transmute::<
            unsafe extern "system" fn() -> isize,
            unsafe extern "system" fn(*mut Version) -> i32,
        >(symbol);
        get_version(&mut version)
    };
    (status >= 0 && version.major >= 10).then_some(version.build)
}

/// The entry points, resolved once. `None` before Windows 10 1809 and
/// when uxtheme does not export one of them.
fn dark_mode() -> Option<&'static DarkMode> {
    static CALLS: OnceLock<Option<DarkMode>> = OnceLock::new();
    CALLS
        .get_or_init(|| {
            let build = windows_build().filter(|build| *build >= FIRST_DARK_BUILD)?;
            // SAFETY: uxtheme stays loaded for the life of the process. A
            // name below 64K is an ordinal; on the builds accepted above
            // these three have the signatures transmuted to.
            unsafe {
                let uxtheme =
                    LoadLibraryExW(w!("uxtheme.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32).ok()?;
                let ordinal = |number: usize| {
                    GetProcAddress(uxtheme, PCSTR(std::ptr::without_provenance(number)))
                };
                Some(DarkMode {
                    allow_for_window: std::mem::transmute::<
                        unsafe extern "system" fn() -> isize,
                        unsafe extern "system" fn(HWND, bool) -> bool,
                    >(ordinal(133)?),
                    set_app_mode: std::mem::transmute::<
                        unsafe extern "system" fn() -> isize,
                        unsafe extern "system" fn(i32) -> i32,
                    >(ordinal(135)?),
                    flush_menu_themes: std::mem::transmute::<
                        unsafe extern "system" fn() -> isize,
                        unsafe extern "system" fn(),
                    >(ordinal(136)?),
                    modes: build >= FIRST_APP_MODE_BUILD,
                })
            }
        })
        .as_ref()
}

/// Makes menus follow the appearance: the popup and context menus of the
/// application, and the menu bar of each of `windows`. Does nothing on a
/// system without the entry points, and nothing while the appearance has
/// only ever been light. Dispatches no message: the bars are repainted
/// with the next paint.
pub fn apply(app: &App, dark: bool, windows: &[HWND]) {
    let changed = app.menus_dark.get() != dark;
    if !changed && !dark {
        return;
    }
    let Some(calls) = dark_mode() else {
        return;
    };
    // SAFETY: the entry points were resolved for this build of Windows;
    // the windows are live windows of this thread.
    unsafe {
        if changed {
            let mode = match (dark, calls.modes) {
                (true, true) => APP_MODE_FORCE_DARK,
                // Before modes: dark menus are allowed, and the system
                // setting decides.
                (true, false) => 1,
                (false, _) => 0,
            };
            (calls.set_app_mode)(mode);
            (calls.flush_menu_themes)();
            app.menus_dark.set(dark);
        }
        for hwnd in windows {
            (calls.allow_for_window)(*hwnd, dark);
            if changed && !GetMenu(*hwnd).is_invalid() {
                let _ = RedrawWindow(Some(*hwnd), None, None, RDW_FRAME | RDW_INVALIDATE);
            }
        }
    }
}

/// `UAHMENU`, what `WM_UAHDRAWMENU` points to.
#[repr(C)]
struct UahMenu {
    hmenu: HMENU,
    hdc: HDC,
    _flags: u32,
}

/// The start of `UAHDRAWMENUITEM`, what `WM_UAHDRAWMENUITEM` points to.
/// The item metrics that follow the position are not read.
#[repr(C)]
struct UahDrawMenuItem {
    item: DRAWITEMSTRUCT,
    menu: UahMenu,
    /// Position of the item in the bar.
    position: i32,
}

fn fill(hdc: HDC, rect: &RECT, color: COLORREF) -> bool {
    // SAFETY: GDI calls with a live device context and rectangle; the
    // brush is deleted once it is no longer selected anywhere.
    unsafe {
        let brush = CreateSolidBrush(color);
        if brush.is_invalid() {
            return false;
        }
        let filled = FillRect(hdc, rect, brush) != 0;
        let _ = DeleteObject(brush.into());
        filled
    }
}

/// The menu bar of `hwnd` in the coordinates of its window, the ones a
/// window device context draws in.
fn bar_rect(hwnd: HWND) -> Option<RECT> {
    let mut info = MENUBARINFO {
        cbSize: size_of::<MENUBARINFO>() as u32,
        ..Default::default()
    };
    let mut window = RECT::default();
    // SAFETY: both structures are live locals; the size of the first is set.
    unsafe {
        GetMenuBarInfo(hwnd, OBJID_MENU, 0, &mut info).ok()?;
        GetWindowRect(hwnd, &mut window).ok()?;
    }
    Some(RECT {
        left: info.rcBar.left - window.left,
        top: info.rcBar.top - window.top,
        right: info.rcBar.right - window.left,
        bottom: info.rcBar.bottom - window.top,
    })
}

/// Draws the background of the menu bar (`WM_UAHDRAWMENU`) or one of its
/// items (`WM_UAHDRAWMENUITEM`) dark. `false` leaves the message to the
/// default procedure, which draws it light.
pub fn draw_bar(hwnd: HWND, message: u32, lparam: LPARAM) -> bool {
    if lparam.0 == 0 {
        return false;
    }
    if message == WM_UAHDRAWMENU {
        // SAFETY: lParam of this message points to a UAHMENU for the
        // duration of the message.
        let menu = unsafe { &*(lparam.0 as *const UahMenu) };
        return bar_rect(hwnd).is_some_and(|bar| fill(menu.hdc, &bar, BAR));
    }
    // SAFETY: lParam of this message points to a UAHDRAWMENUITEM for the
    // duration of the message.
    let draw = unsafe { &*(lparam.0 as *const UahDrawMenuItem) };
    let Ok(position) = u32::try_from(draw.position) else {
        return false;
    };
    let mut label = [0_u16; 256];
    let mut info = MENUITEMINFOW {
        cbSize: size_of::<MENUITEMINFOW>() as u32,
        fMask: MIIM_STRING,
        dwTypeData: PWSTR(label.as_mut_ptr()),
        // Room is left for the terminator.
        cch: label.len() as u32 - 1,
        ..Default::default()
    };
    let state = draw.item.itemState.0;
    let lit = state & (ODS_HOTLIGHT.0 | ODS_SELECTED.0) != 0;
    let disabled = state & (ODS_GRAYED.0 | ODS_DISABLED.0) != 0;
    let mut format = DT_CENTER | DT_SINGLELINE | DT_VCENTER;
    if state & ODS_NOACCEL.0 != 0 {
        format |= DT_HIDEPREFIX;
    }
    let options = DTTOPTS {
        dwSize: size_of::<DTTOPTS>() as u32,
        dwFlags: DTT_TEXTCOLOR,
        crText: if disabled { DISABLED_TEXT } else { TEXT },
        ..Default::default()
    };
    let mut rect = draw.item.rcItem;
    // SAFETY: the menu and the device context are the message's; the
    // label buffer is as long as `cch` says plus the terminator; the
    // theme handle is closed before returning.
    unsafe {
        if GetMenuItemInfoW(draw.menu.hmenu, position, true, &mut info).is_err() {
            return false;
        }
        let length = (info.cch as usize).min(label.len() - 1);
        // The text is drawn the way the theme draws it (font, prefix),
        // only in another color.
        let theme = OpenThemeData(Some(hwnd), w!("Menu"));
        if theme.is_invalid() {
            return false;
        }
        let drawn = fill(draw.menu.hdc, &rect, if lit { LIT_ITEM } else { BAR })
            && DrawThemeTextEx(
                theme,
                draw.menu.hdc,
                MENU_BARITEM.0,
                MBI_NORMAL.0,
                &label[..length],
                format,
                &mut rect,
                Some(&options),
            )
            .is_ok();
        let _ = CloseThemeData(theme);
        drawn
    }
}

/// Paints over the light line the system leaves between the menu bar and
/// the client area. Called after the default procedure painted the frame.
pub fn paint_bar_edge(hwnd: HWND) {
    if bar_rect(hwnd).is_none() {
        return;
    }
    let (mut client, mut window) = (RECT::default(), RECT::default());
    // SAFETY: plain queries on a live window with live out structures; the
    // window device context is released before returning.
    unsafe {
        if GetClientRect(hwnd, &mut client).is_err() || GetWindowRect(hwnd, &mut window).is_err() {
            return;
        }
        // The client area in screen coordinates.
        let mut corners = [
            POINT {
                x: client.left,
                y: client.top,
            },
            POINT {
                x: client.right,
                y: client.bottom,
            },
        ];
        MapWindowPoints(Some(hwnd), None, &mut corners);
        // The row of pixels right above it, in window coordinates.
        let line = RECT {
            left: corners[0].x - window.left,
            top: corners[0].y - window.top - 1,
            right: corners[1].x - window.left,
            bottom: corners[0].y - window.top,
        };
        let dc = GetWindowDC(Some(hwnd));
        if dc.is_invalid() {
            return;
        }
        fill(dc, &line, BAR);
        ReleaseDC(Some(hwnd), dc);
    }
}

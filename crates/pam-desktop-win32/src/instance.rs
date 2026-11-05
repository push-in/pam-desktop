//! One process per application (`RuntimeConfig::single_instance`): a named
//! mutex tells a second launch that the application already runs, and the
//! second launch hands its link to the first through `WM_COPYDATA` sent to
//! a message-only window whose class is derived from the application id.

use std::rc::Rc;

use pam_desktop_ui::backend::{BackendEvent, MAIN_WINDOW};
use pam_desktop_ui::shell::WindowCommand;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, CreateWindowExW, DefWindowProcW, DestroyWindow, FindWindowExW, GetWindowThreadProcessId, RegisterClassExW,
    SendMessageTimeoutW, HWND_MESSAGE, SMTO_ABORTIFHUNG, WINDOW_EX_STYLE, WINDOW_STYLE, WM_COPYDATA, WNDCLASSEXW,
};

use crate::app::App;
use crate::util::wide;

/// Marks a `WM_COPYDATA` as ours ("PAM1").
const LINK_MAGIC: usize = 0x5041_4D31;
/// Longest link accepted from another launch, in bytes of UTF-8.
const MAX_LINK_BYTES: usize = 8192;
/// How long a second launch waits for the first to be ready.
const HANDOVER_ATTEMPTS: u32 = 50;

/// Keeps the application's mutex until the process ends.
pub struct InstanceLock(HANDLE);

impl Drop for InstanceLock {
    fn drop(&mut self) {
        // SAFETY: the handle was returned by CreateMutexW and is closed once.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

pub enum Launch {
    /// This process is the application.
    First(InstanceLock),
    /// The application already runs and was given the link.
    Handed,
}

/// Names must not contain a backslash after their namespace prefix and
/// are limited in length.
fn object_name(prefix: &str, application_id: &str) -> Vec<u16> {
    let id: String = application_id.chars().map(|character| if character == '\\' { '_' } else { character }).take(200).collect();
    wide(&format!("{prefix}{id}"))
}

fn class_name(application_id: &str) -> Vec<u16> {
    object_name("PamDesktopInstance.", application_id)
}

/// Claims the application for this process, or hands `launch_url` to the
/// process that already has it.
pub fn acquire(application_id: &str, launch_url: Option<&str>) -> Launch {
    let name = object_name("Local\\", application_id);
    // SAFETY: the name is NUL-terminated; the last error is read right
    // after the call that sets it.
    let (mutex, exists) = unsafe {
        let mutex = CreateMutexW(None, false, PCWSTR(name.as_ptr()));
        (mutex, GetLastError() == ERROR_ALREADY_EXISTS)
    };
    let Ok(mutex) = mutex else {
        // Without the mutex nothing can be known: run as the only instance.
        return Launch::First(InstanceLock(HANDLE::default()));
    };
    let lock = InstanceLock(mutex);
    if !exists {
        return Launch::First(lock);
    }
    let class = class_name(application_id);
    let link = launch_url.unwrap_or_default().as_bytes();
    let link = &link[..link.len().min(MAX_LINK_BYTES)];
    for _ in 0..HANDOVER_ATTEMPTS {
        // SAFETY: the class name is NUL-terminated; the copy structure and
        // the bytes it points to outlive the synchronous send.
        unsafe {
            if let Ok(target) = FindWindowExW(Some(HWND_MESSAGE), None, PCWSTR(class.as_ptr()), PCWSTR::null()) {
                if !target.is_invalid() {
                    // This launch has the right to take the foreground;
                    // the running application gets it.
                    let mut process = 0_u32;
                    GetWindowThreadProcessId(target, Some(&mut process));
                    let _ = AllowSetForegroundWindow(process);
                    let data = COPYDATASTRUCT { dwData: LINK_MAGIC, cbData: link.len() as u32, lpData: link.as_ptr().cast_mut().cast() };
                    SendMessageTimeoutW(target, WM_COPYDATA, WPARAM(0), LPARAM(std::ptr::from_ref(&data) as isize), SMTO_ABORTIFHUNG, 5000, None);
                    return Launch::Handed;
                }
            }
        }
        // The first launch holds the mutex but is still starting.
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Launch::Handed
}

/// Another launch arrived: the main window comes forward and the link,
/// when there is one, goes to the application.
fn activated(app: &Rc<App>, link: Option<String>) {
    crate::window::command(app, MAIN_WINDOW, &WindowCommand::Focus);
    if let Some(url) = link {
        app.emit(BackendEvent::OpenUrl(url));
    }
    app.pump();
}

extern "system" fn instance_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if message == WM_COPYDATA && lparam.0 != 0 {
        let handled = std::panic::catch_unwind(|| {
            let Some(app) = App::current() else { return false };
            // SAFETY: lParam of WM_COPYDATA points to a COPYDATASTRUCT whose
            // data is readable for `cbData` bytes during the message.
            let link = unsafe {
                let data = &*(lparam.0 as *const COPYDATASTRUCT);
                if data.dwData != LINK_MAGIC || data.cbData as usize > MAX_LINK_BYTES {
                    return false;
                }
                if data.cbData == 0 || data.lpData.is_null() {
                    None
                } else {
                    // Anything that is not text is dropped (the window still comes forward).
                    std::str::from_utf8(std::slice::from_raw_parts(data.lpData.cast::<u8>(), data.cbData as usize)).ok().map(str::to_owned)
                }
            };
            activated(&app, link.filter(|link| !link.is_empty()));
            true
        });
        if matches!(handled, Ok(true)) {
            return LRESULT(1);
        }
    }
    // SAFETY: forwarding the unmodified message to the default procedure.
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

/// Creates the window other launches look for. Returned so it can be
/// destroyed at shutdown.
pub fn listen(app: &Rc<App>) -> Option<HWND> {
    let class_name = class_name(&app.application_id);
    let class = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(instance_proc),
        hInstance: app.instance,
        lpszClassName: PCWSTR(class_name.as_ptr()),
        ..Default::default()
    };
    // SAFETY: standard registration and creation of a message-only window;
    // the class name outlives both calls.
    unsafe {
        RegisterClassExW(&class);
        CreateWindowExW(WINDOW_EX_STYLE(0), PCWSTR(class_name.as_ptr()), PCWSTR::null(), WINDOW_STYLE(0), 0, 0, 0, 0, Some(HWND_MESSAGE), None, Some(app.instance), None).ok()
    }
}

pub fn stop(window: Option<HWND>) {
    if let Some(window) = window {
        // SAFETY: destroying our own message-only window.
        unsafe {
            let _ = DestroyWindow(window);
        }
    }
}

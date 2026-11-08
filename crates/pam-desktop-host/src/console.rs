//! The console on Windows.
//!
//! A release host is a windowed program, so starting an application opens
//! no console. Its commands (`build`, `dev`…) still print: the host borrows
//! the console of the terminal it was started from.

use std::process::Command;

/// Keeps a console tool started by the host from opening a window.
pub fn hidden(command: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW
        command.creation_flags(0x0800_0000);
    }
    command
}

/// Connects the standard streams to the console of the parent process,
/// when there is one and the streams are not already redirected.
pub fn attach() {
    #[cfg(windows)]
    windows::attach();
}

#[cfg(windows)]
mod windows {
    use windows_sys::Win32::Foundation::{
        GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Console::{
        ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE, STD_HANDLE,
        STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle,
    };

    pub fn attach() {
        // SAFETY: plain calls on process-wide handles; every pointer passed
        // is a NUL-terminated UTF-16 string that outlives the call or null.
        unsafe {
            if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
                // No terminal behind the process, or it already has a console.
                return;
            }
            connect(STD_OUTPUT_HANDLE, "CONOUT$");
            connect(STD_ERROR_HANDLE, "CONOUT$");
            connect(STD_INPUT_HANDLE, "CONIN$");
        }
    }

    /// Points a standard stream at the console unless it is redirected.
    unsafe fn connect(stream: STD_HANDLE, device: &str) {
        // SAFETY: see `attach`.
        unsafe {
            let current: HANDLE = GetStdHandle(stream);
            if !current.is_null() && current != INVALID_HANDLE_VALUE {
                return;
            }
            let name: Vec<u16> = device.encode_utf16().chain(std::iter::once(0)).collect();
            let handle = CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            );
            if handle != INVALID_HANDLE_VALUE {
                SetStdHandle(stream, handle);
            }
        }
    }
}

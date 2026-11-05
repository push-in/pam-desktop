//! The share sheet of the system (`Backend::share`), through the WinRT
//! `DataTransferManager` of the window.
//!
//! The sheet asks for the content with a `DataRequested` event after it
//! was told to open. Everything it needs (title, text, storage items) is
//! prepared before, on the UI thread, and kept in a thread-local slot: the
//! handler must be `Send`, the WinRT objects it hands over are not, and the
//! event is raised on the thread of the window anyway.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use pam_desktop_ui::backend::WindowId;
use pam_desktop_ui::shell::ShareRequest;
use windows::ApplicationModel::DataTransfer::{DataRequestedEventArgs, DataTransferManager};
use windows::Foundation::TypedEventHandler;
use windows::Storage::{IStorageItem, StorageFile};
use windows::Win32::Foundation::{E_FAIL, HWND};
use windows::Win32::UI::Shell::IDataTransferManagerInterop;
use windows::Win32::UI::WindowsAndMessaging::GetWindowTextW;
use windows::core::{Error, HSTRING, Interface, Result};
use windows_collections::IIterable;
use windows_future::AsyncStatus;

use crate::app::App;
use crate::util::from_wide;

/// How long resolving the shared files may take before sharing gives up.
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(3);

/// The share in progress: what the sheet will be given, and the handler
/// registration to undo once it was served or replaced.
struct Share {
    manager: DataTransferManager,
    token: i64,
    title: HSTRING,
    text: HSTRING,
    files: Vec<IStorageItem>,
}

thread_local! {
    static ACTIVE: RefCell<Option<Share>> = const { RefCell::new(None) };
}

/// Unregisters the handler of the previous share, if any.
fn finish() {
    if let Some(share) = ACTIVE.with(|active| active.borrow_mut().take()) {
        let _ = share.manager.RemoveDataRequested(share.token);
    }
}

/// The storage items of the shared files. The lookups are asynchronous but
/// local and quick: they are waited for here (bounded), so the sheet only
/// opens with everything it will be asked for.
fn resolve(files: &[std::path::PathBuf]) -> Result<Vec<IStorageItem>> {
    let pending = files
        .iter()
        .map(|path| StorageFile::GetFileFromPathAsync(&HSTRING::from(path.as_os_str())))
        .collect::<Result<Vec<_>>>()?;
    let deadline = Instant::now() + RESOLVE_TIMEOUT;
    let mut items = Vec::with_capacity(pending.len());
    for operation in pending {
        while operation.Status()? == AsyncStatus::Started {
            if Instant::now() >= deadline {
                let _ = operation.Cancel();
                return Err(Error::from(E_FAIL));
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        items.push(operation.GetResults()?.cast::<IStorageItem>()?);
    }
    Ok(items)
}

/// Hands the prepared content to the sheet.
fn fill(arguments: &DataRequestedEventArgs) -> Result<()> {
    let request = arguments.Request()?;
    let filled = ACTIVE.with(|active| {
        let active = active.borrow();
        let share = active.as_ref().ok_or_else(|| Error::from(E_FAIL))?;
        let data = request.Data()?;
        // A package without a title is rejected by the sheet.
        data.Properties()?.SetTitle(&share.title)?;
        if !share.text.is_empty() {
            data.SetText(&share.text)?;
        }
        if !share.files.is_empty() {
            let items: IIterable<IStorageItem> = share
                .files
                .iter()
                .cloned()
                .map(Some)
                .collect::<Vec<_>>()
                .into();
            data.SetStorageItems(&items, true)?;
        }
        Ok(())
    });
    if filled.is_err() {
        let _ = request.FailWithDisplayText(&HSTRING::from("Nothing to share"));
    }
    filled
}

fn open(hwnd: HWND, request: &ShareRequest) -> Result<()> {
    finish();
    let interop: IDataTransferManagerInterop =
        windows::core::factory::<DataTransferManager, IDataTransferManagerInterop>()?;
    let files = resolve(&request.files)?;
    let title = if request.title.is_empty() {
        let mut buffer = [0_u16; 256];
        // SAFETY: the buffer is live for the call, which reports how much it wrote.
        let length = unsafe { GetWindowTextW(hwnd, &mut buffer) }.max(0) as usize;
        let window_title = from_wide(&buffer[..length.min(buffer.len())]);
        if window_title.is_empty() {
            String::from("Share")
        } else {
            window_title
        }
    } else {
        request.title.clone()
    };
    // SAFETY: `hwnd` is a live top-level window of this thread.
    let manager: DataTransferManager = unsafe { interop.GetForWindow(hwnd)? };
    let token = manager.DataRequested(&TypedEventHandler::new(
        |_, arguments: windows::core::Ref<DataRequestedEventArgs>| {
            let served = arguments.as_ref().map_or(Err(Error::from(E_FAIL)), fill);
            // The request was answered: the registration goes away from the
            // message loop (never from inside its own invocation).
            if let Some(app) = App::current() {
                app.defer(Box::new(|_| finish()));
            }
            served
        },
    ))?;
    ACTIVE.with(|active| {
        *active.borrow_mut() = Some(Share {
            manager,
            token,
            title: HSTRING::from(title),
            text: HSTRING::from(request.text.as_str()),
            files,
        });
    });
    // SAFETY: as above; no state borrow is held while the shell is called.
    if let Err(error) = unsafe { interop.ShowShareUIForWindow(hwnd) } {
        finish();
        return Err(error);
    }
    Ok(())
}

/// `Backend::share`: asks the system to open its share sheet over
/// `window`. Answers whether it was asked; any failure is "not available".
pub fn share(app: &Rc<App>, window: WindowId, request: &ShareRequest) -> bool {
    if request.text.is_empty() && request.files.is_empty() {
        return false;
    }
    let Some(hwnd) = app
        .with_state(|state| state.scene.windows.get(&window).map(|entry| entry.hwnd))
        .flatten()
    else {
        return false;
    };
    open(hwnd, request).is_ok()
}

/// Drops the registration of a share still pending at shutdown.
pub fn shutdown() {
    finish();
}

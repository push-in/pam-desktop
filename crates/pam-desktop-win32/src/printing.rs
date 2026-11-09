//! System print dialog followed by bounded GDI spooling off the UI thread.
use pam_desktop_ui::printing::{Document, PrintCompletion};
use std::sync::atomic::{AtomicBool, Ordering};
use windows::Win32::Foundation::{GlobalFree, HWND, RECT};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Storage::Xps::{AbortDoc, DOCINFOW, EndDoc, EndPage, StartDocW, StartPage};
use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
use windows::Win32::UI::Controls::Dialogs::*;
use windows::core::{PCWSTR, w};

static ACTIVE: AtomicBool = AtomicBool::new(false);

pub fn print(parent: HWND, document: Document, complete: PrintCompletion) {
    if ACTIVE.swap(true, Ordering::AcqRel) {
        complete(Err("A print job is already active".into()));
        return;
    }
    // SAFETY: the dialog and its native allocations belong to this UI thread.
    let result = unsafe { choose_printer(parent, &document) };
    match result {
        Ok(Some((dc, from, to))) => {
            let raw = dc.0 as usize;
            // A printer DC may be used on another thread when exclusively owned.
            // Driver calls can block: no app UI or scene borrow is held here.
            std::thread::spawn(move || {
                let dc = HDC(raw as *mut _);
                let result = unsafe { spool(dc, &document, from, to) };
                unsafe {
                    let _ = DeleteDC(dc);
                }
                ACTIVE.store(false, Ordering::Release);
                complete(result.map(|()| true));
            });
        }
        Ok(None) => {
            ACTIVE.store(false, Ordering::Release);
            complete(Ok(false));
        }
        Err(error) => {
            ACTIVE.store(false, Ordering::Release);
            complete(Err(error));
        }
    }
}

unsafe fn choose_printer(
    parent: HWND,
    document: &Document,
) -> Result<Option<(HDC, usize, usize)>, String> {
    let mut dialog = PRINTDLGW {
        lStructSize: size_of::<PRINTDLGW>() as u32,
        hwndOwner: parent,
        Flags: PD_RETURNDEFAULT,
        ..Default::default()
    };
    // Fetch the driver's complete DEVMODE, including private fields, before
    // requesting the document's paper size. The user can still choose a printer.
    unsafe {
        let _ = PrintDlgW(&mut dialog);
        if !dialog.hDevMode.is_invalid() {
            let mode = GlobalLock(dialog.hDevMode).cast::<DEVMODEW>();
            if !mode.is_null() {
                (*mode).dmFields |= DM_PAPERSIZE | DM_PAPERWIDTH | DM_PAPERLENGTH;
                (*mode).Anonymous1.Anonymous1.dmPaperSize = DMPAPER_USER as i16;
                (*mode).Anonymous1.Anonymous1.dmPaperWidth =
                    (document.width * 254.0 / 72.0).round() as i16;
                (*mode).Anonymous1.Anonymous1.dmPaperLength =
                    (document.height * 254.0 / 72.0).round() as i16;
                let _ = GlobalUnlock(dialog.hDevMode);
            }
        }
        dialog.Flags = PD_RETURNDC | PD_NOSELECTION | PD_USEDEVMODECOPIESANDCOLLATE;
        dialog.nMinPage = 1;
        dialog.nMaxPage = document.pages.len() as u16;
        dialog.nFromPage = 1;
        dialog.nToPage = dialog.nMaxPage;
        dialog.nCopies = 1;
        let accepted = PrintDlgW(&mut dialog).as_bool();
        let error = if accepted {
            0
        } else {
            CommDlgExtendedError().0
        };
        if !dialog.hDevMode.is_invalid() {
            let _ = GlobalFree(Some(dialog.hDevMode));
        }
        if !dialog.hDevNames.is_invalid() {
            let _ = GlobalFree(Some(dialog.hDevNames));
        }
        if !accepted {
            if !dialog.hDC.is_invalid() {
                let _ = DeleteDC(dialog.hDC);
            }
            return if error == 0 {
                Ok(None)
            } else if error == PDERR_NODEFAULTPRN.0 || error == PDERR_NODEVICES.0 {
                // A machine without any printer (Windows has already told the
                // user): say so instead of a bare CommDlgExtendedError code.
                Err(format!("No printer is installed ({error})"))
            } else {
                Err(format!("Native printer dialog failed ({error})"))
            };
        }
        if dialog.hDC.is_invalid() {
            return Err("The printer returned no device context".into());
        }
        let (from, to) = if dialog.Flags.contains(PD_PAGENUMS) {
            (
                usize::from(dialog.nFromPage.saturating_sub(1)),
                usize::from(dialog.nToPage),
            )
        } else {
            (0, document.pages.len())
        };
        Ok(Some((
            dialog.hDC,
            from.min(document.pages.len()),
            to.min(document.pages.len()),
        )))
    }
}

unsafe fn spool(dc: HDC, document: &Document, from: usize, to: usize) -> Result<(), String> {
    let title: Vec<u16> = document.title.encode_utf16().chain(Some(0)).collect();
    let info = DOCINFOW {
        cbSize: size_of::<DOCINFOW>() as i32,
        lpszDocName: PCWSTR(title.as_ptr()),
        ..Default::default()
    };
    // SAFETY: this thread exclusively owns dc and every GDI object below.
    unsafe {
        if StartDocW(dc, &info) <= 0 {
            return Err("The printer could not start the document".into());
        }
        let sx = f64::from(GetDeviceCaps(Some(dc), LOGPIXELSX)) / 72.0;
        let sy = f64::from(GetDeviceCaps(Some(dc), LOGPIXELSY)) / 72.0;
        let ox = GetDeviceCaps(Some(dc), PHYSICALOFFSETX);
        let oy = GetDeviceCaps(Some(dc), PHYSICALOFFSETY);
        let mut failed = false;
        for page in &document.pages[from..to] {
            if StartPage(dc) <= 0 {
                failed = true;
                break;
            }
            SetBkMode(dc, TRANSPARENT);
            for block in page {
                let font = CreateFontW(
                    -(block.size * sy).round() as i32,
                    0,
                    0,
                    0,
                    if block.bold { 700 } else { 400 },
                    0,
                    0,
                    0,
                    DEFAULT_CHARSET,
                    OUT_DEFAULT_PRECIS,
                    CLIP_DEFAULT_PRECIS,
                    DEFAULT_QUALITY,
                    FIXED_PITCH.0 as u32,
                    w!("Consolas"),
                );
                if font.is_invalid() {
                    failed = true;
                    break;
                }
                let previous = SelectObject(dc, HGDIOBJ(font.0));
                let mut rect = RECT {
                    left: (block.x * sx).round() as i32 - ox,
                    top: (block.y * sy).round() as i32 - oy,
                    right: ((block.x + block.width) * sx).round() as i32 - ox,
                    bottom: ((block.y + block.height) * sy).round() as i32 - oy,
                };
                let mut text: Vec<u16> = block.text.encode_utf16().collect();
                let alignment = match block.align {
                    2 => DT_CENTER,
                    3 => DT_RIGHT,
                    _ => DT_LEFT,
                };
                let drawn = DrawTextW(
                    dc,
                    &mut text,
                    &mut rect,
                    alignment | DT_WORDBREAK | DT_NOPREFIX,
                );
                SelectObject(dc, previous);
                let _ = DeleteObject(HGDIOBJ(font.0));
                if drawn == 0 && !block.text.is_empty() {
                    failed = true;
                    break;
                }
            }
            if failed || EndPage(dc) <= 0 {
                failed = true;
                break;
            }
        }
        if failed {
            AbortDoc(dc);
            return Err("The printer could not render a page".into());
        }
        if EndDoc(dc) <= 0 {
            AbortDoc(dc);
            return Err("The printer did not accept the completed job".into());
        }
    }
    Ok(())
}

//! OLE drag and drop: files and text dropped on a window, and text dragged
//! out of the application.

use std::cell::Cell;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::rc::Rc;

use pam_desktop_ui::backend::{BackendEvent, NodeId, WindowId};
use pam_desktop_ui::geometry::Point;
use windows::core::{implement, Ref, Result, BOOL, HRESULT};
use windows::Win32::Foundation::{DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, HGLOBAL, HWND, POINT, POINTL, SIZE, S_OK};
use windows::Win32::Graphics::Gdi::{CreateDIBSection, DeleteObject, ScreenToClient, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, HGDIOBJ};
use windows::Win32::System::Com::{CoCreateInstance, IDataObject, CLSCTX_INPROC_SERVER, DVASPECT_CONTENT, FORMATETC, STGMEDIUM, STGMEDIUM_0, TYMED_HGLOBAL};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::{
    DoDragDrop, IDropSource, IDropSource_Impl, IDropTarget, IDropTarget_Impl, RegisterDragDrop, ReleaseStgMedium, RevokeDragDrop, CF_HDROP, CF_UNICODETEXT,
    DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_NONE,
};
use windows::Win32::System::SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS};
use windows::Win32::UI::Shell::{CLSID_DragDropHelper, DragQueryFileW, IDragSourceHelper, SHCreateDataObject, HDROP, SHDRAGIMAGE};

use crate::app::App;
use crate::util::{from_wide, wide};

fn format(clipboard_format: u16) -> FORMATETC {
    FORMATETC { cfFormat: clipboard_format, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 }
}

fn offers(data: &IDataObject, clipboard_format: u16) -> bool {
    // SAFETY: the format structure is a live local.
    unsafe { data.QueryGetData(&format(clipboard_format)) == S_OK }
}

#[implement(IDropTarget)]
struct DropTarget {
    hwnd: HWND,
    window: WindowId,
    /// The drag in progress carries files or text.
    accepts: Cell<bool>,
}

impl DropTarget {
    fn effect(&self, allowed: *mut DROPEFFECT) {
        if allowed.is_null() {
            return;
        }
        // SAFETY: OLE passes a valid in/out effect pointer (checked for null above).
        unsafe {
            *allowed = if self.accepts.get() && (*allowed).contains(DROPEFFECT_COPY) { DROPEFFECT_COPY } else { DROPEFFECT_NONE };
        }
    }

    fn position(&self, point: &POINTL) -> Option<Point> {
        let app = App::current()?;
        let mut client = POINT { x: point.x, y: point.y };
        // SAFETY: plain coordinate conversion on our window.
        unsafe {
            let _ = ScreenToClient(self.hwnd, &mut client);
        }
        let scale = app.with_state(|state| state.scene.windows.get(&self.window).map(|entry| entry.scale())).flatten()?;
        Some(Point::new(client.x as f32 / scale, client.y as f32 / scale))
    }
}

fn dropped_files(data: &IDataObject) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    // SAFETY: the medium returned by GetData is released after its handle
    // was read; buffers are sized from the lengths the shell reports.
    unsafe {
        let Ok(mut medium) = data.GetData(&format(CF_HDROP.0)) else { return paths };
        let drop = HDROP(medium.u.hGlobal.0);
        let count = DragQueryFileW(drop, u32::MAX, None);
        for index in 0..count {
            let length = DragQueryFileW(drop, index, None) as usize;
            let mut buffer = vec![0_u16; length + 1];
            let copied = DragQueryFileW(drop, index, Some(&mut buffer)) as usize;
            if copied > 0 {
                // File names are not always valid UTF-16: keep them as they are.
                paths.push(PathBuf::from(std::ffi::OsString::from_wide(&buffer[..copied.min(length)])));
            }
        }
        ReleaseStgMedium(&mut medium);
    }
    paths
}

fn dropped_text(data: &IDataObject) -> Option<String> {
    // SAFETY: the block is only read while locked, within its reported
    // size, and the medium is released afterwards.
    unsafe {
        let mut medium = data.GetData(&format(CF_UNICODETEXT.0)).ok()?;
        let block = medium.u.hGlobal;
        let source = GlobalLock(block).cast::<u16>();
        let text = (!source.is_null()).then(|| {
            let text = from_wide(std::slice::from_raw_parts(source, GlobalSize(block) / 2));
            let _ = GlobalUnlock(block);
            text
        });
        ReleaseStgMedium(&mut medium);
        text
    }
}

impl IDropTarget_Impl for DropTarget_Impl {
    fn DragEnter(&self, data: Ref<IDataObject>, _keys: MODIFIERKEYS_FLAGS, _point: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let accepts = data.as_ref().is_some_and(|data| offers(data, CF_HDROP.0) || offers(data, CF_UNICODETEXT.0));
        self.accepts.set(accepts);
        self.effect(effect);
        Ok(())
    }

    fn DragOver(&self, _keys: MODIFIERKEYS_FLAGS, _point: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        self.effect(effect);
        Ok(())
    }

    fn DragLeave(&self) -> Result<()> {
        self.accepts.set(false);
        Ok(())
    }

    fn Drop(&self, data: Ref<IDataObject>, _keys: MODIFIERKEYS_FLAGS, point: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        self.effect(effect);
        self.accepts.set(false);
        let (Some(app), Some(data), Some(position)) = (App::current(), data.as_ref(), self.position(point)) else { return Ok(()) };
        let window = self.window;
        let paths = if offers(data, CF_HDROP.0) { dropped_files(data) } else { Vec::new() };
        if !paths.is_empty() {
            app.emit(BackendEvent::FilesDropped { window, position, paths });
        } else if let Some(text) = dropped_text(data) {
            app.emit(BackendEvent::TextDropped { window, position, text });
        }
        // OLE calls from its own message handling: drain once we are done.
        app.pump();
        Ok(())
    }
}

/// Makes a window accept dropped files and text.
pub fn register(_app: &Rc<App>, hwnd: HWND, window: WindowId) {
    let target: IDropTarget = DropTarget { hwnd, window, accepts: Cell::new(false) }.into();
    // SAFETY: `hwnd` is a live window of this (OLE initialized) thread.
    unsafe {
        let _ = RegisterDragDrop(hwnd, &target);
    }
}

pub fn revoke(hwnd: HWND) {
    // SAFETY: revoking a registration of this thread; failing is harmless.
    unsafe {
        let _ = RevokeDragDrop(hwnd);
    }
}

#[implement(IDropSource)]
struct DropSource;

impl IDropSource_Impl for DropSource_Impl {
    fn QueryContinueDrag(&self, escape_pressed: BOOL, keys: MODIFIERKEYS_FLAGS) -> HRESULT {
        if escape_pressed.as_bool() {
            DRAGDROP_S_CANCEL
        } else if !keys.contains(MK_LBUTTON) {
            // The button that started the drag was released: drop.
            DRAGDROP_S_DROP
        } else {
            S_OK
        }
    }

    fn GiveFeedback(&self, _effect: DROPEFFECT) -> HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

/// A shell data object carrying `text` as Unicode text.
fn text_data_object(text: &str) -> Option<IDataObject> {
    let units = wide(text);
    // SAFETY: the global block is sized for the text and unlocked before
    // the data object takes ownership of it (release flag set).
    unsafe {
        let data: IDataObject = SHCreateDataObject(None, None, None).ok()?;
        let block: HGLOBAL = GlobalAlloc(GMEM_MOVEABLE, units.len() * 2).ok()?;
        let target = GlobalLock(block).cast::<u16>();
        if target.is_null() {
            let _ = windows::Win32::Foundation::GlobalFree(Some(block));
            return None;
        }
        std::ptr::copy_nonoverlapping(units.as_ptr(), target, units.len());
        let _ = GlobalUnlock(block);
        let medium = STGMEDIUM { tymed: TYMED_HGLOBAL.0 as u32, u: STGMEDIUM_0 { hGlobal: block }, pUnkForRelease: std::mem::ManuallyDrop::new(None) };
        if data.SetData(&format(CF_UNICODETEXT.0), &medium, true).is_err() {
            let _ = windows::Win32::Foundation::GlobalFree(Some(block));
            return None;
        }
        Some(data)
    }
}

/// Gives the drag a picture of the source view. Without it the system
/// shows its plain drag cursor.
fn attach_drag_image(app: &Rc<App>, data: &IDataObject, source: NodeId) {
    let snapshot = app
        .with_state(|state| {
            let (pixels, width, height) = crate::render::snapshot_view(state, source)?;
            // The image hangs from the point of the view that was pressed.
            let (window, transform) = state.scene.world_transform(source)?;
            let entry = state.scene.windows.get(&window)?;
            let pressed = transform.invert().map(|inverse| inverse.apply(entry.pointer));
            let scale = entry.scale();
            let grab = pressed.map_or((width as i32 / 2, height as i32 / 2), |point| {
                (((point.x * scale).round() as i32).clamp(0, width as i32), ((point.y * scale).round() as i32).clamp(0, height as i32))
            });
            Some((pixels, width, height, grab))
        })
        .flatten();
    let Some((pixels, width, height, grab)) = snapshot else { return };
    // SAFETY: the DIB section is created with the pixel count copied into
    // it; on success the drag helper owns the bitmap, otherwise it is deleted.
    unsafe {
        let Ok(helper) = CoCreateInstance::<_, IDragSourceHelper>(&CLSID_DragDropHelper, None, CLSCTX_INPROC_SERVER) else { return };
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                // Negative: rows run top to bottom, like the snapshot.
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 32,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let Ok(bitmap) = CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0) else { return };
        if bits.is_null() {
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
            return;
        }
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits.cast::<u8>(), pixels.len());
        let image = SHDRAGIMAGE {
            sizeDragImage: SIZE { cx: width as i32, cy: height as i32 },
            ptOffset: POINT { x: grab.0, y: grab.1 },
            hbmpDragImage: bitmap,
            crColorKey: windows::Win32::Foundation::COLORREF(0xFFFF_FFFF),
        };
        if helper.InitializeFromBitmap(&image, data).is_err() {
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
        }
    }
}

/// `Backend::begin_text_drag`: the modal drag loop runs from the message
/// loop, with the button that started it still down.
pub fn begin_text_drag(app: &Rc<App>, _window: WindowId, source: NodeId, text: &str) {
    let text = text.to_owned();
    app.defer(Box::new(move |app| {
        let dropped = text_data_object(&text).is_some_and(|data| {
            attach_drag_image(app, &data, source);
            let drop_source: IDropSource = DropSource.into();
            let mut effect = DROPEFFECT_NONE;
            // Only copying is offered: the text shown stays where it is,
            // and a target that moves expects the source to delete.
            // SAFETY: both objects are live for the modal call; no state
            // borrow is held while OLE spins its drag loop.
            let result = unsafe { DoDragDrop(&data, &drop_source, DROPEFFECT_COPY, &mut effect) };
            result == DRAGDROP_S_DROP && effect != DROPEFFECT_NONE
        });
        app.emit(BackendEvent::DragFinished { source, dropped });
        app.pump();
    }));
}

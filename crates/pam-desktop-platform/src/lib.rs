//! Small, audited operating-system calls used by the safe desktop shell.

#![cfg_attr(not(any(target_os = "windows", target_os = "macos")), allow(dead_code))]

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ProgressState {
    Hidden = 1,
    Indeterminate = 2,
    Normal = 3,
    Paused = 4,
    Error = 5,
}

impl TryFrom<u8> for ProgressState {
    type Error = &'static str;

    fn try_from(value: u8) -> Result<Self, &'static str> {
        match value {
            1 => Ok(Self::Hidden),
            2 => Ok(Self::Indeterminate),
            3 => Ok(Self::Normal),
            4 => Ok(Self::Paused),
            5 => Ok(Self::Error),
            _ => Err("invalid taskbar progress state"),
        }
    }
}

#[cfg(target_os = "windows")]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    reason = "badge raster coordinates and validated unit progress are bounded to tiny fixed ranges"
)]
mod windows_status {
    use std::ffi::c_void;

    use windows::Win32::Foundation::{HWND, RPC_E_CHANGED_MODE};
    use windows::Win32::Graphics::Gdi::{CreateBitmap, DeleteObject};
    use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    };
    use windows::Win32::UI::Shell::{
        Common::{IObjectArray, IObjectCollection},
        DestinationList, EnumerableObjectCollection, ICustomDestinationList, IShellLinkW,
        ITaskbarList3,
        PropertiesSystem::IPropertyStore,
        ShellLink, TBPF_ERROR, TBPF_INDETERMINATE, TBPF_NOPROGRESS, TBPF_NORMAL, TBPF_PAUSED,
        TaskbarList,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateIconIndirect, DestroyIcon, HICON, ICONINFO,
    };
    use windows::core::{HSTRING, Interface as _, PCWSTR};

    use super::ProgressState;

    const ICON_SIZE: usize = 32;
    const PKEY_TITLE: windows::Win32::Foundation::PROPERTYKEY =
        windows::Win32::Foundation::PROPERTYKEY {
            fmtid: windows::core::GUID::from_u128(0xf29f85e0_4ff9_1068_ab91_08002b27b3d9),
            pid: 2,
        };

    thread_local! {
        static COM_READY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    pub fn set_badge(native_window: isize, count: Option<u32>) -> Result<(), String> {
        if native_window == 0 {
            return Err("invalid Windows taskbar badge window".to_owned());
        }
        let taskbar = taskbar()?;
        let hwnd = HWND(native_window as *mut _);
        let Some(count) = count else {
            // SAFETY: the HWND is owned by the live Winit window; a null icon
            // is the documented way to clear an overlay.
            return unsafe {
                taskbar
                    .SetOverlayIcon(hwnd, HICON::default(), PCWSTR::null())
                    .map_err(|error| format!("cannot clear Windows taskbar badge: {error}"))
            };
        };
        let pixels = badge_pixels(count);
        // SAFETY: both bitmaps copy from valid buffers, ICONINFO owns neither
        // bitmap, CreateIconIndirect copies their image data, and every GDI
        // handle is released exactly once after SetOverlayIcon returns.
        unsafe {
            let color = CreateBitmap(
                ICON_SIZE as i32,
                ICON_SIZE as i32,
                1,
                32,
                Some(pixels.as_ptr().cast::<c_void>()),
            );
            let mask = CreateBitmap(ICON_SIZE as i32, ICON_SIZE as i32, 1, 1, None);
            if color.is_invalid() || mask.is_invalid() {
                if !color.is_invalid() {
                    let _ = DeleteObject(color.into());
                }
                if !mask.is_invalid() {
                    let _ = DeleteObject(mask.into());
                }
                return Err("cannot allocate Windows taskbar badge bitmaps".to_owned());
            }
            let icon = CreateIconIndirect(&ICONINFO {
                fIcon: true.into(),
                hbmMask: mask,
                hbmColor: color,
                ..Default::default()
            });
            let _ = DeleteObject(color.into());
            let _ = DeleteObject(mask.into());
            let icon =
                icon.map_err(|error| format!("cannot create Windows taskbar badge: {error}"))?;
            let description = windows::core::HSTRING::from(format!("{count} notifications"));
            let result = taskbar
                .SetOverlayIcon(hwnd, icon, &description)
                .map_err(|error| format!("cannot set Windows taskbar badge: {error}"));
            let _ = DestroyIcon(icon);
            result
        }
    }

    #[allow(
        clippy::cast_possible_truncation,
        reason = "validated unit progress is deliberately quantized to a 10,000-step taskbar scale"
    )]
    pub fn set_progress(
        native_window: isize,
        progress: f64,
        state: ProgressState,
    ) -> Result<(), String> {
        if native_window == 0 || !progress.is_finite() || !(0.0..=1.0).contains(&progress) {
            return Err("invalid Windows taskbar progress request".to_owned());
        }
        let flag = match state {
            ProgressState::Hidden => TBPF_NOPROGRESS,
            ProgressState::Indeterminate => TBPF_INDETERMINATE,
            ProgressState::Normal => TBPF_NORMAL,
            ProgressState::Paused => TBPF_PAUSED,
            ProgressState::Error => TBPF_ERROR,
        };
        let taskbar = taskbar()?;
        // SAFETY: the HWND originates from Winit for the live target window,
        // and both methods copy scalar values during the call.
        unsafe {
            taskbar
                .SetProgressState(HWND(native_window as *mut _), flag)
                .map_err(|error| format!("cannot set Windows taskbar state: {error}"))?;
            if !matches!(state, ProgressState::Hidden | ProgressState::Indeterminate) {
                let completed = (progress * 10_000.0).round() as u64;
                taskbar
                    .SetProgressValue(HWND(native_window as *mut _), completed, 10_000)
                    .map_err(|error| format!("cannot set Windows taskbar progress: {error}"))?;
            }
        }
        Ok(())
    }

    pub fn publish_quick_actions(
        application_id: &str,
        launcher: &std::path::Path,
        actions: &[super::QuickAction<'_>],
    ) -> Result<(), String> {
        initialize_com()?;
        let launcher = launcher
            .to_str()
            .ok_or_else(|| "Windows quick-action launcher path is not UTF-8".to_owned())?;
        // SAFETY: every COM object is represented by an owning smart pointer;
        // strings are copied during each call, and CommitList atomically
        // publishes the completed task collection for this application ID.
        unsafe {
            let destinations: ICustomDestinationList =
                CoCreateInstance(&DestinationList, None, CLSCTX_INPROC_SERVER)
                    .map_err(|error| format!("cannot create Windows Jump List: {error}"))?;
            destinations
                .SetAppID(&HSTRING::from(application_id))
                .map_err(|error| format!("cannot bind Windows Jump List application: {error}"))?;
            let mut minimum_slots = 0;
            let _: IObjectArray = destinations
                .BeginList(&raw mut minimum_slots)
                .map_err(|error| format!("cannot begin Windows Jump List: {error}"))?;
            let collection: IObjectCollection =
                CoCreateInstance(&EnumerableObjectCollection, None, CLSCTX_INPROC_SERVER)
                    .map_err(|error| format!("cannot create Windows Jump List tasks: {error}"))?;
            for action in actions.iter().take(minimum_slots.max(1) as usize) {
                let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
                    .map_err(|error| format!("cannot create Windows quick action: {error}"))?;
                link.SetPath(&HSTRING::from(launcher)).map_err(|error| {
                    format!("cannot set Windows quick-action launcher: {error}")
                })?;
                link.SetArguments(&HSTRING::from(format!("--pam-quick-action={}", action.id)))
                    .map_err(|error| {
                        format!("cannot set Windows quick-action argument: {error}")
                    })?;
                if !action.description.is_empty() {
                    link.SetDescription(&HSTRING::from(action.description))
                        .map_err(|error| {
                            format!("cannot set Windows quick-action description: {error}")
                        })?;
                }
                let properties: IPropertyStore = link
                    .cast()
                    .map_err(|error| format!("cannot edit Windows quick-action title: {error}"))?;
                let title = PROPVARIANT::from(action.label);
                properties
                    .SetValue(&PKEY_TITLE, &raw const title)
                    .and_then(|()| properties.Commit())
                    .map_err(|error| {
                        format!("cannot commit Windows quick-action title: {error}")
                    })?;
                collection
                    .AddObject(&link)
                    .map_err(|error| format!("cannot append Windows quick action: {error}"))?;
            }
            let tasks: IObjectArray = collection
                .cast()
                .map_err(|error| format!("cannot finalize Windows quick actions: {error}"))?;
            destinations
                .AddUserTasks(&tasks)
                .and_then(|()| destinations.CommitList())
                .map_err(|error| format!("cannot publish Windows Jump List: {error}"))
        }
    }

    fn taskbar() -> Result<ITaskbarList3, String> {
        initialize_com()?;
        // SAFETY: CoCreateInstance returns an owned COM smart pointer; the
        // current application UI thread was initialized immediately above.
        unsafe {
            let taskbar: ITaskbarList3 = CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER)
                .map_err(|error| format!("cannot create Windows taskbar interface: {error}"))?;
            taskbar
                .HrInit()
                .map_err(|error| format!("cannot initialize Windows taskbar interface: {error}"))?;
            Ok(taskbar)
        }
    }

    fn initialize_com() -> Result<(), String> {
        COM_READY.with(|ready| {
            if ready.get() {
                return Ok(());
            }
            // SAFETY: the reserved pointer is null as required and this runs
            // on the persistent Winit UI thread. PAM intentionally retains
            // the apartment until process exit so taskbar interfaces remain valid.
            let result = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
            if result.is_err() && result != RPC_E_CHANGED_MODE {
                return Err(format!(
                    "cannot initialize Windows COM apartment: {result:?}"
                ));
            }
            ready.set(true);
            Ok(())
        })
    }

    fn badge_pixels(count: u32) -> [u32; ICON_SIZE * ICON_SIZE] {
        let mut pixels = [0_u32; ICON_SIZE * ICON_SIZE];
        for y in 0..ICON_SIZE {
            for x in 0..ICON_SIZE {
                let dx = x as isize - 16;
                let dy = y as isize - 16;
                if dx * dx + dy * dy <= 15 * 15 {
                    pixels[y * ICON_SIZE + x] = 0xffff_2d55;
                }
            }
        }
        let label = if count > 99 {
            "99+".to_owned()
        } else {
            count.to_string()
        };
        draw_label(&mut pixels, &label);
        pixels
    }

    fn draw_label(pixels: &mut [u32; ICON_SIZE * ICON_SIZE], label: &str) {
        const GLYPHS: [[u8; 5]; 11] = [
            [0b111, 0b101, 0b101, 0b101, 0b111],
            [0b010, 0b110, 0b010, 0b010, 0b111],
            [0b111, 0b001, 0b111, 0b100, 0b111],
            [0b111, 0b001, 0b111, 0b001, 0b111],
            [0b101, 0b101, 0b111, 0b001, 0b001],
            [0b111, 0b100, 0b111, 0b001, 0b111],
            [0b111, 0b100, 0b111, 0b101, 0b111],
            [0b111, 0b001, 0b010, 0b010, 0b010],
            [0b111, 0b101, 0b111, 0b101, 0b111],
            [0b111, 0b101, 0b111, 0b001, 0b111],
            [0b000, 0b010, 0b111, 0b010, 0b000],
        ];
        let scale = 2;
        let width = label.chars().count() * 7 - 1;
        let origin_x = (ICON_SIZE - width) / 2;
        let origin_y = 11;
        for (index, character) in label.chars().enumerate() {
            let glyph = character
                .to_digit(10)
                .map_or(GLYPHS[10], |digit| GLYPHS[digit as usize]);
            for (row, bits) in glyph.into_iter().enumerate() {
                for column in 0..3 {
                    if bits & (1 << (2 - column)) == 0 {
                        continue;
                    }
                    for offset_y in 0..scale {
                        for offset_x in 0..scale {
                            let x = origin_x + index * 7 + column * scale + offset_x;
                            let y = origin_y + row * scale + offset_y;
                            pixels[y * ICON_SIZE + x] = 0xffff_ffff;
                        }
                    }
                }
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn numeric_badges_have_transparent_edges_and_visible_content() {
            let pixels = badge_pixels(128);
            assert_eq!(pixels[0], 0);
            assert!(pixels.iter().any(|pixel| *pixel == 0xffff_2d55));
            assert!(pixels.iter().any(|pixel| *pixel == 0xffff_ffff));
        }

        #[test]
        fn rejects_invalid_windows_before_calling_com() {
            assert!(set_badge(0, Some(1)).is_err());
            assert!(set_progress(0, 0.5, ProgressState::Normal).is_err());
        }
    }
}

pub struct QuickAction<'a> {
    pub id: &'a str,
    pub label: &'a str,
    pub description: &'a str,
}

/// Publishes operating-system launcher quick actions when the host is running
/// from a packaged application launcher.
///
/// # Errors
///
/// Returns an error when the platform shell refuses the atomic publication.
pub fn publish_quick_actions(
    application_id: &str,
    launcher: &std::path::Path,
    actions: &[QuickAction<'_>],
) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        windows_status::publish_quick_actions(application_id, launcher, actions)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (application_id, launcher, actions);
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod macos_status {
    use objc2::{MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{
        NSApplication, NSImageView, NSProgressIndicator, NSProgressIndicatorStyle, NSView,
    };
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

    use super::ProgressState;

    pub fn set_badge(count: Option<u32>) -> Result<(), String> {
        let mtm = MainThreadMarker::new()
            .ok_or_else(|| "macOS Dock badges must be changed on the main thread".to_owned())?;
        let application = NSApplication::sharedApplication(mtm);
        let label = count.map(|count| NSString::from_str(&count.to_string()));
        application.dockTile().setBadgeLabel(label.as_deref());
        Ok(())
    }

    pub fn set_progress(progress: f64, state: ProgressState) -> Result<(), String> {
        if !progress.is_finite() || !(0.0..=1.0).contains(&progress) {
            return Err("invalid macOS Dock progress request".to_owned());
        }
        let mtm = MainThreadMarker::new()
            .ok_or_else(|| "macOS Dock progress must be changed on the main thread".to_owned())?;
        let application = NSApplication::sharedApplication(mtm);
        let tile = application.dockTile();
        if state == ProgressState::Hidden {
            tile.setContentView(None);
            tile.display();
            return Ok(());
        }
        let size = tile.size();
        let root = NSView::initWithFrame(
            NSView::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), size),
        );
        if let Some(icon) = application.applicationIconImage() {
            let image = NSImageView::initWithFrame(
                NSImageView::alloc(mtm),
                NSRect::new(NSPoint::new(0.0, 0.0), size),
            );
            image.setImage(Some(&icon));
            root.addSubview(&image);
        }
        let indicator = NSProgressIndicator::initWithFrame(
            NSProgressIndicator::alloc(mtm),
            NSRect::new(
                NSPoint::new(size.width * 0.1, size.height * 0.08),
                NSSize::new(size.width * 0.8, 10.0),
            ),
        );
        indicator.setStyle(NSProgressIndicatorStyle::Bar);
        indicator.setMinValue(0.0);
        indicator.setMaxValue(1.0);
        indicator.setDoubleValue(progress);
        let indeterminate = state == ProgressState::Indeterminate;
        indicator.setIndeterminate(indeterminate);
        if indeterminate {
            // SAFETY: AppKit requires animation control on the main thread;
            // MainThreadMarker above proves that invariant and nil sender is supported.
            unsafe { indicator.startAnimation(None) };
        }
        root.addSubview(&indicator);
        tile.setContentView(Some(&root));
        tile.display();
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn rejects_invalid_progress_before_touching_appkit() {
            assert!(set_progress(f64::NAN, ProgressState::Normal).is_err());
            assert!(set_progress(1.1, ProgressState::Normal).is_err());
        }
    }
}

/// Sets or clears the operating-system application badge.
///
/// # Errors
///
/// Returns an error when the call is made off the required UI thread or the
/// current platform has no implemented badge backend.
pub fn set_badge(native_window: isize, count: Option<u32>) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        windows_status::set_badge(native_window, count)
    }
    #[cfg(target_os = "macos")]
    {
        let _ = native_window;
        macos_status::set_badge(count)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = (native_window, count);
        Err("application badges are unavailable on this platform".to_owned())
    }
}

/// Updates the native taskbar progress indicator for a live window.
///
/// # Errors
///
/// Returns an error for an invalid window/progress value, unavailable COM
/// integration, or a platform without a taskbar progress backend.
pub fn set_progress(
    native_window: isize,
    progress: f64,
    state: ProgressState,
) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        windows_status::set_progress(native_window, progress, state)
    }
    #[cfg(target_os = "macos")]
    {
        let _ = native_window;
        macos_status::set_progress(progress, state)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = (native_window, progress, state);
        Err("taskbar progress is unavailable on this platform".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_states_are_stable_sequential_integers() {
        assert_eq!(ProgressState::Hidden as u8, 1);
        assert_eq!(ProgressState::Indeterminate as u8, 2);
        assert_eq!(ProgressState::Normal as u8, 3);
        assert_eq!(ProgressState::Paused as u8, 4);
        assert_eq!(ProgressState::Error as u8, 5);
        assert!(ProgressState::try_from(0).is_err());
        assert!(ProgressState::try_from(6).is_err());
    }
}

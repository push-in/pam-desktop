//! The icon of a Windows launcher. The launcher of a bundle is a copy of
//! the host under the application's name, so its icon has to be written
//! into the copy: one `RT_ICON` resource per image and the `RT_GROUP_ICON`
//! that lists them, which is what the shell reads from an executable.
//!
//! Building the resources is plain byte work and runs everywhere; only
//! writing them into the executable needs Windows.

use std::path::Path;

/// The largest image an icon holds, stored as 0 in its one-byte fields.
const LARGEST: u32 = 256;

/// The resources that make the icon of an executable.
#[derive(Debug, PartialEq, Eq)]
pub struct IconResources {
    /// The `GRPICONDIR` of the `RT_GROUP_ICON` resource.
    pub group: Vec<u8>,
    /// Each `RT_ICON` resource: its identifier and its image.
    pub images: Vec<(u16, Vec<u8>)>,
}

/// The side of a square PNG image, read from its header.
fn png_side(image: &[u8]) -> Option<u32> {
    const SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if image.get(..8) != Some(SIGNATURE) || image.get(12..16) != Some(b"IHDR") {
        return None;
    }
    let number = |range: std::ops::Range<usize>| {
        image
            .get(range)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u32::from_be_bytes)
    };
    let (width, height) = (number(16..20)?, number(20..24)?);
    (width == height).then_some(width)
}

/// Builds the icon resources from PNG images, one per size. Icons hold
/// PNG images as they are since Windows Vista.
pub fn icon_resources(images: &[Vec<u8>]) -> Result<IconResources, String> {
    let count = u16::try_from(images.len())
        .ok()
        .filter(|count| *count > 0)
        .ok_or("an icon needs between 1 and 65535 images")?;
    // GRPICONDIR: reserved, type 1 (icon), count; then 14 bytes per image.
    let mut group = Vec::with_capacity(6 + 14 * images.len());
    group.extend_from_slice(&0_u16.to_le_bytes());
    group.extend_from_slice(&1_u16.to_le_bytes());
    group.extend_from_slice(&count.to_le_bytes());
    let mut resources = Vec::with_capacity(images.len());
    for (index, image) in images.iter().enumerate() {
        let side = png_side(image)
            .filter(|side| (1..=LARGEST).contains(side))
            .ok_or("an icon image must be a square PNG of at most 256 pixels")?;
        let length =
            u32::try_from(image.len()).map_err(|_| "an icon image is too large".to_owned())?;
        // Identifiers start at 1; the count above fits in 16 bits.
        let id = index as u16 + 1;
        // GRPICONDIRENTRY: width, height, colors in the palette, reserved,
        // planes, bits per pixel, bytes of the image, its RT_ICON identifier.
        let byte = (side % LARGEST) as u8;
        group.extend_from_slice(&[byte, byte, 0, 0]);
        group.extend_from_slice(&1_u16.to_le_bytes());
        group.extend_from_slice(&32_u16.to_le_bytes());
        group.extend_from_slice(&length.to_le_bytes());
        group.extend_from_slice(&id.to_le_bytes());
        resources.push((id, image.clone()));
    }
    Ok(IconResources {
        group,
        images: resources,
    })
}

/// Writes the icon into `executable`, which must not be running or signed
/// yet: a resource written after a signature would break it.
#[cfg(windows)]
pub fn write(executable: &Path, resources: &IconResources) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::LibraryLoader::{
        BeginUpdateResourceW, EndUpdateResourceW, UpdateResourceW,
    };

    // Resource types and identifiers are integers passed as pointers
    // (`MAKEINTRESOURCE`).
    const RT_ICON: *const u16 = std::ptr::without_provenance(3);
    const RT_GROUP_ICON: *const u16 = std::ptr::without_provenance(14);
    // The group the window class of the host loads its icon from, and the
    // first one the shell finds. The host has no icon of its own; were it
    // given one, it would have to use this identifier and language for
    // the application's to replace it.
    const GROUP_ID: u16 = 1;
    // US English, the language resource compilers give by default.
    const LANGUAGE: u16 = 0x0409;

    let failure = |step: &str| {
        // Read before anything else can change the thread's last error.
        let error = std::io::Error::last_os_error();
        format!(
            "cannot write the application icon into {}: {step} failed: {error}",
            executable.display()
        )
    };
    let path: Vec<u16> = executable
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut entries: Vec<(*const u16, u16, &[u8])> = resources
        .images
        .iter()
        .map(|(id, image)| (RT_ICON, *id, image.as_slice()))
        .collect();
    entries.push((RT_GROUP_ICON, GROUP_ID, &resources.group));
    // SAFETY: the path is NUL-terminated; every data pointer and length
    // come from a live slice; the update handle is closed on every path,
    // discarding the changes when one of them failed.
    unsafe {
        // The resources the host already has (its manifest) are kept.
        let update = BeginUpdateResourceW(path.as_ptr(), 0);
        if update.is_null() {
            return Err(failure("BeginUpdateResourceW"));
        }
        for (kind, id, data) in entries {
            let Ok(length) = u32::try_from(data.len()) else {
                EndUpdateResourceW(update, 1);
                return Err("an icon image is too large".to_owned());
            };
            let name = std::ptr::without_provenance(usize::from(id));
            if UpdateResourceW(update, kind, name, LANGUAGE, data.as_ptr().cast(), length) == 0 {
                let error = failure("UpdateResourceW");
                EndUpdateResourceW(update, 1);
                return Err(error);
            }
        }
        if EndUpdateResourceW(update, 0) == 0 {
            return Err(failure("EndUpdateResourceW"));
        }
    }
    Ok(())
}

/// Executables only carry icons on Windows, and only Windows can write
/// them: the packager never gets here on another system.
#[cfg(not(windows))]
pub fn write(executable: &Path, _resources: &IconResources) -> Result<(), String> {
    Err(format!(
        "cannot write the application icon into {}: resources are only written on Windows",
        executable.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The start of a PNG file of `width` by `height`, with `extra` bytes after it.
    fn png(width: u32, height: u32, extra: usize) -> Vec<u8> {
        let mut image = Vec::from(*b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR");
        image.extend_from_slice(&width.to_be_bytes());
        image.extend_from_slice(&height.to_be_bytes());
        image.extend(std::iter::repeat_n(0xAB, extra));
        image
    }

    #[test]
    fn builds_the_group_and_one_resource_per_image() {
        let images = [png(16, 16, 10), png(48, 48, 300), png(256, 256, 70_000)];
        let resources = icon_resources(&images).unwrap();

        let mut group = vec![0, 0, 1, 0, 3, 0];
        // 16 px: 34 bytes (0x22), identifier 1.
        group.extend_from_slice(&[16, 16, 0, 0, 1, 0, 32, 0, 0x22, 0, 0, 0, 1, 0]);
        // 48 px: 324 bytes (0x144), identifier 2.
        group.extend_from_slice(&[48, 48, 0, 0, 1, 0, 32, 0, 0x44, 0x01, 0, 0, 2, 0]);
        // 256 px is stored as 0: 70024 bytes (0x11188), identifier 3.
        group.extend_from_slice(&[0, 0, 0, 0, 1, 0, 32, 0, 0x88, 0x11, 0x01, 0, 3, 0]);
        assert_eq!(resources.group, group);
        assert_eq!(
            resources.images,
            vec![
                (1, images[0].clone()),
                (2, images[1].clone()),
                (3, images[2].clone())
            ]
        );
    }

    #[test]
    fn refuses_what_an_icon_cannot_hold() {
        assert!(icon_resources(&[]).is_err());
        assert!(icon_resources(&[b"GIF89a".to_vec()]).is_err());
        assert!(icon_resources(&[png(32, 16, 0)]).is_err());
        assert!(icon_resources(&[png(512, 512, 0)]).is_err());
        assert!(icon_resources(&[png(0, 0, 0)]).is_err());
        // A header cut short has no size to read.
        assert!(icon_resources(&[png(16, 16, 0)[..20].to_vec()]).is_err());
    }

    #[cfg(not(windows))]
    #[test]
    fn only_windows_writes_resources() {
        let resources = icon_resources(&[png(16, 16, 0)]).unwrap();
        let error = write(Path::new("launcher.exe"), &resources).unwrap_err();
        assert!(error.contains("launcher.exe") && error.contains("Windows"));
    }
}

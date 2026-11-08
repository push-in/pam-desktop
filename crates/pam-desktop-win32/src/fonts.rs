//! The application's packaged fonts (`asset://…ttf|otf` families, such as
//! icon fonts): read from the asset root, never outside it, and kept for the
//! process, at most 32 files and 64 MiB. DirectWrite sees each one through a
//! font collection of its own; GDI (the `EDIT` controls) through a private
//! registration of the file.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use windows::Win32::Graphics::DirectWrite::{
    IDWriteFactory, IDWriteFactory5, IDWriteFontCollection,
};
use windows::Win32::Graphics::Gdi::{AddFontResourceExW, FONT_RESOURCE_CHARACTERISTICS};
use windows::core::{Interface, PCWSTR};

use crate::util::{from_wide, wide};

const MAX_FONTS: usize = 32;
const MAX_FILE_BYTES: u64 = 8 << 20;
const MAX_TOTAL_BYTES: u64 = 64 << 20;
/// `FR_PRIVATE`: the font is only visible to this process.
const FR_PRIVATE: FONT_RESOURCE_CHARACTERISTICS = FONT_RESOURCE_CHARACTERISTICS(0x10);

/// A packaged font: its family name and the collection that holds it.
#[derive(Clone)]
pub struct AssetFont {
    pub family: String,
    pub collection: IDWriteFontCollection,
}

#[derive(Default)]
struct Registry {
    root: PathBuf,
    fonts: HashMap<String, Option<AssetFont>>,
    bytes: u64,
}

thread_local! {
    static REGISTRY: RefCell<Registry> = RefCell::new(Registry::default());
}

/// `Backend::set_asset_root`: where `asset://` fonts are read from.
pub fn set_root(root: &Path) {
    REGISTRY.with(|registry| {
        *registry.borrow_mut() = Registry {
            root: root.canonicalize().unwrap_or_default(),
            ..Registry::default()
        };
    });
}

fn asset_path(root: &Path, source: &str) -> Option<PathBuf> {
    let relative = source.strip_prefix("asset://")?;
    if root.as_os_str().is_empty()
        || relative.is_empty()
        || relative.contains(['\\', '\0', '?', '#', ':'])
    {
        return None;
    }
    let path = Path::new(relative);
    if !path
        .components()
        .all(|part| matches!(part, Component::Normal(_)))
        || !matches!(
            path.extension()?.to_str()?.to_ascii_lowercase().as_str(),
            "ttf" | "otf"
        )
    {
        return None;
    }
    let resolved = root.join(path).canonicalize().ok()?;
    (resolved.starts_with(root) && resolved.is_file()).then_some(resolved)
}

/// The packaged font `family` names (`asset://fonts/Icons.ttf`), loaded the
/// first time; `None` when it is not one or it cannot be loaded.
pub fn asset(dwrite: &IDWriteFactory, family: &str) -> Option<AssetFont> {
    let source = family.split(',').next()?.trim().trim_matches(['"', '\'']);
    if !source.starts_with("asset://") {
        return None;
    }
    REGISTRY.with(|registry| {
        let mut registry = registry.borrow_mut();
        if let Some(font) = registry.fonts.get(source) {
            return font.clone();
        }
        if registry.fonts.len() >= MAX_FONTS {
            return None;
        }
        let path = asset_path(&registry.root, source);
        let size = path
            .as_ref()
            .and_then(|path| path.metadata().ok())
            .map_or(u64::MAX, |metadata| metadata.len());
        let font = path
            .filter(|_| size <= MAX_FILE_BYTES && registry.bytes + size <= MAX_TOTAL_BYTES)
            .and_then(|path| load(dwrite, &path));
        if font.is_some() {
            registry.bytes += size;
        }
        registry.fonts.insert(source.to_owned(), font.clone());
        font
    })
}

fn load(dwrite: &IDWriteFactory, path: &Path) -> Option<AssetFont> {
    let file = wide(&path.to_string_lossy());
    // SAFETY: COM calls on live DirectWrite objects; the path is
    // NUL-terminated and outlives every call that reads it.
    unsafe {
        let factory = dwrite.cast::<IDWriteFactory5>().ok()?;
        let reference = dwrite
            .CreateFontFileReference(PCWSTR(file.as_ptr()), None)
            .ok()?;
        let builder = factory.CreateFontSetBuilder().ok()?;
        builder.AddFontFile(&reference).ok()?;
        let set = builder.CreateFontSet().ok()?;
        let collection: IDWriteFontCollection = factory
            .CreateFontCollectionFromFontSet(&set)
            .ok()?
            .cast()
            .ok()?;
        if collection.GetFontFamilyCount() == 0 {
            return None;
        }
        let names = collection.GetFontFamily(0).ok()?.GetFamilyNames().ok()?;
        let length = names.GetStringLength(0).ok()? as usize;
        let mut name = vec![0_u16; length + 1];
        names.GetString(0, &mut name).ok()?;
        let family = from_wide(&name[..length]);
        if family.is_empty() {
            return None;
        }
        // The `EDIT` controls draw with GDI, which finds the family once
        // the file is registered for this process.
        let _ = AddFontResourceExW(PCWSTR(file.as_ptr()), FR_PRIVATE, None);
        Some(AssetFont { family, collection })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_assets_stay_inside_the_root() {
        let root = std::env::temp_dir().join(format!("pam-win32-fonts-{}", std::process::id()));
        std::fs::create_dir_all(root.join("fonts")).unwrap();
        std::fs::write(root.join("fonts/Icons.TTF"), b"test").unwrap();
        std::fs::write(root.join("secret.txt"), b"test").unwrap();
        let root = root.canonicalize().unwrap();
        assert_eq!(
            asset_path(&root, "asset://fonts/Icons.TTF"),
            Some(root.join("fonts").join("Icons.TTF"))
        );
        for source in [
            "asset://../fonts/Icons.TTF",
            "asset:///fonts/Icons.TTF",
            "asset://fonts\\Icons.TTF",
            "asset://C:/Windows/Fonts/arial.ttf",
            "asset://secret.txt",
            "asset://fonts/Icons.TTF?x",
            "fonts/Icons.TTF",
        ] {
            assert!(asset_path(&root, source).is_none(), "{source}");
        }
        assert!(asset_path(Path::new(""), "asset://fonts/Icons.TTF").is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
}

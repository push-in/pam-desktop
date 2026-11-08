//! The application's packaged fonts (`asset://…ttf|otf` families, such as
//! icon fonts): read from the asset root, never outside it, and kept for the
//! process, at most 32 files and 64 MiB.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use objc2::rc::Retained;
use objc2_app_kit::NSFont;
use objc2_core_foundation::{CFData, CFRetained, CGFloat};
use objc2_core_text::{CTFont, CTFontDescriptor, CTFontManagerCreateFontDescriptorFromData};

#[derive(Default)]
struct Registry {
    root: PathBuf,
    descriptors: HashMap<String, Option<CFRetained<CTFontDescriptor>>>,
    bytes: u64,
}

thread_local! {
    static REGISTRY: RefCell<Registry> = RefCell::new(Registry::default());
}

pub fn set_root(root: &Path) {
    REGISTRY.with(|registry| {
        *registry.borrow_mut() = Registry {
            root: root.canonicalize().unwrap_or_default(),
            ..Registry::default()
        };
    });
}

fn asset_path(root: &Path, source: &str) -> Option<PathBuf> {
    let relative = Path::new(source.strip_prefix("asset://")?);
    if root.as_os_str().is_empty()
        || source.contains(['\\', '\0', '?', '#'])
        || !relative
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
        || !matches!(relative.extension()?.to_str()?, "ttf" | "otf")
    {
        return None;
    }
    let resolved = root.join(relative).canonicalize().ok()?;
    (resolved.starts_with(root) && resolved.is_file()).then_some(resolved)
}

/// The packaged font `source` at `size`, or `None` when it is not an
/// `asset://` font or cannot be loaded.
pub fn font(source: &str, size: CGFloat) -> Option<Retained<NSFont>> {
    if !source.starts_with("asset://") {
        return None;
    }
    let descriptor = REGISTRY.with(|registry| {
        let mut registry = registry.borrow_mut();
        if let Some(descriptor) = registry.descriptors.get(source) {
            return descriptor.clone();
        }
        if registry.descriptors.len() >= 32 {
            return None;
        }
        let bytes = asset_path(&registry.root, source)
            .and_then(|path| std::fs::read(path).ok())
            .filter(|bytes| {
                bytes.len() <= 8 << 20 && registry.bytes + bytes.len() as u64 <= 64 << 20
            });
        // SAFETY: the data is an owned copy of the file.
        let descriptor = bytes.as_ref().and_then(|bytes| unsafe {
            CTFontManagerCreateFontDescriptorFromData(&CFData::from_bytes(bytes))
        });
        if descriptor.is_some() {
            registry.bytes += bytes.map_or(0, |bytes| bytes.len() as u64);
        }
        registry
            .descriptors
            .insert(source.to_owned(), descriptor.clone());
        descriptor
    })?;
    // SAFETY: a null matrix is the identity; `CTFont` is toll-free bridged
    // with `NSFont`, so the owned reference moves into a `Retained<NSFont>`.
    unsafe {
        let font = CTFont::with_font_descriptor(&descriptor, size, std::ptr::null());
        Retained::from_raw(CFRetained::into_raw(font).as_ptr().cast::<NSFont>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_assets_stay_inside_the_root() {
        let root = std::env::temp_dir().join(format!("pam-appkit-fonts-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("font.ttf"), b"test").unwrap();
        std::fs::write(root.join("secret.txt"), b"test").unwrap();
        let root = root.canonicalize().unwrap();
        assert_eq!(
            asset_path(&root, "asset://font.ttf"),
            Some(root.join("font.ttf"))
        );
        for source in [
            "asset://../font.ttf",
            "asset:///font.ttf",
            "asset://secret.txt",
            "asset://font.ttf?x",
        ] {
            assert!(asset_path(&root, source).is_none(), "{source}");
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}

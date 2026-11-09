//! Process-local, bounded registration of the application's packaged fonts.
use gtk::glib::translate::ToGlibPtr;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::path::{Component, Path, PathBuf};

#[link(name = "pangoft2-1.0")]
unsafe extern "C" {
    fn pango_fc_font_map_config_changed(map: *mut c_void);
}

pub fn invalidate_map(map: &gtk::pango::FontMap) {
    // SAFETY: GTK's Linux PangoCairo map is a Fontconfig map. Its config
    // changed, not just its serial: discard cached fallback font sets too.
    let pointer: *mut gtk::pango::ffi::PangoFontMap = map.to_glib_none().0;
    unsafe {
        pango_fc_font_map_config_changed(pointer.cast());
    }
}

#[link(name = "fontconfig")]
unsafe extern "C" {
    fn FcConfigGetCurrent() -> *mut c_void;
    fn FcConfigAppFontAddFile(config: *mut c_void, file: *const u8) -> c_int;
    fn FcFreeTypeQuery(
        file: *const u8,
        index: u32,
        blanks: *mut c_void,
        count: *mut c_int,
    ) -> *mut c_void;
    fn FcPatternGetString(
        pattern: *const c_void,
        object: *const c_char,
        index: c_int,
        value: *mut *mut u8,
    ) -> c_int;
    fn FcPatternDestroy(pattern: *mut c_void);
    fn FcPatternCreate() -> *mut c_void;
    fn FcConfigSubstitute(config: *mut c_void, pattern: *mut c_void, kind: c_int) -> c_int;
    fn FcDefaultSubstitute(pattern: *mut c_void);
    fn FcPatternGetInteger(
        pattern: *const c_void,
        object: *const c_char,
        index: c_int,
        value: *mut c_int,
    ) -> c_int;
    fn FcPatternGetBool(
        pattern: *const c_void,
        object: *const c_char,
        index: c_int,
        value: *mut c_int,
    ) -> c_int;
    fn FcPatternAddString(pattern: *mut c_void, object: *const c_char, value: *const u8) -> c_int;
    fn FcObjectSetCreate() -> *mut c_void;
    fn FcObjectSetAdd(set: *mut c_void, object: *const c_char) -> c_int;
    fn FcObjectSetDestroy(set: *mut c_void);
    fn FcFontList(config: *mut c_void, pattern: *mut c_void, set: *mut c_void) -> *mut FcFontSet;
    fn FcFontSetDestroy(set: *mut FcFontSet);
}

#[repr(C)]
struct FcFontSet {
    nfont: c_int,
    sfont: c_int,
    fonts: *mut *mut c_void,
}

/// Whether Fontconfig has a font of exactly this family (case-insensitive),
/// not one its aliases would substitute.
fn has_family(name: &str) -> bool {
    let Ok(family) = CString::new(name) else {
        return false;
    };
    // SAFETY: pattern, object set and font set are created and destroyed
    // here; Fontconfig copies the family string.
    unsafe {
        let pattern = FcPatternCreate();
        let objects = FcObjectSetCreate();
        if pattern.is_null() || objects.is_null() {
            return false;
        }
        FcPatternAddString(pattern, c"family".as_ptr(), family.as_ptr().cast());
        FcObjectSetAdd(objects, c"family".as_ptr());
        let fonts = FcFontList(std::ptr::null_mut(), pattern, objects);
        let found = !fonts.is_null() && (*fonts).nfont > 0;
        if !fonts.is_null() {
            FcFontSetDestroy(fonts);
        }
        FcObjectSetDestroy(objects);
        FcPatternDestroy(pattern);
        found
    }
}

/// Metric-compatible families Chromium lets stand in for each other on Linux
/// (Skia's font equivalence classes): `Courier New` uses Liberation Mono.
const EQUIVALENT: [&[&str]; 5] = [
    &["arial", "arimo", "liberation sans"],
    &["times new roman", "tinos", "liberation serif"],
    &["courier new", "cousine", "liberation mono"],
    &["calibri", "carlito"],
    &["cambria", "caladea"],
];

thread_local! {
    static INSTALLED: RefCell<HashMap<String, Option<String>>> = RefCell::default();
}

/// The installed family a CSS family name resolves to, as Chromium matches
/// it on Linux: the family itself or a metric-compatible equivalent, never a
/// Fontconfig alias (`Consolas` does not become DejaVu Sans Mono), so an
/// uninstalled name lets the list go on to its next family.
fn installed(name: &str) -> Option<String> {
    let key = name.to_ascii_lowercase();
    if let Some(found) = INSTALLED.with(|cache| cache.borrow().get(&key).cloned()) {
        return found;
    }
    let found = if has_family(name) {
        Some(name.to_owned())
    } else {
        EQUIVALENT
            .iter()
            .find(|class| class.contains(&key.as_str()))
            .and_then(|class| class.iter().find(|other| has_family(other)))
            .map(|other| (*other).to_owned())
    };
    INSTALLED.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() < 256 {
            cache.insert(key, found.clone());
        }
    });
    found
}

/// The hinting fontconfig gives a font by default (`hintnone` ..
/// `hintfull`), which Chromium and Qt follow when no desktop settings
/// daemon says otherwise.
pub fn default_hint_style() -> Option<&'static str> {
    // SAFETY: a pattern owned and destroyed here; FcMatchPattern is 0.
    unsafe {
        let pattern = FcPatternCreate();
        if pattern.is_null() {
            return None;
        }
        FcConfigSubstitute(std::ptr::null_mut(), pattern, 0);
        FcDefaultSubstitute(pattern);
        let (mut hinting, mut style) = (1, 0);
        let has_hinting = FcPatternGetBool(pattern, c"hinting".as_ptr(), 0, &raw mut hinting) == 0;
        let has_style = FcPatternGetInteger(pattern, c"hintstyle".as_ptr(), 0, &raw mut style) == 0;
        FcPatternDestroy(pattern);
        if has_hinting && hinting == 0 {
            return Some("hintnone");
        }
        has_style.then_some(match style {
            0 => "hintnone",
            1 => "hintslight",
            2 => "hintmedium",
            _ => "hintfull",
        })
    }
}

#[derive(Default)]
struct Registry {
    root: PathBuf,
    families: HashMap<String, String>,
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
    let relative = source.strip_prefix("asset://")?;
    if root.as_os_str().is_empty()
        || relative.is_empty()
        || relative.contains(['\\', '\0', '?', '#'])
    {
        return None;
    }
    let path = Path::new(relative);
    if !path
        .components()
        .all(|part| matches!(part, Component::Normal(_)))
    {
        return None;
    }
    if !matches!(path.extension()?.to_str()?, "ttf" | "otf") {
        return None;
    }
    let resolved = root.join(path).canonicalize().ok()?;
    (resolved.starts_with(root) && resolved.is_file()).then_some(resolved)
}

/// Returns the font's actual family and whether the native font map changed.
pub fn family(source: &str) -> (String, bool) {
    if !source.starts_with("asset://") {
        return (source.to_owned(), false);
    }
    REGISTRY.with(|registry| {
        let mut registry = registry.borrow_mut();
        if let Some(family) = registry.families.get(source) {
            return (family.clone(), false);
        }
        if registry.families.len() >= 32 {
            return (String::new(), false);
        }
        let Some(path) = asset_path(&registry.root, source) else {
            return (String::new(), false);
        };
        let Ok(metadata) = path.metadata() else {
            return (String::new(), false);
        };
        if metadata.len() > 8 * 1024 * 1024 || registry.bytes + metadata.len() > 64 * 1024 * 1024 {
            return (String::new(), false);
        }
        let Ok(file) = CString::new(path.as_os_str().as_encoded_bytes()) else {
            return (String::new(), false);
        };
        // SAFETY: all strings are NUL terminated; the Fontconfig pattern owns
        // its family until it is copied below and the pattern is destroyed.
        let loaded = unsafe {
            let mut count = 0;
            let pattern =
                FcFreeTypeQuery(file.as_ptr().cast(), 0, std::ptr::null_mut(), &mut count);
            if pattern.is_null() {
                return (String::new(), false);
            }
            let mut name = std::ptr::null_mut();
            let result = FcPatternGetString(pattern, c"family".as_ptr(), 0, &mut name);
            let family = (result == 0 && !name.is_null())
                .then(|| CStr::from_ptr(name.cast()).to_string_lossy().into_owned());
            FcPatternDestroy(pattern);
            family
                .filter(|_| FcConfigAppFontAddFile(FcConfigGetCurrent(), file.as_ptr().cast()) != 0)
        };
        if let Some(family) = loaded {
            registry.bytes += metadata.len();
            registry.families.insert(source.to_owned(), family.clone());
            (family, true)
        } else {
            (String::new(), false)
        }
    })
}

thread_local! {
    /// The desktop's interface font family (`gtk-font-name`), read once.
    static UI_FAMILY: std::cell::OnceCell<String> = const { std::cell::OnceCell::new() };
}

/// The family CSS `system-ui` names here: the interface font GTK uses
/// (`gtk-font-name`, as Chromium reads it on Linux), else Fontconfig's
/// `sans-serif`.
fn ui_family() -> String {
    if !gtk::is_initialized_main_thread() {
        return String::from("sans-serif");
    }
    UI_FAMILY.with(|family| {
        family
            .get_or_init(|| {
                gtk::Settings::default()
                    .and_then(|settings| settings.gtk_font_name())
                    .and_then(|name| {
                        gtk::pango::FontDescription::from_string(&name)
                            .family()
                            .map(|family| family.to_string())
                    })
                    .filter(|family| !family.trim().is_empty())
                    .unwrap_or_else(|| String::from("sans-serif"))
            })
            .clone()
    })
}

/// A CSS `font-family` list as a Pango family list: packaged fonts become
/// their registered family, the CSS generic families Fontconfig's aliases
/// (`system-ui` the interface font). Fontconfig then uses the first family
/// installed. Like Chromium, Safari's `ui-monospace`,
/// `ui-serif`, `ui-sans-serif` and `ui-rounded` match nothing: the list goes
/// on to its next family (Tailwind's `font-mono` lands on Liberation Mono,
/// not on Fontconfig's `monospace`), and a named family that is not installed
/// is skipped rather than replaced by a Fontconfig alias. Also returns whether
/// the native font map changed.
pub fn stack(list: &str) -> (String, bool) {
    use pam_desktop_ui::engine::font_family::{Family, families};
    let mut changed = false;
    let mut generic = false;
    let mut names: Vec<String> = Vec::new();
    let linux = list
        .split(',')
        .filter(|entry| {
            let name = entry
                .trim()
                .trim_matches(['"', '\''])
                .trim()
                .to_ascii_lowercase();
            !matches!(
                name.as_str(),
                "ui-monospace" | "ui-serif" | "ui-sans-serif" | "ui-rounded"
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let mut named = false;
    for entry in families(&linux) {
        generic |= entry.is_generic();
        named = true;
        let name = match entry {
            Family::Asset(source) => {
                let (name, loaded) = family(source);
                changed |= loaded;
                name
            }
            Family::SystemUi | Family::Rounded => ui_family(),
            Family::Monospace => String::from("monospace"),
            Family::Serif => String::from("serif"),
            Family::Named(name) => match installed(name) {
                Some(name) => name,
                None => continue,
            },
        };
        // Pango separates families by commas.
        let name = name.replace(',', " ");
        if !name.is_empty() && !names.contains(&name) {
            names.push(name);
        }
    }
    // A list of named fonts none of which is installed ends in the
    // interface font, as on the other backends.
    if !generic && named {
        let ui = ui_family();
        if !names.contains(&ui) {
            names.push(ui);
        }
    }
    (names.join(","), changed)
}

/// A CSS `font-family` list for GTK's style provider from a resolved Pango
/// family list (see [`stack`]): every family quoted, characters that could
/// end the declaration dropped.
pub fn css_list(pango: &str) -> String {
    pango
        .split(',')
        .map(|name| name.trim().replace(['"', '\\', ';', '{', '}'], ""))
        .filter(|name| !name.is_empty())
        .map(|name| format!("\"{name}\""))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resolved_lists_become_gtk_css_lists() {
        assert_eq!(
            css_list("Plus Jakarta Sans,Cantarell,sans-serif"),
            "\"Plus Jakarta Sans\", \"Cantarell\", \"sans-serif\""
        );
        assert_eq!(css_list("a\";}b, ,"), "\"ab\"");
        assert_eq!(css_list(""), "");
        // A packaged face that is not loadable falls through to the rest of
        // the list instead of emptying it (the field drew a fallback face).
        assert_eq!(
            stack("asset://fonts/missing.ttf, monospace"),
            (String::from("monospace"), false)
        );
    }

    #[test]
    fn css_font_lists_become_pango_family_lists() {
        // Uninstalled names and the macOS-only `ui-*` families match
        // nothing, as in Chromium on Linux.
        assert_eq!(
            stack("ui-monospace, NoSuchFont Pam, monospace, serif"),
            (String::from("monospace,serif"), false)
        );
        assert_eq!(stack(" , "), (String::new(), false));
        // Named fonts only: the interface font closes the list.
        assert_eq!(
            stack("NoSuchFont Pam, \"Other Missing Pam\""),
            (String::from("sans-serif"), false)
        );
    }

    #[test]
    fn font_assets_reject_escape_and_non_font_files() {
        let root = std::env::temp_dir().join(format!("pam-font-assets-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("font.ttf"), b"test").unwrap();
        std::fs::write(root.join("secret.txt"), b"test").unwrap();
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
        std::os::unix::fs::symlink("/etc/hosts", root.join("escape.ttf")).unwrap();
        assert!(asset_path(&root, "asset://escape.ttf").is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
}

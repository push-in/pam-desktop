//! CSS `font-family` lists, shared by every text backend.
//!
//! A run's family is the authored CSS list (`"Inter, system-ui, sans-serif"`).
//! Backends walk it in order and use the first family they have. The CSS
//! generic families name the platform's own fonts: [`Family::SystemUi`] is
//! the UI font (Segoe UI on Windows, San Francisco on macOS, the desktop's
//! interface font on Linux), [`Family::Monospace`] its monospace companion.

/// One entry of a CSS `font-family` list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family<'a> {
    /// A packaged font (`asset://fonts/Inter.ttf`).
    Asset(&'a str),
    /// `system-ui`, `-apple-system`, `BlinkMacSystemFont`, `ui-sans-serif`,
    /// `sans-serif` and the legacy `system`: the platform UI font.
    SystemUi,
    /// `monospace`, `ui-monospace`: the platform monospace font.
    Monospace,
    /// `serif`, `ui-serif`.
    Serif,
    /// `ui-rounded` (the UI font where the platform has no rounded design).
    Rounded,
    /// An installed family by name (`Segoe UI`, `DejaVu Sans`).
    Named(&'a str),
}

impl<'a> Family<'a> {
    fn parse(entry: &'a str) -> Option<Self> {
        let entry = entry.trim().trim_matches(['"', '\'']).trim();
        if entry.is_empty() {
            return None;
        }
        if entry.starts_with("asset://") {
            return Some(Self::Asset(entry));
        }
        Some(match entry.to_ascii_lowercase().as_str() {
            "system-ui" | "system" | "-apple-system" | "blinkmacsystemfont" | "ui-sans-serif"
            | "sans-serif" => Self::SystemUi,
            "monospace" | "ui-monospace" => Self::Monospace,
            "serif" | "ui-serif" => Self::Serif,
            "ui-rounded" => Self::Rounded,
            _ => Self::Named(entry),
        })
    }

    /// Whether this entry is one of the CSS generic families.
    #[must_use]
    pub fn is_generic(self) -> bool {
        !matches!(self, Self::Asset(_) | Self::Named(_))
    }
}

/// The entries of a CSS `font-family` list, in order, empty ones skipped.
pub fn families(list: &str) -> impl Iterator<Item = Family<'_>> {
    list.split(',').filter_map(Family::parse)
}

/// The first packaged font (`asset://…`) a list names.
#[must_use]
pub fn first_asset(list: &str) -> Option<&str> {
    families(list).find_map(|family| match family {
        Family::Asset(source) => Some(source),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_keep_their_order_and_map_the_generic_families() {
        let list = "\"Segoe UI\", system-ui, -apple-system, BlinkMacSystemFont, ui-monospace, \
                    Menlo, monospace, serif, ui-rounded, asset://fonts/Inter.ttf, , sans-serif";
        assert_eq!(
            families(list).collect::<Vec<_>>(),
            [
                Family::Named("Segoe UI"),
                Family::SystemUi,
                Family::SystemUi,
                Family::SystemUi,
                Family::Monospace,
                Family::Named("Menlo"),
                Family::Monospace,
                Family::Serif,
                Family::Rounded,
                Family::Asset("asset://fonts/Inter.ttf"),
                Family::SystemUi,
            ]
        );
        assert_eq!(first_asset(list), Some("asset://fonts/Inter.ttf"));
        assert_eq!(first_asset("DejaVu Sans"), None);
        assert!(Family::Monospace.is_generic() && !Family::Named("Inter").is_generic());
    }
}

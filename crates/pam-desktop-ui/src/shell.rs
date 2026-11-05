//! Desktop shell vocabulary: menus, accelerators, file dialogs, the tray,
//! notifications, window commands, cursors.
//!
//! These types travel from PHP (as JSON inside module calls) to the backend,
//! which maps them onto the platform's own menus, panels and status items.

use std::path::PathBuf;

use crate::backend::Modifiers;
use crate::geometry::Size;

/// A keyboard shortcut: modifiers plus one key (a W3C `key` value, letters
/// lower-case).
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Accelerator {
    pub modifiers: Modifiers,
    pub key: String,
}

impl Accelerator {
    /// Parses `CmdOrCtrl+Shift+N`, `Alt+F4`, `Ctrl+,`, `F11`…
    ///
    /// `CmdOrCtrl` (also `CommandOrControl`, `Mod`) is Command on macOS and
    /// Control elsewhere. Keys may be single characters, W3C key names
    /// (`Enter`, `ArrowLeft`), short names (`Left`, `Esc`, `Space`, `Plus`)
    /// or physical codes (`KeyP`, `Digit1`).
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let mut modifiers = Modifiers::default();
        let mut key: Option<String> = None;
        // The `+` key itself is written as a trailing `+` (`Ctrl++`).
        let text = text.trim();
        let text = if text == "+" {
            key = Some(String::from("+"));
            ""
        } else if let Some(prefix) = text.strip_suffix("++") {
            key = Some(String::from("+"));
            prefix
        } else {
            text
        };
        for token in text.split('+').map(str::trim).filter(|_| !text.is_empty()) {
            if token.is_empty() {
                return None;
            }
            match token.to_ascii_lowercase().as_str() {
                "cmdorctrl" | "commandorcontrol" | "mod" => {
                    if cfg!(target_os = "macos") {
                        modifiers.meta = true;
                    } else {
                        modifiers.control = true;
                    }
                }
                "cmd" | "command" | "meta" | "super" | "win" => modifiers.meta = true,
                "ctrl" | "control" => modifiers.control = true,
                "alt" | "option" | "opt" => modifiers.alt = true,
                "shift" => modifiers.shift = true,
                _ => {
                    if key.is_some() {
                        return None;
                    }
                    key = Some(Self::key_name(token)?);
                }
            }
        }
        Some(Self {
            modifiers,
            key: key?,
        })
    }

    fn key_name(token: &str) -> Option<String> {
        let lower = token.to_ascii_lowercase();
        if let Some(letter) = lower.strip_prefix("key").filter(|rest| rest.len() == 1) {
            return Some(letter.to_owned());
        }
        if let Some(digit) = lower.strip_prefix("digit").filter(|rest| rest.len() == 1) {
            return Some(digit.to_owned());
        }
        let named = match lower.as_str() {
            "enter" | "return" => "Enter",
            "esc" | "escape" => "Escape",
            "space" | "spacebar" => " ",
            "tab" => "Tab",
            "backspace" => "Backspace",
            "delete" | "del" => "Delete",
            "insert" => "Insert",
            "up" | "arrowup" => "ArrowUp",
            "down" | "arrowdown" => "ArrowDown",
            "left" | "arrowleft" => "ArrowLeft",
            "right" | "arrowright" => "ArrowRight",
            "home" => "Home",
            "end" => "End",
            "pageup" => "PageUp",
            "pagedown" => "PageDown",
            "plus" => "+",
            "minus" => "-",
            "comma" => ",",
            "period" => ".",
            "slash" => "/",
            _ => "",
        };
        if !named.is_empty() {
            return Some(named.to_owned());
        }
        if let Some(number) = lower
            .strip_prefix('f')
            .and_then(|rest| rest.parse::<u8>().ok())
        {
            return (1..=24).contains(&number).then(|| format!("F{number}"));
        }
        let mut characters = token.chars();
        match (characters.next(), characters.next()) {
            (Some(character), None) if !character.is_control() => {
                Some(character.to_lowercase().collect())
            }
            _ => None,
        }
    }

    /// Whether a key event with these modifiers triggers the accelerator.
    #[must_use]
    pub fn matches(&self, key: &str, modifiers: Modifiers) -> bool {
        self.modifiers == modifiers && (self.key == key || self.key.eq_ignore_ascii_case(key))
    }
}

/// Menu items the platform implements itself.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MenuRole {
    Quit,
    About,
    Preferences,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    Minimize,
    Zoom,
    ToggleFullScreen,
    CloseWindow,
    Hide,
    HideOthers,
    ShowAll,
}

impl MenuRole {
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "quit" => Self::Quit,
            "about" => Self::About,
            "preferences" => Self::Preferences,
            "undo" => Self::Undo,
            "redo" => Self::Redo,
            "cut" => Self::Cut,
            "copy" => Self::Copy,
            "paste" => Self::Paste,
            "selectAll" => Self::SelectAll,
            "minimize" => Self::Minimize,
            "zoom" => Self::Zoom,
            "toggleFullScreen" => Self::ToggleFullScreen,
            "closeWindow" => Self::CloseWindow,
            "hide" => Self::Hide,
            "hideOthers" => Self::HideOthers,
            "showAll" => Self::ShowAll,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum MenuNode {
    Item {
        /// Reported back when the item is chosen.
        id: String,
        label: String,
        accelerator: Option<Accelerator>,
        enabled: bool,
        /// `Some` makes it a check item.
        checked: Option<bool>,
        role: Option<MenuRole>,
        destructive: bool,
    },
    Separator,
    Submenu {
        label: String,
        enabled: bool,
        children: Vec<MenuNode>,
    },
}

const MAX_MENU_DEPTH: usize = 6;
const MAX_MENU_ITEMS: usize = 512;

/// Decodes the JSON menu the PHP SDK sends:
/// `[{"id","label","shortcut","enabled","checked","role","destructive"} |
///   {"separator":true} | {"label","children":[…]}]`.
#[must_use]
pub fn parse_menu(json: &str) -> Option<Vec<MenuNode>> {
    let value = serde_json::from_str::<serde_json::Value>(json).ok()?;
    let mut budget = MAX_MENU_ITEMS;
    parse_menu_level(&value, 0, &mut budget)
}

fn parse_menu_level(
    value: &serde_json::Value,
    depth: usize,
    budget: &mut usize,
) -> Option<Vec<MenuNode>> {
    if depth > MAX_MENU_DEPTH {
        return None;
    }
    let entries = value.as_array()?;
    let mut nodes = Vec::with_capacity(entries.len());
    for entry in entries {
        *budget = budget.checked_sub(1)?;
        let text = |key: &str| entry.get(key).and_then(serde_json::Value::as_str);
        let flag = |key: &str| entry.get(key).and_then(serde_json::Value::as_bool);
        if flag("separator") == Some(true) {
            nodes.push(MenuNode::Separator);
            continue;
        }
        let label = text("label")
            .unwrap_or_default()
            .chars()
            .take(256)
            .collect::<String>();
        let enabled = flag("enabled").unwrap_or(true);
        if let Some(children) = entry.get("children") {
            nodes.push(MenuNode::Submenu {
                label,
                enabled,
                children: parse_menu_level(children, depth + 1, budget)?,
            });
            continue;
        }
        let role = text("role").and_then(MenuRole::parse);
        let id = text("id").unwrap_or_default().to_owned();
        if id.is_empty() && role.is_none() {
            return None;
        }
        nodes.push(MenuNode::Item {
            id,
            label,
            accelerator: text("shortcut")
                .filter(|shortcut| !shortcut.is_empty())
                .and_then(Accelerator::parse),
            enabled,
            checked: flag("checked"),
            role,
            destructive: flag("destructive").unwrap_or(false),
        });
    }
    Some(nodes)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileDialogKind {
    Open,
    OpenMultiple,
    Save,
    SelectFolder,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileFilter {
    pub name: String,
    /// Extensions without the dot (`png`); `*` accepts everything.
    pub extensions: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileDialogConfig {
    pub kind: FileDialogKind,
    pub title: String,
    /// Suggested file name of a save dialog.
    pub default_name: String,
    pub directory: Option<PathBuf>,
    pub filters: Vec<FileFilter>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TrayConfig {
    /// A PNG file; the application icon when absent.
    pub icon: Option<PathBuf>,
    pub tooltip: String,
    pub menu: Vec<MenuNode>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotificationConfig {
    /// Reported back when the notification is activated.
    pub id: String,
    pub title: String,
    pub body: String,
}

/// A registered shortcut. Global ones fire while the application is in the
/// background, where the platform allows it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Shortcut {
    pub id: String,
    pub accelerator: Accelerator,
    pub global: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum WindowCommand {
    SetTitle(String),
    SetSize(Size),
    SetMinSize(Option<Size>),
    SetResizable(bool),
    Minimize,
    Maximize,
    /// Leaves the maximized, minimized or full screen state.
    Restore,
    SetFullScreen(bool),
    /// Brings the window to the front and gives it keyboard focus.
    Focus,
    Center,
    SetAlwaysOnTop(bool),
    Show,
    Hide,
    /// Closes without asking PHP again.
    Close,
    /// Progress on the taskbar or dock icon, `None` removes it.
    SetProgress(Option<f32>),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WindowState {
    pub maximized: bool,
    pub minimized: bool,
    pub full_screen: bool,
    pub focused: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum Cursor {
    #[default]
    Default,
    Pointer,
    Text,
    Grab,
    Grabbing,
    NotAllowed,
    Crosshair,
    Wait,
    Move,
    ResizeHorizontal,
    ResizeVertical,
}

impl Cursor {
    /// CSS `cursor` keywords.
    #[must_use]
    pub fn parse(name: &str) -> Self {
        match name {
            "pointer" => Self::Pointer,
            "text" => Self::Text,
            "grab" => Self::Grab,
            "grabbing" => Self::Grabbing,
            "not-allowed" => Self::NotAllowed,
            "crosshair" => Self::Crosshair,
            "wait" | "progress" => Self::Wait,
            "move" => Self::Move,
            "ew-resize" | "col-resize" => Self::ResizeHorizontal,
            "ns-resize" | "row-resize" => Self::ResizeVertical,
            _ => Self::Default,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accelerators_parse_modifiers_names_and_codes() {
        let save = Accelerator::parse("Ctrl+Shift+S").expect("valid");
        assert!(save.modifiers.control && save.modifiers.shift && !save.modifiers.alt);
        assert_eq!(save.key, "s");
        assert!(save.matches("S", save.modifiers));
        assert!(!save.matches("s", Modifiers::default()));
        assert_eq!(
            Accelerator::parse("Alt+F4").map(|shortcut| shortcut.key),
            Some("F4".into())
        );
        assert_eq!(
            Accelerator::parse("CmdOrCtrl+KeyP").map(|shortcut| shortcut.key),
            Some("p".into())
        );
        assert_eq!(
            Accelerator::parse("Ctrl++").map(|shortcut| shortcut.key),
            Some("+".into())
        );
        assert_eq!(
            Accelerator::parse("Ctrl+Plus").map(|shortcut| shortcut.key),
            Some("+".into())
        );
        assert_eq!(
            Accelerator::parse("Esc").map(|shortcut| shortcut.key),
            Some("Escape".into())
        );
        assert_eq!(
            Accelerator::parse("Shift+Left").map(|shortcut| shortcut.key),
            Some("ArrowLeft".into())
        );
        let portable = Accelerator::parse("CmdOrCtrl+N").expect("valid");
        assert_eq!(portable.modifiers.meta, cfg!(target_os = "macos"));
        assert_eq!(portable.modifiers.control, !cfg!(target_os = "macos"));
        assert!(Accelerator::parse("Ctrl").is_none());
        assert!(Accelerator::parse("Ctrl+A+B").is_none());
        assert!(Accelerator::parse("Ctrl+F99").is_none());
        assert!(Accelerator::parse("").is_none());
    }

    #[test]
    fn menus_decode_items_separators_and_submenus() {
        let menu = parse_menu(
            r#"[{"label":"File","children":[
                {"id":"new","label":"New","shortcut":"CmdOrCtrl+N"},
                {"separator":true},
                {"id":"wrap","label":"Wrap","checked":true,"enabled":false},
                {"role":"quit","label":"Quit"}
            ]}]"#,
        )
        .expect("valid");
        let MenuNode::Submenu {
            label, children, ..
        } = &menu[0]
        else {
            panic!("submenu expected")
        };
        assert_eq!(label, "File");
        assert_eq!(children.len(), 4);
        assert!(
            matches!(&children[0], MenuNode::Item { id, accelerator: Some(_), .. } if id == "new")
        );
        assert_eq!(children[1], MenuNode::Separator);
        assert!(matches!(
            &children[2],
            MenuNode::Item {
                checked: Some(true),
                enabled: false,
                ..
            }
        ));
        assert!(matches!(
            &children[3],
            MenuNode::Item {
                role: Some(MenuRole::Quit),
                ..
            }
        ));
        // An item needs an id or a role.
        assert!(parse_menu(r#"[{"label":"Broken"}]"#).is_none());
        assert!(parse_menu("{}").is_none());
    }

    #[test]
    fn menu_depth_is_bounded() {
        let mut json = String::from(r#"{"id":"leaf","label":"Leaf"}"#);
        for _ in 0..8 {
            json = format!(r#"{{"label":"Level","children":[{json}]}}"#);
        }
        assert!(parse_menu(&format!("[{json}]")).is_none());
    }
}

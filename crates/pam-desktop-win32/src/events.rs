//! Translation of Win32 keyboard state into the core's vocabulary: W3C
//! `key` names, modifiers and virtual keys of accelerators.

use pam_desktop_ui::backend::Modifiers;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, GetKeyboardState, ToUnicode, VkKeyScanW, VIRTUAL_KEY, VK_APPS, VK_BACK, VK_BROWSER_BACK, VK_CAPITAL, VK_CONTROL, VK_DELETE,
    VK_DOWN, VK_END, VK_ESCAPE, VK_F1, VK_F24, VK_HOME, VK_INSERT, VK_LCONTROL, VK_LEFT, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU, VK_NEXT,
    VK_NUMLOCK, VK_OEM_2, VK_OEM_COMMA, VK_OEM_MINUS, VK_OEM_PERIOD, VK_OEM_PLUS, VK_PAUSE, VK_PRIOR, VK_RCONTROL, VK_RETURN, VK_RIGHT,
    VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SCROLL, VK_SHIFT, VK_SNAPSHOT, VK_SPACE, VK_TAB, VK_UP,
};

fn is_down(key: VIRTUAL_KEY) -> bool {
    // SAFETY: GetKeyState has no preconditions.
    unsafe { GetKeyState(i32::from(key.0)) < 0 }
}

/// Modifier keys held as of the message being handled.
pub fn modifiers() -> Modifiers {
    Modifiers { shift: is_down(VK_SHIFT), control: is_down(VK_CONTROL), alt: is_down(VK_MENU), meta: is_down(VK_LWIN) || is_down(VK_RWIN) }
}

fn named(key: VIRTUAL_KEY) -> Option<&'static str> {
    Some(match key {
        VK_RETURN => "Enter",
        VK_ESCAPE => "Escape",
        VK_TAB => "Tab",
        VK_BACK => "Backspace",
        VK_DELETE => "Delete",
        VK_INSERT => "Insert",
        VK_LEFT => "ArrowLeft",
        VK_RIGHT => "ArrowRight",
        VK_UP => "ArrowUp",
        VK_DOWN => "ArrowDown",
        VK_HOME => "Home",
        VK_END => "End",
        VK_PRIOR => "PageUp",
        VK_NEXT => "PageDown",
        VK_SPACE => " ",
        VK_SHIFT | VK_LSHIFT | VK_RSHIFT => "Shift",
        VK_CONTROL | VK_LCONTROL | VK_RCONTROL => "Control",
        VK_MENU | VK_LMENU | VK_RMENU => "Alt",
        VK_LWIN | VK_RWIN => "Meta",
        VK_CAPITAL => "CapsLock",
        VK_NUMLOCK => "NumLock",
        VK_SCROLL => "ScrollLock",
        VK_APPS => "ContextMenu",
        VK_SNAPSHOT => "PrintScreen",
        VK_PAUSE => "Pause",
        VK_BROWSER_BACK => "BrowserBack",
        _ => return None,
    })
}

/// Whether the key is a modifier by itself.
pub fn is_modifier(key: u32) -> bool {
    u16::try_from(key).is_ok_and(|key| matches!(named(VIRTUAL_KEY(key)), Some("Shift" | "Control" | "Alt" | "Meta")))
}

/// The text a key produces with `state`, without disturbing dead keys.
fn typed(key: u32, scan_code: u32, state: &[u8; 256]) -> Option<String> {
    let mut buffer = [0_u16; 8];
    // Bit 2 keeps the keyboard state (pending dead keys) untouched.
    // SAFETY: the state array and the buffer are live for the call.
    let count = unsafe { ToUnicode(key, scan_code, Some(state), &mut buffer, 0x4) };
    if count < 0 {
        return Some(String::from("Dead"));
    }
    let text = String::from_utf16_lossy(&buffer[..count.clamp(0, 8) as usize]);
    (!text.is_empty() && !text.chars().any(char::is_control)).then_some(text)
}

/// The W3C UI Events `key` value of a virtual key (`WM_KEYDOWN` wParam)
/// with the scan code from its lParam.
pub fn key_name(key: u32, scan_code: u32) -> String {
    let Ok(code) = u16::try_from(key) else { return String::from("Unidentified") };
    if let Some(name) = named(VIRTUAL_KEY(code)) {
        return name.to_owned();
    }
    if (VK_F1.0..=VK_F24.0).contains(&code) {
        return format!("F{}", code - VK_F1.0 + 1);
    }
    let mut state = [0_u8; 256];
    // SAFETY: the array has the 256 entries the function fills.
    if unsafe { GetKeyboardState(&mut state) }.is_err() {
        return String::from("Unidentified");
    }
    if let Some(text) = typed(key, scan_code, &state) {
        return text;
    }
    // Control and Alt chords produce control characters or nothing: name
    // the key by what it types without them (Ctrl+S is "s").
    for modifier in [VK_CONTROL, VK_LCONTROL, VK_RCONTROL, VK_MENU, VK_LMENU, VK_RMENU] {
        state[usize::from(modifier.0)] = 0;
    }
    typed(key, scan_code, &state).unwrap_or_else(|| String::from("Unidentified"))
}

/// The virtual key of an accelerator key name (a W3C `key` value, letters
/// lower-case), for accelerator tables and hot keys.
pub fn virtual_key(name: &str) -> Option<u16> {
    let fixed = match name {
        "Enter" => Some(VK_RETURN),
        "Escape" => Some(VK_ESCAPE),
        "Tab" => Some(VK_TAB),
        "Backspace" => Some(VK_BACK),
        "Delete" => Some(VK_DELETE),
        "Insert" => Some(VK_INSERT),
        "ArrowLeft" => Some(VK_LEFT),
        "ArrowRight" => Some(VK_RIGHT),
        "ArrowUp" => Some(VK_UP),
        "ArrowDown" => Some(VK_DOWN),
        "Home" => Some(VK_HOME),
        "End" => Some(VK_END),
        "PageUp" => Some(VK_PRIOR),
        "PageDown" => Some(VK_NEXT),
        " " => Some(VK_SPACE),
        "+" => Some(VK_OEM_PLUS),
        "-" => Some(VK_OEM_MINUS),
        "," => Some(VK_OEM_COMMA),
        "." => Some(VK_OEM_PERIOD),
        "/" => Some(VK_OEM_2),
        _ => None,
    };
    if let Some(key) = fixed {
        return Some(key.0);
    }
    if let Some(number) = name.strip_prefix('F').and_then(|rest| rest.parse::<u16>().ok()) {
        return (1..=24).contains(&number).then(|| VK_F1.0 + number - 1);
    }
    let mut characters = name.chars();
    let (Some(character), None) = (characters.next(), characters.next()) else { return None };
    if character.is_ascii_alphanumeric() {
        // Letters and digits are their own virtual keys.
        return Some(character.to_ascii_uppercase() as u16);
    }
    let mut units = [0_u16; 2];
    let encoded = character.encode_utf16(&mut units);
    if encoded.len() != 1 {
        return None;
    }
    // The key that types the character on the current layout.
    // SAFETY: VkKeyScanW has no preconditions.
    let scanned = unsafe { VkKeyScanW(encoded[0]) };
    (scanned != -1).then_some((scanned & 0xFF) as u16)
}

/// How an accelerator reads in a menu (`Ctrl+Shift+N`).
pub fn accelerator_label(accelerator: &pam_desktop_ui::shell::Accelerator) -> String {
    let mut label = String::new();
    if accelerator.modifiers.control {
        label.push_str("Ctrl+");
    }
    if accelerator.modifiers.alt {
        label.push_str("Alt+");
    }
    if accelerator.modifiers.shift {
        label.push_str("Shift+");
    }
    if accelerator.modifiers.meta {
        label.push_str("Win+");
    }
    match accelerator.key.as_str() {
        " " => label.push_str("Space"),
        "ArrowLeft" => label.push_str("Left"),
        "ArrowRight" => label.push_str("Right"),
        "ArrowUp" => label.push_str("Up"),
        "ArrowDown" => label.push_str("Down"),
        "Escape" => label.push_str("Esc"),
        "Delete" => label.push_str("Del"),
        key if key.chars().count() == 1 => label.extend(key.chars().flat_map(char::to_uppercase)),
        key => label.push_str(key),
    }
    label
}

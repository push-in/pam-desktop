use pam_desktop_protocol::{Bootstrap, Effect, ShortcutState};

use crate::native::DialogRequest;

#[derive(Clone, Debug)]
pub enum ShellEvent {
    MenuSelected(String),
    TrayActivated {
        button: u8,
    },
    Shortcut {
        native_id: u32,
        state: ShortcutState,
    },
}

#[derive(Debug)]
pub enum HostEvent {
    ServoWake,
    ApplyEffects(Vec<Effect>),
    ReloadViews,
    Reconfigure(Box<Bootstrap>),
    Dialog(DialogRequest),
    Shell(ShellEvent),
    Exit,
}

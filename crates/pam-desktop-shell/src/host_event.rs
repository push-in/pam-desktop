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
    #[cfg(feature = "servo-engine")]
    Accessibility(accesskit_winit::Event),
    ApplyEffects(Vec<Effect>),
    ReloadViews,
    Reconfigure(Box<Bootstrap>),
    Dialog(DialogRequest),
    Shell(ShellEvent),
    Exit,
}

#[cfg(feature = "servo-engine")]
impl From<accesskit_winit::Event> for HostEvent {
    fn from(event: accesskit_winit::Event) -> Self {
        Self::Accessibility(event)
    }
}

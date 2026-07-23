use pam_desktop_protocol::{Bootstrap, Effect};

use crate::native::DialogRequest;

#[derive(Debug)]
pub enum HostEvent {
    ServoWake,
    ApplyEffects(Vec<Effect>),
    ReloadViews,
    Reconfigure(Bootstrap),
    Dialog(DialogRequest),
}

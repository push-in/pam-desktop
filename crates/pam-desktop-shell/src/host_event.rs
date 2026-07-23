use pam_desktop_protocol::{Bootstrap, Effect};

#[derive(Clone, Debug)]
pub enum HostEvent {
    ServoWake,
    ApplyEffects(Vec<Effect>),
    ReloadViews,
    Reconfigure(Bootstrap),
}

use pam_desktop_protocol::Effect;

#[derive(Clone, Debug)]
pub enum HostEvent {
    ServoWake,
    ApplyEffects(Vec<Effect>),
}

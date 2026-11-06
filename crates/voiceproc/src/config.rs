#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Suppression {
    Off,
    Light,
    Strong,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessingConfig {
    pub echo_cancellation: bool,
    pub noise_suppression: Suppression,
    pub automatic_gain: bool,
}

impl ProcessingConfig {
    pub const OFF: Self = Self {
        echo_cancellation: false,
        noise_suppression: Suppression::Off,
        automatic_gain: false,
    };

    pub fn is_off(self) -> bool {
        self == Self::OFF
    }
}

// Wired into Room by the voice controller; drop this once it is.
#![allow(dead_code, unused_imports)]

mod rules;
mod send;

pub(crate) use rules::{SelfVoice, desired_listeners, listens_to};
pub(crate) use send::{VoiceSend, silence_packet};

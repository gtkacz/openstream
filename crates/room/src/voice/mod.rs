// Wired into Room by the voice controller; drop this once it is.
#![allow(dead_code, unused_imports)]

mod rules;

pub(crate) use rules::{SelfVoice, desired_listeners, listens_to};

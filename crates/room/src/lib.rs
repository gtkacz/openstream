//! A room: membership over signed gossip presence, published lives with lazy encoders, and watches.

pub mod codecs;
pub mod error;
pub mod membership;

pub use error::RoomError;

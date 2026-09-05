//! A room: membership over signed gossip presence, published lives with lazy encoders, and watches.

pub mod codecs;
pub mod error;
pub mod membership;
pub mod registry;
pub mod snapshot;

pub use error::RoomError;
pub use registry::ChangeNotify;

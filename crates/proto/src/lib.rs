//! Wire types shared by every brp crate. No I/O lives here.
pub mod bitrate;
pub mod clock;
pub mod constants;
pub mod error;
pub mod frame;
pub mod messages;
pub mod pixel;
pub mod presence;
pub mod templates;
pub mod ticket;
pub use bitrate::default_bitrate_kbps;
pub use clock::monotonic_us;
pub use error::ProtoError;
pub use frame::{EncodedFrame, FrameHeader, FrameKind};
pub use messages::{
    AudioParams, Codec, CodecParams, Preset, PresetError, PublisherMessage, SourceKind,
    ViewerMessage, decode, encode,
};
pub use pixel::PixelFormat;
pub use presence::{LiveInfo, Presence, Signed};
pub use templates::template_presets;
pub use ticket::RoomTicket;

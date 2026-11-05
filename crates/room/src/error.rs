use iroh::EndpointId;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RoomError {
    #[error(transparent)]
    Net(#[from] brp_net::NetError),
    #[error(transparent)]
    Capture(#[from] brp_capture::CaptureError),
    #[error(transparent)]
    Codec(#[from] brp_codec::CodecError),
    #[error(transparent)]
    Audio(#[from] brp_audio::AudioError),
    #[error(transparent)]
    Proto(#[from] brp_proto::ProtoError),
    #[error("Could not reach the room. Check your connection and try again. ({0})")]
    Gossip(String),
    #[error(
        "No one in the room answered. Check your connection or ask for a new invite link, then try again."
    )]
    JoinTimeout,
    #[error("That person is no longer in the room")]
    UnknownMember(EndpointId),
    #[error("That stream is no longer available")]
    UnknownLive(u32),
    #[error("That quality is no longer available. Choose another.")]
    UnknownPreset(u32),
    #[error("You are sharing as many streams as a room allows. Stop one to share another.")]
    TooManyLives,
    #[error("You are not watching that stream")]
    NotWatching,
}

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
    Proto(#[from] brp_proto::ProtoError),
    #[error("gossip failed: {0}")]
    Gossip(String),
    #[error("no room member answered within the join timeout")]
    JoinTimeout,
    #[error("{0} is not a member of the room")]
    UnknownMember(EndpointId),
    #[error("unknown live {0}")]
    UnknownLive(u32),
    #[error("unknown preset {0}")]
    UnknownPreset(u32),
    #[error("the room limit of lives per participant is reached")]
    TooManyLives,
    #[error("not watching that live")]
    NotWatching,
}

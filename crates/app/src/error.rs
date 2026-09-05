use thiserror::Error;
#[derive(Debug, Error)]
pub enum AppError {
    #[error(transparent)]
    Capture(#[from] brp_capture::CaptureError),
    #[error(transparent)]
    Codec(#[from] brp_codec::CodecError),
    #[error(transparent)]
    Net(#[from] brp_net::NetError),
    #[error(transparent)]
    Room(#[from] brp_room::RoomError),
    #[error("preset rejected: {0}")]
    Preset(#[from] brp_proto::PresetError),
    #[error("invalid ticket: {0}")]
    Ticket(#[from] iroh_tickets::ParseError),
    #[error("the ticket lists no bootstrap peer")]
    EmptyTicket,
    #[error("identity file: {0}")]
    Identity(String),
    #[error("window system: {0}")]
    Window(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

use thiserror::Error;
#[derive(Debug, Error)]
pub enum AppError {
    #[error(transparent)]
    Capture(#[from] brp_capture::CaptureError),
    #[error(transparent)]
    Codec(#[from] brp_codec::CodecError),
    #[error(transparent)]
    Room(#[from] brp_room::RoomError),
    #[error("preset rejected: {0}")]
    Preset(#[from] brp_proto::PresetError),
    #[error("That link or ticket is not valid. Ask someone in the room for a new one. ({0})")]
    Ticket(#[from] iroh_tickets::ParseError),
    #[error("{0}")]
    Identity(String),
    #[error(
        "Could not open a window. Check your display and graphics driver, then try again. ({0})"
    )]
    Window(String),
    #[error("{0}")]
    Settings(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

use thiserror::Error;
#[derive(Debug, Error)]
pub enum NetError {
    #[error("failed to bind endpoint: {0}")]
    Bind(String),
    #[error("failed to connect: {0}")]
    Connect(String),
    #[error("connection failed: {0}")]
    Connection(String),
    #[error("stream failed: {0}")]
    Stream(String),
    #[error("peer violated the protocol: {0}")]
    Protocol(&'static str),
    #[error("subscription rejected by publisher: {0}")]
    Rejected(String),
    #[error(transparent)]
    Proto(#[from] brp_proto::ProtoError),
}
impl NetError {
    pub(crate) fn connection(e: impl std::fmt::Display) -> Self {
        Self::Connection(e.to_string())
    }

    pub(crate) fn stream(e: impl std::fmt::Display) -> Self {
        Self::Stream(e.to_string())
    }
}

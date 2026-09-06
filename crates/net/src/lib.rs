//! iroh transport: endpoint setup, control framing, and media source contracts.

pub mod client;
pub mod endpoint;
pub mod error;
pub mod framing;
pub mod policy;
pub mod server;
pub mod source;

pub use client::{AudioStream, MediaClient, PathKind, ViewerSubscription};
pub use endpoint::{RelaySetting, bind_endpoint};
pub use error::NetError;
pub use policy::{AllowAll, ConnectionPolicy};
pub use server::MediaServer;
pub use source::{AudioSubscription, LiveSource, ReceivedFrame, SubscribeRejected, Subscription};

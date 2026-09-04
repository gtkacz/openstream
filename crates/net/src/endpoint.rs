use crate::NetError;
use brp_proto::constants::MEDIA_ALPN;
use iroh::endpoint::presets;
use iroh::{Endpoint, RelayMode, SecretKey};
/// Controls whether the endpoint uses iroh's relay infrastructure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelaySetting {
    /// Use the library's public relays for hole punching and fallback.
    Default,
    /// Disable relays for LAN and directly reachable peers.
    Disabled,
}

/// Binds an iroh endpoint for the media protocol.
pub async fn bind_endpoint(secret: SecretKey, relay: RelaySetting) -> Result<Endpoint, NetError> {
    let builder = match relay {
        RelaySetting::Default => Endpoint::builder(presets::N0),
        RelaySetting::Disabled => {
            Endpoint::builder(presets::Minimal).relay_mode(RelayMode::Disabled)
        }
    };
    builder
        .secret_key(secret)
        .alpns(vec![MEDIA_ALPN.to_vec()])
        .bind()
        .await
        .map_err(|e| NetError::Bind(e.to_string()))
}

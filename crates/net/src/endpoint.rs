use crate::NetError;
use brp_proto::constants::MEDIA_ALPN;
use iroh::endpoint::presets;
use iroh::{Endpoint, RelayMode, SecretKey};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelaySetting {
    Default,
    Disabled,
}
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

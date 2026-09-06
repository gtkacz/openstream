use crate::NetError;
use brp_proto::constants::MEDIA_ALPN;
use iroh::address_lookup::memory::MemoryLookup;
use iroh::endpoint::presets;
use iroh::{Endpoint, EndpointAddr, RelayMap, RelayMode, RelayUrl, SecretKey};

/// Controls which relay infrastructure the endpoint uses for hole punching and fallback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelaySetting {
    /// The library's public relays.
    Default,
    /// One self-hosted relay instead of the public ones.
    Custom(RelayUrl),
    /// No relays: LAN and directly reachable peers only.
    Disabled,
}

/// Binds an iroh endpoint for the media protocol.
///
/// `known_peers` are addresses the caller already holds, typically a ticket's bootstrap list.
/// iroh 1.1 has no way to add an address to a bound endpoint, so they go in at build time.
pub async fn bind_endpoint(
    secret: SecretKey,
    relay: RelaySetting,
    known_peers: Vec<EndpointAddr>,
) -> Result<Endpoint, NetError> {
    let lookup = MemoryLookup::new();
    for peer in known_peers {
        lookup.add_endpoint_info(peer);
    }
    let builder = match relay {
        RelaySetting::Default => Endpoint::builder(presets::N0),
        RelaySetting::Custom(url) => {
            Endpoint::builder(presets::N0).relay_mode(RelayMode::Custom(RelayMap::from(url)))
        }
        RelaySetting::Disabled => {
            Endpoint::builder(presets::Minimal).relay_mode(RelayMode::Disabled)
        }
    };
    builder
        .address_lookup(lookup)
        .secret_key(secret)
        .alpns(vec![MEDIA_ALPN.to_vec()])
        .bind()
        .await
        .map_err(|e| NetError::Bind(e.to_string()))
}

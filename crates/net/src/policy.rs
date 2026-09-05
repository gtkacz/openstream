use iroh::EndpointId;

/// Decides which peers may open media connections. The room answers with its membership set.
pub trait ConnectionPolicy: Send + Sync + 'static {
    fn allows(&self, peer: EndpointId) -> bool;
}

impl<F> ConnectionPolicy for F
where
    F: Fn(EndpointId) -> bool + Send + Sync + 'static,
{
    fn allows(&self, peer: EndpointId) -> bool {
        self(peer)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct AllowAll;

impl ConnectionPolicy for AllowAll {
    fn allows(&self, _peer: EndpointId) -> bool {
        true
    }
}

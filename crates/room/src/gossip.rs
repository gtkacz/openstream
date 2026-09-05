//! Joins the room topic and keeps membership current with signed presence.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use brp_proto::constants::PROTOCOL_VERSION;
use brp_proto::{Presence, Signed, decode, encode};
use bytes::Bytes;
use iroh::{EndpointId, PublicKey, SecretKey};
use iroh_gossip::api::{Event, GossipReceiver, GossipSender};
use iroh_gossip::net::Gossip;
use iroh_gossip::proto::TopicId;
use n0_future::StreamExt;
use tokio::sync::{Notify, mpsc};

use crate::error::RoomError;
use crate::membership::{Applied, Membership};
use crate::registry::{ChangeNotify, LiveRegistry};

pub(crate) async fn join(
    gossip: &Gossip,
    topic: TopicId,
    bootstrap: Vec<EndpointId>,
    timeout: Duration,
) -> Result<(GossipSender, GossipReceiver), RoomError> {
    let mut topic = gossip
        .subscribe(topic, bootstrap.clone())
        .await
        .map_err(|e| RoomError::Gossip(e.to_string()))?;
    // A creator has nobody to join; everyone else must reach a neighbour or the ticket is dead.
    if !bootstrap.is_empty() {
        tokio::time::timeout(timeout, topic.joined())
            .await
            .map_err(|_| RoomError::JoinTimeout)?
            .map_err(|e| RoomError::Gossip(e.to_string()))?;
    }
    Ok(topic.split())
}

pub(crate) struct PresenceLoop {
    pub secret: SecretKey,
    pub nickname: String,
    pub sender: GossipSender,
    pub receiver: GossipReceiver,
    pub membership: Arc<Mutex<Membership>>,
    pub registry: Arc<LiveRegistry>,
    pub dirty: Arc<Notify>,
    pub heartbeat: Duration,
    pub on_change: ChangeNotify,
    pub expired: mpsc::Sender<PublicKey>,
}

impl PresenceLoop {
    pub async fn run(mut self) {
        let me = self.secret.public();
        // The first tick fires immediately, which doubles as the join announcement.
        let mut heartbeat = tokio::time::interval(self.heartbeat);
        loop {
            tokio::select! {
                _ = heartbeat.tick() => {
                    self.broadcast().await;
                    self.expire().await;
                }
                _ = self.dirty.notified() => self.broadcast().await,
                event = self.receiver.next() => match event {
                    Some(Ok(Event::Received(message))) => self.receive(me, &message.content),
                    Some(Ok(Event::Lagged)) => tracing::warn!("gossip lagged; the next heartbeat repairs the catalog"),
                    Some(Ok(other)) => tracing::debug!(?other, "gossip neighbour event"),
                    Some(Err(error)) => {
                        tracing::error!(%error, "gossip receiver failed");
                        break;
                    }
                    None => break,
                },
            }
        }
    }

    async fn broadcast(&self) {
        let presence = Presence {
            version: PROTOCOL_VERSION,
            ts_unix_ms: unix_ms(),
            nickname: self.nickname.clone(),
            lives: self.registry.live_infos(),
        };
        let bytes = match Signed::sign(&self.secret, &presence).and_then(|signed| encode(&signed)) {
            Ok(bytes) => bytes,
            Err(error) => {
                tracing::error!(%error, "could not sign presence");
                return;
            }
        };
        if let Err(error) = self.sender.broadcast(Bytes::from(bytes)).await {
            tracing::warn!(%error, "presence broadcast failed");
        }
    }

    async fn expire(&self) {
        let expired = lock(&self.membership).expire(Instant::now());
        if expired.is_empty() {
            return;
        }
        (self.on_change)();
        for id in expired {
            let _ = self.expired.send(id).await;
        }
    }

    fn receive(&self, me: PublicKey, content: &[u8]) {
        let signed: Signed = match decode(content) {
            Ok(signed) => signed,
            Err(error) => {
                tracing::debug!(%error, "dropping undecodable gossip message");
                return;
            }
        };
        let presence: Presence = match signed.verify() {
            Ok(presence) => presence,
            Err(error) => {
                tracing::debug!(author = %signed.author.fmt_short(), %error, "dropping presence");
                return;
            }
        };
        if let Err(error) = presence.validate() {
            tracing::debug!(author = %signed.author.fmt_short(), %error, "dropping invalid presence");
            return;
        }
        if signed.author == me {
            return;
        }
        match lock(&self.membership).apply(signed.author, presence, Instant::now()) {
            Applied::Inserted | Applied::Updated => (self.on_change)(),
            Applied::Refreshed | Applied::Stale => {}
        }
    }
}

pub(crate) fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

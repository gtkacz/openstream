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
use crate::membership::{Applied, Member, Membership};
use crate::registry::{ChangeNotify, LiveRegistry};

pub(crate) async fn join(
    gossip: &Gossip,
    topic: TopicId,
    bootstrap: Vec<EndpointId>,
    timeout: Duration,
    attempts: u32,
) -> Result<(GossipSender, GossipReceiver), RoomError> {
    // A creator has nobody to join; everyone else must reach a neighbour or the ticket is dead.
    if bootstrap.is_empty() {
        let topic = gossip
            .subscribe(topic, Vec::new())
            .await
            .map_err(|e| RoomError::Gossip(e.to_string()))?;
        return Ok(topic.split());
    }
    // At least one attempt always runs: a zero- or underflow-configured attempt count must not
    // reject a live ticket without a single dial.
    for _ in 0..attempts.max(1) {
        let mut topic = gossip
            .subscribe(topic, bootstrap.clone())
            .await
            .map_err(|e| RoomError::Gossip(e.to_string()))?;
        match tokio::time::timeout(timeout, topic.joined()).await {
            Ok(Ok(())) => return Ok(topic.split()),
            Ok(Err(error)) => tracing::debug!(%error, "gossip join attempt failed"),
            Err(_) => tracing::debug!("gossip join attempt timed out"),
        }
        // Dropping the topic handle makes the actor leave the topic, so the next attempt
        // re-dials the bootstrap list from scratch.
    }
    Err(RoomError::JoinTimeout)
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
    /// Publishers whose presence just changed its audio flag. A publisher that stops sharing goes
    /// quiet rather than closing anything, so presence is the viewer's only notice either way.
    pub audio_changed: mpsc::Sender<PublicKey>,
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
                    Some(Ok(Event::Received(message))) => self.receive(me, &message.content).await,
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
        let mut presence = Presence {
            version: PROTOCOL_VERSION,
            ts_unix_ms: unix_ms(),
            nickname: self.nickname.clone(),
            lives: self.registry.live_infos(),
            audio_mix: Some(self.registry.audio_mix()),
            quality_presets: Vec::new(),
        };
        presence.hide_quality_presets();
        let bytes = match Signed::sign(&self.secret, &presence).and_then(|signed| encode(&signed)) {
            Ok(bytes) => bytes,
            Err(error) => {
                tracing::error!(%error, "could not sign presence");
                return;
            }
        };
        tracing::debug!(
            lives = presence.lives.len(),
            bytes = bytes.len(),
            "presence broadcast"
        );
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

    async fn receive(&self, me: PublicKey, content: &[u8]) {
        let signed: Signed = match decode(content) {
            Ok(signed) => signed,
            Err(error) => {
                tracing::debug!(%error, "dropping undecodable gossip message");
                return;
            }
        };
        let mut presence: Presence = match signed.verify() {
            Ok(presence) => presence,
            Err(error) => {
                tracing::debug!(author = %signed.author.fmt_short(), %error, "dropping presence");
                return;
            }
        };
        presence.restore_quality_presets();
        if let Err(error) = presence.validate() {
            tracing::debug!(author = %signed.author.fmt_short(), %error, "dropping invalid presence");
            return;
        }
        if signed.author == me {
            return;
        }
        let (applied, audio_changed) = {
            let mut membership = lock(&self.membership);
            let before = membership
                .get(&signed.author)
                .is_some_and(Member::has_audio);
            let applied = membership.apply(signed.author, presence, Instant::now());
            let after = membership
                .get(&signed.author)
                .is_some_and(Member::has_audio);
            (applied, after != before)
        };
        match applied {
            Applied::Inserted | Applied::Updated => {
                tracing::debug!(
                    author = %signed.author.fmt_short(),
                    ?applied,
                    lives = lock(&self.membership)
                        .get(&signed.author)
                        .map_or(0, |m| m.presence.lives.len()),
                    "presence applied"
                );
                (self.on_change)()
            }
            Applied::Refreshed | Applied::Stale => {}
        }
        if audio_changed {
            let _ = self.audio_changed.send(signed.author).await;
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use brp_net::RelaySetting;
    use iroh::protocol::Router;
    use iroh::{EndpointAddr, SecretKey};
    use iroh_gossip::net::Gossip;
    use iroh_gossip::proto::TopicId;

    use super::*;

    /// A bound endpoint whose router answers gossip, like a room's endpoint before its
    /// `gossip::join` has subscribed. `known` seeds the address lookup, the same way a
    /// ticket's bootstrap list does.
    async fn peer(known: Vec<EndpointAddr>) -> (iroh::Endpoint, Gossip, Router) {
        let endpoint = brp_net::bind_endpoint(SecretKey::generate(), RelaySetting::Disabled, known)
            .await
            .unwrap();
        let gossip = Gossip::builder().spawn(endpoint.clone());
        let router = Router::builder(endpoint.clone())
            .accept(iroh_gossip::ALPN, gossip.clone())
            .spawn();
        (endpoint, gossip, router)
    }

    #[tokio::test]
    async fn a_join_retries_until_the_bootstrap_answers() {
        let (a_endpoint, a_gossip, _a_router) = peer(Vec::new()).await;
        // b can reach a only through the address seeded at bind time.
        let (_b_endpoint, b_gossip, _b_router) = peer(vec![a_endpoint.addr()]).await;
        let topic = TopicId::from_bytes([7u8; 32]);
        let bootstrap = vec![a_endpoint.addr().id];

        let joined = tokio::spawn(async move {
            join(&b_gossip, topic, bootstrap, Duration::from_millis(300), 4).await
        });
        // Attempts one and two fail while a has not subscribed to the topic.
        tokio::time::sleep(Duration::from_millis(700)).await;
        let _a = join(&a_gossip, topic, Vec::new(), Duration::from_secs(5), 1)
            .await
            .unwrap();

        let (sender, _receiver) = joined.await.unwrap().unwrap();
        drop(sender);
    }

    #[tokio::test]
    async fn even_zero_configured_attempts_makes_one() {
        let (a_endpoint, a_gossip, _a_router) = peer(Vec::new()).await;
        let (_b_endpoint, b_gossip, _b_router) = peer(vec![a_endpoint.addr()]).await;
        let topic = TopicId::from_bytes([7u8; 32]);
        let _a = join(&a_gossip, topic, Vec::new(), Duration::from_secs(5), 1)
            .await
            .unwrap();
        // Zero configured attempts still dials once, instead of rejecting a reachable
        // bootstrap sight unseen.
        let (sender, _receiver) = join(
            &b_gossip,
            topic,
            vec![a_endpoint.addr().id],
            Duration::from_secs(5),
            0,
        )
        .await
        .unwrap();
        drop(sender);
    }
}

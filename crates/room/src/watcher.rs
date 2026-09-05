//! Remote lives this participant watches: one media connection per publisher, one decode pipeline
//! per watch, reconnection with backoff while the publisher stays a member.

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use brp_codec::RawFrame;
use brp_net::{MediaClient, NetError, PathKind};
use brp_pipeline::{FrameNotify, LatestSlot, Viewer, ViewerSink, ViewerStats};
use brp_proto::constants::{
    RESUBSCRIBE_BACKOFF_INITIAL, RESUBSCRIBE_BACKOFF_MAX, SOURCE_PRESET_ID,
};
use brp_proto::{PublisherMessage, ViewerMessage};
use iroh::{Endpoint, EndpointAddr, PublicKey};
use tokio::runtime::Handle;
use tokio::sync::oneshot;

use crate::codecs::DecoderFactory;
use crate::error::RoomError;
use crate::gossip::lock;
use crate::membership::Membership;
use crate::registry::ChangeNotify;
use crate::snapshot::{WatchState, WatchView};

/// What a tile renders from. Stable across reconnects of the same watch.
#[derive(Clone)]
pub struct WatchHandle {
    pub slot: Arc<LatestSlot<RawFrame>>,
    pub stats: Arc<ViewerStats>,
}

type WatchKey = (PublicKey, u32);

/// Identifies one watch task. The generation tells a task that was replaced by a preset switch
/// apart from its successor under the same key, so its last state writes are ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WatchTask {
    key: WatchKey,
    generation: u64,
}

struct WatchEntry {
    generation: u64,
    preset_id: u32,
    state: WatchState,
    handle: WatchHandle,
    /// Dropping this ends the watch task. Replacing a watch drops the old one, which is how a
    /// preset switch is an unsubscribe followed by a subscribe.
    _cancel: oneshot::Sender<()>,
}

#[derive(Default)]
struct Inner {
    clients: HashMap<PublicKey, Arc<MediaClient>>,
    watches: HashMap<WatchKey, WatchEntry>,
    next_generation: u64,
}

pub struct Watcher {
    endpoint: Endpoint,
    runtime: Handle,
    decoders: Arc<dyn DecoderFactory>,
    membership: Arc<Mutex<Membership>>,
    on_change: ChangeNotify,
    on_frame: FrameNotify,
    inner: Mutex<Inner>,
}

enum Outcome {
    Cancelled,
    Ended,
    Lost,
}

impl Watcher {
    pub fn new(
        endpoint: Endpoint,
        runtime: Handle,
        decoders: Arc<dyn DecoderFactory>,
        membership: Arc<Mutex<Membership>>,
        on_change: ChangeNotify,
        on_frame: FrameNotify,
    ) -> Arc<Self> {
        Arc::new(Self {
            endpoint,
            runtime,
            decoders,
            membership,
            on_change,
            on_frame,
            inner: Mutex::default(),
        })
    }

    pub fn watch(
        self: &Arc<Self>,
        publisher: PublicKey,
        live_id: u32,
        preset_id: u32,
    ) -> Result<WatchHandle, RoomError> {
        if !lock(&self.membership).is_member(&publisher) {
            return Err(RoomError::UnknownMember(publisher));
        }
        let handle = WatchHandle {
            slot: LatestSlot::new(),
            stats: Arc::new(ViewerStats::default()),
        };
        let (cancel_tx, cancel_rx) = oneshot::channel();
        let task = {
            let mut inner = lock(&self.inner);
            let generation = inner.next_generation;
            inner.next_generation += 1;
            inner.watches.insert(
                (publisher, live_id),
                WatchEntry {
                    generation,
                    preset_id,
                    state: WatchState::Connecting,
                    handle: handle.clone(),
                    _cancel: cancel_tx,
                },
            );
            WatchTask {
                key: (publisher, live_id),
                generation,
            }
        };
        tokio::spawn(
            self.clone()
                .run_watch(task, preset_id, handle.clone(), cancel_rx),
        );
        (self.on_change)();
        Ok(handle)
    }

    pub fn unwatch(&self, publisher: PublicKey, live_id: u32) -> Result<(), RoomError> {
        lock(&self.inner)
            .watches
            .remove(&(publisher, live_id))
            .ok_or(RoomError::NotWatching)?;
        (self.on_change)();
        Ok(())
    }

    pub fn member_left(&self, id: PublicKey) {
        let mut inner = lock(&self.inner);
        inner.watches.retain(|(publisher, _), _| *publisher != id);
        // Close explicitly so the peer sees an ended connection instead of waiting out an idle timeout.
        if let Some(client) = inner.clients.remove(&id) {
            client.close();
        }
        drop(inner);
        (self.on_change)();
    }

    pub fn path_kind(&self, publisher: &PublicKey) -> PathKind {
        lock(&self.inner)
            .clients
            .get(publisher)
            .map(|c| c.path_kind())
            .unwrap_or(PathKind::Unknown)
    }

    pub fn views(&self) -> Vec<WatchView> {
        lock(&self.inner)
            .watches
            .iter()
            .map(|((publisher, live_id), entry)| WatchView {
                publisher: *publisher,
                live_id: *live_id,
                preset_id: entry.preset_id,
                state: entry.state,
                frames_decoded: entry.handle.stats.frames_decoded.load(Ordering::Relaxed),
                keyframe_requests: entry.handle.stats.keyframe_requests.load(Ordering::Relaxed),
            })
            .collect()
    }

    pub fn stop_all(&self) {
        let mut inner = lock(&self.inner);
        inner.watches.clear();
        for client in inner.clients.drain().map(|(_, client)| client) {
            client.close();
        }
    }

    /// Returns false when the watch was removed or replaced meanwhile, which tells the task to stop.
    fn set_state(&self, task: WatchTask, preset_id: u32, state: WatchState) -> bool {
        let applied = apply_state(&mut lock(&self.inner), task, preset_id, state);
        if applied {
            (self.on_change)();
        }
        applied
    }

    async fn client_for(&self, publisher: PublicKey) -> Result<Arc<MediaClient>, RoomError> {
        if let Some(client) = lock(&self.inner).clients.get(&publisher).cloned() {
            return Ok(client);
        }
        // Address resolution goes through the endpoint's lookups: the ticket's bootstrap list and
        // the addresses gossip learned while joining.
        let client =
            Arc::new(MediaClient::connect(&self.endpoint, EndpointAddr::from(publisher)).await?);
        lock(&self.inner).clients.insert(publisher, client.clone());
        Ok(client)
    }

    fn forget_client(&self, publisher: PublicKey) {
        if let Some(client) = lock(&self.inner).clients.remove(&publisher) {
            client.close();
        }
    }

    /// Spec 6.6: a watched preset the publisher removed falls back to Source while the live remains.
    fn fallback_preset(&self, publisher: PublicKey, live_id: u32, preset_id: u32) -> Option<u32> {
        let membership = lock(&self.membership);
        let live = membership
            .get(&publisher)?
            .presence
            .lives
            .iter()
            .find(|l| l.id == live_id)?;
        let still_offered = live.presets.iter().any(|p| p.id == preset_id);
        if !still_offered && preset_id != SOURCE_PRESET_ID {
            Some(SOURCE_PRESET_ID)
        } else {
            None
        }
    }

    fn live_exists(&self, publisher: PublicKey, live_id: u32) -> bool {
        lock(&self.membership)
            .get(&publisher)
            .is_some_and(|m| m.presence.lives.iter().any(|l| l.id == live_id))
    }

    async fn run_watch(
        self: Arc<Self>,
        task: WatchTask,
        mut preset_id: u32,
        handle: WatchHandle,
        mut cancel: oneshot::Receiver<()>,
    ) {
        let (publisher, live_id) = task.key;
        let mut backoff = RESUBSCRIBE_BACKOFF_INITIAL;
        loop {
            if !lock(&self.membership).is_member(&publisher) {
                self.set_state(task, preset_id, WatchState::Ended);
                return;
            }
            let attempt = async {
                let client = self.client_for(publisher).await?;
                client
                    .subscribe(live_id, preset_id)
                    .await
                    .map_err(RoomError::from)
            };
            let subscription = tokio::select! {
                _ = &mut cancel => return,
                result = attempt => result,
            };
            let subscription = match subscription {
                Ok(subscription) => subscription,
                Err(RoomError::Net(NetError::Rejected(reason))) => {
                    tracing::info!(%reason, live_id, preset_id, "subscription rejected");
                    if let Some(fallback) = self.fallback_preset(publisher, live_id, preset_id) {
                        preset_id = fallback;
                    } else if !self.live_exists(publisher, live_id) {
                        self.set_state(task, preset_id, WatchState::Ended);
                        return;
                    }
                    if !self
                        .wait_before_retry(task, preset_id, &mut backoff, &mut cancel)
                        .await
                    {
                        return;
                    }
                    continue;
                }
                Err(error) => {
                    tracing::debug!(%error, "watch attempt failed");
                    self.forget_client(publisher);
                    if !self
                        .wait_before_retry(task, preset_id, &mut backoff, &mut cancel)
                        .await
                    {
                        return;
                    }
                    continue;
                }
            };
            backoff = RESUBSCRIBE_BACKOFF_INITIAL;

            let decoder = match self.decoders.open(&subscription.params) {
                Ok(decoder) => decoder,
                Err(error) => {
                    tracing::error!(%error, "no decoder for this live");
                    let _ = subscription.control.send(ViewerMessage::Unsubscribe).await;
                    self.set_state(task, preset_id, WatchState::Ended);
                    return;
                }
            };
            let sink = ViewerSink {
                slot: handle.slot.clone(),
                stats: handle.stats.clone(),
                notify: self.on_frame.clone(),
            };
            let viewer = Viewer::start(
                self.runtime.clone(),
                subscription.frames,
                subscription.control.clone(),
                decoder,
                sink,
            );
            if !self.set_state(task, preset_id, WatchState::Live) {
                stop_viewer(viewer).await;
                let _ = subscription.control.send(ViewerMessage::Unsubscribe).await;
                return;
            }

            let mut events = subscription.events;
            let outcome = loop {
                tokio::select! {
                    _ = &mut cancel => break Outcome::Cancelled,
                    event = events.recv() => match event {
                        Some(PublisherMessage::LiveEnded) => break Outcome::Ended,
                        Some(_) => continue,
                        None => break Outcome::Lost,
                    },
                }
            };
            stop_viewer(viewer).await;
            match outcome {
                Outcome::Cancelled => {
                    let _ = subscription.control.send(ViewerMessage::Unsubscribe).await;
                    return;
                }
                Outcome::Ended => {
                    // The publisher removed or restarted this preset. Presence may lag behind, in which
                    // case the retry is rejected and handled above.
                    if let Some(fallback) = self.fallback_preset(publisher, live_id, preset_id) {
                        preset_id = fallback;
                    } else if !self.live_exists(publisher, live_id) {
                        self.set_state(task, preset_id, WatchState::Ended);
                        return;
                    }
                }
                Outcome::Lost => self.forget_client(publisher),
            }
            if !self
                .wait_before_retry(task, preset_id, &mut backoff, &mut cancel)
                .await
            {
                return;
            }
        }
    }

    /// Marks the watch reconnecting and sleeps the current backoff. False means the watch is gone.
    async fn wait_before_retry(
        &self,
        task: WatchTask,
        preset_id: u32,
        backoff: &mut std::time::Duration,
        cancel: &mut oneshot::Receiver<()>,
    ) -> bool {
        if !self.set_state(task, preset_id, WatchState::Reconnecting) {
            return false;
        }
        tokio::select! {
            _ = cancel => return false,
            _ = tokio::time::sleep(*backoff) => {}
        }
        *backoff = (*backoff * 2).min(RESUBSCRIBE_BACKOFF_MAX);
        true
    }
}

/// Writes a task's state onto its entry. A task whose entry was removed, or replaced by a newer
/// generation after a preset switch, gets false and must stop instead of clobbering its successor.
fn apply_state(inner: &mut Inner, task: WatchTask, preset_id: u32, state: WatchState) -> bool {
    let Some(entry) = inner.watches.get_mut(&task.key) else {
        return false;
    };
    if entry.generation != task.generation {
        return false;
    }
    entry.state = state;
    entry.preset_id = preset_id;
    true
}

/// `Viewer::stop` joins the decode thread, so it runs off the async executor.
async fn stop_viewer(viewer: Viewer) {
    let _ = tokio::task::spawn_blocking(move || viewer.stop()).await;
}

#[cfg(test)]
mod tests {
    use iroh::SecretKey;

    use super::*;

    fn entry(generation: u64, preset_id: u32) -> WatchEntry {
        WatchEntry {
            generation,
            preset_id,
            state: WatchState::Connecting,
            handle: WatchHandle {
                slot: LatestSlot::new(),
                stats: Arc::new(ViewerStats::default()),
            },
            _cancel: oneshot::channel().0,
        }
    }

    #[test]
    fn a_replaced_task_cannot_write_over_its_successor() {
        let key = (SecretKey::generate().public(), 1);
        let mut inner = Inner::default();
        inner.watches.insert(key, entry(1, 3));
        let stale = WatchTask { key, generation: 0 };
        assert!(!apply_state(&mut inner, stale, 2, WatchState::Live));
        let current = &inner.watches[&key];
        assert_eq!(
            (current.preset_id, current.state),
            (3, WatchState::Connecting)
        );
    }

    #[test]
    fn the_current_task_updates_state_and_preset() {
        let key = (SecretKey::generate().public(), 1);
        let mut inner = Inner::default();
        inner.watches.insert(key, entry(4, 1));
        let current = WatchTask { key, generation: 4 };
        assert!(apply_state(&mut inner, current, 2, WatchState::Live));
        let entry = &inner.watches[&key];
        assert_eq!((entry.preset_id, entry.state), (2, WatchState::Live));
    }

    #[test]
    fn a_removed_watch_tells_its_task_to_stop() {
        let key = (SecretKey::generate().public(), 1);
        let mut inner = Inner::default();
        assert!(!apply_state(
            &mut inner,
            WatchTask { key, generation: 0 },
            1,
            WatchState::Ended
        ));
    }
}

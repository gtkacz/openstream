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

struct WatchEntry {
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
        lock(&self.inner).watches.insert(
            (publisher, live_id),
            WatchEntry {
                preset_id,
                state: WatchState::Connecting,
                handle: handle.clone(),
                _cancel: cancel_tx,
            },
        );
        tokio::spawn(self.clone().run_watch(
            publisher,
            live_id,
            preset_id,
            handle.clone(),
            cancel_rx,
        ));
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
        inner.clients.remove(&id);
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
        inner.clients.clear();
    }

    /// Returns false when the watch was removed meanwhile, which tells the task to stop.
    fn set_state(&self, key: WatchKey, preset_id: u32, state: WatchState) -> bool {
        let mut inner = lock(&self.inner);
        let Some(entry) = inner.watches.get_mut(&key) else {
            return false;
        };
        entry.state = state;
        entry.preset_id = preset_id;
        drop(inner);
        (self.on_change)();
        true
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
        lock(&self.inner).clients.remove(&publisher);
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
        publisher: PublicKey,
        live_id: u32,
        mut preset_id: u32,
        handle: WatchHandle,
        mut cancel: oneshot::Receiver<()>,
    ) {
        let key = (publisher, live_id);
        let mut backoff = RESUBSCRIBE_BACKOFF_INITIAL;
        loop {
            if !lock(&self.membership).is_member(&publisher) {
                self.set_state(key, preset_id, WatchState::Ended);
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
                        self.set_state(key, preset_id, WatchState::Ended);
                        return;
                    }
                    if !self
                        .wait_before_retry(key, preset_id, &mut backoff, &mut cancel)
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
                        .wait_before_retry(key, preset_id, &mut backoff, &mut cancel)
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
                    self.set_state(key, preset_id, WatchState::Ended);
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
            if !self.set_state(key, preset_id, WatchState::Live) {
                stop_viewer(viewer).await;
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
                        self.set_state(key, preset_id, WatchState::Ended);
                        return;
                    }
                }
                Outcome::Lost => self.forget_client(publisher),
            }
            if !self
                .wait_before_retry(key, preset_id, &mut backoff, &mut cancel)
                .await
            {
                return;
            }
        }
    }

    /// Marks the watch reconnecting and sleeps the current backoff. False means the watch is gone.
    async fn wait_before_retry(
        &self,
        key: WatchKey,
        preset_id: u32,
        backoff: &mut std::time::Duration,
        cancel: &mut oneshot::Receiver<()>,
    ) -> bool {
        if !self.set_state(key, preset_id, WatchState::Reconnecting) {
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

/// `Viewer::stop` joins the decode thread, so it runs off the async executor.
async fn stop_viewer(viewer: Viewer) {
    let _ = tokio::task::spawn_blocking(move || viewer.stop()).await;
}

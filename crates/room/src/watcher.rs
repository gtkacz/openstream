//! Remote lives this participant watches: one media connection per publisher, one decode pipeline
//! per watch, reconnection with backoff while the publisher stays a member.

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use brp_codec::RawFrame;
use brp_net::{MediaClient, NetError, PathKind};
use brp_pipeline::{
    AudioViewer, AudioViewerStats, FrameNotify, LatestSlot, Mixer, Viewer, ViewerSink, ViewerStats,
};
use brp_proto::constants::{
    RESUBSCRIBE_BACKOFF_INITIAL, RESUBSCRIBE_BACKOFF_MAX, SOURCE_PRESET_ID,
};
use brp_proto::{PublisherMessage, ViewerMessage};
use iroh::{Endpoint, EndpointAddr, PublicKey};
use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot};

use crate::codecs::DecoderFactory;
use crate::error::RoomError;
use crate::gossip::lock;
use crate::membership::{Member, Membership};
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
    /// This watch asked for (and, once live, carries) the publisher's audio.
    audio: bool,
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
    mixer: Mixer,
    /// False when the output device failed at room start: no watch asks for audio.
    output_ok: bool,
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
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        endpoint: Endpoint,
        runtime: Handle,
        decoders: Arc<dyn DecoderFactory>,
        membership: Arc<Mutex<Membership>>,
        mixer: Mixer,
        output_ok: bool,
        on_change: ChangeNotify,
        on_frame: FrameNotify,
    ) -> Arc<Self> {
        Arc::new(Self {
            endpoint,
            runtime,
            decoders,
            membership,
            mixer,
            output_ok,
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
        let advertised = lock(&self.membership).get(&publisher).is_some_and(|m| {
            m.presence
                .lives
                .iter()
                .any(|l| l.id == live_id && l.has_audio)
        });
        let (task, audio) = {
            let mut inner = lock(&self.inner);
            // Excludes the entry being replaced, so a preset switch on the carrier keeps its audio.
            let audio = wants_audio(
                inner
                    .watches
                    .iter()
                    .filter(|(k, _)| k.1 != live_id || k.0 != publisher)
                    .map(|((p, _), e)| (*p, e.audio)),
                publisher,
                advertised,
                self.output_ok,
            );
            let generation = inner.next_generation;
            inner.next_generation += 1;
            inner.watches.insert(
                (publisher, live_id),
                WatchEntry {
                    generation,
                    preset_id,
                    state: WatchState::Connecting,
                    audio,
                    handle: handle.clone(),
                    _cancel: cancel_tx,
                },
            );
            (
                WatchTask {
                    key: (publisher, live_id),
                    generation,
                },
                audio,
            )
        };
        tokio::spawn(
            self.clone()
                .run_watch(task, preset_id, audio, handle.clone(), cancel_rx),
        );
        (self.on_change)();
        Ok(handle)
    }

    pub fn unwatch(self: &Arc<Self>, publisher: PublicKey, live_id: u32) -> Result<(), RoomError> {
        let successor = {
            let mut inner = lock(&self.inner);
            let removed = inner
                .watches
                .remove(&(publisher, live_id))
                .ok_or(RoomError::NotWatching)?;
            let successor = carrier_successor(&inner, publisher, &removed);
            if !inner.watches.keys().any(|(p, _)| *p == publisher) {
                self.mixer.remove_track(&track_key(&publisher));
            }
            successor
        };
        if let Some((live_id, preset_id)) = successor {
            // Replacing the survivor resubscribes it with audio: the preset-switch path. Losing
            // the audio must not fail the unwatch the caller actually asked for.
            self.move_audio_to(publisher, live_id, preset_id);
        }
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
        self.mixer.remove_track(&track_key(&id));
        (self.on_change)();
    }

    /// Puts a publisher's audio back on one of its watches when nothing carries it any more.
    /// Spec 5.8 asks for this on every path that loses a carrier, not only `unwatch`: a live that
    /// ended, an audio stream the publisher closed, and a publisher that turned sharing back on.
    pub fn reacquire_audio(self: &Arc<Self>, publisher: PublicKey) {
        let advertised = lock(&self.membership)
            .get(&publisher)
            .is_some_and(Member::has_audio);
        if !advertised {
            // A publisher that stops sharing just goes quiet, exactly as one with nothing playing
            // does, so presence is the only notice its carrier ever gets.
            self.drop_carrier(publisher);
            return;
        }
        let candidate = audio_candidate(&lock(&self.inner), publisher, advertised, self.output_ok);
        if let Some((live_id, preset_id)) = candidate {
            self.move_audio_to(publisher, live_id, preset_id);
        }
    }

    /// Clears the carrier flag on every watch of a publisher whose audio is gone, so the next
    /// advertisement finds a watch free to take it.
    fn drop_carrier(&self, publisher: PublicKey) {
        let mut inner = lock(&self.inner);
        let mut cleared = false;
        for ((p, _), entry) in inner.watches.iter_mut() {
            if *p == publisher && entry.audio {
                entry.audio = false;
                cleared = true;
            }
        }
        drop(inner);
        if cleared {
            (self.on_change)();
        }
    }

    /// Re-watches one live through the preset-switch path, which subscribes it with audio.
    fn move_audio_to(self: &Arc<Self>, publisher: PublicKey, live_id: u32, preset_id: u32) {
        if let Err(error) = self.watch(publisher, live_id, preset_id) {
            tracing::debug!(%error, live_id, "could not move the publisher's audio");
        }
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
                audio: entry.audio,
            })
            .collect()
    }

    pub fn stop_all(&self) {
        let mut inner = lock(&self.inner);
        let publishers: Vec<PublicKey> = inner.watches.keys().map(|(p, _)| *p).collect();
        inner.watches.clear();
        for client in inner.clients.drain().map(|(_, client)| client) {
            client.close();
        }
        drop(inner);
        for publisher in publishers {
            self.mixer.remove_track(&track_key(&publisher));
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

    /// Ends a watch for good. The audio flag is cleared before the state so no snapshot shows a
    /// dead carrier, and before the hand-over so the successor rule sees no carrier at all.
    fn end_watch(self: &Arc<Self>, task: WatchTask, preset_id: u32) {
        self.set_audio_granted(task, false);
        self.set_state(task, preset_id, WatchState::Ended);
        self.reacquire_audio(task.key.0);
    }

    /// Records whether the request actually got audio, under the same generation check as
    /// `apply_state`: a denied request must not keep blocking the publisher's other watches.
    fn set_audio_granted(&self, task: WatchTask, granted: bool) {
        let mut inner = lock(&self.inner);
        if let Some(entry) = inner.watches.get_mut(&task.key)
            && entry.generation == task.generation
        {
            entry.audio = granted;
        }
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
        want_audio: bool,
        handle: WatchHandle,
        mut cancel: oneshot::Receiver<()>,
    ) {
        let (publisher, live_id) = task.key;
        let mut backoff = RESUBSCRIBE_BACKOFF_INITIAL;
        loop {
            if !lock(&self.membership).is_member(&publisher) {
                self.end_watch(task, preset_id);
                return;
            }
            let attempt = async {
                let client = self.client_for(publisher).await?;
                client
                    .subscribe(live_id, preset_id, want_audio)
                    .await
                    .map_err(RoomError::from)
            };
            let subscription = tokio::select! {
                _ = &mut cancel => return,
                result = attempt => result,
            };
            let mut subscription = match subscription {
                Ok(subscription) => subscription,
                Err(RoomError::Net(NetError::Rejected(reason))) => {
                    tracing::info!(%reason, live_id, preset_id, "subscription rejected");
                    if let Some(fallback) = self.fallback_preset(publisher, live_id, preset_id) {
                        preset_id = fallback;
                    } else if !self.live_exists(publisher, live_id) {
                        self.end_watch(task, preset_id);
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
                    self.end_watch(task, preset_id);
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
            let mut audio_viewer = subscription.audio.take().and_then(|audio| {
                match self.decoders.open_audio(&audio.params) {
                    Ok(decoder) => Some(AudioViewer::start(
                        self.runtime.clone(),
                        audio.packets,
                        decoder,
                        self.mixer.add_track(track_key(&publisher)),
                        Arc::new(AudioViewerStats::default()),
                    )),
                    Err(error) => {
                        tracing::warn!(%error, "no audio decoder; continuing with video only");
                        None
                    }
                }
            });
            let mut audio_ended = audio_viewer.as_mut().and_then(AudioViewer::take_ended);
            self.set_audio_granted(task, audio_viewer.is_some());
            if !self.set_state(task, preset_id, WatchState::Live) {
                stop_viewer(viewer).await;
                stop_audio_viewer(audio_viewer).await;
                let _ = subscription.control.send(ViewerMessage::Unsubscribe).await;
                return;
            }

            let mut events = subscription.events;
            let outcome = loop {
                tokio::select! {
                    _ = &mut cancel => break Outcome::Cancelled,
                    _ = audio_stream_end(&mut audio_ended) => {
                        // The publisher's audio stream closed while the video watch is healthy:
                        // the publisher toggled sharing off, or its capture died. Drop the carrier
                        // flag and let spec 5.8's rule find the audio again if it is still there.
                        audio_ended = None;
                        stop_audio_viewer(audio_viewer.take()).await;
                        self.set_audio_granted(task, false);
                        (self.on_change)();
                        self.reacquire_audio(publisher);
                    }
                    event = events.recv() => match event {
                        Some(PublisherMessage::LiveEnded) => break Outcome::Ended,
                        Some(_) => continue,
                        None => break Outcome::Lost,
                    },
                }
            };
            stop_viewer(viewer).await;
            stop_audio_viewer(audio_viewer).await;
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
                        self.end_watch(task, preset_id);
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

/// `AudioViewer::stop` joins its decode thread, so it runs off the async executor too.
async fn stop_audio_viewer(viewer: Option<AudioViewer>) {
    if let Some(viewer) = viewer {
        let _ = tokio::task::spawn_blocking(move || viewer.stop()).await;
    }
}

/// Completes once the audio decode thread has finished; never, for a watch without audio.
async fn audio_stream_end(ended: &mut Option<mpsc::Receiver<()>>) {
    match ended {
        Some(rx) => {
            let _ = rx.recv().await;
        }
        None => std::future::pending().await,
    }
}

/// One stream per publisher: a watch asks for audio only when nothing else of that publisher does.
pub(crate) fn wants_audio(
    mut entries: impl Iterator<Item = (PublicKey, bool)>,
    publisher: PublicKey,
    advertised: bool,
    output_ok: bool,
) -> bool {
    advertised && output_ok && !entries.any(|(p, audio)| p == publisher && audio)
}

/// When a carrier is removed, the publisher's lowest remaining live id inherits the audio.
fn carrier_successor(
    inner: &Inner,
    publisher: PublicKey,
    removed: &WatchEntry,
) -> Option<(u32, u32)> {
    if !removed.audio {
        return None;
    }
    inner
        .watches
        .iter()
        .filter(|((p, _), _)| *p == publisher)
        .map(|((_, live_id), entry)| (*live_id, entry.preset_id))
        .min()
}

/// Which watch should take a publisher's audio over when none of them carries it: the lowest live
/// id still watched. Nothing is chosen while a carrier exists, the publisher does not advertise
/// audio, or the output is broken.
fn audio_candidate(
    inner: &Inner,
    publisher: PublicKey,
    advertised: bool,
    output_ok: bool,
) -> Option<(u32, u32)> {
    if !advertised || !output_ok {
        return None;
    }
    if inner
        .watches
        .iter()
        .any(|((p, _), entry)| *p == publisher && entry.audio)
    {
        return None;
    }
    inner
        .watches
        .iter()
        // An ended watch carries nothing, and re-watching it would only end again.
        .filter(|((p, _), entry)| *p == publisher && entry.state != WatchState::Ended)
        .map(|((_, live_id), entry)| (*live_id, entry.preset_id))
        .min()
}

fn track_key(publisher: &PublicKey) -> brp_pipeline::TrackKey {
    *publisher.as_bytes()
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
            audio: false,
            handle: WatchHandle {
                slot: LatestSlot::new(),
                stats: Arc::new(ViewerStats::default()),
            },
            _cancel: oneshot::channel().0,
        }
    }

    fn key(seed: u8) -> PublicKey {
        SecretKey::from_bytes(&[seed; 32]).public()
    }

    #[test]
    fn the_first_watch_of_a_publisher_carries_audio_and_the_second_does_not() {
        let a = key(1);
        assert!(wants_audio([].into_iter(), a, true, true));
        assert!(!wants_audio([(a, true)].into_iter(), a, true, true));
        assert!(
            wants_audio([(a, false)].into_iter(), a, true, true),
            "an existing silent watch does not block"
        );
        assert!(
            wants_audio([(key(2), true)].into_iter(), a, true, true),
            "other publishers do not count"
        );
    }

    #[test]
    fn no_audio_is_asked_for_when_unadvertised_or_the_output_is_broken() {
        let a = key(1);
        assert!(!wants_audio([].into_iter(), a, false, true));
        assert!(!wants_audio([].into_iter(), a, true, false));
    }

    #[test]
    fn closing_the_carrier_moves_audio_to_the_publishers_surviving_watch() {
        let a = key(1);
        let mut inner = Inner::default();
        let mut carrier = entry(1, 1);
        carrier.audio = true;
        inner.watches.insert((a, 1), carrier);
        inner.watches.insert((a, 2), entry(2, 3));
        inner.watches.insert((key(2), 5), entry(3, 1));
        let removed = inner.watches.remove(&(a, 1)).unwrap();
        assert_eq!(carrier_successor(&inner, a, &removed), Some((2, 3)));
        let quiet = inner.watches.remove(&(a, 2)).unwrap();
        assert_eq!(
            carrier_successor(&inner, a, &quiet),
            None,
            "a non-carrier moves nothing"
        );
    }

    #[test]
    fn a_publisher_without_a_carrier_hands_its_audio_to_the_lowest_live_watched() {
        let a = key(1);
        let mut inner = Inner::default();
        inner.watches.insert((a, 4), entry(1, 1));
        inner.watches.insert((a, 2), entry(2, 3));
        inner.watches.insert((key(2), 1), entry(3, 1));
        assert_eq!(audio_candidate(&inner, a, true, true), Some((2, 3)));
        assert_eq!(
            audio_candidate(&inner, a, false, true),
            None,
            "the publisher stopped advertising audio"
        );
        assert_eq!(
            audio_candidate(&inner, a, true, false),
            None,
            "the output device failed at room start"
        );
        assert_eq!(
            audio_candidate(&inner, key(3), true, true),
            None,
            "nothing of that publisher is watched"
        );
    }

    #[test]
    fn nothing_is_re_acquired_while_a_carrier_holds_the_audio() {
        let a = key(1);
        let mut inner = Inner::default();
        let mut carrier = entry(1, 1);
        carrier.audio = true;
        inner.watches.insert((a, 4), carrier);
        inner.watches.insert((a, 2), entry(2, 3));
        assert_eq!(audio_candidate(&inner, a, true, true), None);
    }

    #[test]
    fn an_ended_watch_is_never_asked_to_carry_the_audio() {
        let a = key(1);
        let mut inner = Inner::default();
        let mut ended = entry(1, 1);
        ended.state = WatchState::Ended;
        inner.watches.insert((a, 2), ended);
        assert_eq!(audio_candidate(&inner, a, true, true), None);
        inner.watches.insert((a, 5), entry(2, 1));
        assert_eq!(audio_candidate(&inner, a, true, true), Some((5, 1)));
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

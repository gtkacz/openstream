use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use brp_proto::constants::{
    AUDIO_PRESET_ID, AUDIO_SENDER_BACKLOG_PACKETS, MAX_AUDIO_PACKET_BYTES, MAX_CONTROL_BYTES,
    MAX_FRAME_BYTES, MEDIA_ALPN, MEDIA_IDLE_TIMEOUT, MEDIA_KEEP_ALIVE, RECEIVE_BUDGET_BYTES,
    RECEIVE_QUEUE_DURATION, RECEIVE_QUEUE_MAX_FRAMES, RECEIVE_QUEUE_MIN_FRAMES, REFUSED_NOT_MEMBER,
};
use brp_proto::{
    AudioParams, CodecParams, FrameHeader, FrameKind, PublisherMessage, ViewerMessage,
};
use iroh::endpoint::{
    ConnectOptions, Connection, ConnectionError, QuicTransportConfig, RecvStream, VarInt,
};
use iroh::{Endpoint, EndpointAddr};
use tokio::sync::mpsc::{self, Receiver, Sender};

use crate::error::NetError;
use crate::framing::{read_msg, write_msg};
use crate::source::ReceivedFrame;

type Routes = Arc<Mutex<HashMap<(u32, u32), Sender<ReceivedFrame>>>>;

pub struct MediaClient {
    conn: Connection,
    routes: Routes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathKind {
    Direct,
    Relayed,
    Unknown,
}

/// Opus packets for one subscription, routed under the audio preset id.
#[derive(Debug)]
pub struct AudioStream {
    pub params: AudioParams,
    pub packets: Receiver<ReceivedFrame>,
}

#[derive(Debug)]
pub struct ViewerSubscription {
    pub params: CodecParams,
    pub frames: Receiver<ReceivedFrame>,
    pub control: Sender<ViewerMessage>,
    pub events: Receiver<PublisherMessage>,
    pub audio: Option<AudioStream>,
}

impl MediaClient {
    pub async fn connect(endpoint: &Endpoint, addr: EndpointAddr) -> Result<Self, NetError> {
        let options = ConnectOptions::new().with_transport_config(media_transport());
        let conn = endpoint
            .connect_with_opts(addr, MEDIA_ALPN, options)
            .await
            .map_err(|error| NetError::Connect(error.to_string()))?
            .await
            .map_err(|error| NetError::Connect(error.to_string()))?;
        let routes: Routes = Arc::default();
        tokio::spawn(receive_frames(conn.clone(), routes.clone()));
        Ok(Self { conn, routes })
    }

    pub fn remote_id(&self) -> iroh::EndpointId {
        self.conn.remote_id()
    }

    /// Whether media currently flows peer to peer or through a relay.
    pub fn path_kind(&self) -> PathKind {
        let paths = self.conn.paths();
        match paths.iter().find(|p| p.is_selected()) {
            Some(p) if p.is_ip() => PathKind::Direct,
            Some(p) if p.is_relay() => PathKind::Relayed,
            _ => PathKind::Unknown,
        }
    }

    /// The round-trip time of the path media takes right now.
    pub fn rtt(&self) -> Option<std::time::Duration> {
        let paths = self.conn.paths();
        paths.iter().find(|p| p.is_selected()).map(|p| p.rtt())
    }

    /// Finishes when the connection does, for whatever reason.
    pub async fn closed(&self) {
        self.conn.closed().await;
    }

    pub fn is_closed(&self) -> bool {
        self.conn.close_reason().is_some()
    }

    /// The publisher closed this connection because it does not count us as a member yet, which
    /// happens when our presence reaches it after its presence reached us.
    pub fn refused(&self) -> bool {
        matches!(
            self.conn.close_reason(),
            Some(ConnectionError::ApplicationClosed(close))
                if close.error_code == REFUSED_NOT_MEMBER.into()
        )
    }

    /// Everything heard from the publisher so far, in packets. With `MEDIA_KEEP_ALIVE` answered even
    /// on a still screen, a count that stops rising means the publisher has gone silent.
    pub fn packets_received(&self) -> u64 {
        self.conn.stats().udp_rx.datagrams
    }

    pub async fn subscribe(
        &self,
        live_id: u32,
        preset_id: u32,
        want_audio: bool,
    ) -> Result<ViewerSubscription, NetError> {
        let (mut send, mut recv) = self.conn.open_bi().await.map_err(NetError::connection)?;
        write_msg(
            &mut send,
            &ViewerMessage::Subscribe {
                live_id,
                preset_id,
                want_audio,
            },
        )
        .await?;
        let (params, audio_params) =
            match read_msg::<PublisherMessage>(&mut recv, MAX_CONTROL_BYTES).await? {
                PublisherMessage::SubscribeAck { video, audio } => (video, audio),
                PublisherMessage::SubscribeError { reason } => {
                    return Err(NetError::Rejected(reason));
                }
                _ => return Err(NetError::Protocol("expected SubscribeAck")),
            };

        let (frame_tx, frame_rx) = mpsc::channel(receive_queue_frames(params.fps));
        let audio = audio_params.map(|params| {
            let (tx, rx) = mpsc::channel(AUDIO_SENDER_BACKLOG_PACKETS);
            (params, tx, rx)
        });
        let audio_tx = audio.as_ref().map(|(_, tx, _)| tx.clone());
        {
            let mut routes = self
                .routes
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            routes.insert((live_id, preset_id), frame_tx.clone());
            if let Some(tx) = &audio_tx {
                routes.insert((live_id, AUDIO_PRESET_ID), tx.clone());
            }
        }

        let (control_tx, mut control_rx) = mpsc::channel::<ViewerMessage>(16);
        tokio::spawn(async move {
            while let Some(message) = control_rx.recv().await {
                if write_msg(&mut send, &message).await.is_err() {
                    break;
                }
            }
            let _ = send.finish();
        });

        let (events_tx, events_rx) = mpsc::channel::<PublisherMessage>(16);
        let routes = self.routes.clone();
        tokio::spawn(async move {
            while let Ok(message) = read_msg::<PublisherMessage>(&mut recv, MAX_CONTROL_BYTES).await
            {
                let ended = matches!(message, PublisherMessage::LiveEnded);
                if events_tx.send(message).await.is_err() || ended {
                    break;
                }
            }
            let mut routes = routes
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            // A resubscribe on the same live can already own these keys by the time this task's
            // control stream ends; only evict the routes this subscription itself registered.
            if routes
                .get(&(live_id, preset_id))
                .is_some_and(|tx| tx.same_channel(&frame_tx))
            {
                routes.remove(&(live_id, preset_id));
            }
            if let Some(audio_tx) = &audio_tx
                && routes
                    .get(&(live_id, AUDIO_PRESET_ID))
                    .is_some_and(|tx| tx.same_channel(audio_tx))
            {
                routes.remove(&(live_id, AUDIO_PRESET_ID));
            }
        });

        Ok(ViewerSubscription {
            params,
            frames: frame_rx,
            control: control_tx,
            events: events_rx,
            audio: audio.map(|(params, _, packets)| AudioStream { params, packets }),
        })
    }

    pub fn close(&self) {
        self.conn.close(0u32.into(), b"done");
    }
}

/// The caller's settings bind both ends: QUIC's idle timeout is the shorter of the two sides', so this
/// holds against a publisher on any release, and it leaves gossip's connections on the defaults.
fn media_transport() -> QuicTransportConfig {
    let idle_ms = VarInt::from_u32(MEDIA_IDLE_TIMEOUT.as_millis() as u32);
    QuicTransportConfig::builder()
        .max_idle_timeout(Some(idle_ms.into()))
        .keep_alive_interval(MEDIA_KEEP_ALIVE)
        .build()
}

/// Room for `RECEIVE_QUEUE_DURATION` of video at this stream's frame rate. Bytes are bounded
/// separately, per connection, by `Charge`.
pub fn receive_queue_frames(fps: u32) -> usize {
    let frames = u64::from(fps) * RECEIVE_QUEUE_DURATION.as_millis() as u64 / 1000;
    (frames as usize).clamp(RECEIVE_QUEUE_MIN_FRAMES, RECEIVE_QUEUE_MAX_FRAMES)
}

/// Audio frames have their own, much smaller, ceiling than video ones.
pub(crate) fn check_payload_len(kind: FrameKind, len: usize) -> bool {
    match kind {
        FrameKind::Audio => len <= MAX_AUDIO_PACKET_BYTES,
        FrameKind::Video => len <= MAX_FRAME_BYTES,
    }
}

/// Routes are keyed by preset id and the audio preset is reserved, so the audio route carries
/// audio frames and the video routes carry video. Anything else is a peer sending what we did not
/// subscribe to.
pub(crate) fn kind_matches_route(kind: FrameKind, preset_id: u32) -> bool {
    (kind == FrameKind::Audio) == (preset_id == AUDIO_PRESET_ID)
}

/// A received frame's share of its connection's `RECEIVE_BUDGET_BYTES`, given back when the frame
/// is dropped: mid-stream, refused by a full queue, or after the viewer is done with it.
#[derive(Debug, Default)]
pub(crate) struct Charge {
    budget: Arc<AtomicUsize>,
    bytes: usize,
}

impl Charge {
    pub(crate) fn new(budget: Arc<AtomicUsize>) -> Self {
        Self { budget, bytes: 0 }
    }

    /// Takes `bytes` more, unless that would put the connection over its budget.
    pub(crate) fn grow(&mut self, bytes: usize) -> bool {
        let grown = self
            .budget
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |held| {
                held.checked_add(bytes)
                    .filter(|&total| total <= RECEIVE_BUDGET_BYTES)
            })
            .is_ok();
        if grown {
            self.bytes += bytes;
        }
        grown
    }
}

impl Drop for Charge {
    fn drop(&mut self) {
        self.budget.fetch_sub(self.bytes, Ordering::Relaxed);
    }
}

/// Reads one frame stream to its end, charging the budget as bytes arrive, so streams still
/// coming in count as much as frames already queued.
async fn read_frame(
    stream: &mut RecvStream,
    budget: &Arc<AtomicUsize>,
) -> Option<(Vec<u8>, Charge)> {
    let mut kept = Some((Vec::new(), Charge::new(budget.clone())));
    let mut read = 0;
    loop {
        match stream.read_chunk(MAX_FRAME_BYTES).await {
            Ok(Some(chunk)) => {
                read += chunk.len();
                if read > MAX_FRAME_BYTES {
                    tracing::debug!("frame stream too long");
                    return None;
                }
                // Over budget, the rest is read and discarded rather than the stream stopped: a
                // publisher's sender gives up on a stopped stream, which would freeze this watch.
                if let Some((bytes, charge)) = &mut kept {
                    if charge.grow(chunk.len()) {
                        bytes.extend_from_slice(&chunk);
                    } else {
                        tracing::debug!("dropping a frame over the receive budget");
                        kept = None;
                    }
                }
            }
            Ok(None) => return kept,
            Err(error) => {
                tracing::debug!(%error, "frame stream ended early");
                return None;
            }
        }
    }
}

/// Reads each frame stream independently so a large keyframe cannot delay later frames.
async fn receive_frames(conn: Connection, routes: Routes) {
    // Malformed frames are a peer misbehaving, not an event per packet: one warning says it.
    let warned = Arc::new(AtomicBool::new(false));
    let budget = Arc::new(AtomicUsize::new(0));
    while let Ok(mut stream) = conn.accept_uni().await {
        let routes = routes.clone();
        let warned = warned.clone();
        let budget = budget.clone();
        tokio::spawn(async move {
            let Some((bytes, charge)) = read_frame(&mut stream, &budget).await else {
                return;
            };
            let (header, payload) = match FrameHeader::decode_prefixed(&bytes) {
                Ok(parsed) => parsed,
                Err(error) => {
                    tracing::warn!(%error, "dropping malformed frame");
                    return;
                }
            };
            if !check_payload_len(header.kind, header.len as usize)
                || !kind_matches_route(header.kind, header.preset_id)
            {
                if !warned.swap(true, Ordering::Relaxed) {
                    tracing::warn!(
                        kind = ?header.kind,
                        preset_id = header.preset_id,
                        len = header.len,
                        "dropping a frame this route cannot carry; further ones are silent"
                    );
                }
                return;
            }
            let route = routes
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(&(header.live_id, header.preset_id))
                .cloned();
            match route {
                Some(sender) => {
                    let frame = ReceivedFrame {
                        header,
                        payload: payload.to_vec(),
                        _charge: charge,
                    };
                    // Never waits for room. QUIC gives its stream credit back as soon as a stream
                    // is read, so a waiting task would not slow the publisher, only hold the frame
                    // in memory. The gap this leaves makes the viewer ask for a keyframe.
                    if let Err(error) = sender.try_send(frame) {
                        tracing::debug!(%error, "frame not handed to the viewer");
                    }
                }
                None => tracing::trace!(
                    live_id = header.live_id,
                    preset_id = header.preset_id,
                    "frame for unknown subscription"
                ),
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_queue_holds_half_a_second_of_video_within_its_bounds() {
        assert_eq!(receive_queue_frames(60), 30);
        assert_eq!(receive_queue_frames(240), 120);
        assert_eq!(receive_queue_frames(1), RECEIVE_QUEUE_MIN_FRAMES);
        assert_eq!(receive_queue_frames(u32::MAX), RECEIVE_QUEUE_MAX_FRAMES);
    }

    #[test]
    fn the_audio_route_takes_only_small_audio_frames() {
        assert!(check_payload_len(FrameKind::Audio, MAX_AUDIO_PACKET_BYTES));
        assert!(!check_payload_len(
            FrameKind::Audio,
            MAX_AUDIO_PACKET_BYTES + 1
        ));
        assert!(check_payload_len(FrameKind::Video, MAX_FRAME_BYTES));
        assert!(!check_payload_len(FrameKind::Video, MAX_FRAME_BYTES + 1));
    }

    #[test]
    fn a_frame_whose_kind_contradicts_its_route_is_refused() {
        assert!(kind_matches_route(FrameKind::Audio, AUDIO_PRESET_ID));
        assert!(kind_matches_route(FrameKind::Video, 1));
        assert!(!kind_matches_route(FrameKind::Video, AUDIO_PRESET_ID));
        assert!(!kind_matches_route(FrameKind::Audio, 1));
    }

    #[test]
    fn a_frame_gives_its_bytes_back_to_the_connection_when_dropped() {
        let budget = Arc::new(AtomicUsize::new(0));
        let mut charge = Charge::new(budget.clone());
        assert!(charge.grow(1_000));
        assert!(charge.grow(500));
        assert_eq!(budget.load(Ordering::Relaxed), 1_500);
        drop(charge);
        assert_eq!(budget.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn a_connection_holds_no_more_than_its_receive_budget() {
        let budget = Arc::new(AtomicUsize::new(0));
        let mut first = Charge::new(budget.clone());
        assert!(first.grow(RECEIVE_BUDGET_BYTES));
        let mut second = Charge::new(budget.clone());
        assert!(!second.grow(1), "a spent budget refuses more");
        assert_eq!(budget.load(Ordering::Relaxed), RECEIVE_BUDGET_BYTES);
        drop(first);
        assert!(second.grow(1), "and takes more once a frame is gone");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_frame_over_the_budget_is_read_through_so_the_publisher_keeps_sending() {
        use crate::{RelaySetting, bind_endpoint};
        use iroh::SecretKey;

        let publisher = bind_endpoint(SecretKey::generate(), RelaySetting::Disabled, vec![])
            .await
            .unwrap();
        publisher.set_alpns(vec![MEDIA_ALPN.to_vec()]);
        let viewer = bind_endpoint(SecretKey::generate(), RelaySetting::Disabled, vec![])
            .await
            .unwrap();
        let (viewer_conn, publisher_conn) =
            tokio::join!(viewer.connect(publisher.addr(), MEDIA_ALPN), async {
                publisher.accept().await.unwrap().await
            });
        let (viewer_conn, publisher_conn) = (viewer_conn.unwrap(), publisher_conn.unwrap());

        // Larger than a stream's initial receive window, so the write is still blocked when the
        // viewer refuses it, as a real keyframe's would be.
        let frame = vec![0u8; 4 * 1024 * 1024];
        let writer = tokio::spawn(async move {
            let mut send = publisher_conn.open_uni().await.unwrap();
            send.write_all(&frame)
                .await
                .expect("the viewer stopped the stream");
            send.finish().unwrap();
            send.stopped().await.unwrap()
        });
        let mut stream = viewer_conn.accept_uni().await.unwrap();
        let budget = Arc::new(AtomicUsize::new(RECEIVE_BUDGET_BYTES));
        assert!(read_frame(&mut stream, &budget).await.is_none());
        drop(stream);

        assert_eq!(writer.await.unwrap(), None, "read to its end, not stopped");
        assert_eq!(budget.load(Ordering::Relaxed), RECEIVE_BUDGET_BYTES);
    }
}

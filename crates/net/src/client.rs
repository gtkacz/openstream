use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use brp_proto::constants::{
    AUDIO_PRESET_ID, AUDIO_SENDER_BACKLOG_PACKETS, MAX_AUDIO_PACKET_BYTES, MAX_CONTROL_BYTES,
    MAX_FRAME_BYTES, MEDIA_ALPN, RECEIVE_QUEUE_FRAMES,
};
use brp_proto::{
    AudioParams, CodecParams, FrameHeader, FrameKind, PublisherMessage, ViewerMessage,
};
use iroh::endpoint::Connection;
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
        let conn = endpoint
            .connect(addr, MEDIA_ALPN)
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

        let (frame_tx, frame_rx) = mpsc::channel(RECEIVE_QUEUE_FRAMES);
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

/// Reads each frame stream independently so a large keyframe cannot delay later frames.
async fn receive_frames(conn: Connection, routes: Routes) {
    // Malformed frames are a peer misbehaving, not an event per packet: one warning says it.
    let warned = Arc::new(AtomicBool::new(false));
    while let Ok(mut stream) = conn.accept_uni().await {
        let routes = routes.clone();
        let warned = warned.clone();
        tokio::spawn(async move {
            let bytes = match stream.read_to_end(MAX_FRAME_BYTES).await {
                Ok(bytes) => bytes,
                Err(error) => {
                    tracing::debug!(%error, "frame stream ended early");
                    return;
                }
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
                    };
                    if sender.send(frame).await.is_err() {
                        tracing::debug!("viewer dropped its frame receiver");
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
}

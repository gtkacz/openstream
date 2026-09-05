use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use brp_proto::constants::{MAX_CONTROL_BYTES, MAX_FRAME_BYTES, MEDIA_ALPN, RECEIVE_QUEUE_FRAMES};
use brp_proto::{CodecParams, FrameHeader, PublisherMessage, ViewerMessage};
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

#[derive(Debug)]
pub struct ViewerSubscription {
    pub params: CodecParams,
    pub frames: Receiver<ReceivedFrame>,
    pub control: Sender<ViewerMessage>,
    pub events: Receiver<PublisherMessage>,
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
    ) -> Result<ViewerSubscription, NetError> {
        let (mut send, mut recv) = self.conn.open_bi().await.map_err(NetError::connection)?;
        write_msg(
            &mut send,
            &ViewerMessage::Subscribe {
                live_id,
                preset_id,
                want_audio: false,
            },
        )
        .await?;
        let params = match read_msg::<PublisherMessage>(&mut recv, MAX_CONTROL_BYTES).await? {
            PublisherMessage::SubscribeAck { video, .. } => video,
            PublisherMessage::SubscribeError { reason } => return Err(NetError::Rejected(reason)),
            _ => return Err(NetError::Protocol("expected SubscribeAck")),
        };

        let (frame_tx, frame_rx) = mpsc::channel(RECEIVE_QUEUE_FRAMES);
        self.routes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert((live_id, preset_id), frame_tx);

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
            routes
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&(live_id, preset_id));
        });

        Ok(ViewerSubscription {
            params,
            frames: frame_rx,
            control: control_tx,
            events: events_rx,
        })
    }

    pub fn close(&self) {
        self.conn.close(0u32.into(), b"done");
    }
}

/// Reads each frame stream independently so a large keyframe cannot delay later frames.
async fn receive_frames(conn: Connection, routes: Routes) {
    while let Ok(mut stream) = conn.accept_uni().await {
        let routes = routes.clone();
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

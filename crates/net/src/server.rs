use std::fmt;
use std::sync::Arc;
use std::time::Instant;

use brp_proto::constants::{AUDIO_PRESET_ID, MAX_CONTROL_BYTES, REFUSED_NOT_MEMBER};
use brp_proto::{EncodedFrame, FrameHeader, FrameKind, PublisherMessage, ViewerMessage};
use iroh::endpoint::{Connection, SendStream, WriteError};
use iroh::protocol::{AcceptError, ProtocolHandler};
use tokio::sync::mpsc::Receiver;
use tokio::task::JoinHandle;

use crate::error::NetError;
use crate::framing::{read_msg, write_msg};
use crate::policy::ConnectionPolicy;
use crate::source::{KeyframeLimit, LiveSource};

/// Audio streams outrank video: a late video frame is dropped by the pacer, a late audio packet is
/// a dropout.
const AUDIO_STREAM_PRIORITY: i32 = 1;

/// How many subscriptions each peer holds, so the room can say who is watching.
pub type Viewers = Arc<std::sync::Mutex<std::collections::HashMap<iroh::EndpointId, u32>>>;

#[derive(Clone)]
pub struct MediaServer {
    source: Arc<dyn LiveSource>,
    policy: Arc<dyn ConnectionPolicy>,
    viewers: Viewers,
}

impl MediaServer {
    pub fn new(source: Arc<dyn LiveSource>, policy: Arc<dyn ConnectionPolicy>) -> Self {
        Self {
            source,
            policy,
            viewers: Viewers::default(),
        }
    }

    pub fn viewers(&self) -> Viewers {
        self.viewers.clone()
    }
}

impl fmt::Debug for MediaServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MediaServer")
    }
}

impl ProtocolHandler for MediaServer {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let peer = connection.remote_id();
        if !self.policy.allows(peer) {
            tracing::info!(peer = %peer.fmt_short(), "refusing media connection from a non-member");
            connection.close(REFUSED_NOT_MEMBER.into(), b"not a member");
            return Ok(());
        }
        tracing::info!(peer = %peer.fmt_short(), "media connection accepted");
        // Each bidirectional stream is one subscription; keep accepting until the peer closes.
        while let Ok((send, recv)) = connection.accept_bi().await {
            let source = self.source.clone();
            let conn = connection.clone();
            let viewers = self.viewers.clone();
            if let Ok(mut viewers) = viewers.lock() {
                *viewers.entry(peer).or_insert(0) += 1;
            }
            tokio::spawn(async move {
                if let Err(error) = serve_subscription(conn, send, recv, source).await {
                    tracing::debug!(%error, "subscription ended");
                }
                if let Ok(mut viewers) = viewers.lock()
                    && let Some(count) = viewers.get_mut(&peer)
                {
                    *count = count.saturating_sub(1);
                }
            });
        }
        tracing::info!(peer = %peer.fmt_short(), "media connection closed");
        Ok(())
    }
}

async fn serve_subscription(
    conn: Connection,
    mut send: SendStream,
    mut recv: iroh::endpoint::RecvStream,
    source: Arc<dyn LiveSource>,
) -> Result<(), NetError> {
    let first: ViewerMessage = read_msg(&mut recv, MAX_CONTROL_BYTES).await?;
    let ViewerMessage::Subscribe {
        live_id,
        preset_id,
        want_audio,
    } = first
    else {
        write_msg(
            &mut send,
            &PublisherMessage::SubscribeError {
                reason: "first message must be Subscribe".into(),
            },
        )
        .await?;
        return Err(NetError::Protocol(
            "expected Subscribe as the first control message",
        ));
    };

    let subscription = match source.subscribe(live_id, preset_id) {
        Ok(subscription) => subscription,
        Err(rejected) => {
            write_msg(
                &mut send,
                &PublisherMessage::SubscribeError {
                    reason: rejected.to_string(),
                },
            )
            .await?;
            send.finish().map_err(NetError::stream)?;
            return Ok(());
        }
    };

    let audio = if want_audio {
        match source.subscribe_audio(live_id) {
            Ok(audio) => Some(audio),
            Err(rejected) => {
                tracing::debug!(live_id, %rejected, "audio not granted");
                None
            }
        }
    } else {
        None
    };

    write_msg(
        &mut send,
        &PublisherMessage::SubscribeAck {
            video: subscription.params,
            audio: audio.as_ref().map(|a| a.params),
        },
    )
    .await?;

    let sender: JoinHandle<Result<(), NetError>> = tokio::spawn(send_frames(
        conn.clone(),
        send,
        live_id,
        preset_id,
        subscription.frames,
    ));
    let audio_sender = audio.map(|audio| tokio::spawn(send_audio(conn, live_id, audio.packets)));

    // Held here rather than trusted to the viewer: a viewer that keeps asking keeps being refused,
    // and one that has fallen behind asks again a moment later anyway.
    let mut keyframes = KeyframeLimit::default();
    loop {
        match read_msg::<ViewerMessage>(&mut recv, MAX_CONTROL_BYTES).await {
            Ok(ViewerMessage::RequestKeyframe) => {
                if keyframes.allow(Instant::now()) {
                    source.request_keyframe(live_id, preset_id);
                }
            }
            Ok(ViewerMessage::Unsubscribe) | Err(_) => break,
            Ok(ViewerMessage::Stats {
                frames_received,
                frames_dropped,
                decode_fps,
                rtt_ms,
            }) => {
                tracing::trace!(
                    live_id,
                    preset_id,
                    frames_received,
                    frames_dropped,
                    decode_fps,
                    rtt_ms,
                    "viewer stats"
                );
            }
            Ok(other) => tracing::debug!(?other, "control message not supported in this phase"),
        }
    }

    sender.abort();
    if let Some(audio_sender) = audio_sender {
        audio_sender.abort();
    }
    Ok(())
}

async fn write_frame(
    conn: &Connection,
    header: FrameHeader,
    data: &[u8],
    priority: i32,
) -> Result<(), NetError> {
    let mut stream = conn.open_uni().await.map_err(NetError::connection)?;
    stream.set_priority(priority).map_err(NetError::stream)?;
    let prefix = header.encode_prefix()?;
    let written = async {
        stream.write_all(&prefix).await?;
        stream.write_all(data).await
    }
    .await;
    match written {
        // A viewer that stops one frame's stream dropped that frame, not the subscription.
        Err(WriteError::Stopped(_)) => return Ok(()),
        written => written.map_err(NetError::stream)?,
    }
    stream.finish().map_err(NetError::stream)?;
    Ok(())
}

async fn send_frames(
    conn: Connection,
    mut control: SendStream,
    live_id: u32,
    preset_id: u32,
    mut frames: Receiver<Arc<EncodedFrame>>,
) -> Result<(), NetError> {
    while let Some(frame) = frames.recv().await {
        let len = u32::try_from(frame.data.len())
            .map_err(|_| NetError::Protocol("frame larger than u32"))?;
        let header = FrameHeader {
            live_id,
            preset_id,
            kind: FrameKind::Video,
            seq: frame.seq,
            capture_ts_us: frame.capture_ts_us,
            keyframe: frame.keyframe,
            len,
        };
        write_frame(&conn, header, &frame.data, 0).await?;
    }
    write_msg(&mut control, &PublisherMessage::LiveEnded).await?;
    control.finish().map_err(NetError::stream)?;
    Ok(())
}

async fn send_audio(
    conn: Connection,
    live_id: u32,
    mut packets: Receiver<Arc<EncodedFrame>>,
) -> Result<(), NetError> {
    while let Some(packet) = packets.recv().await {
        let len = u32::try_from(packet.data.len())
            .map_err(|_| NetError::Protocol("audio packet larger than u32"))?;
        let header = FrameHeader {
            live_id,
            preset_id: AUDIO_PRESET_ID,
            kind: FrameKind::Audio,
            seq: packet.seq,
            capture_ts_us: packet.capture_ts_us,
            keyframe: true,
            len,
        };
        write_frame(&conn, header, &packet.data, AUDIO_STREAM_PRIORITY).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::endpoint::connected_pair;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_frame_the_viewer_stops_is_dropped_without_failing_the_sender() {
        let ((_viewer, viewer_conn), (_publisher, publisher_conn)) = connected_pair().await;
        // Larger than a stream's initial receive window, so the stop arrives mid-write.
        let data = vec![0u8; 4 * 1024 * 1024];
        let header = FrameHeader {
            live_id: 1,
            preset_id: 1,
            kind: FrameKind::Video,
            seq: 0,
            capture_ts_us: 0,
            keyframe: true,
            len: data.len() as u32,
        };
        let (sent, ()) = tokio::join!(write_frame(&publisher_conn, header, &data, 0), async {
            let mut stream = viewer_conn.accept_uni().await.unwrap();
            stream.stop(0u32.into()).unwrap();
        });
        assert!(sent.is_ok(), "{sent:?}");
    }
}

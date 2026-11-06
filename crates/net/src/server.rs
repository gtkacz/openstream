use std::fmt;
use std::sync::Arc;
use std::time::Instant;

use brp_proto::constants::{
    AUDIO_PRESET_ID, MAX_CONTROL_BYTES, MAX_FRAME_BYTES, REFUSED_NOT_MEMBER,
};
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
            tokio::spawn(async move {
                if let Err(error) =
                    serve_subscription(conn, send, recv, source, peer, viewers).await
                {
                    tracing::debug!(%error, "subscription ended");
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
    peer: iroh::EndpointId,
    viewers: Viewers,
) -> Result<(), NetError> {
    let first: ViewerMessage = read_msg(&mut recv, MAX_CONTROL_BYTES).await?;
    let (live_id, preset_id, want_audio) = match first {
        ViewerMessage::Subscribe {
            live_id,
            preset_id,
            want_audio,
        } => (live_id, preset_id, want_audio),
        // Listening to someone's voice is not watching them, so it stays out of the count.
        ViewerMessage::SubscribeVoice => return serve_voice(conn, send, recv, source, peer).await,
        _ => {
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
        }
    };

    if let Ok(mut viewers) = viewers.lock() {
        *viewers.entry(peer).or_insert(0) += 1;
    }
    let served = serve_watch(conn, send, recv, source, live_id, preset_id, want_audio).await;
    if let Ok(mut viewers) = viewers.lock()
        && let Some(count) = viewers.get_mut(&peer)
    {
        *count = count.saturating_sub(1);
    }
    served
}

async fn serve_watch(
    conn: Connection,
    mut send: SendStream,
    mut recv: iroh::endpoint::RecvStream,
    source: Arc<dyn LiveSource>,
    live_id: u32,
    preset_id: u32,
    want_audio: bool,
) -> Result<(), NetError> {
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

async fn serve_voice(
    conn: Connection,
    mut send: SendStream,
    mut recv: iroh::endpoint::RecvStream,
    source: Arc<dyn LiveSource>,
    peer: iroh::EndpointId,
) -> Result<(), NetError> {
    let voice = match source.subscribe_voice(peer) {
        Ok(voice) => voice,
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
    write_msg(
        &mut send,
        &PublisherMessage::VoiceAck {
            audio: voice.params,
        },
    )
    .await?;
    let sender = tokio::spawn(send_voice(conn, send, voice.packets));
    // Nothing but Unsubscribe means anything on this stream, so any end of it ends the subscription.
    while let Ok(message) = read_msg::<ViewerMessage>(&mut recv, MAX_CONTROL_BYTES).await {
        if matches!(message, ViewerMessage::Unsubscribe) {
            break;
        }
    }
    sender.abort();
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
        Err(WriteError::Stopped(code)) => {
            tracing::debug!(seq = header.seq, %code, "the viewer stopped a frame; dropping it");
            return Ok(());
        }
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
    // An encoder that makes one oversized frame usually keeps making them.
    let mut warned = false;
    while let Some(frame) = frames.recv().await {
        // Every viewer refuses these, so sending one only wastes the link.
        if frame.data.len() > MAX_FRAME_BYTES {
            if !std::mem::replace(&mut warned, true) {
                tracing::warn!(
                    seq = frame.seq,
                    len = frame.data.len(),
                    "skipping a frame larger than viewers accept; further ones are silent"
                );
            }
            continue;
        }
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

async fn send_voice(
    conn: Connection,
    mut control: SendStream,
    mut packets: Receiver<Arc<EncodedFrame>>,
) -> Result<(), NetError> {
    while let Some(packet) = packets.recv().await {
        let len = u32::try_from(packet.data.len())
            .map_err(|_| NetError::Protocol("voice packet larger than u32"))?;
        let header = FrameHeader {
            live_id: 0,
            preset_id: AUDIO_PRESET_ID,
            kind: FrameKind::Voice,
            seq: packet.seq,
            capture_ts_us: packet.capture_ts_us,
            keyframe: true,
            len,
        };
        write_frame(&conn, header, &packet.data, AUDIO_STREAM_PRIORITY).await?;
    }
    control.finish().map_err(NetError::stream)?;
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_frame_no_viewer_would_accept_is_skipped_rather_than_sent() {
        let ((_viewer, viewer_conn), (_publisher, publisher_conn)) = connected_pair().await;
        let (control, _) = publisher_conn.open_bi().await.unwrap();
        let (tx, rx) = tokio::sync::mpsc::channel(2);
        for (seq, len) in [(0, MAX_FRAME_BYTES + 1), (1, 16)] {
            let frame = EncodedFrame {
                seq,
                capture_ts_us: 0,
                keyframe: true,
                data: vec![0; len],
            };
            tx.send(Arc::new(frame)).await.unwrap();
        }
        drop(tx);
        // A clone, so the connection outlives the sender and stays open for the read below.
        let sender = tokio::spawn(send_frames(publisher_conn.clone(), control, 1, 1, rx));

        let mut stream = viewer_conn.accept_uni().await.unwrap();
        let bytes = stream.read_to_end(MAX_FRAME_BYTES).await.unwrap();
        let (header, _) = FrameHeader::decode_prefixed(&bytes).unwrap();
        assert_eq!(header.seq, 1);
        sender.await.unwrap().unwrap();
    }
}

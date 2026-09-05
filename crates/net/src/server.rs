use std::fmt;
use std::sync::Arc;

use brp_proto::constants::{MAX_CONTROL_BYTES, REFUSED_NOT_MEMBER};
use brp_proto::{EncodedFrame, FrameHeader, FrameKind, PublisherMessage, ViewerMessage};
use iroh::endpoint::{Connection, SendStream};
use iroh::protocol::{AcceptError, ProtocolHandler};
use tokio::sync::mpsc::Receiver;
use tokio::task::JoinHandle;

use crate::error::NetError;
use crate::framing::{read_msg, write_msg};
use crate::policy::ConnectionPolicy;
use crate::source::LiveSource;

#[derive(Clone)]
pub struct MediaServer {
    source: Arc<dyn LiveSource>,
    policy: Arc<dyn ConnectionPolicy>,
}

impl MediaServer {
    pub fn new(source: Arc<dyn LiveSource>, policy: Arc<dyn ConnectionPolicy>) -> Self {
        Self { source, policy }
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
            tokio::spawn(async move {
                if let Err(error) = serve_subscription(conn, send, recv, source).await {
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
) -> Result<(), NetError> {
    let first: ViewerMessage = read_msg(&mut recv, MAX_CONTROL_BYTES).await?;
    let ViewerMessage::Subscribe {
        live_id, preset_id, ..
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

    write_msg(
        &mut send,
        &PublisherMessage::SubscribeAck {
            video: subscription.params,
            audio: None,
        },
    )
    .await?;

    let sender: JoinHandle<Result<(), NetError>> = tokio::spawn(send_frames(
        conn,
        send,
        live_id,
        preset_id,
        subscription.frames,
    ));

    loop {
        match read_msg::<ViewerMessage>(&mut recv, MAX_CONTROL_BYTES).await {
            Ok(ViewerMessage::RequestKeyframe) => source.request_keyframe(live_id, preset_id),
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
        let mut stream = conn.open_uni().await.map_err(NetError::connection)?;
        stream
            .write_all(&header.encode_prefix()?)
            .await
            .map_err(NetError::stream)?;
        stream
            .write_all(&frame.data)
            .await
            .map_err(NetError::stream)?;
        stream.finish().map_err(NetError::stream)?;
    }
    write_msg(&mut control, &PublisherMessage::LiveEnded).await?;
    control.finish().map_err(NetError::stream)?;
    Ok(())
}

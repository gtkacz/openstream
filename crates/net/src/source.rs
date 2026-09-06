use brp_proto::{AudioParams, CodecParams, EncodedFrame, FrameHeader};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::mpsc::Receiver;
/// Supplies encoded frames to the media server.
pub trait LiveSource: Send + Sync + 'static {
    fn subscribe(&self, live_id: u32, preset_id: u32) -> Result<Subscription, SubscribeRejected>;
    fn request_keyframe(&self, live_id: u32, preset_id: u32);
    /// The publisher's audio, granted per live so the viewer can pick which watch carries it.
    /// Sources without audio keep the default.
    fn subscribe_audio(&self, live_id: u32) -> Result<AudioSubscription, SubscribeRejected> {
        let _ = live_id;
        Err(SubscribeRejected::NoAudio)
    }
}
/// The stream of frames and codec parameters for one live subscription.
#[derive(Debug)]
pub struct Subscription {
    pub params: CodecParams,
    pub frames: Receiver<Arc<EncodedFrame>>,
}
/// Opus packets for one subscription.
#[derive(Debug)]
pub struct AudioSubscription {
    pub params: AudioParams,
    pub packets: Receiver<Arc<EncodedFrame>>,
}
/// Explains why a requested live or preset cannot be subscribed to.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SubscribeRejected {
    #[error("unknown live {0}")]
    UnknownLive(u32),
    #[error("unknown preset {0}")]
    UnknownPreset(u32),
    #[error("encoder could not start: {0}")]
    EncoderFailed(String),
    #[error("live offers no audio")]
    NoAudio,
}
/// One frame read from the media stream, before reordering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceivedFrame {
    pub header: FrameHeader,
    pub payload: Vec<u8>,
}

use brp_proto::constants::{FORCED_KEYFRAME_MIN_INTERVAL, STALLED_VIEWER_KEYFRAME_MAX_INTERVAL};
use brp_proto::{AudioParams, CodecParams, EncodedFrame, FrameHeader};
use std::sync::Arc;
use std::time::{Duration, Instant};
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
/// How often one viewer may cost a preset's other viewers a keyframe, which is much larger than a
/// normal frame and leaves less of the bitrate for those around it. The first request goes through
/// at once, and requests that keep coming go through further and further apart. A quiet spell as
/// long as the longest wait starts over, so an occasional hiccup never waits.
#[derive(Debug, Default)]
pub struct KeyframeLimit {
    /// When the next request may go through, and the wait that was used to work it out.
    next: Option<(Instant, Duration)>,
}

impl KeyframeLimit {
    pub fn allow(&mut self, now: Instant) -> bool {
        let wait = match self.next {
            Some((next, _)) if now < next => return false,
            Some((next, wait))
                if now.duration_since(next) < STALLED_VIEWER_KEYFRAME_MAX_INTERVAL =>
            {
                (wait * 2).min(STALLED_VIEWER_KEYFRAME_MAX_INTERVAL)
            }
            _ => FORCED_KEYFRAME_MIN_INTERVAL,
        };
        self.next = Some((now + wait, wait));
        true
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_that_keep_coming_go_through_further_apart_up_to_the_longest_wait() {
        let mut limit = KeyframeLimit::default();
        let start = Instant::now();
        let mut allowed = Vec::new();
        for tenths in 0..200u64 {
            let at = Duration::from_millis(tenths * 100);
            if limit.allow(start + at) {
                allowed.push(at.as_secs());
            }
        }
        assert_eq!(allowed, [0, 1, 3, 7, 12, 17]);
    }

    #[test]
    fn a_quiet_spell_starts_over() {
        let mut limit = KeyframeLimit::default();
        let start = Instant::now();
        assert!(limit.allow(start));
        assert!(limit.allow(start + Duration::from_secs(1)));
        assert!(limit.allow(start + Duration::from_secs(3)));
        let later = start + Duration::from_secs(3) + Duration::from_secs(4);
        let quiet = later + STALLED_VIEWER_KEYFRAME_MAX_INTERVAL;
        assert!(limit.allow(quiet));
        assert!(limit.allow(quiet + FORCED_KEYFRAME_MIN_INTERVAL));
    }
}

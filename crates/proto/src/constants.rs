use std::time::Duration;
pub const PROTOCOL_VERSION: u8 = 1;
pub const MEDIA_ALPN: &[u8] = b"brp/media/1";
pub const TICKET_KIND: &str = "brp";
pub const MAX_FRAME_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_CONTROL_BYTES: usize = 64 * 1024;
/// Frames buffered between the QUIC reader and the decoder before back-pressure reaches the publisher.
pub const RECEIVE_QUEUE_FRAMES: usize = 8;
pub const SENDER_BACKLOG_FRAMES: usize = 2;
pub const FORCED_KEYFRAME_MIN_INTERVAL: Duration = Duration::from_secs(1);
pub const REORDER_MAX_WAIT: Duration = Duration::from_millis(200);
pub const RESUBSCRIBE_BACKOFF_INITIAL: Duration = Duration::from_secs(1);
pub const RESUBSCRIBE_BACKOFF_MAX: Duration = Duration::from_secs(30);
pub const ENCODER_IDLE_STOP_GRACE: Duration = Duration::from_secs(5);
/// Compositors deliver frames only on damage, so a viewer joining while the screen is static would
/// never see its requested keyframe. After this long without a new frame the last one is re-encoded.
pub const IDLE_KEYFRAME_RETRY: Duration = Duration::from_millis(500);
pub const MIN_BITRATE_KBPS: u32 = 1_000;
pub const MAX_BITRATE_KBPS: u32 = 250_000;
pub const RELAY_ONLINE_TIMEOUT: Duration = Duration::from_secs(5);
pub const STATS_LOG_INTERVAL: Duration = Duration::from_secs(2);
/// The compositor negotiates the stream format within a frame or two; this covers a slow first connection.
pub const PORTAL_FORMAT_TIMEOUT: Duration = Duration::from_secs(10);
pub const PRESENCE_HEARTBEAT: Duration = Duration::from_secs(5);
/// Four missed heartbeats before a peer vanishes from the room.
pub const MEMBER_EXPIRY: Duration = Duration::from_secs(20);
/// Keeps a presence message under gossip's 4 KB default cap.
pub const MAX_LIVES_PER_PARTICIPANT: usize = 8;
pub const MAX_PRESETS_PER_LIVE: usize = 6;
/// Bounds how late an idle encoder is noticed relative to the stop grace.
pub const REGISTRY_HOUSEKEEPING: Duration = Duration::from_secs(1);
/// Three heartbeats to reach the first neighbour before a join is reported failed.
pub const JOIN_TIMEOUT: Duration = Duration::from_secs(15);
/// Derived preset heights offered when smaller than the source.
pub const TEMPLATE_HEIGHTS: [u32; 3] = [1080, 720, 480];
pub const NICKNAME_MAX_LEN: usize = 32;
/// QUIC application close code the media server uses for callers outside the room.
pub const REFUSED_NOT_MEMBER: u32 = 1;
pub const SOURCE_PRESET_ID: u32 = 1;
/// Compositor capture timestamps jitter by well under a millisecond; a frame this close to its
/// slot is treated as on time rather than skipped.
pub const PACER_JITTER_TOLERANCE: Duration = Duration::from_millis(1);
/// Graphics Capture normally delivers its first frame within milliseconds; a monitor still silent
/// after this is served by desktop duplication instead.
pub const CAPTURE_FALLBACK_TIMEOUT: Duration = Duration::from_secs(2);
pub const AUDIO_SAMPLE_RATE: u32 = 48_000;
pub const AUDIO_CHANNELS: u8 = 2;
/// 20 ms at 48 kHz: the Opus frame the master spec picked.
pub const AUDIO_FRAME_SAMPLES: usize = 960;
pub const AUDIO_PACKET_DURATION: Duration = Duration::from_millis(20);
pub const OPUS_BITRATE_KBPS: u32 = 128;
/// Audio frames have no preset; the master spec reserves zero for them.
pub const AUDIO_PRESET_ID: u32 = 0;
/// 200 ms of slack before a stalled viewer loses audio. Video's backlog of two frames is tuned
/// for keyframe recovery, which audio has no need of.
pub const AUDIO_SENDER_BACKLOG_PACKETS: usize = 10;
pub const JITTER_INITIAL_DEPTH: Duration = Duration::from_millis(60);
pub const JITTER_STEP: Duration = Duration::from_millis(20);
/// Beyond this the added delay annoys more than the dropouts it prevents.
pub const JITTER_MAX_DEPTH: Duration = Duration::from_millis(200);
/// Long enough that a burst of late packets does not make the depth oscillate.
pub const JITTER_SHRINK_AFTER: Duration = Duration::from_secs(10);
/// Room for the jitter maximum plus decode scheduling slack.
pub const MIXER_TRACK_CAPACITY: Duration = Duration::from_millis(500);
/// Opus's largest single-frame packet is 1275 bytes, so this is generous. Without it an audio
/// route would accept `MAX_FRAME_BYTES`, and a peer could park gigabytes in one subscription's
/// channel and jitter buffer.
pub const MAX_AUDIO_PACKET_BYTES: usize = 4096;
/// How long a platform audio backend has to report its capture ready, and how long the PipeWire
/// backend waits for its own stream to become linkable before calling the capture dead. Long
/// enough for a busy daemon, short enough that a wedged one does not hold a subscriber forever.
pub const AUDIO_CAPTURE_START_TIMEOUT: Duration = Duration::from_secs(5);
/// Silence pushed ahead of a track's first packet. The decoder produces 20 ms per 20 ms tick, so
/// a track that starts empty never builds a reserve and every device callback larger than one
/// packet underruns: this covers a 1024-frame quantum (21 ms, PipeWire's stock default) plus the
/// scheduling jitter of the decode thread.
pub const MIXER_TRACK_CUSHION: Duration = Duration::from_millis(80);

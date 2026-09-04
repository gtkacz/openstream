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

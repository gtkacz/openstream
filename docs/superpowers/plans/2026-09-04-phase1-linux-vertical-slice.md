# Phase 1: Linux Vertical Slice Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** One Linux publisher captures a monitor through the desktop portal, hardware-encodes it, and serves it over iroh QUIC to one Linux viewer that decodes and renders it in a native window, joined by a ticket printed on the command line.

**Architecture:** A Cargo workspace of six crates. `proto` holds wire types, `net` wraps iroh with one QUIC stream per frame, `capture` and `codec` hide PipeWire and FFmpeg behind traits with fake implementations for tests, `pipeline` wires capture to encoder to fan-out on the publisher and receive to reorder to decoder to a latest-frame slot on the viewer, and `app` owns a winit loop that renders NV12 through wgpu with an egui stats overlay. Every stage between threads is a bounded hand-off that drops the oldest frame so latency cannot accumulate.

**Tech Stack:** Rust 2024 edition, tokio, iroh 1.1, iroh-tickets 1.0, postcard, ffmpeg-sys-next 9 against system FFmpeg 8, ashpd 0.13 + pipewire 0.10, wgpu 30, winit 0.30, egui 0.36.

**Spec:** `docs/superpowers/specs/2026-09-04-p2p-screen-sharing-design.md`. Read sections 5, 6, 7, and 11 before starting. This plan implements phase 1 of section 11 only.

## Global Constraints

- Linux only in this phase. Every platform-specific module is under `#[cfg(target_os = "linux")]`. No Windows or macOS code.
- Media ALPN is exactly `brp/media/1`. Protocol version is `1`.
- Constants from spec section 6.6 are defined once in `crates/proto/src/constants.rs` and imported everywhere. Never re-type a tuning value.
- Rooms, gossip, presence, membership gating, audio, pop-out windows, and preset switching are later phases. Phase 1 has exactly one live with id `1` and one preset with id `1`. The media server accepts any authenticated caller; membership gating arrives with gossip in phase 2.
- Frames flow between threads through bounded hand-offs that drop the oldest item. No unbounded queue may hold video frames, except the receive path into the reorder buffer, which QUIC flow control already bounds.
- Nothing on the media path panics. Every crate has a `thiserror` error enum and functions return `Result`.
- Comments explain why, never what. Doc comments on public items state the contract. No task or ticket ids in code.
- Commit after every task with a Conventional Commits subject (`feat:`, `test:`, `chore:`, `docs:`), imperative mood, no trailers.
- Run Rust through the system toolchain (`cargo`, stable 1.97). Formatting is `cargo fmt`, linting is `cargo clippy --workspace --all-targets -- -D warnings`. Both must pass before each commit.
- Build prerequisites on the development machine (Fedora 44): `ffmpeg-devel` 8.1, `pipewire-devel` 1.6, the `clang` and `clang-devel` packages for bindgen (only `clang-libs` is installed today, which is not enough), `xdg-desktop-portal` with a desktop backend running. FFmpeg headers must be 7.1 or newer because the swscale flags are an enum from that version on.
- CI runs in a `fedora:44` container so it gets the same FFmpeg 8 and PipeWire 1.6 headers as development. Ubuntu LTS images ship FFmpeg 6.1 and would not build.
- Software fallback encoder is AV1 through `libsvtav1`; software decoders are FFmpeg's `h264`, `hevc`, and `libdav1d`. Never depend on `libx264` in the probe list, because the shipped Windows build will be LGPL.

## File Structure

```
Cargo.toml                          workspace, shared dependency versions
.gitignore                          target/
crates/proto/                       brp-proto: wire types, constants, ticket. No I/O.
  src/lib.rs                        re-exports
  src/constants.rs                  every tuning constant from spec 6.6
  src/messages.rs                   Codec, SourceKind, Preset, CodecParams, ViewerMessage, PublisherMessage, encode/decode
  src/frame.rs                      FrameKind, FrameHeader, EncodedFrame, prefixed framing
  src/clock.rs                      monotonic_us()
  src/bitrate.rs                    default_bitrate_kbps()
  src/ticket.rs                     RoomTicket implementing iroh_tickets::Ticket
  src/error.rs                      ProtoError
crates/codec/                       brp-codec: encoder/decoder traits, fake codec, FFmpeg implementation
  src/lib.rs
  src/raw.rs                        RawFrame (NV12 in CPU memory)
  src/traits.rs                     EncoderConfig, VideoEncoder, VideoDecoder, FrameConverter
  src/fake.rs                       FakeEncoder, FakeDecoder, GrayConverter
  src/error.rs                      CodecError
  src/select.rs                     open_encoder() probe order, open_decoder()
  src/ffmpeg/mod.rs                 module root, one-time av_log level setup
  src/ffmpeg/ffi.rs                 RAII wrappers: Frame, Packet, CodecContext, error mapping
  src/ffmpeg/convert.rs             SwsConverter: BGRA/BGRx -> NV12 with scaling
  src/ffmpeg/encoder.rs             FfmpegEncoder for software-input encoders (nvenc, libsvtav1)
  src/ffmpeg/vaapi.rs               VaapiEncoder: hw device + hw frames upload
  src/ffmpeg/decoder.rs             FfmpegDecoder with optional hw_device_ctx
  tests/codec_smoke.rs              gated real-codec checks
crates/capture/                     brp-capture: capture trait, synthetic source, Linux portal backend
  src/lib.rs
  src/frame.rs                      PixelFormat, CaptureFrame, SourceInfo, SourceRequest, traits
  src/synthetic.rs                  SyntheticSource test pattern generator
  examples/portal_dump.rs           manual capture check
  src/error.rs                      CaptureError
  src/linux/mod.rs                  PortalCapture backend
  src/linux/portal.rs               ashpd ScreenCast session -> (fd, node id)
  src/linux/pipewire.rs             PipeWire main loop thread consuming the stream
crates/net/                         brp-net: iroh endpoint, media server and client
  src/lib.rs
  src/error.rs                      NetError
  src/endpoint.rs                   bind_endpoint(), RelaySetting
  src/framing.rs                    length-prefixed control messages
  src/source.rs                     LiveSource trait, Subscription (what the server pulls frames from)
  src/server.rs                     MediaServer ProtocolHandler
  src/client.rs                     MediaClient, ViewerSubscription
  tests/loopback.rs                 two endpoints over loopback
crates/pipeline/                    brp-pipeline: publisher and viewer wiring
  src/lib.rs
  src/error.rs                      PipelineError
  src/slot.rs                       LatestSlot<T>
  src/fanout.rs                     FanOut with the sender backlog rule, KeyframeRequest
  src/reorder.rs                    Reorder buffer with gap rules
  src/publisher.rs                  Publisher: capture -> convert -> encode -> fan-out, implements LiveSource
  src/viewer.rs                     Viewer: frames -> reorder -> decode -> LatestSlot<RawFrame>
  tests/publisher.rs, tests/viewer.rs
crates/app/                         brp: the binary
  src/main.rs                       clap dispatch, tokio runtime, tracing init
  src/cli.rs                        Cli, PublishArgs, WatchArgs
  src/error.rs                      AppError
  src/identity.rs                   load or create the secret key file
  src/publish.rs                    headless publisher command
  src/watch.rs                      viewer command: connect, subscribe, run window
  src/render/mod.rs                 GpuContext (instance, device, queue)
  src/render/video.rs               VideoRenderer: NV12 textures, aspect-fit quad
  src/render/nv12.wgsl              shader
  src/render/ui.rs                  EguiLayer: egui-winit + egui-wgpu glue
  src/window.rs                     App: winit ApplicationHandler, one window, redraw on NewFrame
.github/workflows/ci.yml            fmt, clippy, tests on ubuntu-latest
README.md                           build prerequisites, usage
```

---

### Task 1: Workspace scaffold and proto wire types

**Files:**
- Create: `Cargo.toml`, `.gitignore`
- Create: `crates/proto/Cargo.toml`, `crates/proto/src/lib.rs`, `crates/proto/src/constants.rs`, `crates/proto/src/error.rs`, `crates/proto/src/messages.rs`, `crates/proto/src/frame.rs`, `crates/proto/src/clock.rs`, `crates/proto/src/bitrate.rs`
- Create empty library crates so the workspace resolves: `crates/{codec,capture,net,pipeline}/Cargo.toml` and `src/lib.rs`, and `crates/app/Cargo.toml` with `src/main.rs`

**Interfaces:**
- Produces: `brp_proto::{constants::*, Codec, SourceKind, Preset, CodecParams, AudioParams, ViewerMessage, PublisherMessage, FrameKind, FrameHeader, EncodedFrame, ProtoError, encode, decode, monotonic_us, default_bitrate_kbps}` with the exact definitions below. Every later task imports these.

- [ ] **Step 1: Create the workspace manifest and gitignore**

`Cargo.toml`:

```toml
[workspace]
resolver = "3"
members = ["crates/*"]

[workspace.package]
version = "0.1.0"
edition = "2024"
license = "MIT"
rust-version = "1.91"

[workspace.dependencies]
brp-proto = { path = "crates/proto" }
brp-codec = { path = "crates/codec" }
brp-capture = { path = "crates/capture" }
brp-net = { path = "crates/net" }
brp-pipeline = { path = "crates/pipeline" }

serde = { version = "1.0", features = ["derive"] }
postcard = { version = "1.1", features = ["alloc"] }
thiserror = "2.0"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
tokio = { version = "1.53", features = ["rt-multi-thread", "macros", "sync", "time", "signal"] }
bytes = "1.12"
```

Later tasks add their own entries (iroh, ffmpeg-sys-next, ashpd, pipewire, wgpu, winit, egui) to this table when they first need them.

`.gitignore`:

```
/target
```

- [ ] **Step 2: Create the five library crates and the binary crate as empty shells**

For each of `codec`, `capture`, `net`, `pipeline`, create `crates/<name>/Cargo.toml`:

```toml
[package]
name = "brp-<name>"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true

[dependencies]
```

and an empty `crates/<name>/src/lib.rs` containing only a crate doc comment such as `//! Codec traits and implementations.` (adapt per crate).

`crates/app/Cargo.toml`:

```toml
[package]
name = "brp"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true

[[bin]]
name = "brp"
path = "src/main.rs"

[dependencies]
```

`crates/app/src/main.rs`:

```rust
fn main() {}
```

- [ ] **Step 3: Write the proto crate manifest**

`crates/proto/Cargo.toml`:

```toml
[package]
name = "brp-proto"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true

[dependencies]
serde.workspace = true
postcard.workspace = true
thiserror.workspace = true
```

- [ ] **Step 4: Write the failing round-trip tests for messages and frame headers**

`crates/proto/src/messages.rs` (tests only for now; the module body comes in step 6):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewer_message_round_trips_through_postcard() {
        let msg = ViewerMessage::Subscribe { live_id: 1, preset_id: 1, want_audio: false };
        let bytes = encode(&msg).unwrap();
        let back: ViewerMessage = decode(&bytes).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn publisher_ack_round_trips_with_extradata() {
        let msg = PublisherMessage::SubscribeAck {
            video: CodecParams { codec: Codec::Hevc, width: 2560, height: 1440, fps: 60, extradata: vec![0, 0, 0, 1, 0x40] },
            audio: None,
        };
        let back: PublisherMessage = decode(&encode(&msg).unwrap()).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn decode_rejects_truncated_input() {
        let bytes = encode(&ViewerMessage::RequestKeyframe).unwrap();
        let err = decode::<ViewerMessage>(&bytes[..bytes.len().saturating_sub(1)]);
        assert!(matches!(err, Err(ProtoError::Decode(_))) || bytes.len() <= 1);
    }

    #[test]
    fn preset_validation_accepts_source_preset() {
        let p = Preset { id: 1, name: "Source".into(), width: 2560, height: 1440, fps: 60, bitrate_kbps: 40_000, codec: Codec::Hevc };
        assert!(p.validate(2560, 1440, 60).is_ok());
    }

    #[test]
    fn preset_validation_rejects_odd_dimensions_and_out_of_range_bitrate() {
        let odd = Preset { id: 1, name: "x".into(), width: 1921, height: 1080, fps: 60, bitrate_kbps: 20_000, codec: Codec::H264 };
        assert_eq!(odd.validate(1920, 1080, 60), Err(PresetError::OddDimension));
        let too_high = Preset { id: 1, name: "x".into(), width: 1920, height: 1080, fps: 60, bitrate_kbps: 900_000, codec: Codec::H264 };
        assert_eq!(too_high.validate(1920, 1080, 60), Err(PresetError::BitrateOutOfRange));
        let upscaled = Preset { id: 1, name: "x".into(), width: 3840, height: 2160, fps: 60, bitrate_kbps: 20_000, codec: Codec::H264 };
        assert_eq!(upscaled.validate(1920, 1080, 60), Err(PresetError::LargerThanSource));
        let too_fast = Preset { id: 1, name: "x".into(), width: 1920, height: 1080, fps: 120, bitrate_kbps: 20_000, codec: Codec::H264 };
        assert_eq!(too_fast.validate(1920, 1080, 60), Err(PresetError::FasterThanSource));
    }
}
```

`crates/proto/src/frame.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn header() -> FrameHeader {
        FrameHeader { live_id: 1, preset_id: 1, kind: FrameKind::Video, seq: 42, capture_ts_us: 1_000_000, keyframe: true, len: 3 }
    }

    #[test]
    fn prefixed_frame_splits_into_header_and_payload() {
        let mut bytes = header().encode_prefix().unwrap();
        bytes.extend_from_slice(&[9, 8, 7]);
        let (h, payload) = FrameHeader::decode_prefixed(&bytes).unwrap();
        assert_eq!(h, header());
        assert_eq!(payload, &[9, 8, 7]);
    }

    #[test]
    fn prefixed_frame_rejects_length_mismatch() {
        let mut bytes = header().encode_prefix().unwrap();
        bytes.extend_from_slice(&[9, 8]);
        assert!(matches!(FrameHeader::decode_prefixed(&bytes), Err(ProtoError::LengthMismatch { declared: 3, actual: 2 })));
    }

    #[test]
    fn prefixed_frame_rejects_oversized_declared_length() {
        let mut h = header();
        h.len = (crate::constants::MAX_FRAME_BYTES + 1) as u32;
        let bytes = h.encode_prefix().unwrap();
        assert!(matches!(FrameHeader::decode_prefixed(&bytes), Err(ProtoError::FrameTooLarge(_))));
    }
}
```

`crates/proto/src/bitrate.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_anchors_match_moonlight_table() {
        assert_eq!(default_bitrate_kbps(1280, 720, 60), 10_000);
        assert_eq!(default_bitrate_kbps(1920, 1080, 60), 20_000);
        assert_eq!(default_bitrate_kbps(2560, 1440, 60), 40_000);
        assert_eq!(default_bitrate_kbps(3840, 2160, 60), 80_000);
    }

    #[test]
    fn scales_linearly_with_frame_rate() {
        assert_eq!(default_bitrate_kbps(1920, 1080, 30), 10_000);
        assert_eq!(default_bitrate_kbps(1920, 1080, 120), 40_000);
    }

    #[test]
    fn interpolates_between_anchors_and_clamps_to_limits() {
        let mid = default_bitrate_kbps(2048, 1080, 60);
        assert!(mid > 20_000 && mid < 40_000, "got {mid}");
        assert_eq!(default_bitrate_kbps(320, 200, 1), crate::constants::MIN_BITRATE_KBPS);
        assert_eq!(default_bitrate_kbps(7680, 4320, 240), crate::constants::MAX_BITRATE_KBPS);
    }
}
```

- [ ] **Step 5: Run the tests to verify they fail to compile**

Run: `cargo test -p brp-proto`
Expected: compilation errors naming `ViewerMessage`, `FrameHeader`, `default_bitrate_kbps` as unresolved.

- [ ] **Step 6: Write the proto modules**

`crates/proto/src/constants.rs`:

```rust
use std::time::Duration;

pub const PROTOCOL_VERSION: u8 = 1;
pub const MEDIA_ALPN: &[u8] = b"brp/media/1";
pub const TICKET_KIND: &str = "brp";

/// A 4K keyframe at the maximum bitrate is a few megabytes; anything past this is a protocol violation.
pub const MAX_FRAME_BYTES: usize = 32 * 1024 * 1024;
/// Control messages carry at most codec extradata, which is under a kilobyte.
pub const MAX_CONTROL_BYTES: usize = 64 * 1024;

pub const SENDER_BACKLOG_FRAMES: usize = 2;
pub const FORCED_KEYFRAME_MIN_INTERVAL: Duration = Duration::from_secs(1);
pub const REORDER_MAX_WAIT: Duration = Duration::from_millis(200);
pub const RESUBSCRIBE_BACKOFF_INITIAL: Duration = Duration::from_secs(1);
pub const RESUBSCRIBE_BACKOFF_MAX: Duration = Duration::from_secs(30);
pub const ENCODER_IDLE_STOP_GRACE: Duration = Duration::from_secs(5);
pub const MIN_BITRATE_KBPS: u32 = 1_000;
pub const MAX_BITRATE_KBPS: u32 = 250_000;
```

`crates/proto/src/error.rs`:

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProtoError {
    #[error("failed to encode message: {0}")]
    Encode(postcard::Error),
    #[error("failed to decode message: {0}")]
    Decode(postcard::Error),
    #[error("frame declares {declared} bytes but {actual} follow the header")]
    LengthMismatch { declared: u32, actual: usize },
    #[error("frame of {0} bytes exceeds the maximum frame size")]
    FrameTooLarge(u32),
    #[error("ticket is malformed: {0}")]
    Ticket(String),
}
```

`crates/proto/src/messages.rs` (above the tests from step 4):

```rust
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use thiserror::Error;

use crate::constants::{MAX_BITRATE_KBPS, MIN_BITRATE_KBPS};
use crate::error::ProtoError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Codec {
    H264,
    Hevc,
    Av1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceKind {
    Monitor,
    Window,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preset {
    pub id: u32,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate_kbps: u32,
    pub codec: Codec,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PresetError {
    #[error("width and height must be even for 4:2:0 encoding")]
    OddDimension,
    #[error("preset exceeds the source resolution")]
    LargerThanSource,
    #[error("preset frame rate exceeds the source frame rate")]
    FasterThanSource,
    #[error("bitrate must be between {MIN_BITRATE_KBPS} and {MAX_BITRATE_KBPS} kbps")]
    BitrateOutOfRange,
}

impl Preset {
    pub fn validate(&self, source_width: u32, source_height: u32, source_fps: u32) -> Result<(), PresetError> {
        if self.width == 0 || self.height == 0 || self.width % 2 != 0 || self.height % 2 != 0 {
            return Err(PresetError::OddDimension);
        }
        if self.width > source_width || self.height > source_height {
            return Err(PresetError::LargerThanSource);
        }
        if self.fps == 0 || self.fps > source_fps {
            return Err(PresetError::FasterThanSource);
        }
        if !(MIN_BITRATE_KBPS..=MAX_BITRATE_KBPS).contains(&self.bitrate_kbps) {
            return Err(PresetError::BitrateOutOfRange);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodecParams {
    pub codec: Codec,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub extradata: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioParams {
    pub sample_rate: u32,
    pub channels: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ViewerMessage {
    Subscribe { live_id: u32, preset_id: u32, want_audio: bool },
    SwitchPreset { preset_id: u32 },
    RequestKeyframe,
    Unsubscribe,
    Stats { frames_received: u32, frames_dropped: u32, decode_fps: u16, rtt_ms: u16 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PublisherMessage {
    SubscribeAck { video: CodecParams, audio: Option<AudioParams> },
    SubscribeError { reason: String },
    PresetSwitched { preset_id: u32, video: CodecParams },
    LiveEnded,
}

pub fn encode<T: Serialize>(msg: &T) -> Result<Vec<u8>, ProtoError> {
    postcard::to_allocvec(msg).map_err(ProtoError::Encode)
}

pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, ProtoError> {
    postcard::from_bytes(bytes).map_err(ProtoError::Decode)
}
```

`crates/proto/src/frame.rs` (above its tests):

```rust
use serde::{Deserialize, Serialize};

use crate::constants::MAX_FRAME_BYTES;
use crate::error::ProtoError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FrameKind {
    Video,
    Audio,
}

/// Prefix of every frame stream. `len` is the payload length that follows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameHeader {
    pub live_id: u32,
    pub preset_id: u32,
    pub kind: FrameKind,
    pub seq: u64,
    pub capture_ts_us: u64,
    pub keyframe: bool,
    pub len: u32,
}

/// One compressed frame as produced by an encoder, before any viewer-specific header is attached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedFrame {
    pub seq: u64,
    pub capture_ts_us: u64,
    pub keyframe: bool,
    pub data: Vec<u8>,
}

impl FrameHeader {
    pub fn encode_prefix(&self) -> Result<Vec<u8>, ProtoError> {
        postcard::to_allocvec(self).map_err(ProtoError::Encode)
    }

    /// Splits a complete frame stream into its header and payload, validating the declared length.
    pub fn decode_prefixed(bytes: &[u8]) -> Result<(FrameHeader, &[u8]), ProtoError> {
        let (header, payload): (FrameHeader, &[u8]) = postcard::take_from_bytes(bytes).map_err(ProtoError::Decode)?;
        if header.len as usize > MAX_FRAME_BYTES {
            return Err(ProtoError::FrameTooLarge(header.len));
        }
        if payload.len() != header.len as usize {
            return Err(ProtoError::LengthMismatch { declared: header.len, actual: payload.len() });
        }
        Ok((header, payload))
    }
}
```

`crates/proto/src/clock.rs`:

```rust
use std::sync::OnceLock;
use std::time::Instant;

static EPOCH: OnceLock<Instant> = OnceLock::new();

/// Microseconds since the first call in this process. Monotonic, never wall-clock, so it survives NTP jumps.
pub fn monotonic_us() -> u64 {
    let epoch = EPOCH.get_or_init(Instant::now);
    epoch.elapsed().as_micros() as u64
}
```

`crates/proto/src/bitrate.rs` (above its tests):

```rust
use crate::constants::{MAX_BITRATE_KBPS, MIN_BITRATE_KBPS};

/// Moonlight's default table at 60 fps, as (pixel count, kbps). Interpolated linearly between rows.
const ANCHORS_60FPS: [(u64, u64); 4] = [
    (1280 * 720, 10_000),
    (1920 * 1080, 20_000),
    (2560 * 1440, 40_000),
    (3840 * 2160, 80_000),
];

pub fn default_bitrate_kbps(width: u32, height: u32, fps: u32) -> u32 {
    let pixels = u64::from(width) * u64::from(height);
    let at_60 = interpolate(pixels);
    let scaled = at_60 * u64::from(fps) / 60;
    scaled.clamp(u64::from(MIN_BITRATE_KBPS), u64::from(MAX_BITRATE_KBPS)) as u32
}

fn interpolate(pixels: u64) -> u64 {
    let (first_px, first_kbps) = ANCHORS_60FPS[0];
    if pixels <= first_px {
        return first_kbps * pixels / first_px;
    }
    for window in ANCHORS_60FPS.windows(2) {
        let (lo_px, lo_kbps) = window[0];
        let (hi_px, hi_kbps) = window[1];
        if pixels <= hi_px {
            return lo_kbps + (hi_kbps - lo_kbps) * (pixels - lo_px) / (hi_px - lo_px);
        }
    }
    let (last_px, last_kbps) = ANCHORS_60FPS[ANCHORS_60FPS.len() - 1];
    last_kbps * pixels / last_px
}
```

`crates/proto/src/lib.rs`:

```rust
//! Wire types shared by every brp crate. No I/O lives here.

pub mod bitrate;
pub mod clock;
pub mod constants;
pub mod error;
pub mod frame;
pub mod messages;

pub use bitrate::default_bitrate_kbps;
pub use clock::monotonic_us;
pub use error::ProtoError;
pub use frame::{EncodedFrame, FrameHeader, FrameKind};
pub use messages::{
    AudioParams, Codec, CodecParams, Preset, PresetError, PublisherMessage, SourceKind, ViewerMessage, decode, encode,
};
```

- [ ] **Step 7: Run the tests and verify they pass**

Run: `cargo test -p brp-proto`
Expected: all tests in `messages::tests`, `frame::tests`, `bitrate::tests` pass. Then `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings` with no output.

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml Cargo.lock .gitignore crates/
git commit -m "feat: scaffold workspace and proto wire types"
```

### Task 2: Room ticket

**Files:**
- Modify: `Cargo.toml` (workspace dependencies), `crates/proto/Cargo.toml`, `crates/proto/src/lib.rs`
- Create: `crates/proto/src/ticket.rs`

**Interfaces:**
- Consumes: `brp_proto::constants::TICKET_KIND`.
- Produces: `brp_proto::RoomTicket { pub topic: [u8; 32], pub bootstrap: Vec<iroh_base::EndpointAddr> }` with `RoomTicket::new(topic, bootstrap)`, `RoomTicket::random_topic() -> [u8; 32]`, `impl iroh_tickets::Ticket`, `impl Display` (string form starts with `brp`), `impl FromStr<Err = iroh_tickets::ParseError>`. The app prints `ticket.to_string()` and parses with `RoomTicket::from_str`.

Background: `iroh_tickets::Ticket` (1.0.0) is `const KIND: &'static str; fn encode_bytes(&self) -> Vec<u8>; fn decode_bytes(bytes: &[u8]) -> Result<Self, ParseError>;` with provided `encode_string`/`decode_string` that prepend `KIND` and base32-encode. `ParseError` has `From<postcard::Error>` and `ParseError::verification_failed(&'static str)`. `EndpointAddr` derives serde. The wire format is wrapped in a versioned enum so phase 2 can extend the ticket without breaking old strings.

- [ ] **Step 1: Add dependencies**

Append to `[workspace.dependencies]` in the root `Cargo.toml`:

```toml
iroh = { version = "1.1", default-features = false, features = ["tls-ring", "portmapper"] }
iroh-base = { version = "1.1", features = ["key"] }
iroh-tickets = "1.0"
rand = "0.10"
```

In `crates/proto/Cargo.toml` add under `[dependencies]`:

```toml
iroh-base.workspace = true
iroh-tickets.workspace = true
rand.workspace = true
```

- [ ] **Step 2: Write the failing tests**

`crates/proto/src/ticket.rs`:

```rust
#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::str::FromStr;

    use iroh_base::{EndpointAddr, SecretKey};

    use super::*;

    fn sample_addr() -> EndpointAddr {
        let id = SecretKey::from_bytes(&[7u8; 32]).public();
        EndpointAddr::new(id).with_ip_addr(SocketAddr::from(([192, 168, 1, 10], 4433)))
    }

    #[test]
    fn ticket_string_round_trips_and_carries_the_kind_prefix() {
        let ticket = RoomTicket::new([1u8; 32], vec![sample_addr()]);
        let text = ticket.to_string();
        assert!(text.starts_with("brp"), "got {text}");
        assert_eq!(RoomTicket::from_str(&text).unwrap(), ticket);
    }

    #[test]
    fn ticket_rejects_foreign_kind() {
        let err = RoomTicket::from_str("endpointaaaaaaaaaaaaaaaa");
        assert!(matches!(err, Err(iroh_tickets::ParseError::Kind { .. })));
    }

    #[test]
    fn ticket_rejects_empty_bootstrap_list() {
        let text = RoomTicket::new([1u8; 32], vec![]).to_string();
        assert!(matches!(RoomTicket::from_str(&text), Err(iroh_tickets::ParseError::Verify { .. })));
    }

    #[test]
    fn random_topics_differ() {
        assert_ne!(RoomTicket::random_topic(), RoomTicket::random_topic());
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p brp-proto ticket`
Expected: compile errors, `RoomTicket` unresolved.

- [ ] **Step 4: Implement the ticket**

Prepend to `crates/proto/src/ticket.rs`:

```rust
use std::fmt;
use std::str::FromStr;

use iroh_base::EndpointAddr;
use iroh_tickets::{ParseError, Ticket};
use serde::{Deserialize, Serialize};

use crate::constants::TICKET_KIND;

/// Everything a newcomer needs to reach a room: the gossip topic and at least one peer already in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomTicket {
    pub topic: [u8; 32],
    pub bootstrap: Vec<EndpointAddr>,
}

/// Versioned envelope so a future ticket layout can coexist with strings already shared.
#[derive(Serialize, Deserialize)]
enum TicketWireFormat {
    V1(RoomTicket),
}

impl RoomTicket {
    pub fn new(topic: [u8; 32], bootstrap: Vec<EndpointAddr>) -> Self {
        Self { topic, bootstrap }
    }

    pub fn random_topic() -> [u8; 32] {
        rand::random()
    }
}

impl Ticket for RoomTicket {
    const KIND: &'static str = TICKET_KIND;

    fn encode_bytes(&self) -> Vec<u8> {
        postcard::to_allocvec(&TicketWireFormat::V1(self.clone())).expect("ticket types serialize infallibly")
    }

    fn decode_bytes(bytes: &[u8]) -> Result<Self, ParseError> {
        let TicketWireFormat::V1(ticket) = postcard::from_bytes(bytes)?;
        if ticket.bootstrap.is_empty() {
            return Err(ParseError::verification_failed("ticket lists no bootstrap peers"));
        }
        Ok(ticket)
    }
}

impl fmt::Display for RoomTicket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&Ticket::encode_string(self))
    }
}

impl FromStr for RoomTicket {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ticket::decode_string(s)
    }
}
```

Add to `crates/proto/src/lib.rs`:

```rust
pub mod ticket;
pub use ticket::RoomTicket;
```

- [ ] **Step 5: Run the tests and verify they pass**

Run: `cargo test -p brp-proto`
Expected: all pass, including the four ticket tests. Then `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock crates/proto
git commit -m "feat: add versioned room ticket"
```

### Task 3: Codec traits, NV12 frame type, and the fake codec

**Files:**
- Modify: `crates/proto/src/lib.rs`; Create: `crates/proto/src/pixel.rs`
- Modify: `crates/codec/Cargo.toml`, `crates/codec/src/lib.rs`
- Create: `crates/codec/src/raw.rs`, `crates/codec/src/traits.rs`, `crates/codec/src/error.rs`, `crates/codec/src/fake.rs`

**Interfaces:**
- Consumes: `brp_proto::{Codec, CodecParams, EncodedFrame}`.
- Produces: `brp_proto::PixelFormat { Bgra, Bgrx, Rgba, Rgbx }`; `brp_codec::RawFrame { width, height, y_stride, uv_stride, y: Vec<u8>, uv: Vec<u8>, capture_ts_us }` with `RawFrame::black(w, h, ts)` and `validate()`; `brp_codec::EncoderConfig { width, height, fps, bitrate_kbps, codec }`; traits `VideoEncoder { name(), params(), encode(&RawFrame, force_keyframe) -> Result<Vec<EncodedFrame>> }`, `VideoDecoder { decode(&EncodedFrame) -> Result<Vec<RawFrame>> }`, `FrameConverter { convert(&InputImage) -> Result<RawFrame> }`; `brp_codec::InputImage<'a> { width, height, stride, format: PixelFormat, data: &'a [u8], capture_ts_us }`; `brp_codec::CodecError`; `brp_codec::fake::{FakeEncoder::new(cfg, keyframe_interval), FakeDecoder, SolidConverter::new(w, h)}`.

- [ ] **Step 1: Add the pixel format to proto**

`crates/proto/src/pixel.rs`:

```rust
use serde::{Deserialize, Serialize};

/// Byte order of a captured 32-bit pixel. The `x` variants carry an ignored fourth byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PixelFormat {
    Bgra,
    Bgrx,
    Rgba,
    Rgbx,
}

impl PixelFormat {
    pub const fn bytes_per_pixel(self) -> usize {
        4
    }
}
```

Add to `crates/proto/src/lib.rs`: `pub mod pixel;` and `pub use pixel::PixelFormat;`.

- [ ] **Step 2: Codec crate manifest**

`crates/codec/Cargo.toml` dependencies:

```toml
[dependencies]
brp-proto.workspace = true
serde.workspace = true
postcard.workspace = true
thiserror.workspace = true
tracing.workspace = true
```

- [ ] **Step 3: Write the failing tests**

`crates/codec/src/raw.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn black_frame_has_limited_range_black_and_tight_strides() {
        let f = RawFrame::black(6, 4, 77);
        assert_eq!((f.y_stride, f.uv_stride), (6, 6));
        assert_eq!(f.y.len(), 24);
        assert_eq!(f.uv.len(), 12);
        assert!(f.y.iter().all(|&v| v == 16));
        assert!(f.uv.iter().all(|&v| v == 128));
        assert_eq!(f.capture_ts_us, 77);
        assert!(f.validate().is_ok());
    }

    #[test]
    fn validate_rejects_short_buffers_and_odd_sizes() {
        let mut f = RawFrame::black(6, 4, 0);
        f.y.pop();
        assert!(matches!(f.validate(), Err(CodecError::InvalidFrame(_))));
        let odd = RawFrame { width: 5, ..RawFrame::black(6, 4, 0) };
        assert!(matches!(odd.validate(), Err(CodecError::InvalidFrame(_))));
    }
}
```

`crates/codec/src/fake.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use brp_proto::Codec;

    use super::*;
    use crate::traits::{EncoderConfig, FrameConverter, VideoDecoder, VideoEncoder};

    fn cfg() -> EncoderConfig {
        EncoderConfig { width: 8, height: 4, fps: 30, bitrate_kbps: 2_000, codec: Codec::H264 }
    }

    #[test]
    fn fake_codec_round_trips_frames_and_numbers_them() {
        let mut enc = FakeEncoder::new(cfg(), 3);
        let mut dec = FakeDecoder;
        for i in 0..5u64 {
            let frame = RawFrame::black(8, 4, i * 1000);
            let packets = enc.encode(&frame, false).unwrap();
            assert_eq!(packets.len(), 1);
            assert_eq!(packets[0].seq, i);
            assert_eq!(packets[0].capture_ts_us, i * 1000);
            let decoded = dec.decode(&packets[0]).unwrap();
            assert_eq!(decoded, vec![frame]);
        }
    }

    #[test]
    fn keyframes_follow_the_interval_and_can_be_forced() {
        let mut enc = FakeEncoder::new(cfg(), 3);
        let flags: Vec<bool> = (0..7).map(|_| enc.encode(&RawFrame::black(8, 4, 0), false).unwrap()[0].keyframe).collect();
        assert_eq!(flags, vec![true, false, false, true, false, false, true]);
        assert!(enc.encode(&RawFrame::black(8, 4, 0), true).unwrap()[0].keyframe);
    }

    #[test]
    fn decoder_rejects_garbage() {
        let bad = brp_proto::EncodedFrame { seq: 0, capture_ts_us: 0, keyframe: true, data: vec![0xff, 0x00, 0x13] };
        assert!(matches!(FakeDecoder.decode(&bad), Err(CodecError::FakePayload(_))));
    }

    #[test]
    fn solid_converter_produces_target_size_and_keeps_timestamp() {
        let mut conv = SolidConverter::new(4, 2);
        let pixels = vec![0u8; 16 * 8 * 4];
        let img = InputImage { width: 16, height: 8, stride: 64, format: brp_proto::PixelFormat::Bgra, data: &pixels, capture_ts_us: 5 };
        let out = conv.convert(&img).unwrap();
        assert_eq!((out.width, out.height, out.capture_ts_us), (4, 2, 5));
        assert!(out.validate().is_ok());
    }
}
```

- [ ] **Step 4: Run the tests to verify they fail**

Run: `cargo test -p brp-codec`
Expected: compile errors for `RawFrame`, `FakeEncoder`, `SolidConverter`.

- [ ] **Step 5: Implement the modules**

`crates/codec/src/error.rs`:

```rust
use brp_proto::Codec;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CodecError {
    #[error("no {0:?} encoder could be opened on this machine")]
    NoEncoder(Codec),
    #[error("no {0:?} decoder could be opened on this machine")]
    NoDecoder(Codec),
    #[error("{call} failed with code {code}: {message}")]
    Ffmpeg { call: &'static str, code: i32, message: String },
    #[error("invalid frame: {0}")]
    InvalidFrame(String),
    #[error("fake codec payload is corrupt: {0}")]
    FakePayload(postcard::Error),
}
```

`crates/codec/src/raw.rs` (above its tests):

```rust
use serde::{Deserialize, Serialize};

use crate::error::CodecError;

/// A picture in NV12 layout: full-resolution luma plane, then interleaved half-resolution chroma.
/// Strides may exceed the width when a decoder pads rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawFrame {
    pub width: u32,
    pub height: u32,
    pub y_stride: usize,
    pub uv_stride: usize,
    pub y: Vec<u8>,
    pub uv: Vec<u8>,
    pub capture_ts_us: u64,
}

/// Limited-range black: luma 16, chroma 128.
const BLACK_LUMA: u8 = 16;
const NEUTRAL_CHROMA: u8 = 128;

impl RawFrame {
    pub fn black(width: u32, height: u32, capture_ts_us: u64) -> Self {
        let y_stride = width as usize;
        let chroma_rows = (height as usize).div_ceil(2);
        Self {
            width,
            height,
            y_stride,
            uv_stride: y_stride,
            y: vec![BLACK_LUMA; y_stride * height as usize],
            uv: vec![NEUTRAL_CHROMA; y_stride * chroma_rows],
            capture_ts_us,
        }
    }

    pub fn chroma_rows(&self) -> usize {
        (self.height as usize).div_ceil(2)
    }

    pub fn validate(&self) -> Result<(), CodecError> {
        if self.width == 0 || self.height == 0 || self.width % 2 != 0 || self.height % 2 != 0 {
            return Err(CodecError::InvalidFrame(format!("dimensions {}x{} must be even and non-zero", self.width, self.height)));
        }
        if self.y_stride < self.width as usize || self.uv_stride < self.width as usize {
            return Err(CodecError::InvalidFrame("stride shorter than width".into()));
        }
        if self.y.len() < self.y_stride * self.height as usize {
            return Err(CodecError::InvalidFrame("luma buffer shorter than stride * height".into()));
        }
        if self.uv.len() < self.uv_stride * self.chroma_rows() {
            return Err(CodecError::InvalidFrame("chroma buffer shorter than stride * rows".into()));
        }
        Ok(())
    }
}
```

`crates/codec/src/traits.rs`:

```rust
use brp_proto::{Codec, CodecParams, EncodedFrame, PixelFormat};

use crate::error::CodecError;
use crate::raw::RawFrame;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncoderConfig {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate_kbps: u32,
    pub codec: Codec,
}

/// Borrowed view of captured pixels handed to a converter.
#[derive(Debug, Clone, Copy)]
pub struct InputImage<'a> {
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub format: PixelFormat,
    pub data: &'a [u8],
    pub capture_ts_us: u64,
}

pub trait VideoEncoder: Send {
    /// Implementation name shown in the UI, so a silent fall back to software is visible.
    fn name(&self) -> &'static str;
    fn params(&self) -> CodecParams;
    /// Low-latency encoders return exactly one packet per frame; the Vec covers encoders that buffer at start-up.
    fn encode(&mut self, frame: &RawFrame, force_keyframe: bool) -> Result<Vec<EncodedFrame>, CodecError>;
}

pub trait VideoDecoder: Send {
    fn decode(&mut self, frame: &EncodedFrame) -> Result<Vec<RawFrame>, CodecError>;
}

/// Scales and converts captured pixels into the NV12 input of one preset's encoder.
pub trait FrameConverter: Send {
    fn convert(&mut self, src: &InputImage<'_>) -> Result<RawFrame, CodecError>;
}
```

`crates/codec/src/fake.rs` (above its tests):

```rust
//! Pass-through codec for tests: the "bitstream" is the postcard encoding of the frame itself.

use brp_proto::{CodecParams, EncodedFrame};

use crate::error::CodecError;
use crate::raw::RawFrame;
use crate::traits::{EncoderConfig, FrameConverter, InputImage, VideoDecoder, VideoEncoder};

pub struct FakeEncoder {
    cfg: EncoderConfig,
    next_seq: u64,
    keyframe_interval: u64,
}

impl FakeEncoder {
    pub fn new(cfg: EncoderConfig, keyframe_interval: u32) -> Self {
        Self { cfg, next_seq: 0, keyframe_interval: u64::from(keyframe_interval.max(1)) }
    }
}

impl VideoEncoder for FakeEncoder {
    fn name(&self) -> &'static str {
        "fake"
    }

    fn params(&self) -> CodecParams {
        CodecParams { codec: self.cfg.codec, width: self.cfg.width, height: self.cfg.height, fps: self.cfg.fps, extradata: b"fake".to_vec() }
    }

    fn encode(&mut self, frame: &RawFrame, force_keyframe: bool) -> Result<Vec<EncodedFrame>, CodecError> {
        let keyframe = force_keyframe || self.next_seq % self.keyframe_interval == 0;
        let data = postcard::to_allocvec(frame).map_err(CodecError::FakePayload)?;
        let packet = EncodedFrame { seq: self.next_seq, capture_ts_us: frame.capture_ts_us, keyframe, data };
        self.next_seq += 1;
        Ok(vec![packet])
    }
}

pub struct FakeDecoder;

impl VideoDecoder for FakeDecoder {
    fn decode(&mut self, frame: &EncodedFrame) -> Result<Vec<RawFrame>, CodecError> {
        let raw: RawFrame = postcard::from_bytes(&frame.data).map_err(CodecError::FakePayload)?;
        Ok(vec![raw])
    }
}

/// Ignores the pixels and emits a flat frame at the target size. Lets pipeline tests run without swscale.
pub struct SolidConverter {
    width: u32,
    height: u32,
}

impl SolidConverter {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

impl FrameConverter for SolidConverter {
    fn convert(&mut self, src: &InputImage<'_>) -> Result<RawFrame, CodecError> {
        Ok(RawFrame::black(self.width, self.height, src.capture_ts_us))
    }
}
```

`crates/codec/src/lib.rs`:

```rust
//! Video encoder and decoder traits with a fake implementation for tests and an FFmpeg implementation for real use.

pub mod error;
pub mod fake;
pub mod raw;
pub mod traits;

pub use error::CodecError;
pub use raw::RawFrame;
pub use traits::{EncoderConfig, FrameConverter, InputImage, VideoDecoder, VideoEncoder};
```

- [ ] **Step 6: Run the tests and verify they pass**

Run: `cargo test -p brp-codec -p brp-proto`
Expected: all pass. Then `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`.

- [ ] **Step 7: Commit**

```bash
git add crates/proto crates/codec
git commit -m "feat: add codec traits, NV12 frame type, and fake codec"
```

### Task 4: Capture traits and the synthetic test source

**Files:**
- Modify: `crates/capture/Cargo.toml`, `crates/capture/src/lib.rs`
- Create: `crates/capture/src/frame.rs`, `crates/capture/src/error.rs`, `crates/capture/src/synthetic.rs`

**Interfaces:**
- Consumes: `brp_proto::{PixelFormat, SourceKind, monotonic_us}`.
- Produces: `brp_capture::CaptureFrame { width, height, stride, format: PixelFormat, data: Vec<u8>, capture_ts_us }`; `SourceInfo { width, height, fps }`; `SourceRequest { kind: SourceKind, target_fps: u32 }`; `FrameSink = Box<dyn FnMut(CaptureFrame) + Send + 'static>`; `trait CaptureBackend: Send + Sync { fn start(&self, request: SourceRequest, sink: FrameSink) -> StartFuture<'_>; }` where `StartFuture<'a> = Pin<Box<dyn Future<Output = Result<Box<dyn CaptureSession>, CaptureError>> + Send + 'a>>`; `trait CaptureSession: Send { fn info(&self) -> SourceInfo; fn stop(self: Box<Self>); }`; `CaptureError`; `SyntheticSource { width, height, fps }` whose frames encode their index little-endian in the first pixel.

- [ ] **Step 1: Manifest**

`crates/capture/Cargo.toml`:

```toml
[dependencies]
brp-proto.workspace = true
thiserror.workspace = true
tracing.workspace = true

[dev-dependencies]
tokio = { workspace = true, features = ["rt", "macros", "time"] }
```

- [ ] **Step 2: Write the failing test**

`crates/capture/src/synthetic.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use brp_proto::SourceKind;

    use super::*;
    use crate::frame::{CaptureBackend, SourceRequest};

    #[tokio::test]
    async fn synthetic_source_paces_frames_and_numbers_them() {
        let frames = Arc::new(Mutex::new(Vec::new()));
        let sink_frames = frames.clone();
        let source = SyntheticSource { width: 64, height: 32, fps: 100 };
        let session = source
            .start(SourceRequest { kind: SourceKind::Monitor, target_fps: 100 }, Box::new(move |f| sink_frames.lock().unwrap().push(f)))
            .await
            .unwrap();
        assert_eq!(session.info(), SourceInfo { width: 64, height: 32, fps: 100 });

        tokio::time::sleep(Duration::from_millis(120)).await;
        session.stop();
        let count_at_stop = frames.lock().unwrap().len();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(frames.lock().unwrap().len(), count_at_stop, "frames arrived after stop");

        let frames = frames.lock().unwrap();
        assert!(frames.len() >= 6, "only {} frames in 120 ms at 100 fps", frames.len());
        for (i, f) in frames.iter().enumerate() {
            assert_eq!((f.width, f.height, f.stride, f.format), (64, 32, 256, brp_proto::PixelFormat::Bgra));
            assert_eq!(f.data.len(), 256 * 32);
            assert_eq!(u32::from_le_bytes([f.data[0], f.data[1], f.data[2], f.data[3]]), i as u32);
        }
        assert!(frames.windows(2).all(|w| w[1].capture_ts_us > w[0].capture_ts_us));
    }
}
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test -p brp-capture`
Expected: compile errors for `SyntheticSource`, `SourceRequest`.

- [ ] **Step 4: Implement**

`crates/capture/src/error.rs`:

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CaptureError {
    #[error("the desktop portal request was denied or cancelled")]
    PortalDenied,
    #[error("desktop portal call failed: {0}")]
    Portal(String),
    #[error("PipeWire error: {0}")]
    PipeWire(String),
    #[error("the source stopped delivering frames: {0}")]
    SourceLost(String),
    #[error("unsupported pixel format from the compositor: {0}")]
    UnsupportedFormat(String),
}
```

`crates/capture/src/frame.rs`:

```rust
use std::future::Future;
use std::pin::Pin;

use brp_proto::{PixelFormat, SourceKind};

use crate::error::CaptureError;

/// One captured picture in 32-bit packed pixels, rows `stride` bytes apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureFrame {
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub format: PixelFormat,
    pub data: Vec<u8>,
    pub capture_ts_us: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceInfo {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceRequest {
    pub kind: SourceKind,
    pub target_fps: u32,
}

pub type FrameSink = Box<dyn FnMut(CaptureFrame) + Send + 'static>;

pub type StartFuture<'a> = Pin<Box<dyn Future<Output = Result<Box<dyn CaptureSession>, CaptureError>> + Send + 'a>>;

pub trait CaptureBackend: Send + Sync {
    /// Resolves only once the source format is negotiated, so `info()` on the session is immediately valid.
    fn start(&self, request: SourceRequest, sink: FrameSink) -> StartFuture<'_>;
}

pub trait CaptureSession: Send {
    fn info(&self) -> SourceInfo;
    /// Stops delivery and releases the source. Dropping a session without calling this must also stop it.
    fn stop(self: Box<Self>);
}
```

`crates/capture/src/synthetic.rs` (above its tests):

```rust
//! Deterministic test pattern: a moving vertical bar, with the frame index written into the first pixel.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use brp_proto::{PixelFormat, monotonic_us};

use crate::frame::{CaptureBackend, CaptureFrame, CaptureSession, FrameSink, SourceInfo, SourceRequest, StartFuture};

#[derive(Debug, Clone, Copy)]
pub struct SyntheticSource {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

struct SyntheticSession {
    info: SourceInfo,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl CaptureBackend for SyntheticSource {
    fn start(&self, _request: SourceRequest, mut sink: FrameSink) -> StartFuture<'_> {
        let info = SourceInfo { width: self.width, height: self.height, fps: self.fps.max(1) };
        Box::pin(async move {
            let stop = Arc::new(AtomicBool::new(false));
            let stop_flag = stop.clone();
            let thread = thread::Builder::new()
                .name("synthetic-capture".into())
                .spawn(move || {
                    let interval = Duration::from_secs_f64(1.0 / f64::from(info.fps));
                    let started = Instant::now();
                    let mut index: u32 = 0;
                    while !stop_flag.load(Ordering::Relaxed) {
                        // Schedule from the start time rather than from "now" so sleep jitter does not accumulate into drift.
                        let due = started + interval * index;
                        if let Some(wait) = due.checked_duration_since(Instant::now()) {
                            thread::sleep(wait);
                        }
                        sink(render(info, index));
                        index = index.wrapping_add(1);
                    }
                })
                .expect("spawning a thread only fails when the system is out of resources");
            Ok(Box::new(SyntheticSession { info, stop, thread: Some(thread) }) as Box<dyn CaptureSession>)
        })
    }
}

fn render(info: SourceInfo, index: u32) -> CaptureFrame {
    let stride = info.width as usize * PixelFormat::Bgra.bytes_per_pixel();
    let mut data = vec![0u8; stride * info.height as usize];
    let bar_x = (index % info.width) as usize;
    for row in data.chunks_exact_mut(stride) {
        let px = &mut row[bar_x * 4..bar_x * 4 + 4];
        px.copy_from_slice(&[255, 255, 255, 255]);
    }
    data[..4].copy_from_slice(&index.to_le_bytes());
    CaptureFrame { width: info.width, height: info.height, stride, format: PixelFormat::Bgra, data, capture_ts_us: monotonic_us() }
}

impl CaptureSession for SyntheticSession {
    fn info(&self) -> SourceInfo {
        self.info
    }

    fn stop(mut self: Box<Self>) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for SyntheticSession {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
```

`crates/capture/src/lib.rs`:

```rust
//! Screen capture behind a platform-neutral trait, plus a synthetic source for tests.

pub mod error;
pub mod frame;
pub mod synthetic;

pub use error::CaptureError;
pub use frame::{CaptureBackend, CaptureFrame, CaptureSession, FrameSink, SourceInfo, SourceRequest, StartFuture};
pub use synthetic::SyntheticSource;
```

- [ ] **Step 5: Run the test and verify it passes**

Run: `cargo test -p brp-capture`
Expected: pass. Then `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`.

- [ ] **Step 6: Commit**

```bash
git add crates/capture
git commit -m "feat: add capture traits and synthetic test source"
```

### Task 5: LatestSlot, the drop-oldest hand-off

**Files:**
- Modify: `crates/pipeline/Cargo.toml`, `crates/pipeline/src/lib.rs`
- Create: `crates/pipeline/src/slot.rs`, `crates/pipeline/src/error.rs`

**Interfaces:**
- Produces: `brp_pipeline::LatestSlot<T>` with `new() -> Arc<Self>`, `put(&self, T)`, `take(&self) -> Option<T>` (blocks; `None` once closed and empty), `take_timeout(&self, Duration) -> SlotWait<T>` where `enum SlotWait<T> { Value(T), Timeout, Closed }`, `try_take(&self) -> Option<T>`, `close(&self)`, `dropped(&self) -> u64`. `brp_pipeline::PipelineError`.

- [ ] **Step 1: Manifest**

`crates/pipeline/Cargo.toml`:

```toml
[dependencies]
brp-proto.workspace = true
brp-codec.workspace = true
brp-capture.workspace = true
brp-net.workspace = true
thiserror.workspace = true
tracing.workspace = true
tokio.workspace = true

[dev-dependencies]
tokio = { workspace = true, features = ["rt", "rt-multi-thread", "macros", "time"] }
```

`brp-net` is empty until Task 8; the dependency resolves because the crate exists.

- [ ] **Step 2: Write the failing tests**

`crates/pipeline/src/slot.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use std::thread;
    use std::time::Duration;

    use super::*;

    #[test]
    fn put_overwrites_unread_value_and_counts_the_drop() {
        let slot = LatestSlot::new();
        slot.put(1);
        slot.put(2);
        assert_eq!(slot.try_take(), Some(2));
        assert_eq!(slot.dropped(), 1);
        assert_eq!(slot.try_take(), None);
    }

    #[test]
    fn take_blocks_until_a_value_arrives() {
        let slot = LatestSlot::new();
        let producer = {
            let slot = slot.clone();
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(20));
                slot.put(7);
            })
        };
        assert_eq!(slot.take(), Some(7));
        producer.join().unwrap();
    }

    #[test]
    fn take_timeout_reports_timeout_then_value_then_closed() {
        let slot = LatestSlot::new();
        assert!(matches!(slot.take_timeout(Duration::from_millis(10)), SlotWait::Timeout));
        slot.put("x");
        assert!(matches!(slot.take_timeout(Duration::from_millis(10)), SlotWait::Value("x")));
        slot.close();
        assert!(matches!(slot.take_timeout(Duration::from_millis(10)), SlotWait::Closed));
        assert_eq!(slot.take(), None);
    }

    #[test]
    fn close_wakes_a_blocked_taker() {
        let slot: Arc<LatestSlot<u8>> = LatestSlot::new();
        let waiter = {
            let slot = slot.clone();
            thread::spawn(move || slot.take())
        };
        thread::sleep(Duration::from_millis(20));
        slot.close();
        assert_eq!(waiter.join().unwrap(), None);
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p brp-pipeline`
Expected: compile errors for `LatestSlot`.

- [ ] **Step 4: Implement**

`crates/pipeline/src/error.rs`:

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PipelineError {
    #[error("capture failed: {0}")]
    Capture(#[from] brp_capture::CaptureError),
    #[error("codec failed: {0}")]
    Codec(#[from] brp_codec::CodecError),
    #[error("network failed: {0}")]
    Net(#[from] brp_net::NetError),
    #[error("unknown live {0}")]
    UnknownLive(u32),
    #[error("unknown preset {0}")]
    UnknownPreset(u32),
}
```

Until Task 8 exists, temporarily omit the `Net` variant and add it back in Task 10.

`crates/pipeline/src/slot.rs` (above its tests):

```rust
//! Single-value mailbox between two threads. A producer that outruns its consumer overwrites the
//! unread value, so the consumer always sees the newest frame and latency never accumulates.

use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

pub struct LatestSlot<T> {
    state: Mutex<State<T>>,
    ready: Condvar,
}

struct State<T> {
    value: Option<T>,
    closed: bool,
    dropped: u64,
}

pub enum SlotWait<T> {
    Value(T),
    Timeout,
    Closed,
}

impl<T> LatestSlot<T> {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { state: Mutex::new(State { value: None, closed: false, dropped: 0 }), ready: Condvar::new() })
    }

    pub fn put(&self, value: T) {
        let mut state = self.lock();
        if state.closed {
            return;
        }
        if state.value.replace(value).is_some() {
            state.dropped += 1;
        }
        self.ready.notify_one();
    }

    pub fn try_take(&self) -> Option<T> {
        self.lock().value.take()
    }

    pub fn take(&self) -> Option<T> {
        let mut state = self.lock();
        loop {
            if let Some(v) = state.value.take() {
                return Some(v);
            }
            if state.closed {
                return None;
            }
            state = self.ready.wait(state).unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }

    pub fn take_timeout(&self, timeout: Duration) -> SlotWait<T> {
        let mut state = self.lock();
        loop {
            if let Some(v) = state.value.take() {
                return SlotWait::Value(v);
            }
            if state.closed {
                return SlotWait::Closed;
            }
            let (next, result) = self.ready.wait_timeout(state, timeout).unwrap_or_else(|poisoned| poisoned.into_inner());
            state = next;
            if result.timed_out() {
                return match state.value.take() {
                    Some(v) => SlotWait::Value(v),
                    None if state.closed => SlotWait::Closed,
                    None => SlotWait::Timeout,
                };
            }
        }
    }

    pub fn close(&self) {
        self.lock().closed = true;
        self.ready.notify_all();
    }

    pub fn dropped(&self) -> u64 {
        self.lock().dropped
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State<T>> {
        // A poisoned lock only means a producer panicked mid-put; the slot state itself is still consistent.
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}
```

`crates/pipeline/src/lib.rs`:

```rust
//! Publisher and viewer pipelines: the glue between capture, codec, and network.

pub mod error;
pub mod slot;

pub use error::PipelineError;
pub use slot::{LatestSlot, SlotWait};
```

- [ ] **Step 5: Run the tests and verify they pass**

Run: `cargo test -p brp-pipeline`
Expected: 4 passing tests. Then fmt and clippy.

- [ ] **Step 6: Commit**

```bash
git add crates/pipeline
git commit -m "feat: add drop-oldest LatestSlot hand-off"
```

### Task 6: FanOut with the sender backlog rule and KeyframeRequest

**Files:**
- Create: `crates/pipeline/src/fanout.rs`
- Modify: `crates/pipeline/src/lib.rs`

**Interfaces:**
- Consumes: `brp_proto::{EncodedFrame, constants::{SENDER_BACKLOG_FRAMES, FORCED_KEYFRAME_MIN_INTERVAL}}`.
- Produces: `brp_pipeline::KeyframeRequest` (`Clone`) with `new()`, `request()`, `pending() -> bool`, `take_if_allowed(now: Instant) -> bool`; `brp_pipeline::FanOut` with `new(keyframe: KeyframeRequest)`, `add(&mut self) -> tokio::sync::mpsc::Receiver<Arc<EncodedFrame>>`, `push(&mut self, frame: Arc<EncodedFrame>) -> PushOutcome { delivered: usize, skipped: usize }`, `subscriber_count(&self) -> usize`.

Rule from spec 6.5: a subscriber whose channel is full when a frame arrives is marked as waiting for a keyframe, receives nothing until the next keyframe, and triggers a keyframe request. A new subscriber starts in the waiting state so it never receives a P-frame before its first keyframe.

- [ ] **Step 1: Write the failing tests**

`crates/pipeline/src/fanout.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use brp_proto::constants::FORCED_KEYFRAME_MIN_INTERVAL;

    use super::*;

    fn frame(seq: u64, keyframe: bool) -> Arc<EncodedFrame> {
        Arc::new(EncodedFrame { seq, capture_ts_us: seq * 16_667, keyframe, data: vec![seq as u8] })
    }

    #[test]
    fn keyframe_request_is_rate_limited() {
        let kf = KeyframeRequest::new();
        let t0 = Instant::now();
        assert!(!kf.take_if_allowed(t0));
        kf.request();
        assert!(kf.pending());
        assert!(kf.take_if_allowed(t0));
        assert!(!kf.pending());
        kf.request();
        assert!(!kf.take_if_allowed(t0 + Duration::from_millis(100)), "second forced keyframe inside the interval");
        assert!(kf.pending(), "request stays pending until allowed");
        assert!(kf.take_if_allowed(t0 + FORCED_KEYFRAME_MIN_INTERVAL));
    }

    #[test]
    fn new_subscriber_waits_for_a_keyframe_and_requests_one() {
        let kf = KeyframeRequest::new();
        let mut fanout = FanOut::new(kf.clone());
        let mut rx = fanout.add();
        assert!(kf.pending());
        let out = fanout.push(frame(1, false));
        assert_eq!((out.delivered, out.skipped), (0, 1));
        let out = fanout.push(frame(2, true));
        assert_eq!((out.delivered, out.skipped), (1, 0));
        assert_eq!(rx.try_recv().unwrap().seq, 2);
        fanout.push(frame(3, false));
        assert_eq!(rx.try_recv().unwrap().seq, 3);
    }

    #[test]
    fn full_channel_skips_until_next_keyframe_and_requests_one() {
        let kf = KeyframeRequest::new();
        let mut fanout = FanOut::new(kf.clone());
        let mut rx = fanout.add();
        assert!(kf.take_if_allowed(Instant::now()));
        fanout.push(frame(1, true));
        fanout.push(frame(2, false));
        assert!(!kf.pending());
        let out = fanout.push(frame(3, false));
        assert_eq!((out.delivered, out.skipped), (0, 1), "channel of two was full");
        assert!(kf.pending(), "backlog must request a keyframe");
        assert_eq!(rx.try_recv().unwrap().seq, 1);
        assert_eq!(rx.try_recv().unwrap().seq, 2);
        let out = fanout.push(frame(4, false));
        assert_eq!((out.delivered, out.skipped), (0, 1), "still waiting for a keyframe even with room");
        let out = fanout.push(frame(5, true));
        assert_eq!((out.delivered, out.skipped), (1, 0));
        assert_eq!(rx.try_recv().unwrap().seq, 5);
    }

    #[test]
    fn dropped_receiver_is_removed() {
        let mut fanout = FanOut::new(KeyframeRequest::new());
        let rx = fanout.add();
        assert_eq!(fanout.subscriber_count(), 1);
        drop(rx);
        fanout.push(frame(1, true));
        assert_eq!(fanout.subscriber_count(), 0);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p brp-pipeline fanout`
Expected: compile errors.

- [ ] **Step 3: Implement**

`crates/pipeline/src/fanout.rs` (above its tests):

```rust
//! Delivers each encoded frame to every subscriber of one preset without re-encoding.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use brp_proto::EncodedFrame;
use brp_proto::constants::{FORCED_KEYFRAME_MIN_INTERVAL, SENDER_BACKLOG_FRAMES};
use tokio::sync::mpsc::{self, Receiver, Sender, error::TrySendError};

/// Shared between the fan-out (which asks) and the encoder loop (which grants, rate-limited).
#[derive(Clone, Default)]
pub struct KeyframeRequest {
    inner: Arc<KeyframeInner>,
}

#[derive(Default)]
struct KeyframeInner {
    requested: AtomicBool,
    last_forced: Mutex<Option<Instant>>,
}

impl KeyframeRequest {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn request(&self) {
        self.inner.requested.store(true, Ordering::Release);
    }

    pub fn pending(&self) -> bool {
        self.inner.requested.load(Ordering::Acquire)
    }

    /// Consumes the pending request if the rate limit allows a forced keyframe now.
    pub fn take_if_allowed(&self, now: Instant) -> bool {
        if !self.pending() {
            return false;
        }
        let mut last = self.inner.last_forced.lock().unwrap_or_else(|p| p.into_inner());
        let allowed = last.is_none_or(|t| now.duration_since(t) >= FORCED_KEYFRAME_MIN_INTERVAL);
        if allowed {
            *last = Some(now);
            self.inner.requested.store(false, Ordering::Release);
        }
        allowed
    }
}

struct Subscriber {
    tx: Sender<Arc<EncodedFrame>>,
    waiting_for_keyframe: bool,
}

pub struct FanOut {
    subscribers: Vec<Subscriber>,
    keyframe: KeyframeRequest,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PushOutcome {
    pub delivered: usize,
    pub skipped: usize,
}

impl FanOut {
    pub fn new(keyframe: KeyframeRequest) -> Self {
        Self { subscribers: Vec::new(), keyframe }
    }

    pub fn add(&mut self) -> Receiver<Arc<EncodedFrame>> {
        let (tx, rx) = mpsc::channel(SENDER_BACKLOG_FRAMES);
        self.subscribers.push(Subscriber { tx, waiting_for_keyframe: true });
        self.keyframe.request();
        rx
    }

    pub fn subscriber_count(&self) -> usize {
        self.subscribers.len()
    }

    pub fn push(&mut self, frame: Arc<EncodedFrame>) -> PushOutcome {
        let mut outcome = PushOutcome::default();
        let mut need_keyframe = false;
        self.subscribers.retain_mut(|sub| {
            if sub.waiting_for_keyframe && !frame.keyframe {
                outcome.skipped += 1;
                return true;
            }
            match sub.tx.try_send(frame.clone()) {
                Ok(()) => {
                    sub.waiting_for_keyframe = false;
                    outcome.delivered += 1;
                    true
                }
                Err(TrySendError::Full(_)) => {
                    sub.waiting_for_keyframe = true;
                    need_keyframe = true;
                    outcome.skipped += 1;
                    true
                }
                Err(TrySendError::Closed(_)) => false,
            }
        });
        if need_keyframe {
            self.keyframe.request();
        }
        outcome
    }
}
```

Add to `crates/pipeline/src/lib.rs`: `pub mod fanout;` and `pub use fanout::{FanOut, KeyframeRequest, PushOutcome};`.

- [ ] **Step 4: Run the tests and verify they pass**

Run: `cargo test -p brp-pipeline`
Expected: all pass. fmt, clippy.

- [ ] **Step 5: Commit**

```bash
git add crates/pipeline
git commit -m "feat: add fan-out with sender backlog rule and keyframe requests"
```

### Task 7: Reorder buffer with the gap rules

**Files:**
- Create: `crates/pipeline/src/reorder.rs`
- Modify: `crates/pipeline/src/lib.rs`

**Interfaces:**
- Consumes: `brp_proto::FrameHeader`.
- Produces: `brp_pipeline::IncomingFrame { header: FrameHeader, data: Vec<u8> }`; `brp_pipeline::Reorder` with `new(max_wait: Duration)`, `push(&mut self, frame: IncomingFrame, now: Instant) -> Drained`, `poll(&mut self, now: Instant) -> Drained`; `Drained { ready: Vec<IncomingFrame>, request_keyframe: bool }`. Task 11's viewer builds `IncomingFrame` from `brp_net::ReceivedFrame` and drives `Reorder`.

Rules from spec 6.5: decode in sequence order; on a gap wait for the missing frame, unless a later keyframe has already arrived, in which case discard everything before it; if the wait exceeds the cap, discard pending frames, wait for the next keyframe, and request one. Additionally, before the first keyframe nothing can be decoded, so non-keyframes are dropped while waiting.

- [ ] **Step 1: Write the failing tests**

`crates/pipeline/src/reorder.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use std::time::Duration;

    use brp_proto::FrameKind;

    use super::*;

    const WAIT: Duration = Duration::from_millis(200);

    fn f(seq: u64, keyframe: bool) -> IncomingFrame {
        IncomingFrame {
            header: FrameHeader { live_id: 1, preset_id: 1, kind: FrameKind::Video, seq, capture_ts_us: 0, keyframe, len: 0 },
            data: Vec::new(),
        }
    }

    fn seqs(d: &Drained) -> Vec<u64> {
        d.ready.iter().map(|x| x.header.seq).collect()
    }

    #[test]
    fn drops_non_keyframes_until_the_first_keyframe() {
        let mut r = Reorder::new(WAIT);
        let t = Instant::now();
        assert!(r.push(f(5, false), t).ready.is_empty());
        assert_eq!(seqs(&r.push(f(6, true), t)), vec![6]);
        assert_eq!(seqs(&r.push(f(7, false), t)), vec![7]);
    }

    #[test]
    fn reorders_frames_that_complete_out_of_order() {
        let mut r = Reorder::new(WAIT);
        let t = Instant::now();
        r.push(f(0, true), t);
        assert!(r.push(f(2, false), t).ready.is_empty());
        assert_eq!(seqs(&r.push(f(1, false), t)), vec![1, 2]);
    }

    #[test]
    fn a_later_keyframe_skips_the_gap_immediately() {
        let mut r = Reorder::new(WAIT);
        let t = Instant::now();
        r.push(f(0, true), t);
        assert!(r.push(f(2, false), t).ready.is_empty());
        assert_eq!(seqs(&r.push(f(3, true), t)), vec![3]);
        assert!(r.push(f(1, false), t).ready.is_empty(), "late frame from before the jump is stale");
        assert_eq!(seqs(&r.push(f(4, false), t)), vec![4]);
    }

    #[test]
    fn gap_past_the_wait_cap_requests_a_keyframe_and_resets() {
        let mut r = Reorder::new(WAIT);
        let t = Instant::now();
        r.push(f(0, true), t);
        r.push(f(2, false), t);
        let early = r.poll(t + Duration::from_millis(100));
        assert!(early.ready.is_empty() && !early.request_keyframe);
        let late = r.poll(t + WAIT);
        assert!(late.ready.is_empty() && late.request_keyframe);
        assert!(r.push(f(1, false), t + WAIT).ready.is_empty(), "pending were discarded and we wait for a keyframe");
        assert!(r.push(f(3, false), t + WAIT).ready.is_empty());
        assert_eq!(seqs(&r.push(f(4, true), t + WAIT)), vec![4]);
    }

    #[test]
    fn duplicates_and_stale_frames_are_dropped() {
        let mut r = Reorder::new(WAIT);
        let t = Instant::now();
        r.push(f(0, true), t);
        assert!(r.push(f(0, true), t).ready.is_empty());
        r.push(f(1, false), t);
        assert!(r.push(f(1, false), t).ready.is_empty());
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p brp-pipeline reorder`
Expected: compile errors.

- [ ] **Step 3: Implement**

`crates/pipeline/src/reorder.rs` (above its tests):

```rust
//! Puts frames that arrive on independent QUIC streams back into encoder order.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use brp_proto::FrameHeader;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingFrame {
    pub header: FrameHeader,
    pub data: Vec<u8>,
}

#[derive(Debug, Default)]
pub struct Drained {
    pub ready: Vec<IncomingFrame>,
    pub request_keyframe: bool,
}

pub struct Reorder {
    /// `None` while waiting for a keyframe, which is the state at start-up and after a gap times out.
    next_seq: Option<u64>,
    pending: BTreeMap<u64, IncomingFrame>,
    gap_since: Option<Instant>,
    max_wait: Duration,
}

impl Reorder {
    pub fn new(max_wait: Duration) -> Self {
        Self { next_seq: None, pending: BTreeMap::new(), gap_since: None, max_wait }
    }

    pub fn push(&mut self, frame: IncomingFrame, now: Instant) -> Drained {
        let mut out = Drained::default();
        match self.next_seq {
            None => {
                if frame.header.keyframe {
                    self.restart_from(frame, &mut out);
                }
            }
            Some(next) => {
                if frame.header.seq < next || self.pending.contains_key(&frame.header.seq) {
                    return out;
                }
                self.pending.insert(frame.header.seq, frame);
                self.drain(now, &mut out);
            }
        }
        out
    }

    pub fn poll(&mut self, now: Instant) -> Drained {
        let mut out = Drained::default();
        if let Some(since) = self.gap_since
            && now.duration_since(since) >= self.max_wait
        {
            self.pending.clear();
            self.gap_since = None;
            self.next_seq = None;
            out.request_keyframe = true;
        }
        out
    }

    fn restart_from(&mut self, keyframe: IncomingFrame, out: &mut Drained) {
        let seq = keyframe.header.seq;
        self.pending = self.pending.split_off(&(seq + 1));
        self.gap_since = None;
        self.next_seq = Some(seq + 1);
        out.ready.push(keyframe);
        self.drain_contiguous(out);
    }

    fn drain(&mut self, now: Instant, out: &mut Drained) {
        self.drain_contiguous(out);
        let Some(next) = self.next_seq else { return };
        if self.pending.is_empty() {
            self.gap_since = None;
            return;
        }
        let later_keyframe = self.pending.iter().find(|(seq, f)| **seq > next && f.header.keyframe).map(|(seq, _)| *seq);
        match later_keyframe {
            Some(seq) => {
                let keyframe = self.pending.remove(&seq).expect("found above");
                self.restart_from(keyframe, out);
            }
            None => {
                if self.gap_since.is_none() {
                    self.gap_since = Some(now);
                }
            }
        }
    }

    fn drain_contiguous(&mut self, out: &mut Drained) {
        while let Some(next) = self.next_seq {
            let Some(frame) = self.pending.remove(&next) else { break };
            out.ready.push(frame);
            self.next_seq = Some(next + 1);
        }
    }
}
```

Add to `crates/pipeline/src/lib.rs`: `pub mod reorder;` and `pub use reorder::{Drained, IncomingFrame, Reorder};`.

- [ ] **Step 4: Run the tests and verify they pass**

Run: `cargo test -p brp-pipeline`
Expected: all pass. fmt, clippy.

- [ ] **Step 5: Commit**

```bash
git add crates/pipeline
git commit -m "feat: add reorder buffer with keyframe gap rules"
```

### Task 8: Network endpoint, control framing, and the LiveSource contract

**Files:**
- Modify: `crates/net/Cargo.toml`, `crates/net/src/lib.rs`
- Create: `crates/net/src/error.rs`, `crates/net/src/endpoint.rs`, `crates/net/src/framing.rs`, `crates/net/src/source.rs`

**Interfaces:**
- Consumes: `brp_proto::{constants::MEDIA_ALPN, encode, decode, ProtoError, CodecParams, EncodedFrame, FrameHeader}`.
- Produces: `brp_net::NetError`; `brp_net::RelaySetting { Default, Disabled }`; `brp_net::bind_endpoint(secret: iroh::SecretKey, relay: RelaySetting) -> Result<iroh::Endpoint, NetError>`; `brp_net::framing::{encode_framed<T: Serialize>(&T) -> Result<Vec<u8>, NetError>, write_msg(&mut SendStream, &T), read_msg<T: DeserializeOwned>(&mut RecvStream, max: usize) -> Result<T, NetError>}`; `brp_net::{LiveSource, Subscription { params: CodecParams, frames: tokio::sync::mpsc::Receiver<Arc<EncodedFrame>> }, SubscribeRejected}`; `brp_net::ReceivedFrame { header: FrameHeader, payload: Vec<u8> }`.

Verified iroh 1.1 facts used here: `Endpoint::builder(preset)` with `presets::N0` (relays plus address lookup) or `presets::Minimal` (nothing); `.secret_key(..).alpns(vec![..]).relay_mode(..).bind().await -> Result<Endpoint, BindError>`; stream types are `iroh::endpoint::{SendStream, RecvStream}` with `write_all(&[u8]).await`, `read_exact(&mut [u8]).await`, `read_to_end(limit).await -> Vec<u8>`, and a synchronous `finish()`.

- [ ] **Step 1: Manifest**

`crates/net/Cargo.toml`:

```toml
[dependencies]
brp-proto.workspace = true
iroh.workspace = true
tokio.workspace = true
serde.workspace = true
thiserror.workspace = true
tracing.workspace = true

[dev-dependencies]
tokio = { workspace = true, features = ["rt-multi-thread", "macros", "time"] }
```

- [ ] **Step 2: Write the failing unit test for framing**

`crates/net/src/framing.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use brp_proto::ViewerMessage;

    use super::*;

    #[test]
    fn framed_message_is_length_prefixed_little_endian() {
        let msg = ViewerMessage::Subscribe { live_id: 1, preset_id: 1, want_audio: false };
        let framed = encode_framed(&msg).unwrap();
        let body = brp_proto::encode(&msg).unwrap();
        assert_eq!(&framed[..4], &(body.len() as u32).to_le_bytes());
        assert_eq!(&framed[4..], &body[..]);
    }
}
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test -p brp-net`
Expected: compile error, `encode_framed` unresolved.

- [ ] **Step 4: Implement**

`crates/net/src/error.rs`:

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum NetError {
    #[error("failed to bind endpoint: {0}")]
    Bind(String),
    #[error("failed to connect: {0}")]
    Connect(String),
    #[error("connection failed: {0}")]
    Connection(String),
    #[error("stream failed: {0}")]
    Stream(String),
    #[error("peer violated the protocol: {0}")]
    Protocol(&'static str),
    #[error("subscription rejected by publisher: {0}")]
    Rejected(String),
    #[error(transparent)]
    Proto(#[from] brp_proto::ProtoError),
}

impl NetError {
    pub(crate) fn connection(e: impl std::fmt::Display) -> Self {
        Self::Connection(e.to_string())
    }

    pub(crate) fn stream(e: impl std::fmt::Display) -> Self {
        Self::Stream(e.to_string())
    }
}
```

`crates/net/src/endpoint.rs`:

```rust
use brp_proto::constants::MEDIA_ALPN;
use iroh::endpoint::presets;
use iroh::{Endpoint, RelayMode, SecretKey};

use crate::error::NetError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelaySetting {
    /// The library's public relays for hole punching and fallback. Media should still flow direct.
    Default,
    /// No relay at all; only peers reachable by IP can connect. Right for LAN and tests.
    Disabled,
}

pub async fn bind_endpoint(secret: SecretKey, relay: RelaySetting) -> Result<Endpoint, NetError> {
    let builder = match relay {
        RelaySetting::Default => Endpoint::builder(presets::N0),
        RelaySetting::Disabled => Endpoint::builder(presets::Minimal).relay_mode(RelayMode::Disabled),
    };
    builder
        .secret_key(secret)
        .alpns(vec![MEDIA_ALPN.to_vec()])
        .bind()
        .await
        .map_err(|e| NetError::Bind(e.to_string()))
}
```

`crates/net/src/framing.rs` (above its test):

```rust
//! Control streams carry length-prefixed postcard messages so several messages can share one stream.

use iroh::endpoint::{RecvStream, SendStream};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::NetError;

pub fn encode_framed<T: Serialize>(msg: &T) -> Result<Vec<u8>, NetError> {
    let body = brp_proto::encode(msg)?;
    let len = u32::try_from(body.len()).map_err(|_| NetError::Protocol("control message too large"))?;
    let mut framed = Vec::with_capacity(4 + body.len());
    framed.extend_from_slice(&len.to_le_bytes());
    framed.extend_from_slice(&body);
    Ok(framed)
}

pub async fn write_msg<T: Serialize>(stream: &mut SendStream, msg: &T) -> Result<(), NetError> {
    let framed = encode_framed(msg)?;
    stream.write_all(&framed).await.map_err(NetError::stream)
}

pub async fn read_msg<T: DeserializeOwned>(stream: &mut RecvStream, max: usize) -> Result<T, NetError> {
    let mut len = [0u8; 4];
    stream.read_exact(&mut len).await.map_err(NetError::stream)?;
    let len = u32::from_le_bytes(len) as usize;
    if len > max {
        return Err(NetError::Protocol("control message exceeds the size limit"));
    }
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).await.map_err(NetError::stream)?;
    Ok(brp_proto::decode(&body)?)
}
```

`crates/net/src/source.rs`:

```rust
use std::sync::Arc;

use brp_proto::{CodecParams, EncodedFrame, FrameHeader};
use thiserror::Error;
use tokio::sync::mpsc::Receiver;

/// What the media server pulls frames from. The pipeline's publisher implements it.
pub trait LiveSource: Send + Sync + 'static {
    fn subscribe(&self, live_id: u32, preset_id: u32) -> Result<Subscription, SubscribeRejected>;
    fn request_keyframe(&self, live_id: u32, preset_id: u32);
}

#[derive(Debug)]
pub struct Subscription {
    pub params: CodecParams,
    pub frames: Receiver<Arc<EncodedFrame>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SubscribeRejected {
    #[error("unknown live {0}")]
    UnknownLive(u32),
    #[error("unknown preset {0}")]
    UnknownPreset(u32),
}

/// One frame as read off its QUIC stream on the viewer side, before reordering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceivedFrame {
    pub header: FrameHeader,
    pub payload: Vec<u8>,
}
```

`crates/net/src/lib.rs`:

```rust
//! iroh transport: endpoint setup, the media server, and the media client.

pub mod endpoint;
pub mod error;
pub mod framing;
pub mod source;

pub use endpoint::{RelaySetting, bind_endpoint};
pub use error::NetError;
pub use source::{LiveSource, ReceivedFrame, SubscribeRejected, Subscription};
```

- [ ] **Step 5: Run the tests and verify they pass**

Run: `cargo test -p brp-net`
Expected: pass. fmt, clippy.

- [ ] **Step 6: Commit**

```bash
git add Cargo.lock crates/net
git commit -m "feat: add iroh endpoint setup, control framing, and LiveSource contract"
```

### Task 9: Media server and client with a loopback integration test

**Files:**
- Create: `crates/net/src/server.rs`, `crates/net/src/client.rs`, `crates/net/tests/loopback.rs`
- Modify: `crates/net/src/lib.rs`

**Interfaces:**
- Consumes: Task 8 types; `brp_proto::{ViewerMessage, PublisherMessage, FrameHeader, FrameKind, constants::{MAX_CONTROL_BYTES, MAX_FRAME_BYTES, MEDIA_ALPN, RECEIVE_QUEUE_FRAMES}}`.
- Produces: `brp_net::MediaServer::new(source: Arc<dyn LiveSource>)` implementing `iroh::protocol::ProtocolHandler`; `brp_net::MediaClient::connect(endpoint: &Endpoint, addr: EndpointAddr) -> Result<MediaClient, NetError>`, `MediaClient::subscribe(&self, live_id, preset_id) -> Result<ViewerSubscription, NetError>`, `MediaClient::close(&self)`; `brp_net::ViewerSubscription { params: CodecParams, frames: Receiver<ReceivedFrame>, control: Sender<ViewerMessage>, events: Receiver<PublisherMessage> }`.

Add to `crates/proto/src/constants.rs` (spec 6.5 leaves the viewer's receive queue implicit; a few frames of slack keep decode jitter from stalling the QUIC reader, while QUIC flow control bounds the rest):

```rust
/// Frames buffered between the QUIC reader and the decoder before back-pressure reaches the publisher.
pub const RECEIVE_QUEUE_FRAMES: usize = 8;
```

Verified iroh 1.1 facts: `ProtocolHandler` requires `Debug + Send + Sync + 'static` and one `async fn accept(&self, connection: Connection) -> Result<(), AcceptError>`; the connection is dropped when `accept` returns, so the handler must loop until the connection closes. `Connection` is `Clone`, has `accept_bi()`, `accept_uni()`, `open_uni()`, `open_bi()`, `remote_id()`, `close(code.into(), reason)`, `closed().await`. `Router::builder(endpoint).accept(alpn, handler).spawn()` and `router.shutdown().await`. With `RelayMode::Disabled`, `endpoint.addr()` is immediately connectable on the same machine; never await `online()` without relays.

- [ ] **Step 1: Write the failing integration test**

`crates/net/tests/loopback.rs`:

```rust
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use brp_net::{LiveSource, MediaClient, MediaServer, NetError, RelaySetting, SubscribeRejected, Subscription, bind_endpoint};
use brp_proto::constants::MEDIA_ALPN;
use brp_proto::{Codec, CodecParams, EncodedFrame, FrameKind, ViewerMessage};
use iroh::SecretKey;
use iroh::protocol::Router;
use tokio::sync::mpsc;

struct ScriptedSource {
    params: CodecParams,
    frames: Mutex<Option<mpsc::Receiver<Arc<EncodedFrame>>>>,
    keyframe_requests: AtomicUsize,
}

impl LiveSource for ScriptedSource {
    fn subscribe(&self, live_id: u32, preset_id: u32) -> Result<Subscription, SubscribeRejected> {
        if live_id != 1 {
            return Err(SubscribeRejected::UnknownLive(live_id));
        }
        if preset_id != 1 {
            return Err(SubscribeRejected::UnknownPreset(preset_id));
        }
        let frames = self.frames.lock().unwrap().take().expect("single subscription in this test");
        Ok(Subscription { params: self.params.clone(), frames })
    }

    fn request_keyframe(&self, _live_id: u32, _preset_id: u32) {
        self.keyframe_requests.fetch_add(1, Ordering::SeqCst);
    }
}

fn params() -> CodecParams {
    CodecParams { codec: Codec::Hevc, width: 640, height: 360, fps: 30, extradata: vec![1, 2, 3] }
}

#[tokio::test]
async fn frames_travel_from_source_to_viewer_over_loopback() {
    let (tx, rx) = mpsc::channel(8);
    let source = Arc::new(ScriptedSource { params: params(), frames: Mutex::new(Some(rx)), keyframe_requests: AtomicUsize::new(0) });

    let server_ep = bind_endpoint(SecretKey::generate(), RelaySetting::Disabled).await.unwrap();
    let router = Router::builder(server_ep.clone()).accept(MEDIA_ALPN, MediaServer::new(source.clone())).spawn();
    let client_ep = bind_endpoint(SecretKey::generate(), RelaySetting::Disabled).await.unwrap();

    let client = MediaClient::connect(&client_ep, server_ep.addr()).await.unwrap();
    let mut sub = client.subscribe(1, 1).await.unwrap();
    assert_eq!(sub.params, params());

    for seq in 0..5u64 {
        let frame = EncodedFrame { seq, capture_ts_us: seq * 1000, keyframe: seq == 0, data: vec![seq as u8; 100 + seq as usize] };
        tx.send(Arc::new(frame)).await.unwrap();
    }
    for seq in 0..5u64 {
        let received = tokio::time::timeout(Duration::from_secs(5), sub.frames.recv()).await.expect("frame in time").expect("channel open");
        assert_eq!(received.header.seq, seq);
        assert_eq!(received.header.kind, FrameKind::Video);
        assert_eq!((received.header.live_id, received.header.preset_id), (1, 1));
        assert_eq!(received.header.keyframe, seq == 0);
        assert_eq!(received.header.len as usize, received.payload.len());
        assert_eq!(received.payload, vec![seq as u8; 100 + seq as usize]);
    }

    sub.control.send(ViewerMessage::RequestKeyframe).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while source.keyframe_requests.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("keyframe request reached the source");

    let rejected = client.subscribe(2, 1).await;
    assert!(matches!(rejected, Err(NetError::Rejected(ref r)) if r.contains("unknown live 2")), "{rejected:?}");

    drop(tx);
    let ended = tokio::time::timeout(Duration::from_secs(5), sub.events.recv()).await.expect("event in time");
    assert!(matches!(ended, Some(brp_proto::PublisherMessage::LiveEnded)));

    client.close();
    router.shutdown().await.unwrap();
    client_ep.close().await;
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p brp-net --test loopback`
Expected: compile errors for `MediaServer`, `MediaClient`.

- [ ] **Step 3: Implement the server**

`crates/net/src/server.rs`:

```rust
use std::fmt;
use std::sync::Arc;

use brp_proto::constants::MAX_CONTROL_BYTES;
use brp_proto::{EncodedFrame, FrameHeader, FrameKind, PublisherMessage, ViewerMessage};
use iroh::endpoint::{Connection, SendStream};
use iroh::protocol::{AcceptError, ProtocolHandler};
use tokio::sync::mpsc::Receiver;
use tokio::task::JoinHandle;

use crate::error::NetError;
use crate::framing::{read_msg, write_msg};
use crate::source::LiveSource;

#[derive(Clone)]
pub struct MediaServer {
    source: Arc<dyn LiveSource>,
}

impl MediaServer {
    pub fn new(source: Arc<dyn LiveSource>) -> Self {
        Self { source }
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
        tracing::info!(peer = %peer.fmt_short(), "media connection accepted");
        // Each accept_bi is one subscription. The loop ends when the peer closes the connection.
        while let Ok((send, recv)) = connection.accept_bi().await {
            let source = self.source.clone();
            let conn = connection.clone();
            tokio::spawn(async move {
                if let Err(e) = serve_subscription(conn, send, recv, source).await {
                    tracing::debug!(error = %e, "subscription ended");
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
    let ViewerMessage::Subscribe { live_id, preset_id, .. } = first else {
        write_msg(&mut send, &PublisherMessage::SubscribeError { reason: "first message must be Subscribe".into() }).await?;
        return Err(NetError::Protocol("expected Subscribe as the first control message"));
    };
    let subscription = match source.subscribe(live_id, preset_id) {
        Ok(s) => s,
        Err(rejected) => {
            write_msg(&mut send, &PublisherMessage::SubscribeError { reason: rejected.to_string() }).await?;
            send.finish().map_err(NetError::stream)?;
            return Ok(());
        }
    };
    write_msg(&mut send, &PublisherMessage::SubscribeAck { video: subscription.params, audio: None }).await?;

    let sender: JoinHandle<Result<(), NetError>> = tokio::spawn(send_frames(conn, send, live_id, preset_id, subscription.frames));

    loop {
        match read_msg::<ViewerMessage>(&mut recv, MAX_CONTROL_BYTES).await {
            Ok(ViewerMessage::RequestKeyframe) => source.request_keyframe(live_id, preset_id),
            Ok(ViewerMessage::Unsubscribe) | Err(_) => break,
            Ok(ViewerMessage::Stats { frames_received, frames_dropped, decode_fps, rtt_ms }) => {
                tracing::trace!(live_id, preset_id, frames_received, frames_dropped, decode_fps, rtt_ms, "viewer stats");
            }
            Ok(other) => tracing::debug!(?other, "control message not supported in this phase"),
        }
    }
    // Dropping the frame receiver makes the fan-out forget this viewer on its next push.
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
        let header = FrameHeader {
            live_id,
            preset_id,
            kind: FrameKind::Video,
            seq: frame.seq,
            capture_ts_us: frame.capture_ts_us,
            keyframe: frame.keyframe,
            len: u32::try_from(frame.data.len()).map_err(|_| NetError::Protocol("frame larger than u32"))?,
        };
        let mut stream = conn.open_uni().await.map_err(NetError::connection)?;
        stream.write_all(&header.encode_prefix()?).await.map_err(NetError::stream)?;
        stream.write_all(&frame.data).await.map_err(NetError::stream)?;
        stream.finish().map_err(NetError::stream)?;
    }
    write_msg(&mut control, &PublisherMessage::LiveEnded).await?;
    control.finish().map_err(NetError::stream)?;
    Ok(())
}
```

- [ ] **Step 4: Implement the client**

`crates/net/src/client.rs`:

```rust
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

#[derive(Debug)]
pub struct ViewerSubscription {
    pub params: CodecParams,
    pub frames: Receiver<ReceivedFrame>,
    pub control: Sender<ViewerMessage>,
    pub events: Receiver<PublisherMessage>,
}

impl MediaClient {
    pub async fn connect(endpoint: &Endpoint, addr: EndpointAddr) -> Result<Self, NetError> {
        let conn = endpoint.connect(addr, MEDIA_ALPN).await.map_err(|e| NetError::Connect(e.to_string()))?;
        let routes: Routes = Arc::default();
        tokio::spawn(receive_frames(conn.clone(), routes.clone()));
        Ok(Self { conn, routes })
    }

    pub fn remote_id(&self) -> iroh::EndpointId {
        self.conn.remote_id()
    }

    pub async fn subscribe(&self, live_id: u32, preset_id: u32) -> Result<ViewerSubscription, NetError> {
        let (mut send, mut recv) = self.conn.open_bi().await.map_err(NetError::connection)?;
        write_msg(&mut send, &ViewerMessage::Subscribe { live_id, preset_id, want_audio: false }).await?;
        let params = match read_msg::<PublisherMessage>(&mut recv, MAX_CONTROL_BYTES).await? {
            PublisherMessage::SubscribeAck { video, .. } => video,
            PublisherMessage::SubscribeError { reason } => return Err(NetError::Rejected(reason)),
            _ => return Err(NetError::Protocol("expected SubscribeAck")),
        };

        let (frame_tx, frame_rx) = mpsc::channel(RECEIVE_QUEUE_FRAMES);
        self.routes.lock().unwrap_or_else(|p| p.into_inner()).insert((live_id, preset_id), frame_tx);

        let (control_tx, mut control_rx) = mpsc::channel::<ViewerMessage>(16);
        tokio::spawn(async move {
            while let Some(msg) = control_rx.recv().await {
                if write_msg(&mut send, &msg).await.is_err() {
                    break;
                }
            }
            let _ = send.finish();
        });

        let (events_tx, events_rx) = mpsc::channel::<PublisherMessage>(16);
        let routes = self.routes.clone();
        tokio::spawn(async move {
            while let Ok(msg) = read_msg::<PublisherMessage>(&mut recv, MAX_CONTROL_BYTES).await {
                let ended = matches!(msg, PublisherMessage::LiveEnded);
                if events_tx.send(msg).await.is_err() || ended {
                    break;
                }
            }
            routes.lock().unwrap_or_else(|p| p.into_inner()).remove(&(live_id, preset_id));
        });

        Ok(ViewerSubscription { params, frames: frame_rx, control: control_tx, events: events_rx })
    }

    pub fn close(&self) {
        self.conn.close(0u32.into(), b"done");
    }
}

/// One frame per unidirectional stream. Each stream is read on its own task so a large keyframe
/// never delays the small frames behind it.
async fn receive_frames(conn: Connection, routes: Routes) {
    while let Ok(mut stream) = conn.accept_uni().await {
        let routes = routes.clone();
        tokio::spawn(async move {
            let bytes = match stream.read_to_end(MAX_FRAME_BYTES).await {
                Ok(b) => b,
                Err(e) => {
                    tracing::debug!(error = %e, "frame stream ended early");
                    return;
                }
            };
            let (header, payload) = match FrameHeader::decode_prefixed(&bytes) {
                Ok(parsed) => parsed,
                Err(e) => {
                    tracing::warn!(error = %e, "dropping malformed frame");
                    return;
                }
            };
            let route = routes.lock().unwrap_or_else(|p| p.into_inner()).get(&(header.live_id, header.preset_id)).cloned();
            match route {
                Some(tx) => {
                    let frame = ReceivedFrame { header, payload: payload.to_vec() };
                    if tx.send(frame).await.is_err() {
                        tracing::debug!("viewer dropped its frame receiver");
                    }
                }
                None => tracing::trace!(live_id = header.live_id, preset_id = header.preset_id, "frame for unknown subscription"),
            }
        });
    }
}
```

Add to `crates/net/src/lib.rs`:

```rust
pub mod client;
pub mod server;

pub use client::{MediaClient, ViewerSubscription};
pub use server::MediaServer;
```

- [ ] **Step 5: Run the test and verify it passes**

Run: `cargo test -p brp-net`
Expected: unit test and the loopback test pass. If the loopback connect fails with "No addressing information available", the endpoint address had no IP entries; the fix is to bind with `RelaySetting::Disabled` on both sides, which the test already does. fmt, clippy.

- [ ] **Step 6: Commit**

```bash
git add crates/proto/src/constants.rs crates/net
git commit -m "feat: add media server and client with one QUIC stream per frame"
```

### Task 10: Publisher pipeline

**Files:**
- Create: `crates/pipeline/src/publisher.rs`, `crates/pipeline/tests/publisher.rs`
- Modify: `crates/pipeline/src/lib.rs`, `crates/pipeline/src/error.rs` (add the `Net` variant), `crates/proto/src/constants.rs`

**Interfaces:**
- Consumes: `LatestSlot`, `FanOut`, `KeyframeRequest` (Tasks 5, 6); `brp_capture::{CaptureFrame, CaptureSession}`; `brp_codec::{FrameConverter, InputImage, VideoEncoder}`; `brp_net::{LiveSource, Subscription, SubscribeRejected}`.
- Produces: `brp_pipeline::Publisher` (`Clone`, implements `brp_net::LiveSource`) with `Publisher::start(live_id, preset_id, slot: Arc<LatestSlot<CaptureFrame>>, session: Box<dyn CaptureSession>, converter: Box<dyn FrameConverter>, encoder: Box<dyn VideoEncoder>) -> Publisher`, `params() -> &CodecParams`, `encoder_name() -> &'static str`, `stats() -> &PublisherStats`, `subscriber_count() -> usize`, `stop(&self)`. `PublisherStats { frames_encoded: AtomicU64, bytes_encoded: AtomicU64 }` plus `frames_dropped_at_input()` read from the slot.

The caller creates the slot first, hands `move |f| slot.put(f)` to the capture backend as the sink, then passes the started session and the slot here. That keeps capture start-up, which is async and portal-driven, out of this synchronous constructor.

- [ ] **Step 1: Add the idle retry constant**

Append to `crates/proto/src/constants.rs`:

```rust
/// Compositors deliver frames only on damage, so a viewer joining while the screen is static would
/// never see its requested keyframe. After this long without a new frame the last one is re-encoded.
pub const IDLE_KEYFRAME_RETRY: Duration = Duration::from_millis(500);
```

- [ ] **Step 2: Write the failing tests**

`crates/pipeline/tests/publisher.rs`:

```rust
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use brp_capture::{CaptureBackend, CaptureFrame, CaptureSession, SourceInfo, SourceRequest, SyntheticSource};
use brp_codec::EncoderConfig;
use brp_codec::fake::{FakeEncoder, SolidConverter};
use brp_net::{LiveSource, SubscribeRejected};
use brp_pipeline::{LatestSlot, Publisher};
use brp_proto::{Codec, PixelFormat, SourceKind};

fn cfg() -> EncoderConfig {
    EncoderConfig { width: 32, height: 16, fps: 60, bitrate_kbps: 5_000, codec: Codec::H264 }
}

#[tokio::test]
async fn subscriber_receives_a_keyframe_first_then_ordered_frames() {
    let slot = LatestSlot::new();
    let sink_slot = slot.clone();
    let session = SyntheticSource { width: 64, height: 32, fps: 60 }
        .start(SourceRequest { kind: SourceKind::Monitor, target_fps: 60 }, Box::new(move |f| sink_slot.put(f)))
        .await
        .unwrap();
    let publisher = Publisher::start(1, 1, slot, session, Box::new(SolidConverter::new(32, 16)), Box::new(FakeEncoder::new(cfg(), 30)));
    assert_eq!(publisher.encoder_name(), "fake");
    assert_eq!((publisher.params().width, publisher.params().height), (32, 16));

    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut sub = publisher.subscribe(1, 1).unwrap();
    let first = tokio::time::timeout(Duration::from_secs(2), sub.frames.recv()).await.unwrap().unwrap();
    assert!(first.keyframe, "a new subscriber must start on a keyframe");
    let mut prev = first.seq;
    for _ in 0..5 {
        let f = tokio::time::timeout(Duration::from_secs(2), sub.frames.recv()).await.unwrap().unwrap();
        assert!(f.seq > prev);
        prev = f.seq;
    }
    assert_eq!(publisher.subscriber_count(), 1);
    assert!(publisher.stats().frames_encoded.load(Ordering::Relaxed) >= 6);

    assert_eq!(publisher.subscribe(2, 1).unwrap_err(), SubscribeRejected::UnknownLive(2));
    assert_eq!(publisher.subscribe(1, 9).unwrap_err(), SubscribeRejected::UnknownPreset(9));

    drop(sub);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(publisher.subscriber_count(), 0, "dropped receivers are forgotten on the next push");
    publisher.stop();
}

struct StaticSession;

impl CaptureSession for StaticSession {
    fn info(&self) -> SourceInfo {
        SourceInfo { width: 8, height: 8, fps: 60 }
    }
    fn stop(self: Box<Self>) {}
}

#[tokio::test]
async fn static_screen_still_serves_a_late_subscriber_a_keyframe() {
    let slot = LatestSlot::new();
    let publisher = Publisher::start(1, 1, slot.clone(), Box::new(StaticSession), Box::new(SolidConverter::new(8, 8)), Box::new(FakeEncoder::new(cfg(), 1_000)));
    slot.put(CaptureFrame { width: 8, height: 8, stride: 32, format: PixelFormat::Bgra, data: vec![0; 256], capture_ts_us: 1 });
    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut sub = publisher.subscribe(1, 1).unwrap();
    let frame = tokio::time::timeout(Duration::from_millis(1_500), sub.frames.recv()).await.expect("re-encoded within the idle retry").unwrap();
    assert!(frame.keyframe);
    assert_eq!(frame.capture_ts_us, 1, "the last captured frame was re-encoded");
    publisher.stop();
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p brp-pipeline --test publisher`
Expected: compile errors for `Publisher`.

- [ ] **Step 4: Implement**

`crates/pipeline/src/publisher.rs`:

```rust
//! Capture slot -> convert -> encode -> fan-out, on one dedicated thread per preset.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use brp_capture::{CaptureFrame, CaptureSession};
use brp_codec::{FrameConverter, InputImage, VideoEncoder};
use brp_net::{LiveSource, SubscribeRejected, Subscription};
use brp_proto::CodecParams;
use brp_proto::constants::IDLE_KEYFRAME_RETRY;

use crate::fanout::{FanOut, KeyframeRequest};
use crate::slot::{LatestSlot, SlotWait};

#[derive(Default)]
pub struct PublisherStats {
    pub frames_encoded: AtomicU64,
    pub bytes_encoded: AtomicU64,
}

#[derive(Clone)]
pub struct Publisher {
    inner: Arc<Inner>,
}

struct Inner {
    live_id: u32,
    preset_id: u32,
    params: CodecParams,
    encoder_name: &'static str,
    fanout: Mutex<FanOut>,
    keyframe: KeyframeRequest,
    stop: AtomicBool,
    stats: PublisherStats,
    slot: Arc<LatestSlot<CaptureFrame>>,
    thread: Mutex<Option<JoinHandle<()>>>,
    session: Mutex<Option<Box<dyn CaptureSession>>>,
}

impl Publisher {
    pub fn start(
        live_id: u32,
        preset_id: u32,
        slot: Arc<LatestSlot<CaptureFrame>>,
        session: Box<dyn CaptureSession>,
        converter: Box<dyn FrameConverter>,
        encoder: Box<dyn VideoEncoder>,
    ) -> Self {
        let keyframe = KeyframeRequest::new();
        let inner = Arc::new(Inner {
            live_id,
            preset_id,
            params: encoder.params(),
            encoder_name: encoder.name(),
            fanout: Mutex::new(FanOut::new(keyframe.clone())),
            keyframe,
            stop: AtomicBool::new(false),
            stats: PublisherStats::default(),
            slot,
            thread: Mutex::new(None),
            session: Mutex::new(Some(session)),
        });
        let worker = inner.clone();
        let handle = thread::Builder::new()
            .name(format!("brp-encode-{live_id}-{preset_id}"))
            .spawn(move || encode_loop(worker, converter, encoder))
            .expect("spawning a thread only fails when the system is out of resources");
        *lock(&inner.thread) = Some(handle);
        Self { inner }
    }

    pub fn params(&self) -> &CodecParams {
        &self.inner.params
    }

    pub fn encoder_name(&self) -> &'static str {
        self.inner.encoder_name
    }

    pub fn stats(&self) -> &PublisherStats {
        &self.inner.stats
    }

    pub fn frames_dropped_at_input(&self) -> u64 {
        self.inner.slot.dropped()
    }

    pub fn subscriber_count(&self) -> usize {
        lock(&self.inner.fanout).subscriber_count()
    }

    pub fn stop(&self) {
        self.inner.stop.store(true, Ordering::Relaxed);
        self.inner.slot.close();
        if let Some(handle) = lock(&self.inner.thread).take() {
            let _ = handle.join();
        }
        if let Some(session) = lock(&self.inner.session).take() {
            session.stop();
        }
    }
}

impl LiveSource for Publisher {
    fn subscribe(&self, live_id: u32, preset_id: u32) -> Result<Subscription, SubscribeRejected> {
        if live_id != self.inner.live_id {
            return Err(SubscribeRejected::UnknownLive(live_id));
        }
        if preset_id != self.inner.preset_id {
            return Err(SubscribeRejected::UnknownPreset(preset_id));
        }
        let frames = lock(&self.inner.fanout).add();
        Ok(Subscription { params: self.inner.params.clone(), frames })
    }

    fn request_keyframe(&self, live_id: u32, preset_id: u32) {
        if live_id == self.inner.live_id && preset_id == self.inner.preset_id {
            self.inner.keyframe.request();
        }
    }
}

fn encode_loop(inner: Arc<Inner>, mut converter: Box<dyn FrameConverter>, mut encoder: Box<dyn VideoEncoder>) {
    let mut last: Option<CaptureFrame> = None;
    while !inner.stop.load(Ordering::Relaxed) {
        let frame = match inner.slot.take_timeout(IDLE_KEYFRAME_RETRY) {
            SlotWait::Value(f) => {
                last = Some(f);
                last.as_ref()
            }
            SlotWait::Timeout if inner.keyframe.pending() => last.as_ref(),
            SlotWait::Timeout => continue,
            SlotWait::Closed => break,
        };
        let Some(frame) = frame else { continue };
        let force = inner.keyframe.take_if_allowed(Instant::now());
        let image = InputImage {
            width: frame.width,
            height: frame.height,
            stride: frame.stride,
            format: frame.format,
            data: &frame.data,
            capture_ts_us: frame.capture_ts_us,
        };
        let raw = match converter.convert(&image) {
            Ok(raw) => raw,
            Err(e) => {
                tracing::error!(error = %e, "frame conversion failed");
                continue;
            }
        };
        match encoder.encode(&raw, force) {
            Ok(packets) => {
                for packet in packets {
                    inner.stats.frames_encoded.fetch_add(1, Ordering::Relaxed);
                    inner.stats.bytes_encoded.fetch_add(packet.data.len() as u64, Ordering::Relaxed);
                    lock(&inner.fanout).push(Arc::new(packet));
                }
            }
            Err(e) => tracing::error!(error = %e, "encode failed"),
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}
```

Add the `Net` variant to `PipelineError` in `crates/pipeline/src/error.rs` as shown in Task 5. Add to `crates/pipeline/src/lib.rs`: `pub mod publisher;` and `pub use publisher::{Publisher, PublisherStats};`.

- [ ] **Step 5: Run the tests and verify they pass**

Run: `cargo test -p brp-pipeline`
Expected: unit tests plus both publisher tests pass. fmt, clippy.

- [ ] **Step 6: Commit**

```bash
git add crates/proto/src/constants.rs crates/pipeline
git commit -m "feat: add publisher pipeline with idle keyframe retry"
```

### Task 11: Viewer pipeline

**Files:**
- Create: `crates/pipeline/src/viewer.rs`, `crates/pipeline/tests/viewer.rs`
- Modify: `crates/pipeline/src/lib.rs`

**Interfaces:**
- Consumes: `Reorder`, `IncomingFrame`, `LatestSlot` (Tasks 5, 7); `brp_net::ReceivedFrame`; `brp_codec::{RawFrame, VideoDecoder}`; `brp_proto::{EncodedFrame, ViewerMessage, constants::REORDER_MAX_WAIT}`.
- Produces: `brp_pipeline::Viewer::start(runtime: tokio::runtime::Handle, frames: Receiver<ReceivedFrame>, control: Sender<ViewerMessage>, decoder: Box<dyn VideoDecoder>, notify: FrameNotify) -> Viewer`, `Viewer::slot() -> Arc<LatestSlot<RawFrame>>`, `Viewer::stats() -> Arc<ViewerStats>`, `Viewer::stop(self)`; `FrameNotify = Arc<dyn Fn() + Send + Sync>`; `ViewerStats { frames_received, frames_decoded, keyframe_requests: AtomicU64 }`. The app's window passes a notify closure that posts a winit user event.

- [ ] **Step 1: Write the failing tests**

`crates/pipeline/tests/viewer.rs`:

```rust
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use brp_codec::fake::{FakeDecoder, FakeEncoder};
use brp_codec::{EncoderConfig, RawFrame, VideoEncoder};
use brp_net::ReceivedFrame;
use brp_pipeline::Viewer;
use brp_proto::constants::REORDER_MAX_WAIT;
use brp_proto::{Codec, EncodedFrame, FrameHeader, FrameKind, ViewerMessage};
use tokio::sync::mpsc;

fn encoded_frames(n: u64) -> Vec<EncodedFrame> {
    let mut enc = FakeEncoder::new(EncoderConfig { width: 8, height: 4, fps: 30, bitrate_kbps: 1_000, codec: Codec::H264 }, 1_000);
    (0..n).map(|i| enc.encode(&RawFrame::black(8, 4, i * 100), false).unwrap().remove(0)).collect()
}

fn received(f: &EncodedFrame) -> ReceivedFrame {
    ReceivedFrame {
        header: FrameHeader { live_id: 1, preset_id: 1, kind: FrameKind::Video, seq: f.seq, capture_ts_us: f.capture_ts_us, keyframe: f.keyframe, len: f.data.len() as u32 },
        payload: f.data.clone(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn decodes_in_sequence_order_and_publishes_latest_frame() {
    let frames = encoded_frames(3);
    let (tx, rx) = mpsc::channel(8);
    let (ctl_tx, _ctl_rx) = mpsc::channel(8);
    let notified = Arc::new(AtomicUsize::new(0));
    let n = notified.clone();
    let viewer = Viewer::start(tokio::runtime::Handle::current(), rx, ctl_tx, Box::new(FakeDecoder), Arc::new(move || {
        n.fetch_add(1, Ordering::SeqCst);
    }));

    tx.send(received(&frames[0])).await.unwrap();
    tx.send(received(&frames[2])).await.unwrap();
    tx.send(received(&frames[1])).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    assert_eq!(notified.load(Ordering::SeqCst), 3);
    let latest = viewer.slot().try_take().expect("a decoded frame is waiting");
    assert_eq!(latest.capture_ts_us, 200, "the newest frame wins");
    assert_eq!(viewer.stats().frames_decoded.load(Ordering::SeqCst), 3);
    viewer.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gap_that_outlives_the_wait_cap_requests_a_keyframe() {
    let frames = encoded_frames(5);
    let (tx, rx) = mpsc::channel(8);
    let (ctl_tx, mut ctl_rx) = mpsc::channel(8);
    let viewer = Viewer::start(tokio::runtime::Handle::current(), rx, ctl_tx, Box::new(FakeDecoder), Arc::new(|| {}));

    tx.send(received(&frames[0])).await.unwrap();
    tx.send(received(&frames[2])).await.unwrap();
    let msg = tokio::time::timeout(REORDER_MAX_WAIT * 3, ctl_rx.recv()).await.expect("request within the cap").unwrap();
    assert_eq!(msg, ViewerMessage::RequestKeyframe);
    assert_eq!(viewer.stats().keyframe_requests.load(Ordering::SeqCst), 1);

    let mut key = received(&frames[4]);
    key.header.keyframe = true;
    tx.send(key).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(viewer.stats().frames_decoded.load(Ordering::SeqCst), 2, "frame 0 and the recovery keyframe");
    viewer.stop();
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p brp-pipeline --test viewer`
Expected: compile errors for `Viewer`.

- [ ] **Step 3: Implement**

`crates/pipeline/src/viewer.rs`:

```rust
//! Network frames -> reorder -> decode -> latest-frame slot, on one dedicated thread per subscription.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Instant;

use brp_codec::{RawFrame, VideoDecoder};
use brp_net::ReceivedFrame;
use brp_proto::constants::REORDER_MAX_WAIT;
use brp_proto::{EncodedFrame, ViewerMessage};
use tokio::runtime::Handle;
use tokio::sync::mpsc::{Receiver, Sender};

use crate::reorder::{Drained, IncomingFrame, Reorder};
use crate::slot::LatestSlot;

pub type FrameNotify = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
pub struct ViewerStats {
    pub frames_received: AtomicU64,
    pub frames_decoded: AtomicU64,
    pub keyframe_requests: AtomicU64,
}

pub struct Viewer {
    slot: Arc<LatestSlot<RawFrame>>,
    stats: Arc<ViewerStats>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Viewer {
    pub fn start(
        runtime: Handle,
        frames: Receiver<ReceivedFrame>,
        control: Sender<ViewerMessage>,
        decoder: Box<dyn VideoDecoder>,
        notify: FrameNotify,
    ) -> Self {
        let slot = LatestSlot::new();
        let stats = Arc::new(ViewerStats::default());
        let stop = Arc::new(AtomicBool::new(false));
        let worker = DecodeLoop { runtime, frames, control, decoder, notify, slot: slot.clone(), stats: stats.clone(), stop: stop.clone() };
        let thread = thread::Builder::new()
            .name("brp-decode".into())
            .spawn(move || worker.run())
            .expect("spawning a thread only fails when the system is out of resources");
        Self { slot, stats, stop, thread: Some(thread) }
    }

    pub fn slot(&self) -> Arc<LatestSlot<RawFrame>> {
        self.slot.clone()
    }

    pub fn stats(&self) -> Arc<ViewerStats> {
        self.stats.clone()
    }

    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

struct DecodeLoop {
    runtime: Handle,
    frames: Receiver<ReceivedFrame>,
    control: Sender<ViewerMessage>,
    decoder: Box<dyn VideoDecoder>,
    notify: FrameNotify,
    slot: Arc<LatestSlot<RawFrame>>,
    stats: Arc<ViewerStats>,
    stop: Arc<AtomicBool>,
}

impl DecodeLoop {
    fn run(mut self) {
        let mut reorder = Reorder::new(REORDER_MAX_WAIT);
        // Polling at a quarter of the cap bounds how late a timed-out gap is noticed.
        let poll = REORDER_MAX_WAIT / 4;
        while !self.stop.load(Ordering::Relaxed) {
            let drained = match self.runtime.block_on(tokio::time::timeout(poll, self.frames.recv())) {
                Ok(Some(frame)) => {
                    self.stats.frames_received.fetch_add(1, Ordering::Relaxed);
                    reorder.push(IncomingFrame { header: frame.header, data: frame.payload }, Instant::now())
                }
                Ok(None) => break,
                Err(_elapsed) => reorder.poll(Instant::now()),
            };
            self.handle(drained);
        }
        self.slot.close();
    }

    fn handle(&mut self, drained: Drained) {
        if drained.request_keyframe {
            self.request_keyframe();
        }
        for frame in drained.ready {
            let encoded = EncodedFrame {
                seq: frame.header.seq,
                capture_ts_us: frame.header.capture_ts_us,
                keyframe: frame.header.keyframe,
                data: frame.data,
            };
            match self.decoder.decode(&encoded) {
                Ok(raws) => {
                    for raw in raws {
                        self.stats.frames_decoded.fetch_add(1, Ordering::Relaxed);
                        self.slot.put(raw);
                        (self.notify)();
                    }
                }
                Err(e) => {
                    tracing::warn!(seq = encoded.seq, error = %e, "decode failed, asking for a keyframe");
                    self.request_keyframe();
                }
            }
        }
    }

    fn request_keyframe(&self) {
        self.stats.keyframe_requests.fetch_add(1, Ordering::Relaxed);
        if self.control.try_send(ViewerMessage::RequestKeyframe).is_err() {
            tracing::debug!("control channel full or closed; keyframe request dropped");
        }
    }
}
```

Add to `crates/pipeline/src/lib.rs`: `pub mod viewer;` and `pub use viewer::{FrameNotify, Viewer, ViewerStats};`.

- [ ] **Step 4: Run the tests and verify they pass**

Run: `cargo test -p brp-pipeline`
Expected: all pass. fmt, clippy.

- [ ] **Step 5: Commit**

```bash
git add crates/pipeline
git commit -m "feat: add viewer pipeline with reorder and keyframe recovery"
```

### Task 12: FFmpeg FFI helpers and the swscale converter

**Files:**
- Modify: `Cargo.toml` (workspace dependencies), `crates/codec/Cargo.toml`, `crates/codec/src/lib.rs`, `crates/codec/src/error.rs`
- Create: `crates/codec/src/ffmpeg/mod.rs`, `crates/codec/src/ffmpeg/ffi.rs`, `crates/codec/src/ffmpeg/convert.rs`

**Interfaces:**
- Consumes: `RawFrame`, `InputImage`, `FrameConverter`, `CodecError` (Task 3).
- Produces: `brp_codec::ffmpeg::SwsConverter::new(src_width, src_height, src_format: PixelFormat, dst_width, dst_height) -> Result<SwsConverter, CodecError>` implementing `FrameConverter`; crate-private `ffmpeg::ffi::{check, Frame, Packet, CodecContext, BufferRef, set_opt, set_opt_int, init_logging}` used by Tasks 13 to 15. New `CodecError::EncoderMissing(&'static str)` and `CodecError::DecoderMissing(&'static str)`.

Verified facts for ffmpeg-sys-next 9.0.0 against FFmpeg 8.1 headers: `default-features = false, features = ["avcodec", "swscale"]` links avcodec, avutil, swscale via pkg-config; bindgen runs at build time and needs the `clang` binary on the machine. C enums are Rust enums (`AVPixelFormat::AV_PIX_FMT_NV12`), `AVFrame.format` is `c_int`, `AVCodecContext.flags` is `c_int` while `AV_CODEC_FLAG_*` are `c_uint`, `AVERROR(e)` is a `const fn`, `EAGAIN` and `AVERROR_EOF` are constants, swscale flags are the `SwsFlags` enum, `AV_ERROR_MAX_STRING_SIZE` is `usize`. `avcodec_close` does not exist; contexts are released with `avcodec_free_context`.

- [ ] **Step 1: Dependencies**

Root `Cargo.toml` `[workspace.dependencies]`:

```toml
ffmpeg-sys-next = { version = "9.0", default-features = false, features = ["avcodec", "swscale"] }
```

`crates/codec/Cargo.toml` `[dependencies]`: add `ffmpeg-sys-next.workspace = true`.

Confirm the machine can build it: `sudo dnf install clang clang-devel` if `clang --version` fails, then `cargo build -p brp-codec`. The build script prints `cargo:rustc-link-lib=avcodec` and friends on success.

- [ ] **Step 2: Write the failing converter test**

`crates/codec/src/ffmpeg/convert.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use brp_proto::PixelFormat;

    use super::*;
    use crate::traits::{FrameConverter, InputImage};

    fn solid(width: u32, height: u32, bgra: [u8; 4]) -> Vec<u8> {
        bgra.iter().copied().cycle().take((width * height * 4) as usize).collect()
    }

    fn convert(width: u32, height: u32, pixels: &[u8], dst: (u32, u32)) -> RawFrame {
        let mut conv = SwsConverter::new(width, height, PixelFormat::Bgra, dst.0, dst.1).unwrap();
        let img = InputImage { width, height, stride: (width * 4) as usize, format: PixelFormat::Bgra, data: pixels, capture_ts_us: 42 };
        conv.convert(&img).unwrap()
    }

    #[test]
    fn white_maps_to_limited_range_white() {
        let out = convert(8, 4, &solid(8, 4, [255, 255, 255, 255]), (8, 4));
        assert_eq!((out.width, out.height, out.capture_ts_us), (8, 4, 42));
        assert!(out.y.iter().all(|&v| (233..=237).contains(&v)), "luma {:?}", &out.y[..8]);
        assert!(out.uv.iter().all(|&v| (126..=130).contains(&v)), "chroma {:?}", &out.uv[..8]);
    }

    #[test]
    fn black_maps_to_limited_range_black_and_scaling_halves_dimensions() {
        let out = convert(16, 8, &solid(16, 8, [0, 0, 0, 255]), (8, 4));
        assert_eq!((out.width, out.height), (8, 4));
        assert_eq!(out.y.len(), 8 * 4);
        assert_eq!(out.uv.len(), 8 * 2);
        assert!(out.y.iter().all(|&v| (14..=18).contains(&v)), "luma {:?}", &out.y[..8]);
        assert!(out.validate().is_ok());
    }

    #[test]
    fn short_input_buffer_is_rejected() {
        let mut conv = SwsConverter::new(8, 4, PixelFormat::Bgra, 8, 4).unwrap();
        let img = InputImage { width: 8, height: 4, stride: 32, format: PixelFormat::Bgra, data: &[0u8; 10], capture_ts_us: 0 };
        assert!(matches!(conv.convert(&img), Err(CodecError::InvalidFrame(_))));
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p brp-codec ffmpeg`
Expected: compile errors for `SwsConverter`.

- [ ] **Step 4: Implement the FFI helpers**

Add to `crates/codec/src/error.rs` inside the enum:

```rust
    #[error("FFmpeg has no encoder named {0}")]
    EncoderMissing(&'static str),
    #[error("FFmpeg has no decoder named {0}")]
    DecoderMissing(&'static str),
```

`crates/codec/src/ffmpeg/mod.rs`:

```rust
//! FFmpeg-backed codec implementations over the raw `ffmpeg-sys-next` bindings.

pub(crate) mod ffi;

pub mod convert;

pub use convert::SwsConverter;
```

`crates/codec/src/ffmpeg/ffi.rs`:

```rust
//! Thin RAII layer over the raw bindings. Every `unsafe` block in the crate goes through here or
//! through the encoder and decoder modules, never through callers.

use std::ffi::{CStr, CString, c_char, c_int};
use std::ptr;
use std::sync::Once;

use ffmpeg_sys_next as ff;

use crate::error::CodecError;

pub(crate) fn init_logging() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe { ff::av_log_set_level(ff::AV_LOG_WARNING as c_int) });
}

pub(crate) fn check(call: &'static str, code: c_int) -> Result<c_int, CodecError> {
    if code < 0 { Err(CodecError::Ffmpeg { call, code, message: error_string(code) }) } else { Ok(code) }
}

pub(crate) fn error_string(code: c_int) -> String {
    let mut buf = [0 as c_char; ff::AV_ERROR_MAX_STRING_SIZE];
    unsafe {
        ff::av_strerror(code, buf.as_mut_ptr(), buf.len());
        CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned()
    }
}

pub(crate) const fn again() -> c_int {
    ff::AVERROR(ff::EAGAIN)
}

pub(crate) fn null_error(call: &'static str) -> CodecError {
    CodecError::Ffmpeg { call, code: ff::AVERROR(ff::ENOMEM), message: "returned null".into() }
}

pub(crate) fn cstring(s: &str) -> Result<CString, CodecError> {
    CString::new(s).map_err(|_| CodecError::InvalidFrame(format!("string contains a NUL byte: {s:?}")))
}

pub(crate) struct Frame(pub(crate) *mut ff::AVFrame);

impl Frame {
    pub(crate) fn new() -> Result<Self, CodecError> {
        let p = unsafe { ff::av_frame_alloc() };
        if p.is_null() { Err(null_error("av_frame_alloc")) } else { Ok(Self(p)) }
    }

    pub(crate) fn unref(&mut self) {
        unsafe { ff::av_frame_unref(self.0) }
    }
}

impl Drop for Frame {
    fn drop(&mut self) {
        unsafe { ff::av_frame_free(&mut self.0) }
    }
}

pub(crate) struct Packet(pub(crate) *mut ff::AVPacket);

impl Packet {
    pub(crate) fn new() -> Result<Self, CodecError> {
        let p = unsafe { ff::av_packet_alloc() };
        if p.is_null() { Err(null_error("av_packet_alloc")) } else { Ok(Self(p)) }
    }

    pub(crate) fn unref(&mut self) {
        unsafe { ff::av_packet_unref(self.0) }
    }

    pub(crate) fn data(&self) -> &[u8] {
        unsafe {
            let p = &*self.0;
            if p.data.is_null() || p.size <= 0 { &[] } else { std::slice::from_raw_parts(p.data, p.size as usize) }
        }
    }

    pub(crate) fn is_keyframe(&self) -> bool {
        unsafe { (*self.0).flags & ff::AV_PKT_FLAG_KEY != 0 }
    }
}

impl Drop for Packet {
    fn drop(&mut self) {
        unsafe { ff::av_packet_free(&mut self.0) }
    }
}

pub(crate) struct CodecContext(pub(crate) *mut ff::AVCodecContext);

impl CodecContext {
    pub(crate) fn alloc(codec: *const ff::AVCodec) -> Result<Self, CodecError> {
        let p = unsafe { ff::avcodec_alloc_context3(codec) };
        if p.is_null() { Err(null_error("avcodec_alloc_context3")) } else { Ok(Self(p)) }
    }

    pub(crate) fn open(&mut self, codec: *const ff::AVCodec) -> Result<(), CodecError> {
        check("avcodec_open2", unsafe { ff::avcodec_open2(self.0, codec, ptr::null_mut()) }).map(|_| ())
    }

    pub(crate) fn extradata(&self) -> Vec<u8> {
        unsafe {
            let c = &*self.0;
            if c.extradata.is_null() || c.extradata_size <= 0 { Vec::new() } else { std::slice::from_raw_parts(c.extradata, c.extradata_size as usize).to_vec() }
        }
    }
}

impl Drop for CodecContext {
    fn drop(&mut self) {
        unsafe { ff::avcodec_free_context(&mut self.0) }
    }
}

pub(crate) struct BufferRef(pub(crate) *mut ff::AVBufferRef);

impl BufferRef {
    /// Takes ownership of a reference returned by FFmpeg. Null is an allocation failure.
    pub(crate) fn from_raw(call: &'static str, p: *mut ff::AVBufferRef) -> Result<Self, CodecError> {
        if p.is_null() { Err(null_error(call)) } else { Ok(Self(p)) }
    }

    /// A new reference for handing to a context field; FFmpeg releases it with the context.
    pub(crate) fn new_ref(&self, call: &'static str) -> Result<*mut ff::AVBufferRef, CodecError> {
        let p = unsafe { ff::av_buffer_ref(self.0) };
        if p.is_null() { Err(null_error(call)) } else { Ok(p) }
    }
}

impl Drop for BufferRef {
    fn drop(&mut self) {
        unsafe { ff::av_buffer_unref(&mut self.0) }
    }
}

/// Sets a private option on the codec's implementation, such as an NVENC preset.
pub(crate) fn set_opt(ctx: &CodecContext, name: &str, value: &str) -> Result<(), CodecError> {
    let (k, v) = (cstring(name)?, cstring(value)?);
    check("av_opt_set", unsafe { ff::av_opt_set((*ctx.0).priv_data, k.as_ptr(), v.as_ptr(), 0) }).map(|_| ())
}

pub(crate) fn set_opt_int(ctx: &CodecContext, name: &str, value: i64) -> Result<(), CodecError> {
    let k = cstring(name)?;
    check("av_opt_set_int", unsafe { ff::av_opt_set_int((*ctx.0).priv_data, k.as_ptr(), value, 0) }).map(|_| ())
}

// FFmpeg contexts, frames, and packets are safe to move between threads as long as only one thread
// touches them at a time, which the owning encoder or decoder guarantees.
unsafe impl Send for Frame {}
unsafe impl Send for Packet {}
unsafe impl Send for CodecContext {}
unsafe impl Send for BufferRef {}
```

- [ ] **Step 5: Implement the converter**

`crates/codec/src/ffmpeg/convert.rs` (above its tests):

```rust
use std::ffi::c_int;
use std::ptr;

use brp_proto::PixelFormat;
use ffmpeg_sys_next as ff;

use crate::error::CodecError;
use crate::ffmpeg::ffi::{check, init_logging};
use crate::raw::RawFrame;
use crate::traits::{FrameConverter, InputImage};

/// libswscale: packed 32-bit RGB in, NV12 at the preset's size out, BT.709 limited range.
pub struct SwsConverter {
    ctx: *mut ff::SwsContext,
    src: (u32, u32, PixelFormat),
    dst_width: u32,
    dst_height: u32,
}

fn av_pix_fmt(format: PixelFormat) -> ff::AVPixelFormat {
    match format {
        PixelFormat::Bgra => ff::AVPixelFormat::AV_PIX_FMT_BGRA,
        PixelFormat::Bgrx => ff::AVPixelFormat::AV_PIX_FMT_BGR0,
        PixelFormat::Rgba => ff::AVPixelFormat::AV_PIX_FMT_RGBA,
        PixelFormat::Rgbx => ff::AVPixelFormat::AV_PIX_FMT_RGB0,
    }
}

impl SwsConverter {
    pub fn new(src_width: u32, src_height: u32, src_format: PixelFormat, dst_width: u32, dst_height: u32) -> Result<Self, CodecError> {
        init_logging();
        if dst_width % 2 != 0 || dst_height % 2 != 0 || dst_width == 0 || dst_height == 0 {
            return Err(CodecError::InvalidFrame(format!("NV12 output needs even non-zero dimensions, got {dst_width}x{dst_height}")));
        }
        let ctx = unsafe {
            ff::sws_getContext(
                src_width as c_int,
                src_height as c_int,
                av_pix_fmt(src_format),
                dst_width as c_int,
                dst_height as c_int,
                ff::AVPixelFormat::AV_PIX_FMT_NV12,
                ff::SwsFlags::SWS_BILINEAR as c_int,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
            )
        };
        if ctx.is_null() {
            return Err(CodecError::InvalidFrame(format!("swscale rejected {src_width}x{src_height} {src_format:?} -> {dst_width}x{dst_height} NV12")));
        }
        // Full-range RGB in, BT.709 limited-range YUV out, matching what the viewer's shader assumes.
        unsafe {
            let coeffs = ff::sws_getCoefficients(ff::SWS_CS_ITU709 as c_int);
            check("sws_setColorspaceDetails", ff::sws_setColorspaceDetails(ctx, coeffs, 1, coeffs, 0, 0, 1 << 16, 1 << 16))?;
        }
        Ok(Self { ctx, src: (src_width, src_height, src_format), dst_width, dst_height })
    }
}

impl FrameConverter for SwsConverter {
    fn convert(&mut self, src: &InputImage<'_>) -> Result<RawFrame, CodecError> {
        let needed = src.stride * src.height as usize;
        if src.data.len() < needed || src.stride < src.width as usize * src.format.bytes_per_pixel() {
            return Err(CodecError::InvalidFrame(format!("input holds {} bytes but stride {} x height {} needs {needed}", src.data.len(), src.stride, src.height)));
        }
        if self.src != (src.width, src.height, src.format) {
            // The compositor can renegotiate the stream size; rebuild rather than fail the live.
            *self = Self::new(src.width, src.height, src.format, self.dst_width, self.dst_height)?;
        }
        let mut out = RawFrame::black(self.dst_width, self.dst_height, src.capture_ts_us);
        let src_planes = [src.data.as_ptr(), ptr::null(), ptr::null(), ptr::null()];
        let src_strides = [src.stride as c_int, 0, 0, 0];
        let dst_planes = [out.y.as_mut_ptr(), out.uv.as_mut_ptr(), ptr::null_mut(), ptr::null_mut()];
        let dst_strides = [out.y_stride as c_int, out.uv_stride as c_int, 0, 0];
        let rows = unsafe {
            ff::sws_scale(self.ctx, src_planes.as_ptr(), src_strides.as_ptr(), 0, src.height as c_int, dst_planes.as_ptr(), dst_strides.as_ptr())
        };
        check("sws_scale", rows)?;
        Ok(out)
    }
}

impl Drop for SwsConverter {
    fn drop(&mut self) {
        unsafe { ff::sws_freeContext(self.ctx) }
    }
}

// One converter is owned by one encoder thread; swscale contexts have no thread affinity.
unsafe impl Send for SwsConverter {}
```

Add to `crates/codec/src/lib.rs`: `pub mod ffmpeg;` and `pub use ffmpeg::SwsConverter;`.

- [ ] **Step 6: Run the tests and verify they pass**

Run: `cargo test -p brp-codec`
Expected: fake-codec tests and the three converter tests pass. If `SWS_CS_ITU709` or `SwsFlags` fail to resolve, the machine has FFmpeg older than 7.1; the plan requires 7.1 or newer. fmt, clippy (allow `clippy::missing_safety_doc` is not needed because no `unsafe fn` is public).

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock crates/codec
git commit -m "feat: add FFmpeg FFI helpers and swscale NV12 converter"
```

### Task 13: FFmpeg encoder for software-input encoders and the probe order

**Files:**
- Create: `crates/codec/src/ffmpeg/encoder.rs`, `crates/codec/src/select.rs`, `crates/codec/tests/codec_smoke.rs`
- Modify: `crates/codec/src/ffmpeg/mod.rs`, `crates/codec/src/lib.rs`

**Interfaces:**
- Consumes: `ffi` helpers (Task 12), `EncoderConfig`, `VideoEncoder`, `RawFrame`, `EncodedFrame`.
- Produces: `brp_codec::ffmpeg::FfmpegEncoder::open(name: &'static str, cfg: &EncoderConfig) -> Result<FfmpegEncoder, CodecError>` implementing `VideoEncoder`; `brp_codec::select::{open_encoder(cfg: &EncoderConfig) -> Result<Box<dyn VideoEncoder>, CodecError>, open_encoder_auto(cfg: EncoderConfig, codec: Option<Codec>) -> Result<Box<dyn VideoEncoder>, CodecError>, PROBE_ORDER}`. Task 14 adds the VAAPI branch inside `open_encoder`; Task 15 adds `open_decoder` to the same file.

Verified FFmpeg 8.1 option names. NVENC: `preset` p1..p7, `tune` ull, `rc` cbr, `zerolatency` 1, `delay` 0, `forced-idr` 1, `rc-lookahead` 0; a frame with `pict_type = AV_PICTURE_TYPE_I` becomes an IDR when `forced-idr` is 1. libsvtav1: `preset` 0..13, `svtav1-params` as `key=value:key=value`; CBR requires the low-delay prediction structure; the wrapper copies `rc_max_rate` into SVT's `max_bit_rate`, which SVT rejects outside CRF mode, so `rc_max_rate` stays 0 for that encoder and CBR is selected with `rc=2`. libsvtav1 accepts `yuv420p` but not `nv12`, so NV12 is de-interleaved for it.

- [ ] **Step 1: Write the failing tests**

`crates/codec/tests/codec_smoke.rs`:

```rust
//! Real-encoder checks. The round-trip tests run only with BRP_CODEC_TESTS=1 because they need
//! whatever encoders this machine's FFmpeg and GPU provide.

use brp_codec::select::{open_encoder, open_encoder_auto};
use brp_codec::{CodecError, EncoderConfig, RawFrame};
use brp_proto::Codec;

fn cfg(codec: Codec) -> EncoderConfig {
    EncoderConfig { width: 320, height: 240, fps: 30, bitrate_kbps: 2_000, codec }
}

fn gated() -> bool {
    std::env::var_os("BRP_CODEC_TESTS").is_some()
}

#[test]
fn opening_an_encoder_never_panics() {
    for codec in [Codec::Hevc, Codec::H264, Codec::Av1] {
        match open_encoder(&cfg(codec)) {
            Ok(enc) => eprintln!("{codec:?}: {}", enc.name()),
            Err(CodecError::NoEncoder(c)) => assert_eq!(c, codec),
            Err(other) => panic!("unexpected error {other}"),
        }
    }
}

#[test]
fn auto_selection_falls_back_to_some_encoder() {
    if !gated() {
        return;
    }
    let enc = open_encoder_auto(cfg(Codec::Hevc), None).expect("at least the software AV1 encoder exists");
    eprintln!("auto picked {} ({:?})", enc.name(), enc.params().codec);
}

#[test]
fn every_available_encoder_produces_a_keyframe_first() {
    if !gated() {
        return;
    }
    for codec in [Codec::Hevc, Codec::H264, Codec::Av1] {
        let Ok(mut enc) = open_encoder(&cfg(codec)) else { continue };
        let mut packets = Vec::new();
        for i in 0..10u64 {
            packets.extend(enc.encode(&RawFrame::black(320, 240, i * 33_333), false).unwrap());
        }
        assert!(!packets.is_empty(), "{} produced nothing", enc.name());
        assert!(packets[0].keyframe, "{} first packet must be a keyframe", enc.name());
        assert!(packets.windows(2).all(|w| w[1].seq == w[0].seq + 1));
        let forced = enc.encode(&RawFrame::black(320, 240, 999_999), true).unwrap();
        assert!(forced.iter().any(|p| p.keyframe), "{} ignored the forced keyframe", enc.name());
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p brp-codec --test codec_smoke`
Expected: compile errors for `select`.

- [ ] **Step 3: Implement the encoder**

`crates/codec/src/ffmpeg/encoder.rs`:

```rust
use std::collections::VecDeque;
use std::ffi::c_int;

use brp_proto::{CodecParams, EncodedFrame};
use ffmpeg_sys_next as ff;

use crate::error::CodecError;
use crate::ffmpeg::ffi::{CodecContext, Frame, Packet, again, check, cstring, init_logging, set_opt, set_opt_int};
use crate::raw::RawFrame;
use crate::traits::{EncoderConfig, VideoEncoder};

#[derive(Clone, Copy, PartialEq, Eq)]
enum InputLayout {
    Nv12,
    I420,
}

pub struct FfmpegEncoder {
    ctx: CodecContext,
    frame: Frame,
    packet: Packet,
    name: &'static str,
    cfg: EncoderConfig,
    layout: InputLayout,
    next_seq: u64,
    next_pts: i64,
    /// Capture timestamps waiting for their packet, keyed by pts, because some encoders emit late.
    in_flight: VecDeque<(i64, u64)>,
}

impl FfmpegEncoder {
    pub fn open(name: &'static str, cfg: &EncoderConfig) -> Result<Self, CodecError> {
        init_logging();
        let cname = cstring(name)?;
        let codec = unsafe { ff::avcodec_find_encoder_by_name(cname.as_ptr()) };
        if codec.is_null() {
            return Err(CodecError::EncoderMissing(name));
        }
        let layout = if name == "libsvtav1" { InputLayout::I420 } else { InputLayout::Nv12 };
        let mut ctx = CodecContext::alloc(codec)?;
        unsafe {
            let c = &mut *ctx.0;
            c.width = cfg.width as c_int;
            c.height = cfg.height as c_int;
            c.time_base = ff::AVRational { num: 1, den: cfg.fps as c_int };
            c.framerate = ff::AVRational { num: cfg.fps as c_int, den: 1 };
            c.pix_fmt = match layout {
                InputLayout::Nv12 => ff::AVPixelFormat::AV_PIX_FMT_NV12,
                InputLayout::I420 => ff::AVPixelFormat::AV_PIX_FMT_YUV420P,
            };
            c.bit_rate = i64::from(cfg.bitrate_kbps) * 1000;
            // Rate-control buffer of one frame keeps bitrate flat at the cost of some quality swings.
            c.rc_buffer_size = (c.bit_rate / i64::from(cfg.fps.max(1))) as c_int;
            c.gop_size = c_int::MAX;
            c.max_b_frames = 0;
            c.flags |= ff::AV_CODEC_FLAG_LOW_DELAY as c_int;
            if layout == InputLayout::Nv12 {
                c.rc_max_rate = c.bit_rate;
            }
        }
        apply_low_latency_options(name, &ctx)?;
        ctx.open(codec)?;

        let frame = Frame::new()?;
        unsafe {
            let f = &mut *frame.0;
            f.width = cfg.width as c_int;
            f.height = cfg.height as c_int;
            f.format = (*ctx.0).pix_fmt as c_int;
            check("av_frame_get_buffer", ff::av_frame_get_buffer(frame.0, 0))?;
        }
        Ok(Self { ctx, frame, packet: Packet::new()?, name, cfg: *cfg, layout, next_seq: 0, next_pts: 0, in_flight: VecDeque::new() })
    }

    fn fill_frame(&mut self, src: &RawFrame) -> Result<(), CodecError> {
        check("av_frame_make_writable", unsafe { ff::av_frame_make_writable(self.frame.0) })?;
        let f = unsafe { &mut *self.frame.0 };
        let (w, h) = (src.width as usize, src.height as usize);
        let rows = src.chroma_rows();
        unsafe {
            for row in 0..h {
                let dst = f.data[0].add(row * f.linesize[0] as usize);
                std::ptr::copy_nonoverlapping(src.y.as_ptr().add(row * src.y_stride), dst, w);
            }
            match self.layout {
                InputLayout::Nv12 => {
                    for row in 0..rows {
                        let dst = f.data[1].add(row * f.linesize[1] as usize);
                        std::ptr::copy_nonoverlapping(src.uv.as_ptr().add(row * src.uv_stride), dst, w);
                    }
                }
                InputLayout::I420 => {
                    for row in 0..rows {
                        let uv = &src.uv[row * src.uv_stride..row * src.uv_stride + w];
                        let u = f.data[1].add(row * f.linesize[1] as usize);
                        let v = f.data[2].add(row * f.linesize[2] as usize);
                        for (i, pair) in uv.chunks_exact(2).enumerate() {
                            *u.add(i) = pair[0];
                            *v.add(i) = pair[1];
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn drain(&mut self, out: &mut Vec<EncodedFrame>) -> Result<(), CodecError> {
        loop {
            let r = unsafe { ff::avcodec_receive_packet(self.ctx.0, self.packet.0) };
            if r == again() || r == ff::AVERROR_EOF {
                return Ok(());
            }
            check("avcodec_receive_packet", r)?;
            let pts = unsafe { (*self.packet.0).pts };
            let capture_ts_us = self.take_capture_ts(pts);
            out.push(EncodedFrame { seq: self.next_seq, capture_ts_us, keyframe: self.packet.is_keyframe(), data: self.packet.data().to_vec() });
            self.next_seq += 1;
            self.packet.unref();
        }
    }

    fn take_capture_ts(&mut self, pts: i64) -> u64 {
        while let Some(&(front_pts, ts)) = self.in_flight.front() {
            self.in_flight.pop_front();
            if front_pts >= pts {
                return ts;
            }
        }
        0
    }
}

fn apply_low_latency_options(name: &str, ctx: &CodecContext) -> Result<(), CodecError> {
    match name {
        "h264_nvenc" | "hevc_nvenc" | "av1_nvenc" => {
            set_opt(ctx, "preset", "p4")?;
            set_opt(ctx, "tune", "ull")?;
            set_opt(ctx, "rc", "cbr")?;
            set_opt_int(ctx, "zerolatency", 1)?;
            set_opt_int(ctx, "delay", 0)?;
            set_opt_int(ctx, "forced-idr", 1)?;
            set_opt_int(ctx, "rc-lookahead", 0)?;
        }
        "h264_amf" | "hevc_amf" | "av1_amf" => {
            set_opt(ctx, "usage", "ultralowlatency")?;
            set_opt(ctx, "rc", "cbr")?;
        }
        "h264_qsv" | "hevc_qsv" | "av1_qsv" => {
            set_opt_int(ctx, "async_depth", 1)?;
            set_opt_int(ctx, "low_power", 1)?;
        }
        "libsvtav1" => {
            set_opt(ctx, "preset", "10")?;
            set_opt(ctx, "svtav1-params", "rc=2:pred-struct=1:rtc=1")?;
        }
        _ => {}
    }
    Ok(())
}

impl VideoEncoder for FfmpegEncoder {
    fn name(&self) -> &'static str {
        self.name
    }

    fn params(&self) -> CodecParams {
        CodecParams { codec: self.cfg.codec, width: self.cfg.width, height: self.cfg.height, fps: self.cfg.fps, extradata: self.ctx.extradata() }
    }

    fn encode(&mut self, frame: &RawFrame, force_keyframe: bool) -> Result<Vec<EncodedFrame>, CodecError> {
        frame.validate()?;
        if (frame.width, frame.height) != (self.cfg.width, self.cfg.height) {
            return Err(CodecError::InvalidFrame(format!("encoder is {}x{} but frame is {}x{}", self.cfg.width, self.cfg.height, frame.width, frame.height)));
        }
        self.fill_frame(frame)?;
        unsafe {
            let f = &mut *self.frame.0;
            f.pts = self.next_pts;
            f.pict_type = if force_keyframe { ff::AVPictureType::AV_PICTURE_TYPE_I } else { ff::AVPictureType::AV_PICTURE_TYPE_NONE };
            f.flags = if force_keyframe { ff::AV_FRAME_FLAG_KEY } else { 0 };
        }
        self.in_flight.push_back((self.next_pts, frame.capture_ts_us));
        self.next_pts += 1;
        check("avcodec_send_frame", unsafe { ff::avcodec_send_frame(self.ctx.0, self.frame.0) })?;
        let mut out = Vec::with_capacity(1);
        self.drain(&mut out)?;
        Ok(out)
    }
}
```

- [ ] **Step 4: Implement selection**

`crates/codec/src/select.rs`:

```rust
//! Encoder and decoder selection: hardware first, in the order the spec fixes, software last.

use brp_proto::Codec;

use crate::error::CodecError;
use crate::ffmpeg::encoder::FfmpegEncoder;
use crate::traits::{EncoderConfig, VideoEncoder};

/// Spec 5.4 order: NVENC, AMD, Intel, then the OS path (VAAPI on Linux), then software AV1.
pub const PROBE_ORDER: &[(&str, Codec)] = &[
    ("hevc_nvenc", Codec::Hevc),
    ("h264_nvenc", Codec::H264),
    ("av1_nvenc", Codec::Av1),
    ("hevc_amf", Codec::Hevc),
    ("h264_amf", Codec::H264),
    ("av1_amf", Codec::Av1),
    ("hevc_qsv", Codec::Hevc),
    ("h264_qsv", Codec::H264),
    ("av1_qsv", Codec::Av1),
    ("hevc_vaapi", Codec::Hevc),
    ("h264_vaapi", Codec::H264),
    ("av1_vaapi", Codec::Av1),
    ("libsvtav1", Codec::Av1),
];

pub fn open_encoder(cfg: &EncoderConfig) -> Result<Box<dyn VideoEncoder>, CodecError> {
    for (name, codec) in PROBE_ORDER.iter().filter(|(_, c)| *c == cfg.codec) {
        match open_named(name, cfg) {
            Ok(enc) => {
                tracing::info!(encoder = name, "encoder opened");
                return Ok(enc);
            }
            Err(e) => tracing::debug!(encoder = name, error = %e, "encoder unavailable"),
        }
    }
    Err(CodecError::NoEncoder(cfg.codec))
}

/// With no codec forced, prefer HEVC, then H.264, then the software AV1 fallback.
pub fn open_encoder_auto(cfg: EncoderConfig, codec: Option<Codec>) -> Result<Box<dyn VideoEncoder>, CodecError> {
    let order: Vec<Codec> = match codec {
        Some(c) => vec![c],
        None => vec![Codec::Hevc, Codec::H264, Codec::Av1],
    };
    let mut last = CodecError::NoEncoder(cfg.codec);
    for c in order {
        match open_encoder(&EncoderConfig { codec: c, ..cfg }) {
            Ok(enc) => return Ok(enc),
            Err(e) => last = e,
        }
    }
    Err(last)
}

fn open_named(name: &'static str, cfg: &EncoderConfig) -> Result<Box<dyn VideoEncoder>, CodecError> {
    Ok(Box::new(FfmpegEncoder::open(name, cfg)?))
}
```

In `crates/codec/src/ffmpeg/mod.rs` add `pub mod encoder;` and `pub use encoder::FfmpegEncoder;`. In `crates/codec/src/lib.rs` add `pub mod select;` and `pub use select::{open_encoder, open_encoder_auto};`.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p brp-codec --test codec_smoke` (ungated) then `BRP_CODEC_TESTS=1 cargo test -p brp-codec --test codec_smoke -- --nocapture`.
Expected ungated: `opening_an_encoder_never_panics` passes and prints which encoders opened. Expected gated on the development machine: HEVC and H264 open through NVENC, AV1 through NVENC as well, all produce a keyframe first and honour the forced keyframe. If `libsvtav1` rejects the CBR parameters, drop `rc=2` from the params string and rely on `bit_rate`; record which variant worked in the commit message. fmt, clippy.

- [ ] **Step 6: Commit**

```bash
git add crates/codec
git commit -m "feat: add FFmpeg encoder with low-latency tuning and probe order"
```

### Task 14: VAAPI encoder with hardware frame upload

**Files:**
- Create: `crates/codec/src/ffmpeg/vaapi.rs`
- Modify: `crates/codec/src/ffmpeg/mod.rs`, `crates/codec/src/select.rs`, `crates/codec/tests/codec_smoke.rs`

**Interfaces:**
- Consumes: `ffi` helpers, `EncoderConfig`, `VideoEncoder`.
- Produces: `brp_codec::ffmpeg::VaapiEncoder::open(name: &'static str, cfg: &EncoderConfig) -> Result<VaapiEncoder, CodecError>` implementing `VideoEncoder`; `select::open_named` routes every `*_vaapi` name here.

Verified FFmpeg 8.1 facts: VAAPI encoders accept only frames living in an `AVHWFramesContext` whose `format` is `AV_PIX_FMT_VAAPI`; `hw_frames_ctx` must be set before `avcodec_open2` and `pix_fmt` must equal the frames context format. Per frame: `av_hwframe_get_buffer(hw_frames_ctx, hw_frame, 0)` then `av_hwframe_transfer_data(hw_frame, sw_frame, 0)`. A frame with `pict_type = AV_PICTURE_TYPE_I` forces an IDR. Options: `rc_mode` CBR, `async_depth` 1 for lowest latency. The frames context is reached through `(*frames_ref).data as *mut AVHWFramesContext`.

- [ ] **Step 1: Add the gated test**

Append to `crates/codec/tests/codec_smoke.rs`:

```rust
#[test]
fn vaapi_encoder_encodes_when_a_render_node_exists() {
    if !gated() {
        return;
    }
    use brp_codec::ffmpeg::VaapiEncoder;
    use brp_codec::VideoEncoder;
    let mut enc = match VaapiEncoder::open("hevc_vaapi", &cfg(Codec::Hevc)).or_else(|_| VaapiEncoder::open("h264_vaapi", &cfg(Codec::H264))) {
        Ok(enc) => enc,
        Err(e) => {
            eprintln!("skipping: no VAAPI encoder ({e})");
            return;
        }
    };
    let mut packets = Vec::new();
    for i in 0..10u64 {
        packets.extend(enc.encode(&RawFrame::black(320, 240, i), false).unwrap());
    }
    assert!(!packets.is_empty() && packets[0].keyframe, "{}", enc.name());
}
```

- [ ] **Step 2: Run the test to verify it fails to compile**

Run: `cargo test -p brp-codec --test codec_smoke`
Expected: `VaapiEncoder` unresolved.

- [ ] **Step 3: Implement**

`crates/codec/src/ffmpeg/vaapi.rs`:

```rust
use std::collections::VecDeque;
use std::ffi::c_int;
use std::ptr;

use brp_proto::{CodecParams, EncodedFrame};
use ffmpeg_sys_next as ff;

use crate::error::CodecError;
use crate::ffmpeg::ffi::{BufferRef, CodecContext, Frame, Packet, again, check, cstring, init_logging, set_opt, set_opt_int};
use crate::raw::RawFrame;
use crate::traits::{EncoderConfig, VideoEncoder};

/// Surfaces the frames context keeps ready. Enough for the encoder's pipeline depth plus our in-flight frame.
const SURFACE_POOL_SIZE: c_int = 20;

pub struct VaapiEncoder {
    ctx: CodecContext,
    _device: BufferRef,
    _frames: BufferRef,
    sw_frame: Frame,
    hw_frame: Frame,
    packet: Packet,
    name: &'static str,
    cfg: EncoderConfig,
    next_seq: u64,
    next_pts: i64,
    in_flight: VecDeque<(i64, u64)>,
}

impl VaapiEncoder {
    pub fn open(name: &'static str, cfg: &EncoderConfig) -> Result<Self, CodecError> {
        init_logging();
        let cname = cstring(name)?;
        let codec = unsafe { ff::avcodec_find_encoder_by_name(cname.as_ptr()) };
        if codec.is_null() {
            return Err(CodecError::EncoderMissing(name));
        }

        let mut device_ptr = ptr::null_mut();
        check("av_hwdevice_ctx_create", unsafe {
            ff::av_hwdevice_ctx_create(&mut device_ptr, ff::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI, ptr::null(), ptr::null_mut(), 0)
        })?;
        let device = BufferRef::from_raw("av_hwdevice_ctx_create", device_ptr)?;

        let frames = BufferRef::from_raw("av_hwframe_ctx_alloc", unsafe { ff::av_hwframe_ctx_alloc(device.0) })?;
        unsafe {
            let fc = &mut *((*frames.0).data as *mut ff::AVHWFramesContext);
            fc.format = ff::AVPixelFormat::AV_PIX_FMT_VAAPI;
            fc.sw_format = ff::AVPixelFormat::AV_PIX_FMT_NV12;
            fc.width = cfg.width as c_int;
            fc.height = cfg.height as c_int;
            fc.initial_pool_size = SURFACE_POOL_SIZE;
        }
        check("av_hwframe_ctx_init", unsafe { ff::av_hwframe_ctx_init(frames.0) })?;

        let mut ctx = CodecContext::alloc(codec)?;
        unsafe {
            let c = &mut *ctx.0;
            c.width = cfg.width as c_int;
            c.height = cfg.height as c_int;
            c.time_base = ff::AVRational { num: 1, den: cfg.fps as c_int };
            c.framerate = ff::AVRational { num: cfg.fps as c_int, den: 1 };
            c.pix_fmt = ff::AVPixelFormat::AV_PIX_FMT_VAAPI;
            c.hw_frames_ctx = frames.new_ref("av_buffer_ref")?;
            c.bit_rate = i64::from(cfg.bitrate_kbps) * 1000;
            c.rc_max_rate = c.bit_rate;
            c.rc_buffer_size = (c.bit_rate / i64::from(cfg.fps.max(1))) as c_int;
            c.gop_size = c_int::MAX;
            c.max_b_frames = 0;
            c.flags |= ff::AV_CODEC_FLAG_LOW_DELAY as c_int;
        }
        set_opt(&ctx, "rc_mode", "CBR")?;
        set_opt_int(&ctx, "async_depth", 1)?;
        ctx.open(codec)?;

        let sw_frame = Frame::new()?;
        unsafe {
            let f = &mut *sw_frame.0;
            f.width = cfg.width as c_int;
            f.height = cfg.height as c_int;
            f.format = ff::AVPixelFormat::AV_PIX_FMT_NV12 as c_int;
            check("av_frame_get_buffer", ff::av_frame_get_buffer(sw_frame.0, 0))?;
        }
        Ok(Self {
            ctx,
            _device: device,
            _frames: frames,
            sw_frame,
            hw_frame: Frame::new()?,
            packet: Packet::new()?,
            name,
            cfg: *cfg,
            next_seq: 0,
            next_pts: 0,
            in_flight: VecDeque::new(),
        })
    }

    fn upload(&mut self, src: &RawFrame) -> Result<(), CodecError> {
        check("av_frame_make_writable", unsafe { ff::av_frame_make_writable(self.sw_frame.0) })?;
        let f = unsafe { &mut *self.sw_frame.0 };
        let w = src.width as usize;
        unsafe {
            for row in 0..src.height as usize {
                ptr::copy_nonoverlapping(src.y.as_ptr().add(row * src.y_stride), f.data[0].add(row * f.linesize[0] as usize), w);
            }
            for row in 0..src.chroma_rows() {
                ptr::copy_nonoverlapping(src.uv.as_ptr().add(row * src.uv_stride), f.data[1].add(row * f.linesize[1] as usize), w);
            }
        }
        self.hw_frame.unref();
        unsafe {
            check("av_hwframe_get_buffer", ff::av_hwframe_get_buffer((*self.ctx.0).hw_frames_ctx, self.hw_frame.0, 0))?;
            check("av_hwframe_transfer_data", ff::av_hwframe_transfer_data(self.hw_frame.0, self.sw_frame.0, 0))?;
        }
        Ok(())
    }

    fn drain(&mut self, out: &mut Vec<EncodedFrame>) -> Result<(), CodecError> {
        loop {
            let r = unsafe { ff::avcodec_receive_packet(self.ctx.0, self.packet.0) };
            if r == again() || r == ff::AVERROR_EOF {
                return Ok(());
            }
            check("avcodec_receive_packet", r)?;
            let pts = unsafe { (*self.packet.0).pts };
            let mut capture_ts_us = 0;
            while let Some((front_pts, ts)) = self.in_flight.pop_front() {
                if front_pts >= pts {
                    capture_ts_us = ts;
                    break;
                }
            }
            out.push(EncodedFrame { seq: self.next_seq, capture_ts_us, keyframe: self.packet.is_keyframe(), data: self.packet.data().to_vec() });
            self.next_seq += 1;
            self.packet.unref();
        }
    }
}

impl VideoEncoder for VaapiEncoder {
    fn name(&self) -> &'static str {
        self.name
    }

    fn params(&self) -> CodecParams {
        CodecParams { codec: self.cfg.codec, width: self.cfg.width, height: self.cfg.height, fps: self.cfg.fps, extradata: self.ctx.extradata() }
    }

    fn encode(&mut self, frame: &RawFrame, force_keyframe: bool) -> Result<Vec<EncodedFrame>, CodecError> {
        frame.validate()?;
        if (frame.width, frame.height) != (self.cfg.width, self.cfg.height) {
            return Err(CodecError::InvalidFrame(format!("encoder is {}x{} but frame is {}x{}", self.cfg.width, self.cfg.height, frame.width, frame.height)));
        }
        self.upload(frame)?;
        unsafe {
            let f = &mut *self.hw_frame.0;
            f.pts = self.next_pts;
            f.pict_type = if force_keyframe { ff::AVPictureType::AV_PICTURE_TYPE_I } else { ff::AVPictureType::AV_PICTURE_TYPE_NONE };
        }
        self.in_flight.push_back((self.next_pts, frame.capture_ts_us));
        self.next_pts += 1;
        check("avcodec_send_frame", unsafe { ff::avcodec_send_frame(self.ctx.0, self.hw_frame.0) })?;
        let mut out = Vec::with_capacity(1);
        self.drain(&mut out)?;
        Ok(out)
    }
}
```

The packet drain duplicates fifteen lines of `FfmpegEncoder`. Leave it duplicated; a shared helper is worth extracting only when a third encoder path appears.

In `crates/codec/src/ffmpeg/mod.rs` add `pub mod vaapi;` and `pub use vaapi::VaapiEncoder;`. In `crates/codec/src/select.rs` change `open_named`:

```rust
fn open_named(name: &'static str, cfg: &EncoderConfig) -> Result<Box<dyn VideoEncoder>, CodecError> {
    if name.ends_with("_vaapi") {
        Ok(Box::new(crate::ffmpeg::VaapiEncoder::open(name, cfg)?))
    } else {
        Ok(Box::new(FfmpegEncoder::open(name, cfg)?))
    }
}
```

- [ ] **Step 4: Run the tests**

Run: `BRP_CODEC_TESTS=1 cargo test -p brp-codec --test codec_smoke -- --nocapture`
Expected: on the development machine the VAAPI test either encodes through the AMD render node or prints a skip reason such as a missing `libva` driver. It must not panic. fmt, clippy.

- [ ] **Step 5: Commit**

```bash
git add crates/codec
git commit -m "feat: add VAAPI encoder with hardware frame upload"
```

### Task 15: FFmpeg decoder with optional hardware acceleration

**Files:**
- Create: `crates/codec/src/ffmpeg/decoder.rs`
- Modify: `crates/codec/src/ffmpeg/mod.rs`, `crates/codec/src/select.rs`, `crates/codec/src/lib.rs`, `crates/codec/tests/codec_smoke.rs`

**Interfaces:**
- Consumes: `ffi` helpers, `CodecParams`, `EncodedFrame`, `RawFrame`, `VideoDecoder`.
- Produces: `brp_codec::ffmpeg::FfmpegDecoder::open(params: &CodecParams, hw: HwDecode) -> Result<FfmpegDecoder, CodecError>` implementing `VideoDecoder`, `enum HwDecode { Auto, Software }`, `FfmpegDecoder::name() -> &'static str`; `brp_codec::select::open_decoder(params: &CodecParams) -> Result<Box<dyn VideoDecoder>, CodecError>`.

Verified FFmpeg 8.1 facts: hardware decode works by setting `hw_device_ctx` and a `get_format` callback that returns the hardware pixel format when the decoder offers it; the pairs are `AV_PIX_FMT_VAAPI` with `AV_HWDEVICE_TYPE_VAAPI` and `AV_PIX_FMT_CUDA` with `AV_HWDEVICE_TYPE_CUDA`; `av_hwframe_transfer_data(sw, hw, 0)` with `sw.format` left at `AV_PIX_FMT_NONE` yields the frames context `sw_format`, NV12 for 8-bit 4:2:0. Software decoders are `h264`, `hevc`, and `libdav1d`, and they output `yuv420p`, so decoded frames are interleaved into NV12. `avcodec_get_hw_config(codec, i)` enumerates `AVCodecHWConfig { pix_fmt, methods, device_type }`.

- [ ] **Step 1: Extend the gated smoke test to a round trip**

Append to `crates/codec/tests/codec_smoke.rs`:

```rust
#[test]
fn every_available_encoder_round_trips_through_the_decoder() {
    if !gated() {
        return;
    }
    use brp_codec::select::open_decoder;
    for codec in [Codec::Hevc, Codec::H264, Codec::Av1] {
        let Ok(mut enc) = open_encoder(&cfg(codec)) else { continue };
        let mut dec = open_decoder(&enc.params()).expect("a decoder for what we can encode");
        let mut decoded = 0;
        for i in 0..12u64 {
            for packet in enc.encode(&RawFrame::black(320, 240, i * 10), false).unwrap() {
                for raw in dec.decode(&packet).unwrap() {
                    assert_eq!((raw.width, raw.height), (320, 240));
                    raw.validate().unwrap();
                    assert!(raw.y.iter().all(|&v| v < 40), "black frame decoded as luma {}", raw.y[0]);
                    decoded += 1;
                }
            }
        }
        assert!(decoded >= 8, "{} -> decoder produced only {decoded} frames", enc.name());
    }
}
```

- [ ] **Step 2: Run the test to verify it fails to compile**

Run: `cargo test -p brp-codec --test codec_smoke`
Expected: `open_decoder` unresolved.

- [ ] **Step 3: Implement the decoder**

`crates/codec/src/ffmpeg/decoder.rs`:

```rust
use std::ffi::{c_int, c_void};
use std::ptr;

use brp_proto::{Codec, CodecParams, EncodedFrame};
use ffmpeg_sys_next as ff;

use crate::error::CodecError;
use crate::ffmpeg::ffi::{BufferRef, CodecContext, Frame, Packet, again, check, cstring, init_logging};
use crate::raw::RawFrame;
use crate::traits::VideoDecoder;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HwDecode {
    Auto,
    Software,
}

pub struct FfmpegDecoder {
    ctx: CodecContext,
    _device: Option<BufferRef>,
    hw_pix_fmt: Option<ff::AVPixelFormat>,
    packet: Packet,
    frame: Frame,
    sw_frame: Frame,
    name: &'static str,
}

/// Device types to try, in order. VAAPI covers AMD and Intel; CUDA covers NVIDIA.
const HW_DEVICE_ORDER: [(ff::AVHWDeviceType, &str); 2] =
    [(ff::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI, "vaapi"), (ff::AVHWDeviceType::AV_HWDEVICE_TYPE_CUDA, "cuda")];

impl FfmpegDecoder {
    pub fn open(params: &CodecParams, hw: HwDecode) -> Result<Self, CodecError> {
        init_logging();
        let (codec, name) = find_decoder(params.codec)?;
        let mut ctx = CodecContext::alloc(codec)?;

        let mut device = None;
        let mut hw_pix_fmt = None;
        if hw == HwDecode::Auto && params.codec != Codec::Av1 {
            for (device_type, label) in HW_DEVICE_ORDER {
                if let Some((dev, fmt)) = try_hw_device(codec, device_type) {
                    unsafe {
                        (*ctx.0).hw_device_ctx = dev.new_ref("av_buffer_ref")?;
                        // The callback cannot capture state, so the wanted format travels in `opaque`.
                        (*ctx.0).opaque = fmt as isize as *mut c_void;
                        (*ctx.0).get_format = Some(pick_hw_format);
                    }
                    tracing::info!(decoder = name, device = label, "hardware decoding enabled");
                    device = Some(dev);
                    hw_pix_fmt = Some(fmt);
                    break;
                }
            }
        }

        unsafe {
            let c = &mut *ctx.0;
            c.flags |= ff::AV_CODEC_FLAG_LOW_DELAY as c_int;
            c.thread_type = ff::FF_THREAD_SLICE as c_int;
            if !params.extradata.is_empty() {
                let size = params.extradata.len();
                let buf = ff::av_mallocz(size + ff::AV_INPUT_BUFFER_PADDING_SIZE as usize) as *mut u8;
                if buf.is_null() {
                    return Err(CodecError::Ffmpeg { call: "av_mallocz", code: ff::AVERROR(ff::ENOMEM), message: "returned null".into() });
                }
                ptr::copy_nonoverlapping(params.extradata.as_ptr(), buf, size);
                c.extradata = buf;
                c.extradata_size = size as c_int;
            }
        }
        ctx.open(codec)?;
        Ok(Self { ctx, _device: device, hw_pix_fmt, packet: Packet::new()?, frame: Frame::new()?, sw_frame: Frame::new()?, name })
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn is_hardware(&self) -> bool {
        self.hw_pix_fmt.is_some()
    }
}

fn find_decoder(codec: Codec) -> Result<(*const ff::AVCodec, &'static str), CodecError> {
    let (ptr, name) = unsafe {
        match codec {
            Codec::H264 => (ff::avcodec_find_decoder(ff::AVCodecID::AV_CODEC_ID_H264), "h264"),
            Codec::Hevc => (ff::avcodec_find_decoder(ff::AVCodecID::AV_CODEC_ID_HEVC), "hevc"),
            Codec::Av1 => (ff::avcodec_find_decoder_by_name(cstring("libdav1d")?.as_ptr()), "libdav1d"),
        }
    };
    if ptr.is_null() { Err(CodecError::DecoderMissing(name)) } else { Ok((ptr, name)) }
}

fn try_hw_device(codec: *const ff::AVCodec, device_type: ff::AVHWDeviceType) -> Option<(BufferRef, ff::AVPixelFormat)> {
    let mut fmt = None;
    for i in 0.. {
        let config = unsafe { ff::avcodec_get_hw_config(codec, i) };
        if config.is_null() {
            break;
        }
        let config = unsafe { &*config };
        if config.device_type == device_type && config.methods & ff::AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX as c_int != 0 {
            fmt = Some(config.pix_fmt);
            break;
        }
    }
    let fmt = fmt?;
    let mut dev = ptr::null_mut();
    let rc = unsafe { ff::av_hwdevice_ctx_create(&mut dev, device_type, ptr::null(), ptr::null_mut(), 0) };
    if rc < 0 {
        return None;
    }
    Some((BufferRef::from_raw("av_hwdevice_ctx_create", dev).ok()?, fmt))
}

unsafe extern "C" fn pick_hw_format(ctx: *mut ff::AVCodecContext, formats: *const ff::AVPixelFormat) -> ff::AVPixelFormat {
    unsafe {
        let wanted = (*ctx).opaque as isize as c_int;
        let mut p = formats;
        while *p != ff::AVPixelFormat::AV_PIX_FMT_NONE {
            if *p as c_int == wanted {
                return *p;
            }
            p = p.add(1);
        }
        // The hardware path is not on offer for this stream; the first entry is the software format.
        *formats
    }
}

impl VideoDecoder for FfmpegDecoder {
    fn decode(&mut self, frame: &EncodedFrame) -> Result<Vec<RawFrame>, CodecError> {
        let size = c_int::try_from(frame.data.len()).map_err(|_| CodecError::InvalidFrame("packet larger than c_int".into()))?;
        unsafe {
            check("av_new_packet", ff::av_new_packet(self.packet.0, size))?;
            ptr::copy_nonoverlapping(frame.data.as_ptr(), (*self.packet.0).data, frame.data.len());
            // Decoders pass pts through untouched, so the capture timestamp rides along with the picture.
            (*self.packet.0).pts = frame.capture_ts_us as i64;
            (*self.packet.0).dts = frame.capture_ts_us as i64;
            if frame.keyframe {
                (*self.packet.0).flags |= ff::AV_PKT_FLAG_KEY;
            }
        }
        let sent = unsafe { ff::avcodec_send_packet(self.ctx.0, self.packet.0) };
        self.packet.unref();
        check("avcodec_send_packet", sent)?;

        let mut out = Vec::with_capacity(1);
        loop {
            let r = unsafe { ff::avcodec_receive_frame(self.ctx.0, self.frame.0) };
            if r == again() || r == ff::AVERROR_EOF {
                return Ok(out);
            }
            check("avcodec_receive_frame", r)?;
            let decoded = unsafe { &*self.frame.0 };
            let source: *const ff::AVFrame = match self.hw_pix_fmt {
                Some(hw) if decoded.format == hw as c_int => {
                    self.sw_frame.unref();
                    check("av_hwframe_transfer_data", unsafe { ff::av_hwframe_transfer_data(self.sw_frame.0, self.frame.0, 0) })?;
                    unsafe { (*self.sw_frame.0).pts = decoded.pts };
                    self.sw_frame.0
                }
                _ => self.frame.0,
            };
            out.push(raw_from_avframe(unsafe { &*source })?);
            self.frame.unref();
        }
    }
}

fn raw_from_avframe(f: &ff::AVFrame) -> Result<RawFrame, CodecError> {
    let (w, h) = (f.width as u32, f.height as u32);
    let mut out = RawFrame::black(w, h, f.pts.max(0) as u64);
    let rows = out.chroma_rows();
    let width = w as usize;
    unsafe {
        for row in 0..h as usize {
            ptr::copy_nonoverlapping(f.data[0].add(row * f.linesize[0] as usize), out.y.as_mut_ptr().add(row * out.y_stride), width);
        }
        if f.format == ff::AVPixelFormat::AV_PIX_FMT_NV12 as c_int {
            for row in 0..rows {
                ptr::copy_nonoverlapping(f.data[1].add(row * f.linesize[1] as usize), out.uv.as_mut_ptr().add(row * out.uv_stride), width);
            }
        } else if f.format == ff::AVPixelFormat::AV_PIX_FMT_YUV420P as c_int || f.format == ff::AVPixelFormat::AV_PIX_FMT_YUVJ420P as c_int {
            for row in 0..rows {
                let u = f.data[1].add(row * f.linesize[1] as usize);
                let v = f.data[2].add(row * f.linesize[2] as usize);
                let dst = &mut out.uv[row * out.uv_stride..row * out.uv_stride + width];
                for (i, pair) in dst.chunks_exact_mut(2).enumerate() {
                    pair[0] = *u.add(i);
                    pair[1] = *v.add(i);
                }
            }
        } else {
            return Err(CodecError::InvalidFrame(format!("decoder produced pixel format {}, expected NV12 or YUV420P", f.format)));
        }
    }
    Ok(out)
}
```

Add to `crates/codec/src/ffmpeg/mod.rs`: `pub mod decoder;` and `pub use decoder::{FfmpegDecoder, HwDecode};`. Add to `crates/codec/src/select.rs`:

```rust
use brp_proto::CodecParams;

use crate::ffmpeg::decoder::{FfmpegDecoder, HwDecode};
use crate::traits::VideoDecoder;

pub fn open_decoder(params: &CodecParams) -> Result<Box<dyn VideoDecoder>, CodecError> {
    match FfmpegDecoder::open(params, HwDecode::Auto) {
        Ok(dec) => {
            tracing::info!(decoder = dec.name(), hardware = dec.is_hardware(), "decoder opened");
            Ok(Box::new(dec))
        }
        Err(e) => {
            tracing::warn!(error = %e, "hardware decoder failed, falling back to software");
            let dec = FfmpegDecoder::open(params, HwDecode::Software).map_err(|_| CodecError::NoDecoder(params.codec))?;
            Ok(Box::new(dec))
        }
    }
}
```

Add `pub use select::open_decoder;` to `crates/codec/src/lib.rs`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p brp-codec` then `BRP_CODEC_TESTS=1 cargo test -p brp-codec --test codec_smoke -- --nocapture`.
Expected: every encoder that opened round-trips at least 8 decoded 320x240 frames. Watch the log line for `hardware = true` on this machine for HEVC and H.264 through VAAPI or CUDA. If VAAPI opens but fails to decode NVENC output, that is a driver limitation; the software fallback must still make the test pass. fmt, clippy.

- [ ] **Step 5: Commit**

```bash
git add crates/codec
git commit -m "feat: add FFmpeg decoder with hardware acceleration and software fallback"
```

### Task 16: Linux capture through the desktop portal and PipeWire

**Files:**
- Modify: `Cargo.toml` (workspace dependencies), `crates/capture/Cargo.toml`, `crates/capture/src/lib.rs`, `crates/proto/src/constants.rs`
- Create: `crates/capture/src/linux/mod.rs`, `crates/capture/src/linux/portal.rs`, `crates/capture/src/linux/pipewire.rs`, `crates/capture/examples/portal_dump.rs`

**Interfaces:**
- Consumes: `CaptureBackend`, `CaptureSession`, `CaptureFrame`, `SourceInfo`, `SourceRequest`, `FrameSink`, `CaptureError` (Task 4); `brp_proto::{PixelFormat, SourceKind, monotonic_us}`.
- Produces: `brp_capture::linux::PortalCapture` (unit struct) implementing `CaptureBackend`, exported as `brp_capture::PortalCapture` under `cfg(target_os = "linux")`.

Verified facts for ashpd 0.13.13 and pipewire 0.10.1. ashpd: `default = ["tokio"]`, the `screencast` feature must be enabled, methods take option structs, `start(&session, None, Default::default()).await?.response()?` returns `Streams`, `open_pipe_wire_remote(&session, Default::default()).await? -> OwnedFd`. pipewire: owning types are `MainLoopRc`, `ContextRc`, `CoreRc`, `StreamBox`; `context.connect_fd_rc(fd, None)`; listener via `add_local_listener_with_user_data(..).param_changed(..).process(..).register()?` whose result must be kept alive; format parsed with `VideoInfoRaw::parse`; buffers via `dequeue_buffer()`, `datas_mut()`, `chunk()`, `data()`; the loop is stopped from another thread through `pipewire::channel`; nothing in the crate is `Send`, so the whole PipeWire loop lives on one dedicated thread. Compositors only emit frames on damage, and omitting the modifier property selects shared-memory buffers.

- [ ] **Step 1: Dependencies and constants**

Root `Cargo.toml` `[workspace.dependencies]`:

```toml
ashpd = { version = "0.13", features = ["screencast"] }
pipewire = "0.10"
```

`crates/capture/Cargo.toml`:

```toml
[target.'cfg(target_os = "linux")'.dependencies]
ashpd.workspace = true
pipewire.workspace = true
tokio = { workspace = true, features = ["rt", "sync", "time"] }

[dev-dependencies]
tokio = { workspace = true, features = ["rt", "rt-multi-thread", "macros", "time"] }
```

Append to `crates/proto/src/constants.rs`:

```rust
/// The compositor negotiates the stream format within a frame or two; this covers a slow first connection.
pub const PORTAL_FORMAT_TIMEOUT: Duration = Duration::from_secs(10);
```

Install `clang` and `clang-devel` if not present; bindgen for libspa needs the compiler's resource headers.

- [ ] **Step 2: Write the failing unit tests for the pure helpers**

`crates/capture/src/linux/pipewire.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fps_prefers_a_fixed_rate_then_max_rate_then_the_request() {
        assert_eq!(negotiated_fps((60, 1), (0, 1), 30), 60);
        assert_eq!(negotiated_fps((0, 1), (144, 1), 30), 144);
        assert_eq!(negotiated_fps((0, 1), (0, 1), 30), 30);
        assert_eq!(negotiated_fps((30000, 1001), (0, 1), 60), 30);
    }

    #[test]
    fn only_32_bit_packed_formats_are_accepted() {
        use pipewire::spa::param::video::VideoFormat;
        assert_eq!(pixel_format(VideoFormat::BGRx), Some(PixelFormat::Bgrx));
        assert_eq!(pixel_format(VideoFormat::BGRA), Some(PixelFormat::Bgra));
        assert_eq!(pixel_format(VideoFormat::RGBx), Some(PixelFormat::Rgbx));
        assert_eq!(pixel_format(VideoFormat::RGBA), Some(PixelFormat::Rgba));
        assert_eq!(pixel_format(VideoFormat::NV12), None);
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p brp-capture`
Expected: compile errors, module `linux` missing.

- [ ] **Step 4: Implement the portal half**

`crates/capture/src/linux/portal.rs`:

```rust
use std::os::fd::OwnedFd;

use ashpd::desktop::PersistMode;
use ashpd::desktop::screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType, Stream};
use ashpd::enumflags2::BitFlags;
use brp_proto::SourceKind;

use crate::error::CaptureError;

/// Keeps the portal session alive; the compositor stops the stream when it is closed.
/// A task owns the session object so this handle never has to name its lifetime-laden type.
pub(crate) struct PortalHandle {
    _keep_alive: tokio::sync::oneshot::Sender<()>,
}

pub(crate) struct PortalStream {
    pub node_id: u32,
    pub fd: OwnedFd,
    pub handle: PortalHandle,
}

pub(crate) async fn open_screencast(kind: SourceKind) -> Result<PortalStream, CaptureError> {
    let proxy = Screencast::new().await.map_err(portal_error)?;
    let session = proxy.create_session(Default::default()).await.map_err(portal_error)?;
    let sources: BitFlags<SourceType> = match kind {
        SourceKind::Monitor => BitFlags::from(SourceType::Monitor),
        SourceKind::Window => BitFlags::from(SourceType::Window),
    };
    proxy
        .select_sources(
            &session,
            SelectSourcesOptions::default()
                .set_cursor_mode(CursorMode::Embedded)
                .set_sources(sources)
                .set_multiple(false)
                .set_persist_mode(PersistMode::DoNot),
        )
        .await
        .map_err(portal_error)?;
    let streams = proxy.start(&session, None, Default::default()).await.map_err(portal_error)?.response().map_err(portal_error)?;
    let stream: Stream = streams.streams().first().cloned().ok_or(CaptureError::PortalDenied)?;
    let fd = proxy.open_pipe_wire_remote(&session, Default::default()).await.map_err(portal_error)?;
    let (keep_alive, released) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        let _session = session;
        let _ = released.await;
    });
    Ok(PortalStream { node_id: stream.pipe_wire_node_id(), fd, handle: PortalHandle { _keep_alive: keep_alive } })
}

fn portal_error(e: ashpd::Error) -> CaptureError {
    match e {
        // The user closed the picker or the compositor refused; both read as "denied" to the caller.
        ashpd::Error::Response(_) | ashpd::Error::NoResponse => CaptureError::PortalDenied,
        other => CaptureError::Portal(other.to_string()),
    }
}
```

- [ ] **Step 5: Implement the PipeWire half**

`crates/capture/src/linux/pipewire.rs` (above its tests):

```rust
//! Consumes the portal's PipeWire node on a dedicated thread and pushes frames into the sink.

use std::io::Cursor;
use std::os::fd::OwnedFd;
use std::sync::mpsc;

use brp_proto::{PixelFormat, monotonic_us};
use ::pipewire as pw;
use pw::spa;
use pw::spa::buffer::DataType;
use pw::spa::param::format::{FormatProperties, MediaSubtype, MediaType};
use pw::spa::param::video::{VideoFormat, VideoInfoRaw};
use pw::spa::param::{ParamType, format_utils};
use pw::spa::pod::serialize::PodSerializer;
use pw::spa::pod::{ChoiceValue, Object, Pod, Property, Value};
use pw::spa::utils::{Choice, ChoiceEnum, ChoiceFlags, Direction, Fraction, Id, Rectangle, SpaTypes};

use crate::error::CaptureError;
use crate::frame::{CaptureFrame, FrameSink, SourceInfo};

/// Sent once the format is known, or when the stream cannot be used.
pub(crate) enum PwEvent {
    Format(SourceInfo),
    Error(CaptureError),
}

struct UserData {
    info: VideoInfoRaw,
    format: Option<PixelFormat>,
    size: (u32, u32),
    events: mpsc::Sender<PwEvent>,
    sink: FrameSink,
    target_fps: u32,
}

pub(crate) fn run_stream(
    fd: OwnedFd,
    node_id: u32,
    target_fps: u32,
    events: mpsc::Sender<PwEvent>,
    sink: FrameSink,
    quit: pw::channel::Receiver<()>,
) -> Result<(), CaptureError> {
    let mainloop = pw::main_loop::MainLoopRc::new(None).map_err(pw_error)?;
    let context = pw::context::ContextRc::new(&mainloop, None).map_err(pw_error)?;
    let core = context.connect_fd_rc(fd, None).map_err(pw_error)?;
    let stream = pw::stream::StreamBox::new(
        &core,
        "brp-screen-capture",
        pw::properties::properties! {
            *pw::keys::MEDIA_TYPE => "Video",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Screen",
        },
    )
    .map_err(pw_error)?;

    let user_data = UserData { info: VideoInfoRaw::default(), format: None, size: (0, 0), events, sink, target_fps };
    let _listener = stream
        .add_local_listener_with_user_data(user_data)
        .param_changed(|_stream, ud, id, param| {
            let Some(param) = param else { return };
            if id != ParamType::Format.as_raw() {
                return;
            }
            let Ok((media_type, media_subtype)) = format_utils::parse_format(param) else { return };
            if media_type != MediaType::Video || media_subtype != MediaSubtype::Raw || ud.info.parse(param).is_err() {
                return;
            }
            let size = ud.info.size();
            ud.size = (size.width, size.height);
            match pixel_format(ud.info.format()) {
                Some(f) => ud.format = Some(f),
                None => {
                    let _ = ud.events.send(PwEvent::Error(CaptureError::UnsupportedFormat(format!("{:?}", ud.info.format()))));
                    return;
                }
            }
            let fr = ud.info.framerate();
            let max = ud.info.max_framerate();
            let fps = negotiated_fps((fr.num, fr.denom), (max.num, max.denom), ud.target_fps);
            let _ = ud.events.send(PwEvent::Format(SourceInfo { width: size.width, height: size.height, fps }));
        })
        .process(|stream, ud| {
            let Some(mut buffer) = stream.dequeue_buffer() else { return };
            let Some(format) = ud.format else { return };
            let (width, height) = ud.size;
            let capture_ts_us = monotonic_us();
            let datas = buffer.datas_mut();
            let Some(data) = datas.first_mut() else { return };
            let data_type = data.type_();
            if data_type != DataType::MemPtr && data_type != DataType::MemFd {
                return;
            }
            let chunk = data.chunk();
            let (offset, size) = (chunk.offset() as usize, chunk.size() as usize);
            let stride = match chunk.stride() {
                s if s > 0 => s as usize,
                _ => width as usize * format.bytes_per_pixel(),
            };
            let Some(bytes) = data.data() else { return };
            let Some(pixels) = bytes.get(offset..offset + size) else { return };
            (ud.sink)(CaptureFrame { width, height, stride, format, data: pixels.to_vec(), capture_ts_us });
        })
        .register()
        .map_err(pw_error)?;

    let format_pod = enum_format_pod(target_fps);
    let mut params = [Pod::from_bytes(&format_pod).ok_or_else(|| CaptureError::PipeWire("format pod did not serialize".into()))?];
    stream
        .connect(Direction::Input, Some(node_id), pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS, &mut params)
        .map_err(pw_error)?;

    let _quit = quit.attach(mainloop.loop_(), {
        let mainloop = mainloop.clone();
        move |_| mainloop.quit()
    });
    mainloop.run();
    Ok(())
}

fn pw_error(e: pw::Error) -> CaptureError {
    CaptureError::PipeWire(e.to_string())
}

pub(crate) fn pixel_format(format: VideoFormat) -> Option<PixelFormat> {
    match format {
        VideoFormat::BGRx => Some(PixelFormat::Bgrx),
        VideoFormat::BGRA => Some(PixelFormat::Bgra),
        VideoFormat::RGBx => Some(PixelFormat::Rgbx),
        VideoFormat::RGBA => Some(PixelFormat::Rgba),
        _ => None,
    }
}

/// A fixed `framerate` wins; `0/1` means variable rate, in which case `max_framerate` is the cap.
pub(crate) fn negotiated_fps(framerate: (u32, u32), max_framerate: (u32, u32), target: u32) -> u32 {
    let ratio = |(num, denom): (u32, u32)| if num == 0 || denom == 0 { 0 } else { (f64::from(num) / f64::from(denom)).round() as u32 };
    match (ratio(framerate), ratio(max_framerate)) {
        (fixed, _) if fixed > 0 => fixed,
        (_, max) if max > 0 => max,
        _ => target.max(1),
    }
}

/// Offers every packed 32-bit format, any size, and asks for `target_fps`. No modifier property, so
/// the compositor hands out shared-memory buffers instead of DMA-BUF.
fn enum_format_pod(target_fps: u32) -> Vec<u8> {
    let id = |v: u32| Value::Id(Id(v));
    let obj = Object {
        type_: SpaTypes::ObjectParamFormat.as_raw(),
        id: ParamType::EnumFormat.as_raw(),
        properties: vec![
            Property::new(FormatProperties::MediaType.as_raw(), id(MediaType::Video.as_raw())),
            Property::new(FormatProperties::MediaSubtype.as_raw(), id(MediaSubtype::Raw.as_raw())),
            Property::new(
                FormatProperties::VideoFormat.as_raw(),
                Value::Choice(ChoiceValue::Id(Choice(
                    ChoiceFlags::empty(),
                    ChoiceEnum::Enum {
                        default: Id(VideoFormat::BGRx.as_raw()),
                        alternatives: vec![
                            Id(VideoFormat::BGRx.as_raw()),
                            Id(VideoFormat::BGRA.as_raw()),
                            Id(VideoFormat::RGBx.as_raw()),
                            Id(VideoFormat::RGBA.as_raw()),
                        ],
                    },
                ))),
            ),
            Property::new(
                FormatProperties::VideoSize.as_raw(),
                Value::Choice(ChoiceValue::Rectangle(Choice(
                    ChoiceFlags::empty(),
                    ChoiceEnum::Range {
                        default: Rectangle { width: 1920, height: 1080 },
                        min: Rectangle { width: 1, height: 1 },
                        max: Rectangle { width: 8192, height: 8192 },
                    },
                ))),
            ),
            Property::new(
                FormatProperties::VideoFramerate.as_raw(),
                Value::Choice(ChoiceValue::Fraction(Choice(
                    ChoiceFlags::empty(),
                    ChoiceEnum::Range { default: Fraction { num: target_fps, denom: 1 }, min: Fraction { num: 0, denom: 1 }, max: Fraction { num: 1000, denom: 1 } },
                ))),
            ),
            Property::new(
                FormatProperties::VideoMaxFramerate.as_raw(),
                Value::Choice(ChoiceValue::Fraction(Choice(
                    ChoiceFlags::empty(),
                    ChoiceEnum::Range { default: Fraction { num: target_fps, denom: 1 }, min: Fraction { num: 1, denom: 1 }, max: Fraction { num: 1000, denom: 1 } },
                ))),
            ),
        ],
    };
    PodSerializer::serialize(Cursor::new(Vec::new()), &Value::Object(obj)).expect("a well-formed object serializes").0.into_inner()
}
```

- [ ] **Step 6: Implement the backend**

`crates/capture/src/linux/mod.rs`:

```rust
//! Linux capture: xdg-desktop-portal picks the source, PipeWire delivers the frames.

mod pipewire;
mod portal;

use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use brp_proto::constants::PORTAL_FORMAT_TIMEOUT;
use ::pipewire as pw;

use crate::error::CaptureError;
use crate::frame::{CaptureBackend, CaptureSession, FrameSink, SourceInfo, SourceRequest, StartFuture};

use self::pipewire::PwEvent;
use self::portal::PortalHandle;

pub struct PortalCapture;

struct PortalSession {
    info: SourceInfo,
    quit: pw::channel::Sender<()>,
    thread: Option<JoinHandle<Result<(), CaptureError>>>,
    _portal: PortalHandle,
}

impl CaptureBackend for PortalCapture {
    fn start(&self, request: SourceRequest, sink: FrameSink) -> StartFuture<'_> {
        Box::pin(async move {
            let stream = portal::open_screencast(request.kind).await?;
            let (events_tx, events_rx) = mpsc::channel();
            let (quit_tx, quit_rx) = pw::channel::channel::<()>();
            let node_id = stream.node_id;
            let fd = stream.fd;
            let thread = thread::Builder::new()
                .name("brp-pipewire".into())
                .spawn(move || pipewire::run_stream(fd, node_id, request.target_fps, events_tx, sink, quit_rx))
                .expect("spawning a thread only fails when the system is out of resources");

            // The format arrives on the PipeWire thread; block a worker rather than the async executor.
            let first = tokio::task::spawn_blocking(move || events_rx.recv_timeout(PORTAL_FORMAT_TIMEOUT))
                .await
                .map_err(|e| CaptureError::PipeWire(format!("format wait task failed: {e}")))?;
            let info = match first {
                Ok(PwEvent::Format(info)) => info,
                Ok(PwEvent::Error(e)) => {
                    let _ = quit_tx.send(());
                    return Err(e);
                }
                Err(_) => {
                    let _ = quit_tx.send(());
                    return Err(CaptureError::SourceLost("no format negotiated before the timeout".into()));
                }
            };
            tracing::info!(width = info.width, height = info.height, fps = info.fps, "portal capture started");
            Ok(Box::new(PortalSession { info, quit: quit_tx, thread: Some(thread), _portal: stream.handle }) as Box<dyn CaptureSession>)
        })
    }
}

impl CaptureSession for PortalSession {
    fn info(&self) -> SourceInfo {
        self.info
    }

    fn stop(mut self: Box<Self>) {
        self.shutdown();
    }
}

impl PortalSession {
    fn shutdown(&mut self) {
        let _ = self.quit.send(());
        if let Some(t) = self.thread.take() {
            match t.join() {
                Ok(Err(e)) => tracing::warn!(error = %e, "pipewire thread ended with an error"),
                Err(_) => tracing::warn!("pipewire thread panicked"),
                Ok(Ok(())) => {}
            }
        }
    }
}

impl Drop for PortalSession {
    fn drop(&mut self) {
        self.shutdown();
    }
}
```

Add to `crates/capture/src/lib.rs`:

```rust
#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "linux")]
pub use linux::PortalCapture;
```

- [ ] **Step 7: Write the manual verification example**

`crates/capture/examples/portal_dump.rs`:

```rust
//! Manual check: pick a monitor in the portal dialog, watch the frame rate, and inspect the first frame.
//! Run: cargo run -p brp-capture --example portal_dump

use std::fs::File;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use brp_capture::{CaptureBackend, CaptureFrame, PortalCapture, SourceRequest};
use brp_proto::{PixelFormat, SourceKind};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let first: Arc<Mutex<Option<CaptureFrame>>> = Arc::default();
    let count = Arc::new(Mutex::new(0u64));
    let (first_sink, count_sink) = (first.clone(), count.clone());
    let session = PortalCapture
        .start(
            SourceRequest { kind: SourceKind::Monitor, target_fps: 60 },
            Box::new(move |f| {
                *count_sink.lock().unwrap() += 1;
                first_sink.lock().unwrap().get_or_insert(f);
            }),
        )
        .await?;
    println!("negotiated {:?}", session.info());
    let started = Instant::now();
    tokio::time::sleep(Duration::from_secs(5)).await;
    let frames = *count.lock().unwrap();
    println!("{frames} frames in {:.1?} = {:.1} fps (move a window; static screens produce no frames)", started.elapsed(), frames as f64 / started.elapsed().as_secs_f64());
    if let Some(f) = first.lock().unwrap().take() {
        let mut out = File::create("/tmp/brp-first-frame.ppm")?;
        writeln!(out, "P6\n{} {}\n255", f.width, f.height)?;
        for row in f.data.chunks_exact(f.stride).take(f.height as usize) {
            for px in row[..f.width as usize * 4].chunks_exact(4) {
                let rgb = match f.format {
                    PixelFormat::Bgra | PixelFormat::Bgrx => [px[2], px[1], px[0]],
                    PixelFormat::Rgba | PixelFormat::Rgbx => [px[0], px[1], px[2]],
                };
                out.write_all(&rgb)?;
            }
        }
        println!("wrote /tmp/brp-first-frame.ppm ({:?}, stride {})", f.format, f.stride);
    }
    session.stop();
    Ok(())
}
```

- [ ] **Step 8: Run the unit tests, then the manual check**

Run: `cargo test -p brp-capture` → both helper tests pass.
Run: `cargo run -p brp-capture --example portal_dump`, choose a monitor in the KDE dialog, move a window for five seconds.
Expected: `negotiated SourceInfo { width: <monitor>, height: <monitor>, fps: <refresh rate> }`, a frame count near the refresh rate times five while the screen changes, and `/tmp/brp-first-frame.ppm` opens in an image viewer showing the desktop with correct colours. Wrong colours mean the format mapping is off; garbage rows mean the stride was ignored. fmt, clippy.

- [ ] **Step 9: Commit**

```bash
git add Cargo.toml Cargo.lock crates/proto/src/constants.rs crates/capture
git commit -m "feat: add Linux portal and PipeWire capture backend"
```

### Task 17: The `brp` binary: CLI, identity, and the headless publish command

**Files:**
- Modify: `Cargo.toml` (workspace dependencies), `crates/app/Cargo.toml`, `crates/app/src/main.rs`, `crates/proto/src/constants.rs`
- Create: `crates/app/src/cli.rs`, `crates/app/src/error.rs`, `crates/app/src/identity.rs`, `crates/app/src/publish.rs`

**Interfaces:**
- Consumes: everything above: `bind_endpoint`, `RelaySetting`, `MediaServer`, `Publisher`, `LatestSlot`, `PortalCapture`, `SwsConverter`, `open_encoder_auto`, `RoomTicket`, `default_bitrate_kbps`, `Preset`.
- Produces: `brp publish [--fps N] [--bitrate-kbps N] [--codec h264|hevc|av1] [--source monitor|window] [--no-relay]`; `identity::load_or_create() -> Result<iroh::SecretKey, AppError>` and `identity::load_or_create_at(&Path)`; `AppError`; `cli::{Cli, Command, PublishArgs, WatchArgs}`. Task 19 adds the `watch` arm to `main`.

- [ ] **Step 1: Dependencies and constants**

Root `Cargo.toml` `[workspace.dependencies]`:

```toml
clap = { version = "4.6", features = ["derive"] }
directories = "6.0"
data-encoding = "2.9"
```

`crates/app/Cargo.toml` `[dependencies]`:

```toml
brp-proto.workspace = true
brp-codec.workspace = true
brp-capture.workspace = true
brp-net.workspace = true
brp-pipeline.workspace = true
iroh.workspace = true
iroh-tickets.workspace = true
tokio = { workspace = true, features = ["rt-multi-thread", "macros", "signal", "time", "sync"] }
clap.workspace = true
directories.workspace = true
data-encoding.workspace = true
thiserror.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
```

Append to `crates/proto/src/constants.rs`:

```rust
/// How long the publisher waits for relay registration before printing a ticket that may lack a relay address.
pub const RELAY_ONLINE_TIMEOUT: Duration = Duration::from_secs(5);
/// Frequent enough to watch bitrate settle, quiet enough for a terminal.
pub const STATS_LOG_INTERVAL: Duration = Duration::from_secs(2);
```

- [ ] **Step 2: Write the failing identity test**

`crates/app/src/identity.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("brp-identity-test-{}-{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&dir);
        dir.join("identity.key")
    }

    #[test]
    fn creates_then_reloads_the_same_key() {
        let path = temp_path("reload");
        let first = load_or_create_at(&path).unwrap();
        let second = load_or_create_at(&path).unwrap();
        assert_eq!(first.to_bytes(), second.to_bytes());
        assert_eq!(fs::read_to_string(&path).unwrap().trim().len(), 64, "32 bytes as lowercase hex");
    }

    #[cfg(unix)]
    #[test]
    fn key_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let path = temp_path("mode");
        load_or_create_at(&path).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn corrupt_key_file_is_reported_not_overwritten() {
        let path = temp_path("corrupt");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "not hex").unwrap();
        assert!(matches!(load_or_create_at(&path), Err(AppError::Identity(_))));
        assert_eq!(fs::read_to_string(&path).unwrap(), "not hex");
    }
}
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test -p brp`
Expected: compile errors, modules missing.

- [ ] **Step 4: Implement error, identity, and CLI**

`crates/app/src/error.rs`:

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error(transparent)]
    Capture(#[from] brp_capture::CaptureError),
    #[error(transparent)]
    Codec(#[from] brp_codec::CodecError),
    #[error(transparent)]
    Net(#[from] brp_net::NetError),
    #[error("preset rejected: {0}")]
    Preset(#[from] brp_proto::PresetError),
    #[error("invalid ticket: {0}")]
    Ticket(#[from] iroh_tickets::ParseError),
    #[error("identity file: {0}")]
    Identity(String),
    #[error("window system: {0}")]
    Window(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
```

`crates/app/src/identity.rs` (above its tests):

```rust
//! The participant's key pair, created on first run and kept in the platform config directory.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::str::FromStr;

use data_encoding::HEXLOWER;
use directories::ProjectDirs;
use iroh::SecretKey;

use crate::error::AppError;

pub fn load_or_create() -> Result<SecretKey, AppError> {
    let dirs = ProjectDirs::from("", "", "brp").ok_or_else(|| AppError::Identity("no home directory to store the identity in".into()))?;
    load_or_create_at(&dirs.config_dir().join("identity.key"))
}

pub fn load_or_create_at(path: &Path) -> Result<SecretKey, AppError> {
    if let Ok(text) = fs::read_to_string(path) {
        return SecretKey::from_str(text.trim()).map_err(|e| AppError::Identity(format!("{} is not a valid key: {e}", path.display())));
    }
    let key = SecretKey::generate();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    write_private(path, HEXLOWER.encode(&key.to_bytes()).as_bytes())?;
    tracing::info!(path = %path.display(), id = %key.public().fmt_short(), "created a new identity");
    Ok(key)
}

#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)?;
    file.write_all(bytes)
}

#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    fs::write(path, bytes)
}
```

`crates/app/src/cli.rs`:

```rust
use brp_proto::{Codec, SourceKind};
use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Parser, Debug)]
#[command(name = "brp", about = "Peer-to-peer screen sharing", version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Capture a monitor or window and serve it to viewers. Prints the ticket to share.
    Publish(PublishArgs),
    /// Watch a live using a ticket printed by a publisher.
    Watch(WatchArgs),
}

#[derive(Args, Debug)]
pub struct PublishArgs {
    /// Frame rate cap. The source's own rate wins when lower.
    #[arg(long, default_value_t = 60)]
    pub fps: u32,
    /// Target bitrate. Defaults to Moonlight's rule for the source resolution and frame rate.
    #[arg(long)]
    pub bitrate_kbps: Option<u32>,
    /// Force a codec instead of preferring HEVC, then H.264, then software AV1.
    #[arg(long, value_enum)]
    pub codec: Option<CodecArg>,
    #[arg(long, value_enum, default_value_t = SourceArg::Monitor)]
    pub source: SourceArg,
    /// Disable relays. Only peers reachable by IP can connect, which is right for a LAN.
    #[arg(long)]
    pub no_relay: bool,
}

#[derive(Args, Debug)]
pub struct WatchArgs {
    pub ticket: String,
    #[arg(long)]
    pub no_relay: bool,
}

#[derive(ValueEnum, Clone, Copy, Debug)]
pub enum CodecArg {
    H264,
    Hevc,
    Av1,
}

impl From<CodecArg> for Codec {
    fn from(c: CodecArg) -> Self {
        match c {
            CodecArg::H264 => Codec::H264,
            CodecArg::Hevc => Codec::Hevc,
            CodecArg::Av1 => Codec::Av1,
        }
    }
}

#[derive(ValueEnum, Clone, Copy, Debug)]
pub enum SourceArg {
    Monitor,
    Window,
}

impl From<SourceArg> for SourceKind {
    fn from(s: SourceArg) -> Self {
        match s {
            SourceArg::Monitor => SourceKind::Monitor,
            SourceArg::Window => SourceKind::Window,
        }
    }
}
```

- [ ] **Step 5: Implement the publish command and main**

`crates/app/src/publish.rs`:

```rust
use std::sync::Arc;
use std::sync::atomic::Ordering;

use brp_capture::{CaptureBackend, PortalCapture, SourceRequest};
use brp_codec::{EncoderConfig, SwsConverter, open_encoder_auto};
use brp_net::{MediaServer, RelaySetting, bind_endpoint};
use brp_pipeline::{LatestSlot, Publisher};
use brp_proto::constants::{MEDIA_ALPN, RELAY_ONLINE_TIMEOUT, STATS_LOG_INTERVAL};
use brp_proto::{Codec, PixelFormat, Preset, RoomTicket, default_bitrate_kbps};
use iroh::protocol::Router;

use crate::cli::PublishArgs;
use crate::error::AppError;
use crate::identity;

const LIVE_ID: u32 = 1;
const PRESET_ID: u32 = 1;

pub async fn run(args: PublishArgs) -> Result<(), AppError> {
    let relay = if args.no_relay { RelaySetting::Disabled } else { RelaySetting::Default };
    let endpoint = bind_endpoint(identity::load_or_create()?, relay).await?;

    let slot = LatestSlot::new();
    let sink_slot = slot.clone();
    let session = PortalCapture
        .start(SourceRequest { kind: args.source.into(), target_fps: args.fps }, Box::new(move |frame| sink_slot.put(frame)))
        .await?;
    let info = session.info();

    let fps = info.fps.min(args.fps).max(1);
    // 4:2:0 encoders need even dimensions; a one-pixel crop is invisible.
    let (width, height) = (info.width & !1, info.height & !1);
    let bitrate_kbps = args.bitrate_kbps.unwrap_or_else(|| default_bitrate_kbps(width, height, fps));
    let forced_codec: Option<Codec> = args.codec.map(Into::into);
    let encoder = open_encoder_auto(EncoderConfig { width, height, fps, bitrate_kbps, codec: forced_codec.unwrap_or(Codec::Hevc) }, forced_codec)?;
    let preset = Preset { id: PRESET_ID, name: "Source".into(), width, height, fps, bitrate_kbps, codec: encoder.params().codec };
    preset.validate(info.width, info.height, info.fps.max(fps))?;

    // The converter rebuilds itself on the first frame if the compositor's format differs.
    let converter = SwsConverter::new(info.width, info.height, PixelFormat::Bgrx, width, height)?;
    let publisher = Publisher::start(LIVE_ID, PRESET_ID, slot, session, Box::new(converter), encoder);
    let router = Router::builder(endpoint.clone()).accept(MEDIA_ALPN, MediaServer::new(Arc::new(publisher.clone()))).spawn();

    if relay == RelaySetting::Default && tokio::time::timeout(RELAY_ONLINE_TIMEOUT, endpoint.online()).await.is_err() {
        tracing::warn!("relay registration timed out; the ticket may only work on the local network");
    }
    let ticket = RoomTicket::new(RoomTicket::random_topic(), vec![endpoint.addr()]);

    println!("Encoder: {} ({:?} {}x{} @ {} fps, {} kbps)", publisher.encoder_name(), preset.codec, width, height, fps, bitrate_kbps);
    println!("Ticket:\n{ticket}\n");
    println!("Share the ticket with a viewer: brp watch <ticket>. Press Ctrl-C to stop.");

    let mut ticker = tokio::time::interval(STATS_LOG_INTERVAL);
    let mut last_bytes = 0u64;
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            _ = ticker.tick() => {
                let bytes = publisher.stats().bytes_encoded.load(Ordering::Relaxed);
                let kbps = (bytes - last_bytes) * 8 / 1000 / STATS_LOG_INTERVAL.as_secs().max(1);
                last_bytes = bytes;
                tracing::info!(
                    viewers = publisher.subscriber_count(),
                    frames = publisher.stats().frames_encoded.load(Ordering::Relaxed),
                    dropped_at_input = publisher.frames_dropped_at_input(),
                    kbps,
                    "publishing"
                );
            }
        }
    }

    publisher.stop();
    if let Err(e) = router.shutdown().await {
        tracing::warn!(error = %e, "router shutdown");
    }
    endpoint.close().await;
    Ok(())
}
```

`crates/app/src/main.rs`:

```rust
//! brp: peer-to-peer screen sharing.

mod cli;
mod error;
mod identity;
mod publish;

use std::process::ExitCode;

use clap::Parser;
use tracing_subscriber::EnvFilter;

use crate::cli::{Cli, Command};

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();
    let cli = Cli::parse();
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("error: could not start the async runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    let result = match cli.command {
        Command::Publish(args) => runtime.block_on(publish::run(args)),
        Command::Watch(_) => {
            eprintln!("watch arrives in a later task");
            Ok(())
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
```

- [ ] **Step 6: Run the tests and the manual check**

Run: `cargo test -p brp` → the three identity tests pass.
Run: `cargo run -- publish --no-relay`, pick a monitor.
Expected: an `Encoder:` line naming a hardware encoder such as `hevc_nvenc` at the monitor's resolution, a ticket starting with `brp`, then a `publishing` log line every two seconds whose `kbps` is non-zero while the screen changes and near zero when static. Ctrl-C exits cleanly within a few seconds. fmt, clippy.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock crates/proto/src/constants.rs crates/app
git commit -m "feat: add brp binary with identity and headless publish command"
```

### Task 18: wgpu NV12 renderer, egui overlay, and the window

**Files:**
- Modify: `Cargo.toml` (workspace dependencies), `crates/app/Cargo.toml`, `crates/app/src/main.rs`
- Create: `crates/app/src/render/mod.rs`, `crates/app/src/render/video.rs`, `crates/app/src/render/nv12.wgsl`, `crates/app/src/render/ui.rs`, `crates/app/src/window.rs`

**Interfaces:**
- Consumes: `brp_codec::RawFrame`, `brp_pipeline::{LatestSlot, ViewerStats}`, `AppError`.
- Produces: `render::GpuContext::new(&ActiveEventLoop, &Arc<Window>) -> Result<GpuContext, AppError>` with `resize(w, h)`; `render::video::{VideoRenderer::new(&Device, TextureFormat), upload(&mut self, &Device, &Queue, &RawFrame), update_fit(&self, &Queue, viewport: (u32, u32)), draw(&self, &mut RenderPass<'static>), video_size() -> Option<(u32, u32)>, fit_scale(video: (u32, u32), viewport: (u32, u32)) -> [f32; 2]}`; `render::ui::{EguiLayer, UiFrame}`; `window::{App::new(title, slot, stats, description) , AppEvent::{NewFrame, Status(String)}}` implementing `winit::application::ApplicationHandler<AppEvent>`. Task 19 drives `App` from the `watch` command.

Verified for wgpu 30.0.1, winit 0.30.13, egui 0.36.1: `Instance::new` and `create_shader_module` take descriptors by value; `request_adapter` returns `Result`; `request_device` takes one descriptor with `experimental_features` and `trace`; `Surface::get_current_texture()` returns the `CurrentSurfaceTexture` enum and frames are presented with `Queue::present`; `RenderPipelineDescriptor` has `multiview_mask` and `cache`; `RenderPassColorAttachment` has `depth_slice`; `RenderPassDescriptor` has `multiview_mask`; `Queue::write_texture` has no 256-byte row alignment rule; `egui::Context::run_ui` replaced `run`; `egui_wgpu::Renderer::new(device, format, RendererOptions)`; `egui_winit::State::new(ctx, ViewportId::ROOT, &window, Some(scale), window.theme(), Some(max_texture_side))`; `TexturesDelta::set` maps ids to a `SmallVec` of deltas; egui prefers a gamma-space surface format chosen by `egui_wgpu::preferred_framebuffer_format`. The whole skeleton this task adapts type-checked against those versions.

- [ ] **Step 1: Dependencies**

Root `Cargo.toml` `[workspace.dependencies]`:

```toml
winit = "0.30"
wgpu = "30"
egui = "0.36"
egui-wgpu = "0.36"
egui-winit = "0.36"
pollster = "1.0"
bytemuck = { version = "1.25", features = ["derive"] }
```

`crates/app/Cargo.toml` `[dependencies]`: add `winit.workspace = true`, `wgpu.workspace = true`, `egui.workspace = true`, `egui-wgpu.workspace = true`, `egui-winit.workspace = true`, `pollster.workspace = true`, `bytemuck.workspace = true`.

- [ ] **Step 2: Write the failing aspect-fit test**

`crates/app/src/render/video.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use super::fit_scale;

    #[test]
    fn wide_video_in_tall_window_letterboxes() {
        let [x, y] = fit_scale((1920, 1080), (1000, 1000));
        assert!((x - 1.0).abs() < 1e-6 && (y - 0.5625).abs() < 1e-4, "{x} {y}");
    }

    #[test]
    fn tall_video_in_wide_window_pillarboxes() {
        let [x, y] = fit_scale((1080, 1920), (1920, 1080));
        assert!((y - 1.0).abs() < 1e-6 && (x - 0.3164).abs() < 1e-3, "{x} {y}");
    }

    #[test]
    fn matching_aspect_fills_the_window() {
        assert_eq!(fit_scale((1280, 720), (1920, 1080)), [1.0, 1.0]);
    }
}
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test -p brp fit_scale`
Expected: compile error, module `render` missing.

- [ ] **Step 4: Implement the GPU context**

`crates/app/src/render/mod.rs`:

```rust
//! wgpu device, surface, and the two things drawn into it: the video quad and the egui overlay.

pub mod ui;
pub mod video;

use std::sync::Arc;

use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

use crate::error::AppError;

pub struct GpuContext {
    pub surface: wgpu::Surface<'static>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub config: wgpu::SurfaceConfiguration,
}

impl GpuContext {
    pub fn new(event_loop: &ActiveEventLoop, window: &Arc<Window>) -> Result<Self, AppError> {
        // Handing wgpu the display connection up front is required for presentation on Wayland with GLES.
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle_from_env(Box::new(event_loop.owned_display_handle())));
        let surface = instance.create_surface(Arc::clone(window)).map_err(|e| AppError::Window(format!("create_surface: {e}")))?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
            apply_limit_buckets: false,
        }))
        .map_err(|e| AppError::Window(format!("no suitable GPU adapter: {e}")))?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("brp"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::default(),
            trace: wgpu::Trace::Off,
        }))
        .map_err(|e| AppError::Window(format!("request_device: {e}")))?;

        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or_else(|| AppError::Window("surface not supported by the adapter".into()))?;
        // egui and the YUV shader both write gamma-encoded colour, so the surface must not be an sRGB view.
        config.format = egui_wgpu::preferred_framebuffer_format(&surface.get_capabilities(&adapter).formats)
            .map_err(|e| AppError::Window(format!("no usable surface format: {e}")))?;
        surface.configure(&device, &config);
        Ok(Self { surface, device, queue, config })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(&self.device, &self.config);
        }
    }
}
```

- [ ] **Step 5: Implement the video renderer and shader**

`crates/app/src/render/nv12.wgsl`:

```wgsl
struct Fit {
    scale: vec2<f32>,
    _pad: vec2<f32>,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@group(0) @binding(0) var y_tex: texture_2d<f32>;
@group(0) @binding(1) var uv_tex: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;
@group(0) @binding(3) var<uniform> fit: Fit;

// Two triangles covering the letterboxed quad, so nothing outside the video is ever sampled.
@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    );
    let p = corners[vi];
    var out: VsOut;
    out.pos = vec4<f32>((p * 2.0 - 1.0) * fit.scale, 0.0, 1.0);
    out.uv = vec2<f32>(p.x, 1.0 - p.y);
    return out;
}

// BT.709 limited range, matching the publisher's swscale configuration.
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let y = (textureSample(y_tex, samp, in.uv).r - 16.0 / 255.0) * (255.0 / 219.0);
    let uv = (textureSample(uv_tex, samp, in.uv).rg - 128.0 / 255.0) * (255.0 / 224.0);
    let r = y + 1.5748 * uv.y;
    let g = y - 0.1873 * uv.x - 0.4681 * uv.y;
    let b = y + 1.8556 * uv.x;
    return vec4<f32>(clamp(vec3<f32>(r, g, b), vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
```

`crates/app/src/render/video.rs` (above its tests):

```rust
use brp_codec::RawFrame;
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

const SHADER: &str = include_str!("nv12.wgsl");

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Fit {
    scale: [f32; 2],
    _pad: [f32; 2],
}

struct Planes {
    width: u32,
    height: u32,
    y: wgpu::Texture,
    uv: wgpu::Texture,
    bind_group: wgpu::BindGroup,
}

pub struct VideoRenderer {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    fit: wgpu::Buffer,
    planes: Option<Planes>,
}

/// Clip-space scale that letterboxes or pillarboxes the video inside the viewport.
pub fn fit_scale(video: (u32, u32), viewport: (u32, u32)) -> [f32; 2] {
    let video_aspect = video.0 as f32 / video.1.max(1) as f32;
    let view_aspect = viewport.0.max(1) as f32 / viewport.1.max(1) as f32;
    if video_aspect > view_aspect { [1.0, view_aspect / video_aspect] } else { [video_aspect / view_aspect, 1.0] }
}

impl VideoRenderer {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("nv12"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
        let texture_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("nv12-layout"),
            entries: &[
                texture_entry(0),
                texture_entry(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("nv12-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("nv12-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_main"), compilation_options: Default::default(), buffers: &[] },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("nv12-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let fit = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("nv12-fit"),
            contents: bytemuck::bytes_of(&Fit { scale: [1.0, 1.0], _pad: [0.0; 2] }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        Self { pipeline, layout, sampler, fit, planes: None }
    }

    pub fn video_size(&self) -> Option<(u32, u32)> {
        self.planes.as_ref().map(|p| (p.width, p.height))
    }

    pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, frame: &RawFrame) {
        if self.planes.as_ref().is_none_or(|p| (p.width, p.height) != (frame.width, frame.height)) {
            self.planes = Some(self.allocate(device, frame.width, frame.height));
        }
        let planes = self.planes.as_ref().expect("allocated above");
        let copy = |texture: &wgpu::Texture| wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All };
        queue.write_texture(
            copy(&planes.y),
            &frame.y,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(frame.y_stride as u32), rows_per_image: Some(frame.height) },
            wgpu::Extent3d { width: frame.width, height: frame.height, depth_or_array_layers: 1 },
        );
        let chroma_rows = frame.chroma_rows() as u32;
        queue.write_texture(
            copy(&planes.uv),
            &frame.uv,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(frame.uv_stride as u32), rows_per_image: Some(chroma_rows) },
            wgpu::Extent3d { width: frame.width / 2, height: chroma_rows, depth_or_array_layers: 1 },
        );
    }

    pub fn update_fit(&self, queue: &wgpu::Queue, viewport: (u32, u32)) {
        if let Some((w, h)) = self.video_size() {
            queue.write_buffer(&self.fit, 0, bytemuck::bytes_of(&Fit { scale: fit_scale((w, h), viewport), _pad: [0.0; 2] }));
        }
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'static>) {
        if let Some(planes) = &self.planes {
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &planes.bind_group, &[]);
            pass.draw(0..6, 0..1);
        }
    }

    fn allocate(&self, device: &wgpu::Device, width: u32, height: u32) -> Planes {
        let make = |label: &str, w: u32, h: u32, format: wgpu::TextureFormat| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let y = make("nv12-y", width, height, wgpu::TextureFormat::R8Unorm);
        let uv = make("nv12-uv", width / 2, height.div_ceil(2), wgpu::TextureFormat::Rg8Unorm);
        let y_view = y.create_view(&wgpu::TextureViewDescriptor::default());
        let uv_view = uv.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("nv12-bind-group"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&y_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&uv_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 3, resource: self.fit.as_entire_binding() },
            ],
        });
        Planes { width, height, y, uv, bind_group }
    }
}
```

- [ ] **Step 6: Implement the egui layer**

`crates/app/src/render/ui.rs`:

```rust
//! egui on top of our own winit loop: input in, tessellated triangles out, painted into our render pass.

use std::time::Duration;

use winit::event::WindowEvent;
use winit::window::Window;

pub struct EguiLayer {
    ctx: egui::Context,
    state: egui_winit::State,
    renderer: egui_wgpu::Renderer,
    viewport_info: egui::ViewportInfo,
}

pub struct UiFrame {
    paint_jobs: Vec<egui::ClippedPrimitive>,
    textures_delta: egui::TexturesDelta,
    pub screen: egui_wgpu::ScreenDescriptor,
    pub repaint_delay: Option<Duration>,
}

impl EguiLayer {
    pub fn new(window: &Window, device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let ctx = egui::Context::default();
        let state = egui_winit::State::new(
            ctx.clone(),
            egui::ViewportId::ROOT,
            window,
            Some(window.scale_factor() as f32),
            window.theme(),
            Some(device.limits().max_texture_dimension_2d as usize),
        );
        let renderer = egui_wgpu::Renderer::new(device, format, egui_wgpu::RendererOptions::default());
        Self { ctx, state, renderer, viewport_info: egui::ViewportInfo::default() }
    }

    pub fn on_window_event(&mut self, window: &Window, event: &WindowEvent) -> egui_winit::EventResponse {
        self.state.on_window_event(window, event)
    }

    pub fn run(&mut self, window: &Window, size_in_pixels: [u32; 2], ui: impl FnOnce(&egui::Context)) -> UiFrame {
        egui_winit::update_viewport_info(&mut self.viewport_info, &self.ctx, window, false);
        let mut raw_input = self.state.take_egui_input(window);
        raw_input.viewports.insert(egui::ViewportId::ROOT, self.viewport_info.clone());
        let output = self.ctx.run_ui(raw_input, |root| ui(root.ctx()));
        self.state.handle_platform_output(window, output.platform_output);
        let paint_jobs = self.ctx.tessellate(output.shapes, output.pixels_per_point);
        UiFrame {
            paint_jobs,
            textures_delta: output.textures_delta,
            screen: egui_wgpu::ScreenDescriptor { size_in_pixels, pixels_per_point: output.pixels_per_point },
            repaint_delay: output.viewport_output.get(&egui::ViewportId::ROOT).map(|v| v.repaint_delay),
        }
    }

    /// Uploads textures and buffers. Must run before the render pass that calls `paint` begins.
    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, frame: &UiFrame) -> Vec<wgpu::CommandBuffer> {
        for (id, deltas) in &frame.textures_delta.set {
            for delta in deltas {
                self.renderer.update_texture(device, queue, *id, delta);
            }
        }
        self.renderer.update_buffers(device, queue, encoder, &frame.paint_jobs, &frame.screen)
    }

    pub fn paint(&self, pass: &mut wgpu::RenderPass<'static>, frame: &UiFrame) {
        self.renderer.render(pass, &frame.paint_jobs, &frame.screen);
    }

    /// Frees textures egui retired this frame. Call after the frame's commands were submitted.
    pub fn cleanup(&mut self, frame: &UiFrame) {
        for id in &frame.textures_delta.free {
            self.renderer.free_texture(id);
        }
    }
}
```

- [ ] **Step 7: Implement the window**

`crates/app/src/window.rs`:

```rust
//! One native window: the video quad underneath, a small egui stats panel on top.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use brp_codec::RawFrame;
use brp_pipeline::{LatestSlot, ViewerStats};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::window::{Window, WindowId};

use crate::render::GpuContext;
use crate::render::ui::EguiLayer;
use crate::render::video::VideoRenderer;

pub enum AppEvent {
    NewFrame,
    Status(String),
}

pub struct App {
    title: String,
    description: String,
    slot: Arc<LatestSlot<RawFrame>>,
    stats: Arc<ViewerStats>,
    status: String,
    window: Option<Arc<Window>>,
    gpu: Option<GpuContext>,
    video: Option<VideoRenderer>,
    ui: Option<EguiLayer>,
    fps_window_start: Instant,
    fps_window_frames: u32,
    fps: f32,
    repaint_at: Option<Instant>,
}

impl App {
    pub fn new(title: String, description: String, slot: Arc<LatestSlot<RawFrame>>, stats: Arc<ViewerStats>) -> Self {
        Self {
            title,
            description,
            slot,
            stats,
            status: "connected, waiting for the first frame".into(),
            window: None,
            gpu: None,
            video: None,
            ui: None,
            fps_window_start: Instant::now(),
            fps_window_frames: 0,
            fps: 0.0,
            repaint_at: None,
        }
    }

    fn redraw(&mut self) {
        let (Some(window), Some(gpu), Some(video), Some(ui)) = (self.window.as_ref(), self.gpu.as_mut(), self.video.as_mut(), self.ui.as_mut()) else {
            return;
        };
        if let Some(frame) = self.slot.try_take() {
            video.upload(&gpu.device, &gpu.queue, &frame);
            self.fps_window_frames += 1;
            let elapsed = self.fps_window_start.elapsed();
            if elapsed >= Duration::from_secs(1) {
                self.fps = self.fps_window_frames as f32 / elapsed.as_secs_f32();
                self.fps_window_frames = 0;
                self.fps_window_start = Instant::now();
            }
        }

        let surface_texture = match gpu.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => return,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                gpu.surface.configure(&gpu.device, &gpu.config);
                window.request_redraw();
                return;
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                tracing::error!("surface validation error; stopping redraws");
                return;
            }
        };
        let target = surface_texture.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let viewport = (gpu.config.width, gpu.config.height);
        video.update_fit(&gpu.queue, viewport);

        let (status, description, fps, video_size) = (&self.status, &self.description, self.fps, video.video_size());
        let (received, decoded, keyframes, dropped) = (
            self.stats.frames_received.load(Ordering::Relaxed),
            self.stats.frames_decoded.load(Ordering::Relaxed),
            self.stats.keyframe_requests.load(Ordering::Relaxed),
            self.slot.dropped(),
        );
        let ui_frame = ui.run(window, [viewport.0, viewport.1], |ctx| {
            egui::Window::new("Stats").resizable(false).show(ctx, |ui| {
                ui.monospace(description);
                ui.monospace(status);
                if let Some((w, h)) = video_size {
                    ui.monospace(format!("video   {w}x{h} at {fps:.1} fps"));
                }
                ui.monospace(format!("frames  received {received}  decoded {decoded}  dropped {dropped}"));
                ui.monospace(format!("keyframe requests {keyframes}"));
            });
        });

        let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
        let ui_buffers = ui.prepare(&gpu.device, &gpu.queue, &mut encoder, &ui_frame);
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("video+ui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            video.draw(&mut pass);
            ui.paint(&mut pass, &ui_frame);
        }
        gpu.queue.submit(ui_buffers.into_iter().chain(std::iter::once(encoder.finish())));
        ui.cleanup(&ui_frame);
        window.pre_present_notify();
        gpu.queue.present(surface_texture);

        self.repaint_at = None;
        if let Some(delay) = ui_frame.repaint_delay {
            if delay.is_zero() {
                window.request_redraw();
            } else if delay < Duration::from_secs(3600) {
                self.repaint_at = Some(Instant::now() + delay);
            }
        }
    }
}

impl ApplicationHandler<AppEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes().with_title(&self.title).with_inner_size(PhysicalSize::new(1280u32, 720u32));
        let window = match event_loop.create_window(attributes) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                tracing::error!(error = %e, "could not create the window");
                event_loop.exit();
                return;
            }
        };
        let gpu = match GpuContext::new(event_loop, &window) {
            Ok(g) => g,
            Err(e) => {
                tracing::error!(error = %e, "could not initialise the GPU");
                event_loop.exit();
                return;
            }
        };
        self.video = Some(VideoRenderer::new(&gpu.device, gpu.config.format));
        self.ui = Some(EguiLayer::new(&window, &gpu.device, gpu.config.format));
        self.gpu = Some(gpu);
        self.window = Some(window);
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: AppEvent) {
        match event {
            AppEvent::NewFrame => {}
            AppEvent::Status(s) => self.status = s,
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _window_id: WindowId, event: WindowEvent) {
        let Some(window) = self.window.clone() else { return };
        if let Some(ui) = self.ui.as_mut() {
            let response = ui.on_window_event(&window, &event);
            if response.repaint {
                window.request_redraw();
            }
            if response.consumed {
                return;
            }
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(PhysicalSize { width, height }) => {
                if let Some(gpu) = self.gpu.as_mut() {
                    gpu.resize(width, height);
                }
                window.request_redraw();
            }
            WindowEvent::RedrawRequested => self.redraw(),
            _ => {}
        }
    }

    fn new_events(&mut self, _event_loop: &ActiveEventLoop, cause: StartCause) {
        if matches!(cause, StartCause::ResumeTimeReached { .. }) {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(match self.repaint_at {
            Some(at) => ControlFlow::WaitUntil(at),
            None => ControlFlow::Wait,
        });
    }
}
```

Add `mod render;` and `mod window;` to `crates/app/src/main.rs`. Until Task 19 uses them, silence the dead-code warnings with `#[allow(dead_code)]` on both `mod` lines and remove the attribute in Task 19.

- [ ] **Step 8: Run the tests**

Run: `cargo test -p brp` then `cargo clippy --workspace --all-targets -- -D warnings`.
Expected: the three `fit_scale` tests pass, the whole workspace builds, clippy is clean. fmt.

- [ ] **Step 9: Commit**

```bash
git add Cargo.toml Cargo.lock crates/app
git commit -m "feat: add wgpu NV12 renderer, egui stats overlay, and window"
```

### Task 19: The watch command and the end-to-end check

**Files:**
- Create: `crates/app/src/watch.rs`
- Modify: `crates/app/src/main.rs`, `crates/app/src/error.rs`

**Interfaces:**
- Consumes: `RoomTicket`, `bind_endpoint`, `MediaClient`, `ViewerSubscription`, `open_decoder`, `Viewer`, `App`, `AppEvent`, `identity`.
- Produces: `brp watch <ticket> [--no-relay]`.

- [ ] **Step 1: Add the missing error variant**

Add to `AppError` in `crates/app/src/error.rs`:

```rust
    #[error("the ticket lists no bootstrap peer")]
    EmptyTicket,
```

- [ ] **Step 2: Implement the command**

`crates/app/src/watch.rs`:

```rust
use std::str::FromStr;
use std::sync::Arc;

use brp_codec::open_decoder;
use brp_net::{MediaClient, RelaySetting, bind_endpoint};
use brp_pipeline::Viewer;
use brp_proto::{PublisherMessage, RoomTicket};
use tokio::runtime::Runtime;
use winit::event_loop::EventLoop;

use crate::cli::WatchArgs;
use crate::error::AppError;
use crate::identity;
use crate::window::{App, AppEvent};

const LIVE_ID: u32 = 1;
const PRESET_ID: u32 = 1;

pub fn run(runtime: &Runtime, args: WatchArgs) -> Result<(), AppError> {
    let ticket = RoomTicket::from_str(&args.ticket)?;
    let bootstrap = ticket.bootstrap.first().cloned().ok_or(AppError::EmptyTicket)?;
    let relay = if args.no_relay { RelaySetting::Disabled } else { RelaySetting::Default };

    // Connect before a window exists so connection failures land on the terminal, not behind a black window.
    let (endpoint, client, subscription) = runtime.block_on(async {
        let endpoint = bind_endpoint(identity::load_or_create()?, relay).await?;
        let client = MediaClient::connect(&endpoint, bootstrap).await?;
        let subscription = client.subscribe(LIVE_ID, PRESET_ID).await?;
        Ok::<_, AppError>((endpoint, client, subscription))
    })?;
    let params = subscription.params.clone();
    let decoder = open_decoder(&params)?;
    let publisher = client.remote_id().fmt_short();
    println!("Subscribed to {publisher}: {:?} {}x{} @ {} fps", params.codec, params.width, params.height, params.fps);

    let event_loop = EventLoop::<AppEvent>::with_user_event().build().map_err(|e| AppError::Window(e.to_string()))?;
    let proxy = event_loop.create_proxy();
    let frame_proxy = proxy.clone();
    let viewer = Viewer::start(
        runtime.handle().clone(),
        subscription.frames,
        subscription.control.clone(),
        decoder,
        Arc::new(move || {
            let _ = frame_proxy.send_event(AppEvent::NewFrame);
        }),
    );

    let mut events = subscription.events;
    runtime.spawn(async move {
        while let Some(msg) = events.recv().await {
            if matches!(msg, PublisherMessage::LiveEnded) {
                let _ = proxy.send_event(AppEvent::Status("live ended by the publisher".into()));
            }
        }
    });

    let description = format!("{:?} {}x{} @ {} fps from {publisher}", params.codec, params.width, params.height, params.fps);
    let mut app = App::new(format!("brp: {publisher}"), description, viewer.slot(), viewer.stats());
    let outcome = event_loop.run_app(&mut app).map_err(|e| AppError::Window(e.to_string()));

    viewer.stop();
    client.close();
    runtime.block_on(endpoint.close());
    outcome
}
```

In `crates/app/src/main.rs`: add `mod watch;`, drop the `#[allow(dead_code)]` attributes from `mod render;` and `mod window;`, and replace the `Command::Watch(_)` arm with `Command::Watch(args) => watch::run(&runtime, args),`.

- [ ] **Step 3: Build and lint**

Run: `cargo build --release && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: clean build, clean clippy, all tests pass. fmt.

- [ ] **Step 4: End-to-end check on one machine**

1. Terminal A: `cargo run --release -- publish --no-relay`. Pick a monitor. Copy the ticket.
2. Terminal B: `cargo run --release -- watch --no-relay <ticket>`.
3. Expected: a 1280x720 window titled with the publisher's short id shows the captured monitor, letterboxed. The stats panel shows the codec line, `video WxH at ~60 fps` while the screen changes, `dropped` staying near zero, and `keyframe requests 1` from the initial subscription.
4. Move a window on the publisher's screen and watch it in the viewer. Latency should be visually immediate on a LAN. For a number, publish a screen showing a millisecond stopwatch, put the viewer window beside it, photograph both, and subtract. Record the value in the commit message.
5. Resize the viewer window: the video keeps its aspect ratio and the panel stays legible.
6. Ctrl-C the publisher. Expected: the viewer's status line changes to `live ended by the publisher` within a second and the last frame stays on screen. Close the window; the process exits.
7. Restart the publisher without `--no-relay`, wait for the ticket, and watch with the default relay setting. Expected: same result. If two machines on different networks are available, repeat across them; that exercises hole punching.

- [ ] **Step 5: Commit**

```bash
git add crates/app
git commit -m "feat: add watch command completing the Linux vertical slice"
```

### Task 20: Continuous integration and README

**Files:**
- Create: `.github/workflows/ci.yml`
- Modify: `README.md`

- [ ] **Step 1: Write the workflow**

`.github/workflows/ci.yml`:

```yaml
name: ci

on:
  push:
    branches: [main]
  pull_request:

jobs:
  linux:
    runs-on: ubuntu-latest
    # Fedora ships FFmpeg 8 and PipeWire 1.6 headers, matching the development machine. Ubuntu LTS
    # images carry FFmpeg 6.1, whose swscale flags are constants rather than an enum and would not build.
    container: fedora:44
    steps:
      - name: Install build dependencies
        run: >
          dnf install -y --setopt=install_weak_deps=False
          git gcc gcc-c++ clang clang-devel pkgconf-pkg-config
          ffmpeg-free-devel pipewire-devel
          libxkbcommon-devel wayland-devel libX11-devel
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      - uses: Swatinem/rust-cache@v2
      - run: cargo fmt --all -- --check
      - run: cargo clippy --workspace --all-targets -- -D warnings
      - run: cargo test --workspace
```

- [ ] **Step 2: Write the README**

Replace `README.md` with:

````markdown
# brp_sharing

Peer-to-peer screen sharing for small groups. Native Rust, QUIC over iroh, no media server. A publisher
captures a monitor and streams it at source resolution and frame rate to viewers who decode and render
it locally.

Status: phase 1, the Linux vertical slice. One publisher, one live, one viewer window. Design and
roadmap live in `docs/superpowers/specs/`.

## Build prerequisites

FFmpeg 7.1 or newer headers, PipeWire headers, and a working `clang` for bindgen.

Fedora:

```
sudo dnf install gcc clang clang-devel pkgconf-pkg-config pipewire-devel ffmpeg-devel
```

`ffmpeg-devel` is the RPM Fusion build. Fedora's own `ffmpeg-free-devel` also builds, but its
`libavcodec` has no H.264 or HEVC decoders, so a viewer on that build needs RPM Fusion's
`libavcodec-freeworld`.

Ubuntu 26.04:

```
sudo apt install build-essential clang libclang-dev pkg-config libpipewire-0.3-dev \
  libavcodec-dev libavutil-dev libswscale-dev
```

Runtime: a desktop with `xdg-desktop-portal` and a backend for your compositor, and a GPU with an
H.264 or HEVC encoder for the publisher. Without one, the publisher falls back to software AV1.

## Usage

```
cargo build --release

# Publisher: pick a monitor in the portal dialog, then share the printed ticket.
./target/release/brp publish [--fps 60] [--bitrate-kbps N] [--codec hevc|h264|av1] [--source monitor|window] [--no-relay]

# Viewer
./target/release/brp watch <ticket> [--no-relay]
```

`--no-relay` skips the public relay servers; use it on a LAN. With relays enabled, hole punching
handles most home routers, and a relayed fallback exists but cannot carry high bitrates.

The default bitrate follows Moonlight's rule: 20 Mbps at 1080p60, 40 Mbps at 1440p60.

## Development

```
cargo test --workspace                 # hardware-free suite
BRP_CODEC_TESTS=1 cargo test -p brp-codec --test codec_smoke -- --nocapture   # real encoders and decoders
cargo run -p brp-capture --example portal_dump                                 # capture sanity check
RUST_LOG=debug ./target/release/brp publish                                    # verbose logging
```

Identity keys live in the platform config directory under `brp/identity.key`.

## License

MIT
````

- [ ] **Step 3: Verify the workflow locally as far as possible**

Run: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: clean. Push a branch and confirm the `ci` job goes green; if the Fedora container lacks a package, fix the `dnf install` list rather than weakening the checks.

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/ci.yml README.md
git commit -m "chore: add CI workflow and README for the Linux slice"
```

## Plan self-review notes

Spec coverage for phase 1 (spec section 11, item 1): transport and ticket flow in Tasks 2, 8, 9, 17, 19; FFmpeg layer in Tasks 12 to 15; NV12 rendering in Task 18; portal capture in Task 16; the latency protection rules of spec 6.5 in Tasks 6, 7, 10, 11; constants of spec 6.6 in Task 1 with the additions listed below.

Constants this plan adds beyond the spec's table, each defined in `crates/proto/src/constants.rs` with its rationale: `MAX_FRAME_BYTES`, `MAX_CONTROL_BYTES`, `RECEIVE_QUEUE_FRAMES`, `IDLE_KEYFRAME_RETRY`, `PORTAL_FORMAT_TIMEOUT`, `RELAY_ONLINE_TIMEOUT`, `STATS_LOG_INTERVAL`, `MIN_BITRATE_KBPS`, `MAX_BITRATE_KBPS`.

Deliberate deviations, all temporary: the media server accepts any authenticated caller until gossip membership exists (phase 2); `libsvtav1` CBR parameters are verified only by the gated smoke test, with the fallback spelled out in Task 13; VAAPI zero-copy is not attempted, frames are uploaded per the spec's first-release plan.

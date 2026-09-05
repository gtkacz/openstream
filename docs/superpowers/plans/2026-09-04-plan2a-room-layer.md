# Plan 2a: Headless Room Layer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A `room` crate in which participants discover each other over signed gossip presence, publish several lives with lazily started encoders, and watch remote lives with reconnection, all testable in one process with fake codecs; the `publish` and `watch` commands rebuilt on it.

**Architecture:** Membership is application state fed by signed presence over iroh-gossip. A live registry owns capture sessions and starts one `Publisher` per subscribed preset from a shared capture fan. A watcher pools one media connection per publisher and drives `Viewer` pipelines into stable frame slots. The media server admits members only. The window comes in plan 2b; here `publish` and `watch` are thin consumers of `Room`.

**Tech Stack:** Rust 2024, tokio, iroh 1.1, iroh-gossip 0.101, n0-future 0.3, postcard, the phase 1 crates.

**Spec:** `docs/superpowers/specs/2026-09-04-slice2-rooms-and-multi-live-design.md`, which refines `docs/superpowers/specs/2026-09-04-p2p-screen-sharing-design.md`. Read both; sections 5, 6, 8, 9, 10 of the slice spec drive this plan.

## Global Constraints

- Linux only. No Windows or macOS code. Audio, pop-outs, and the participant window are out of scope; the window is plan 2b.
- Preset switching is unsubscribe plus subscribe. `SwitchPreset` and `PresetSwitched` are never sent.
- Constants live in `crates/proto/src/constants.rs`. New ones in this plan: `PRESENCE_HEARTBEAT` 5 s, `MEMBER_EXPIRY` 20 s, `MAX_LIVES_PER_PARTICIPANT` 8, `MAX_PRESETS_PER_LIVE` 6, `REGISTRY_HOUSEKEEPING` 1 s, `JOIN_TIMEOUT` 15 s, `TEMPLATE_HEIGHTS` [1080, 720, 480], `NICKNAME_MAX_LEN` 32, `REFUSED_NOT_MEMBER` close code 1. Tests that need shorter timings pass them through `RoomTimings`, never by editing constants.
- Frames between threads go through `LatestSlot` or bounded channels. Captured frames are shared as `Arc<CaptureFrame>` so several encoders read one capture.
- Nothing on the media path panics. Every crate keeps its `thiserror` enum. The new crate's is `RoomError`.
- Comments explain why. Doc comments state contracts on public items. No task ids in code.
- One Conventional Commit per task, imperative subject, no co-author lines. The `Claude-Session:` trailer the harness requires on every commit of this session is expected; no other trailers. `cargo fmt --all` and `cargo clippy --workspace --all-targets -- -D warnings` pass before each commit.
- Tests are deterministic: loopback endpoints with relays disabled, fake codecs, the synthetic source, injected timings. No real GPU, portal, or relay in the test suite.
- Verified library facts this plan relies on: `Gossip::builder().spawn(endpoint)` implements `ProtocolHandler` under `iroh_gossip::ALPN`; `gossip.subscribe(topic, Vec<EndpointId>).await -> GossipTopic`, `topic.joined().await`, `topic.split() -> (GossipSender, GossipReceiver)`, `sender.broadcast(Bytes).await`, receiver is a `Stream` of `Result<Event, ApiError>` with `Event::{NeighborUp, NeighborDown, Received(Message { content, delivered_from, scope }), Lagged}`; `delivered_from` is the last hop; bootstrap addresses register through `iroh::address_lookup::memory::MemoryLookup` passed to `Endpoint::builder(..).address_lookup(lookup)`; `SecretKey::sign(&[u8]) -> Signature`, `PublicKey::verify(&[u8], &Signature) -> Result<(), SignatureError>`, and `Signature` implements serde.

## File Structure

```
crates/proto/src/constants.rs         + slice 2 constants
crates/proto/src/presence.rs           Presence, LiveInfo, Signed<T> sign/verify
crates/proto/src/templates.rs          template_presets()
crates/pipeline/src/fanout.rs          + prune()
crates/pipeline/src/publisher.rs       no session; Arc<CaptureFrame> slot
crates/pipeline/src/viewer.rs          ViewerSink; stable slot
crates/pipeline/src/reorder.rs         keyframe-jump rule
crates/net/src/policy.rs               ConnectionPolicy, AllowAll
crates/net/src/server.rs               policy check on accept
crates/net/src/client.rs               path_kind()
crates/net/src/endpoint.rs             bind_endpoint(secret, relay, known_peers)
crates/room/Cargo.toml                 brp-room
crates/room/src/lib.rs                 Room, RoomConfig, RoomTimings, RoomError re-exports
crates/room/src/error.rs               RoomError
crates/room/src/membership.rs          Membership
crates/room/src/codecs.rs              EncoderFactory, DecoderFactory, FfmpegCodecs, fake::FakeCodecs
crates/room/src/registry.rs            LiveRegistry, CaptureFan
crates/room/src/gossip.rs              join, presence loop
crates/room/src/watcher.rs             Watcher, WatchHandle
crates/room/src/snapshot.rs            RoomSnapshot and views
crates/room/src/room.rs                Room: wiring and commands
crates/room/tests/two_rooms.rs         two rooms over loopback
crates/room/tests/registry.rs          registry with synthetic capture and fake codecs
crates/app/src/publish.rs              on Room
crates/app/src/watch.rs                on Room
crates/app/src/cli.rs                  --ticket, --nickname
```

---

### Task 1: Backfill the phase 1 unit tests that were skipped

**Files:**
- Modify: `crates/pipeline/src/slot.rs`, `crates/pipeline/src/fanout.rs`, `crates/proto/src/ticket.rs`, `crates/capture/src/synthetic.rs`, `crates/codec/src/fake.rs`, `crates/codec/src/raw.rs`

**Interfaces:**
- Consumes the existing public API of those modules unchanged. This task adds tests only; if a test fails, the code under test has a bug worth a separate commit after this one.

- [ ] **Step 1: Slot tests**

Append to `crates/pipeline/src/slot.rs`:

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

- [ ] **Step 2: Fan-out tests**

Append to `crates/pipeline/src/fanout.rs`:

```rust
#[cfg(test)]
mod tests {
    use std::time::Duration;

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
        assert!(!kf.take_if_allowed(t0 + Duration::from_millis(100)));
        assert!(kf.pending());
        assert!(kf.take_if_allowed(t0 + FORCED_KEYFRAME_MIN_INTERVAL));
    }

    #[test]
    fn new_subscriber_waits_for_a_keyframe_and_requests_one() {
        let kf = KeyframeRequest::new();
        let mut fanout = FanOut::new(kf.clone());
        let mut rx = fanout.add();
        assert!(kf.pending());
        assert_eq!(fanout.push(frame(1, false)), PushOutcome { delivered: 0, skipped: 1 });
        assert_eq!(fanout.push(frame(2, true)), PushOutcome { delivered: 1, skipped: 0 });
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
        assert_eq!(fanout.push(frame(3, false)), PushOutcome { delivered: 0, skipped: 1 });
        assert!(kf.pending());
        assert_eq!(rx.try_recv().unwrap().seq, 1);
        assert_eq!(rx.try_recv().unwrap().seq, 2);
        assert_eq!(fanout.push(frame(4, false)), PushOutcome { delivered: 0, skipped: 1 });
        assert_eq!(fanout.push(frame(5, true)), PushOutcome { delivered: 1, skipped: 0 });
        assert_eq!(rx.try_recv().unwrap().seq, 5);
    }

    #[test]
    fn dropped_receiver_is_removed_on_push() {
        let mut fanout = FanOut::new(KeyframeRequest::new());
        let rx = fanout.add();
        assert_eq!(fanout.subscriber_count(), 1);
        drop(rx);
        fanout.push(frame(1, true));
        assert_eq!(fanout.subscriber_count(), 0);
    }
}
```

- [ ] **Step 3: Ticket, synthetic source, fake codec, and raw frame tests**

Append to `crates/proto/src/ticket.rs`:

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
        assert!(matches!(RoomTicket::from_str("endpointaaaaaaaaaaaaaaaa"), Err(ParseError::Kind { .. })));
    }

    #[test]
    fn ticket_rejects_empty_bootstrap_list() {
        let text = RoomTicket::new([1u8; 32], vec![]).to_string();
        assert!(matches!(RoomTicket::from_str(&text), Err(ParseError::Verify { .. })));
    }

    #[test]
    fn random_topics_differ() {
        assert_ne!(RoomTicket::random_topic(), RoomTicket::random_topic());
    }
}
```

Append to `crates/capture/src/synthetic.rs` (add `tokio = { workspace = true, features = ["rt", "macros", "time"] }` under `[dev-dependencies]` in `crates/capture/Cargo.toml` if absent):

```rust
#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use brp_proto::SourceKind;

    use super::*;

    #[tokio::test]
    async fn synthetic_source_paces_frames_and_numbers_them() {
        let frames = Arc::new(Mutex::new(Vec::new()));
        let sink_frames = frames.clone();
        let session = SyntheticSource { width: 64, height: 32, fps: 100 }
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
            assert_eq!((f.width, f.height, f.stride, f.format), (64, 32, 256, PixelFormat::Bgra));
            assert_eq!(f.data.len(), 256 * 32);
            assert_eq!(u32::from_le_bytes([f.data[0], f.data[1], f.data[2], f.data[3]]), i as u32);
        }
        assert!(frames.windows(2).all(|w| w[1].capture_ts_us > w[0].capture_ts_us));
    }
}
```

Append to `crates/codec/src/fake.rs`:

```rust
#[cfg(test)]
mod tests {
    use brp_proto::{Codec, EncodedFrame, PixelFormat};

    use super::*;

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
            assert_eq!((packets[0].seq, packets[0].capture_ts_us), (i, i * 1000));
            assert_eq!(dec.decode(&packets[0]).unwrap(), vec![frame]);
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
        let bad = EncodedFrame { seq: 0, capture_ts_us: 0, keyframe: true, data: vec![0xff, 0x00, 0x13] };
        assert!(matches!(FakeDecoder.decode(&bad), Err(CodecError::FakePayload(_))));
    }

    #[test]
    fn solid_converter_produces_target_size_and_keeps_timestamp() {
        let mut conv = SolidConverter::new(4, 2);
        let pixels = vec![0u8; 16 * 8 * 4];
        let img = InputImage { width: 16, height: 8, stride: 64, format: PixelFormat::Bgra, data: &pixels, capture_ts_us: 5 };
        let out = conv.convert(&img).unwrap();
        assert_eq!((out.width, out.height, out.capture_ts_us), (4, 2, 5));
        assert!(out.validate().is_ok());
    }
}
```

Append to `crates/codec/src/raw.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn black_frame_has_limited_range_black_and_tight_strides() {
        let f = RawFrame::black(6, 4, 77);
        assert_eq!((f.y_stride, f.uv_stride, f.y.len(), f.uv.len()), (6, 6, 24, 12));
        assert!(f.y.iter().all(|&v| v == 16) && f.uv.iter().all(|&v| v == 128));
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

- [ ] **Step 4: Run and commit**

Run: `cargo test --workspace`
Expected: every new test passes against the existing code. If one fails, stop and report the failing test rather than editing it; that is a phase 1 bug and gets its own commit.

```bash
git add crates
git commit -m "test: backfill unit tests for slot, fan-out, ticket, synthetic source, and fake codec"
```

### Task 2: Reorder buffer keyframe-jump rule

**Files:**
- Modify: `crates/pipeline/src/reorder.rs`

**Interfaces:**
- Keeps `Reorder::new(Duration)`, `push(IncomingFrame, Instant) -> Drained`, `poll(Instant) -> Drained`, `Drained { ready, request_keyframe }`, `IncomingFrame { header, data }`.

The phase 1 implementation waits out the whole cap even when a later keyframe is already buffered. Spec 6.5 of the master document says a buffered later keyframe ends the wait at once and discards what precedes it. This task adds the tests for all the rules and makes the jump rule pass.

- [ ] **Step 1: Write the tests**

Append to `crates/pipeline/src/reorder.rs`:

```rust
#[cfg(test)]
mod tests {
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
        assert!(r.push(f(1, false), t + WAIT).ready.is_empty());
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

- [ ] **Step 2: Run to see which fail**

Run: `cargo test -p brp-pipeline reorder`
Expected: `a_later_keyframe_skips_the_gap_immediately` and `duplicates_and_stale_frames_are_dropped` fail; the others pass.

- [ ] **Step 3: Replace the implementation**

Replace the `Reorder` struct and impl in `crates/pipeline/src/reorder.rs` with:

```rust
pub struct Reorder {
    max_wait: Duration,
    /// `None` while waiting for a keyframe: at start-up and after a gap times out.
    next: Option<u64>,
    pending: BTreeMap<u64, IncomingFrame>,
    gap_since: Option<Instant>,
}

impl Reorder {
    pub fn new(max_wait: Duration) -> Self {
        Self { max_wait, next: None, pending: BTreeMap::new(), gap_since: None }
    }

    pub fn push(&mut self, frame: IncomingFrame, now: Instant) -> Drained {
        let mut out = Drained::default();
        match self.next {
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
            self.next = None;
            out.request_keyframe = true;
        }
        out
    }

    /// A keyframe makes everything before it irrelevant; decoding resumes from it.
    fn restart_from(&mut self, keyframe: IncomingFrame, out: &mut Drained) {
        let seq = keyframe.header.seq;
        self.pending = self.pending.split_off(&(seq + 1));
        self.gap_since = None;
        self.next = Some(seq + 1);
        out.ready.push(keyframe);
        self.drain_contiguous(out);
    }

    fn drain(&mut self, now: Instant, out: &mut Drained) {
        self.drain_contiguous(out);
        let Some(next) = self.next else { return };
        if self.pending.is_empty() {
            self.gap_since = None;
            return;
        }
        let later_keyframe = self.pending.iter().find(|(seq, f)| **seq > next && f.header.keyframe).map(|(seq, _)| *seq);
        match later_keyframe {
            Some(seq) => {
                let keyframe = self.pending.remove(&seq).expect("found in the map above");
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
        while let Some(next) = self.next {
            let Some(frame) = self.pending.remove(&next) else { break };
            out.ready.push(frame);
            self.next = Some(next + 1);
        }
    }
}
```

- [ ] **Step 4: Run and commit**

Run: `cargo test -p brp-pipeline`
Expected: all reorder tests pass, and the viewer integration tests still pass. fmt, clippy.

```bash
git add crates/pipeline/src/reorder.rs
git commit -m "fix: let a buffered keyframe end a reorder gap immediately"
```

### Task 3: Publisher reads shared captures and no longer owns the session

**Files:**
- Modify: `crates/pipeline/src/fanout.rs`, `crates/pipeline/src/publisher.rs`, `crates/pipeline/tests/publisher.rs`, `crates/app/src/publish.rs`

**Interfaces:**
- Produces: `FanOut::prune(&mut self)` drops subscribers whose receiver is gone; `Publisher::start(live_id: u32, preset_id: u32, slot: Arc<LatestSlot<Arc<CaptureFrame>>>, converter: Box<dyn FrameConverter>, encoder: Box<dyn VideoEncoder>) -> Publisher`; `Publisher::subscriber_count(&self)` prunes before counting; `Publisher::stop(&self)` joins the encoder thread only. Whoever starts a capture session now owns it.

- [ ] **Step 1: Add the prune test**

Append inside the `tests` module of `crates/pipeline/src/fanout.rs`:

```rust
    #[test]
    fn prune_forgets_closed_receivers_without_a_push() {
        let mut fanout = FanOut::new(KeyframeRequest::new());
        let rx = fanout.add();
        let _kept = fanout.add();
        drop(rx);
        fanout.prune();
        assert_eq!(fanout.subscriber_count(), 1);
    }
```

- [ ] **Step 2: Rewrite the publisher test for the new signature**

Replace `crates/pipeline/tests/publisher.rs` with:

```rust
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use brp_capture::{CaptureBackend, CaptureFrame, SourceRequest, SyntheticSource};
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
        .start(SourceRequest { kind: SourceKind::Monitor, target_fps: 60 }, Box::new(move |frame| sink_slot.put(Arc::new(frame))))
        .await
        .unwrap();
    let publisher = Publisher::start(1, 1, slot, Box::new(SolidConverter::new(32, 16)), Box::new(FakeEncoder::new(cfg(), 30)));
    assert_eq!(publisher.encoder_name(), "fake");
    assert_eq!((publisher.params().width, publisher.params().height), (32, 16));

    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut sub = publisher.subscribe(1, 1).unwrap();
    let first = tokio::time::timeout(Duration::from_secs(2), sub.frames.recv()).await.unwrap().unwrap();
    assert!(first.keyframe);
    let mut previous = first.seq;
    for _ in 0..5 {
        let frame = tokio::time::timeout(Duration::from_secs(2), sub.frames.recv()).await.unwrap().unwrap();
        assert!(frame.seq > previous);
        previous = frame.seq;
    }
    assert_eq!(publisher.subscriber_count(), 1);
    assert!(publisher.stats().frames_encoded.load(Ordering::Relaxed) >= 6);
    assert_eq!(publisher.subscribe(2, 1).unwrap_err(), SubscribeRejected::UnknownLive(2));
    assert_eq!(publisher.subscribe(1, 9).unwrap_err(), SubscribeRejected::UnknownPreset(9));

    drop(sub);
    assert_eq!(publisher.subscriber_count(), 0, "counting prunes closed receivers");
    publisher.stop();
    session.stop();
}

#[tokio::test]
async fn static_screen_still_serves_a_late_subscriber_a_keyframe() {
    let slot = LatestSlot::new();
    let publisher = Publisher::start(1, 1, slot.clone(), Box::new(SolidConverter::new(8, 8)), Box::new(FakeEncoder::new(cfg(), 1_000)));
    slot.put(Arc::new(CaptureFrame { width: 8, height: 8, stride: 32, format: PixelFormat::Bgra, data: vec![0; 256], capture_ts_us: 1 }));
    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut sub = publisher.subscribe(1, 1).unwrap();
    let frame = tokio::time::timeout(Duration::from_millis(1_500), sub.frames.recv()).await.expect("re-encoded within the idle retry").unwrap();
    assert!(frame.keyframe);
    assert_eq!(frame.capture_ts_us, 1);
    publisher.stop();
}
```

- [ ] **Step 3: Run to verify the compile failures**

Run: `cargo test -p brp-pipeline`
Expected: `prune` unresolved; the publisher test fails to compile on the new `start` arity.

- [ ] **Step 4: Implement**

In `crates/pipeline/src/fanout.rs` add to `impl FanOut`:

```rust
    /// Drops subscribers whose receiver is gone. `push` does this as a side effect; callers that
    /// only count, such as idle detection, need it explicitly.
    pub fn prune(&mut self) {
        self.subs.retain(|s| !s.tx.is_closed());
    }
```

In `crates/pipeline/src/publisher.rs`:

- Change the import `use brp_capture::{CaptureFrame, CaptureSession};` to `use brp_capture::CaptureFrame;`.
- Remove the `session` field from `Inner` and the `session: Mutex::new(Some(session))` initialiser.
- Change the `slot` field type to `Arc<LatestSlot<Arc<CaptureFrame>>>` and the `start` signature to `start(live_id: u32, preset_id: u32, slot: Arc<LatestSlot<Arc<CaptureFrame>>>, converter: Box<dyn FrameConverter>, encoder: Box<dyn VideoEncoder>) -> Self`.
- Replace `subscriber_count`:

```rust
    pub fn subscriber_count(&self) -> usize {
        let mut fanout = lock(&self.inner.fanout);
        fanout.prune();
        fanout.subscriber_count()
    }
```

- Replace `stop` so it only stops the thread:

```rust
    pub fn stop(&self) {
        self.inner.stop.store(true, Ordering::Relaxed);
        self.inner.slot.close();
        if let Some(handle) = lock(&self.inner.thread).take() {
            let _ = handle.join();
        }
    }
```

- In `encode_loop`, change `let mut last: Option<CaptureFrame> = None;` to `let mut last: Option<Arc<CaptureFrame>> = None;` and both `last.as_ref()` to `last.as_deref()`. The rest of the loop is unchanged because `frame` stays a `&CaptureFrame`.

In `crates/app/src/publish.rs`:

- Change the slot construction to `let slot: Arc<brp_pipeline::LatestSlot<Arc<brp_capture::CaptureFrame>>> = brp_pipeline::LatestSlot::new();` and the sink to `Box::new(move |frame| sink_slot.put(Arc::new(frame)))`.
- Call `Publisher::start(LIVE_ID, PRESET_ID, slot, Box::new(converter), encoder)` without `session`.
- After `publisher.stop();` add `session.stop();`.

- [ ] **Step 5: Run, lint, commit**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`
Expected: all pass; `publish` still builds.

```bash
git add crates/pipeline crates/app/src/publish.rs
git commit -m "refactor: share captured frames across publishers and move session ownership out"
```

### Task 4: Viewer writes into a caller-owned sink

**Files:**
- Modify: `crates/pipeline/src/viewer.rs`, `crates/pipeline/src/lib.rs`, `crates/pipeline/tests/viewer.rs`, `crates/app/src/watch.rs`

**Interfaces:**
- Produces: `brp_pipeline::ViewerSink { pub slot: Arc<LatestSlot<RawFrame>>, pub stats: Arc<ViewerStats>, pub notify: FrameNotify }` and `Viewer::start(runtime: Handle, frames: Receiver<ReceivedFrame>, control: Sender<ViewerMessage>, decoder: Box<dyn VideoDecoder>, sink: ViewerSink) -> Viewer`. `Viewer::slot()` and `Viewer::stats()` keep returning clones. The decode loop no longer closes the slot when it ends, so a watcher can run several attempts into one slot that a tile keeps reading.

- [ ] **Step 1: Update the viewer tests**

In `crates/pipeline/tests/viewer.rs`, replace each `Viewer::start(tokio::runtime::Handle::current(), rx, ctl_tx, Box::new(FakeDecoder), <notify>)` with:

```rust
    let sink = ViewerSink { slot: LatestSlot::new(), stats: Arc::new(ViewerStats::default()), notify: <notify> };
    let viewer = Viewer::start(tokio::runtime::Handle::current(), rx, ctl_tx, Box::new(FakeDecoder), sink);
```

and add `use brp_pipeline::{LatestSlot, ViewerSink, ViewerStats};` to the imports. Add one test:

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sink_slot_stays_open_after_the_frame_channel_closes() {
    let (tx, rx) = mpsc::channel(8);
    let (ctl_tx, _ctl_rx) = mpsc::channel(8);
    let slot = LatestSlot::new();
    let sink = ViewerSink { slot: slot.clone(), stats: Arc::new(ViewerStats::default()), notify: Arc::new(|| {}) };
    let viewer = Viewer::start(tokio::runtime::Handle::current(), rx, ctl_tx, Box::new(FakeDecoder), sink);
    drop(tx);
    tokio::time::sleep(Duration::from_millis(100)).await;
    viewer.stop();
    slot.put(RawFrame::black(8, 4, 1));
    assert!(slot.try_take().is_some(), "a later attempt must be able to reuse the slot");
}
```

- [ ] **Step 2: Run to verify the compile failure**

Run: `cargo test -p brp-pipeline --test viewer`
Expected: `ViewerSink` unresolved.

- [ ] **Step 3: Implement**

In `crates/pipeline/src/viewer.rs`:

```rust
pub struct ViewerSink {
    pub slot: Arc<LatestSlot<RawFrame>>,
    pub stats: Arc<ViewerStats>,
    pub notify: FrameNotify,
}
```

Change `Viewer::start` to take `sink: ViewerSink` instead of `notify`, build `DecodeLoop` from `sink.slot.clone()`, `sink.stats.clone()`, `sink.notify`, and store `sink.slot` and `sink.stats` on `Viewer`. Delete the `self.slot.close();` line at the end of `DecodeLoop::run`. Export `ViewerSink` from `crates/pipeline/src/lib.rs` alongside `Viewer`.

In `crates/app/src/watch.rs`, build the sink before starting the viewer:

```rust
    let sink = brp_pipeline::ViewerSink {
        slot: brp_pipeline::LatestSlot::new(),
        stats: Arc::new(brp_pipeline::ViewerStats::default()),
        notify: Arc::new(move || {
            let _ = frame_proxy.send_event(AppEvent::NewFrame);
        }),
    };
    let viewer = Viewer::start(runtime.handle().clone(), subscription.frames, subscription.control, decoder, sink);
```

- [ ] **Step 4: Run, lint, commit**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`

```bash
git add crates/pipeline crates/app/src/watch.rs
git commit -m "refactor: let viewers decode into a caller-owned sink"
```

### Task 5: Presence wire types, signing, templates, and constants

**Files:**
- Modify: `crates/proto/src/constants.rs`, `crates/proto/src/error.rs`, `crates/proto/src/lib.rs`
- Create: `crates/proto/src/presence.rs`, `crates/proto/src/templates.rs`

**Interfaces:**
- Produces: constants `PRESENCE_HEARTBEAT`, `MEMBER_EXPIRY`, `MAX_LIVES_PER_PARTICIPANT`, `MAX_PRESETS_PER_LIVE`, `REGISTRY_HOUSEKEEPING`, `JOIN_TIMEOUT`, `TEMPLATE_HEIGHTS`, `NICKNAME_MAX_LEN`, `REFUSED_NOT_MEMBER`, `SOURCE_PRESET_ID`; `brp_proto::{LiveInfo, Presence, Signed}` with `Presence::validate(&self) -> Result<(), ProtoError>`, `Signed::sign<T: Serialize>(secret: &SecretKey, value: &T) -> Result<Signed, ProtoError>`, `Signed::verify<T: DeserializeOwned>(&self) -> Result<T, ProtoError>`, `Signed::author: PublicKey`; `brp_proto::template_presets(source_width, source_height, fps, codec) -> Vec<Preset>`; `ProtoError::{BadSignature, Invalid(String)}`.

- [ ] **Step 1: Constants and error variants**

Append to `crates/proto/src/constants.rs`:

```rust
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
```

Add to `ProtoError`:

```rust
    #[error("signature does not match the author")]
    BadSignature,
    #[error("invalid message: {0}")]
    Invalid(String),
```

- [ ] **Step 2: Write the failing tests**

`crates/proto/src/presence.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use iroh_base::SecretKey;

    use super::*;
    use crate::constants::{MAX_LIVES_PER_PARTICIPANT, NICKNAME_MAX_LEN, PROTOCOL_VERSION};

    fn presence(nickname: &str, lives: usize) -> Presence {
        Presence {
            version: PROTOCOL_VERSION,
            ts_unix_ms: 1_700_000_000_000,
            nickname: nickname.into(),
            lives: (0..lives as u32).map(|i| LiveInfo { id: i + 1, title: format!("live {i}"), kind: crate::SourceKind::Monitor, source_width: 1920, source_height: 1080, source_fps: 60, has_audio: false, presets: vec![] }).collect(),
        }
    }

    #[test]
    fn signed_presence_round_trips_and_names_its_author() {
        let secret = SecretKey::from_bytes(&[3u8; 32]);
        let signed = Signed::sign(&secret, &presence("gt", 1)).unwrap();
        assert_eq!(signed.author, secret.public());
        let back: Presence = signed.verify().unwrap();
        assert_eq!(back, presence("gt", 1));
    }

    #[test]
    fn tampered_payload_fails_verification() {
        let secret = SecretKey::from_bytes(&[3u8; 32]);
        let mut signed = Signed::sign(&secret, &presence("gt", 1)).unwrap();
        signed.payload[0] ^= 0xff;
        assert!(matches!(signed.verify::<Presence>(), Err(ProtoError::BadSignature)));
    }

    #[test]
    fn swapped_author_fails_verification() {
        let secret = SecretKey::from_bytes(&[3u8; 32]);
        let mut signed = Signed::sign(&secret, &presence("gt", 1)).unwrap();
        signed.author = SecretKey::from_bytes(&[4u8; 32]).public();
        assert!(matches!(signed.verify::<Presence>(), Err(ProtoError::BadSignature)));
    }

    #[test]
    fn presence_validation_enforces_version_nickname_and_live_limits() {
        assert!(presence("gt", 2).validate().is_ok());
        let mut wrong_version = presence("gt", 1);
        wrong_version.version = PROTOCOL_VERSION + 1;
        assert!(matches!(wrong_version.validate(), Err(ProtoError::Invalid(_))));
        assert!(matches!(presence(&"x".repeat(NICKNAME_MAX_LEN + 1), 1).validate(), Err(ProtoError::Invalid(_))));
        assert!(matches!(presence("gt", MAX_LIVES_PER_PARTICIPANT + 1).validate(), Err(ProtoError::Invalid(_))));
    }
}
```

`crates/proto/src/templates.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::Codec;
    use crate::constants::SOURCE_PRESET_ID;

    #[test]
    fn source_plus_every_smaller_template_with_even_aspect_preserving_widths() {
        let presets = template_presets(2560, 1440, 60, Codec::Hevc);
        let dims: Vec<(u32, &str, u32, u32)> = presets.iter().map(|p| (p.id, p.name.as_str(), p.width, p.height)).collect();
        assert_eq!(dims, vec![(SOURCE_PRESET_ID, "Source", 2560, 1440), (2, "1080p", 1920, 1080), (3, "720p", 1280, 720), (4, "480p", 852, 480)]);
        assert!(presets.iter().all(|p| p.codec == Codec::Hevc && p.fps == 60));
        assert_eq!(presets[1].bitrate_kbps, 20_000);
    }

    #[test]
    fn odd_source_dimensions_round_down_and_equal_heights_are_not_offered() {
        let presets = template_presets(1281, 721, 30, Codec::H264);
        assert_eq!((presets[0].width, presets[0].height), (1280, 720));
        assert_eq!(presets.len(), 2, "only 480p is strictly smaller than 721");
        assert_eq!((presets[1].id, presets[1].width, presets[1].height), (4, 852, 480));
    }
}
```

- [ ] **Step 3: Run to verify they fail**

Run: `cargo test -p brp-proto`
Expected: compile errors for `Signed`, `template_presets`.

- [ ] **Step 4: Implement**

`crates/proto/src/presence.rs` (above its tests):

```rust
use iroh_base::{PublicKey, SecretKey, Signature};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::constants::{MAX_LIVES_PER_PARTICIPANT, MAX_PRESETS_PER_LIVE, NICKNAME_MAX_LEN, PROTOCOL_VERSION};
use crate::error::ProtoError;
use crate::messages::{Preset, SourceKind};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveInfo {
    pub id: u32,
    pub title: String,
    pub kind: SourceKind,
    pub source_width: u32,
    pub source_height: u32,
    pub source_fps: u32,
    pub has_audio: bool,
    pub presets: Vec<Preset>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Presence {
    pub version: u8,
    pub ts_unix_ms: u64,
    pub nickname: String,
    pub lives: Vec<LiveInfo>,
}

impl Presence {
    pub fn validate(&self) -> Result<(), ProtoError> {
        if self.version != PROTOCOL_VERSION {
            return Err(ProtoError::Invalid(format!("presence version {} is not {PROTOCOL_VERSION}", self.version)));
        }
        if self.nickname.chars().count() > NICKNAME_MAX_LEN {
            return Err(ProtoError::Invalid("nickname too long".into()));
        }
        if self.lives.len() > MAX_LIVES_PER_PARTICIPANT {
            return Err(ProtoError::Invalid("too many lives".into()));
        }
        if self.lives.iter().any(|l| l.presets.len() > MAX_PRESETS_PER_LIVE) {
            return Err(ProtoError::Invalid("too many presets on a live".into()));
        }
        Ok(())
    }
}

/// Gossip reports the last hop, not the author, so authorship travels inside the message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signed {
    pub author: PublicKey,
    pub payload: Vec<u8>,
    pub signature: Signature,
}

impl Signed {
    pub fn sign<T: Serialize>(secret: &SecretKey, value: &T) -> Result<Self, ProtoError> {
        let payload = crate::messages::encode(value)?;
        let signature = secret.sign(&payload);
        Ok(Self { author: secret.public(), payload, signature })
    }

    pub fn verify<T: DeserializeOwned>(&self) -> Result<T, ProtoError> {
        self.author.verify(&self.payload, &self.signature).map_err(|_| ProtoError::BadSignature)?;
        crate::messages::decode(&self.payload)
    }
}
```

`crates/proto/src/templates.rs` (above its tests):

```rust
use crate::bitrate::default_bitrate_kbps;
use crate::constants::{SOURCE_PRESET_ID, TEMPLATE_HEIGHTS};
use crate::messages::{Codec, Preset};

/// Source at even dimensions, then one derived preset per template height strictly below the
/// source, with stable ids so a viewer's choice survives the publisher toggling other templates.
pub fn template_presets(source_width: u32, source_height: u32, fps: u32, codec: Codec) -> Vec<Preset> {
    let (width, height) = (source_width & !1, source_height & !1);
    let mut presets = vec![Preset { id: SOURCE_PRESET_ID, name: "Source".into(), width, height, fps, bitrate_kbps: default_bitrate_kbps(width, height, fps), codec }];
    for (index, &template_height) in TEMPLATE_HEIGHTS.iter().enumerate() {
        if template_height >= source_height {
            continue;
        }
        let template_width = (u64::from(source_width) * u64::from(template_height) / u64::from(source_height)) as u32 & !1;
        presets.push(Preset {
            id: SOURCE_PRESET_ID + 1 + index as u32,
            name: format!("{template_height}p"),
            width: template_width,
            height: template_height,
            fps,
            bitrate_kbps: default_bitrate_kbps(template_width, template_height, fps),
            codec,
        });
    }
    presets
}
```

Add to `crates/proto/src/lib.rs`: `pub mod presence; pub mod templates;` and `pub use presence::{LiveInfo, Presence, Signed}; pub use templates::template_presets;`.

- [ ] **Step 5: Run, lint, commit**

Run: `cargo test -p brp-proto`

```bash
git add crates/proto
git commit -m "feat: add signed presence, live info, preset templates, and room constants"
```

### Task 6: Connection policy, known-peer lookup, and path kind in `net`

**Files:**
- Create: `crates/net/src/policy.rs`
- Modify: `crates/net/src/server.rs`, `crates/net/src/endpoint.rs`, `crates/net/src/client.rs`, `crates/net/src/lib.rs`, `crates/net/tests/loopback.rs`, `crates/app/src/publish.rs`, `crates/app/src/watch.rs`

**Interfaces:**
- Produces: `brp_net::ConnectionPolicy { fn allows(&self, peer: EndpointId) -> bool }` with blanket impl for `Fn(EndpointId) -> bool + Send + Sync + 'static` and the unit `AllowAll`; `MediaServer::new(source: Arc<dyn LiveSource>, policy: Arc<dyn ConnectionPolicy>)`; `bind_endpoint(secret: SecretKey, relay: RelaySetting, known_peers: Vec<EndpointAddr>) -> Result<Endpoint, NetError>`; `MediaClient::path_kind(&self) -> PathKind` with `PathKind { Direct, Relayed, Unknown }`.

- [ ] **Step 1: Extend the loopback test**

In `crates/net/tests/loopback.rs` change both `bind_endpoint(SecretKey::generate(), RelaySetting::Disabled)` calls to pass `vec![]` as the third argument, change `MediaServer::new(source.clone())` to `MediaServer::new(source.clone(), Arc::new(AllowAll))`, add `AllowAll, PathKind` to the `brp_net` import, and after the successful `subscribe` add `assert_eq!(client.path_kind(), PathKind::Direct);`. Append a second test:

```rust
#[tokio::test]
async fn strangers_are_refused_before_any_subscription() {
    let (_tx, rx) = mpsc::channel(8);
    let source = Arc::new(ScriptedSource { params: params(), frames: Mutex::new(Some(rx)), keyframe_requests: AtomicUsize::new(0) });
    let member_ep = bind_endpoint(SecretKey::generate(), RelaySetting::Disabled, vec![]).await.unwrap();
    let stranger_ep = bind_endpoint(SecretKey::generate(), RelaySetting::Disabled, vec![]).await.unwrap();
    let member_id = member_ep.id();
    let policy = Arc::new(move |peer: iroh::EndpointId| peer == member_id);

    let server_ep = bind_endpoint(SecretKey::generate(), RelaySetting::Disabled, vec![]).await.unwrap();
    let router = Router::builder(server_ep.clone()).accept(MEDIA_ALPN, MediaServer::new(source, policy)).spawn();

    let stranger = MediaClient::connect(&stranger_ep, server_ep.addr()).await.unwrap();
    let refused = tokio::time::timeout(Duration::from_secs(5), stranger.subscribe(1, 1)).await.expect("refusal arrives promptly");
    assert!(matches!(refused, Err(NetError::Stream(_)) | Err(NetError::Connection(_))), "{refused:?}");

    let member = MediaClient::connect(&member_ep, server_ep.addr()).await.unwrap();
    assert!(member.subscribe(1, 1).await.is_ok());

    router.shutdown().await.unwrap();
    member_ep.close().await;
    stranger_ep.close().await;
}
```

- [ ] **Step 2: Run to verify the compile failures**

Run: `cargo test -p brp-net --test loopback`
Expected: `AllowAll`, `PathKind`, and the new arities are unresolved.

- [ ] **Step 3: Implement**

`crates/net/src/policy.rs`:

```rust
use iroh::EndpointId;

/// Decides which peers may open media connections. The room answers with its membership set.
pub trait ConnectionPolicy: Send + Sync + 'static {
    fn allows(&self, peer: EndpointId) -> bool;
}

impl<F> ConnectionPolicy for F
where
    F: Fn(EndpointId) -> bool + Send + Sync + 'static,
{
    fn allows(&self, peer: EndpointId) -> bool {
        self(peer)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct AllowAll;

impl ConnectionPolicy for AllowAll {
    fn allows(&self, _peer: EndpointId) -> bool {
        true
    }
}
```

In `crates/net/src/server.rs`: add a `policy: Arc<dyn ConnectionPolicy>` field and constructor parameter, import `brp_proto::constants::REFUSED_NOT_MEMBER` and `crate::policy::ConnectionPolicy`, and insert at the top of `accept`, right after `let peer = connection.remote_id();`:

```rust
        if !self.policy.allows(peer) {
            tracing::info!(peer = %peer.fmt_short(), "refusing media connection from a non-member");
            connection.close(REFUSED_NOT_MEMBER.into(), b"not a member");
            return Ok(());
        }
```

In `crates/net/src/endpoint.rs`:

```rust
use iroh::address_lookup::memory::MemoryLookup;
use iroh::EndpointAddr;

/// `known_peers` are addresses the caller already holds, typically a ticket's bootstrap list.
/// iroh 1.1 has no way to add an address to a bound endpoint, so they go in at build time.
pub async fn bind_endpoint(secret: SecretKey, relay: RelaySetting, known_peers: Vec<EndpointAddr>) -> Result<Endpoint, NetError> {
    let lookup = MemoryLookup::new();
    for peer in known_peers {
        lookup.add_endpoint_info(peer);
    }
    let builder = match relay {
        RelaySetting::Default => Endpoint::builder(presets::N0),
        RelaySetting::Disabled => Endpoint::builder(presets::Minimal).relay_mode(RelayMode::Disabled),
    };
    builder
        .address_lookup(lookup)
        .secret_key(secret)
        .alpns(vec![MEDIA_ALPN.to_vec()])
        .bind()
        .await
        .map_err(|e| NetError::Bind(e.to_string()))
}
```

In `crates/net/src/client.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathKind {
    Direct,
    Relayed,
    Unknown,
}

impl MediaClient {
    /// Whether media currently flows peer to peer or through a relay.
    pub fn path_kind(&self) -> PathKind {
        let paths = self.conn.paths();
        match paths.iter().find(|p| p.is_selected()) {
            Some(p) if p.is_ip() => PathKind::Direct,
            Some(p) if p.is_relay() => PathKind::Relayed,
            _ => PathKind::Unknown,
        }
    }
}
```

`Connection::paths()` returning a list whose items expose `is_selected`, `is_ip`, and `is_relay` was verified; the iteration method name was not. If `PathList` has no `iter`, use `for p in &paths` semantics through `IntoIterator`, which the docs for the type will show. This is the only unverified call in the plan.

In `crates/net/src/lib.rs`: `pub mod policy;`, `pub use policy::{AllowAll, ConnectionPolicy};`, and add `PathKind` to the client re-export.

In `crates/app/src/publish.rs` and `crates/app/src/watch.rs`: pass `vec![]` as the third argument of `bind_endpoint`, and in `publish.rs` construct `MediaServer::new(Arc::new(publisher.clone()), Arc::new(brp_net::AllowAll))`. Task 12 replaces both call sites.

- [ ] **Step 4: Run, lint, commit**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`
Expected: both loopback tests pass; the refusal arrives in well under a second.

```bash
git add crates/net crates/app
git commit -m "feat: gate media connections by policy and seed known peer addresses"
```

### Task 7: Room crate scaffold and membership

**Files:**
- Modify: `Cargo.toml` (workspace members resolve automatically; add dependencies)
- Create: `crates/room/Cargo.toml`, `crates/room/src/lib.rs`, `crates/room/src/error.rs`, `crates/room/src/membership.rs`

**Interfaces:**
- Produces: `brp_room::RoomError`; `brp_room::membership::{Membership::new(expiry: Duration), apply(&mut self, author: PublicKey, presence: Presence, now: Instant) -> Applied, expire(&mut self, now: Instant) -> Vec<PublicKey>, is_member(&self, id: &PublicKey) -> bool, get(&self, id: &PublicKey) -> Option<&Member>, members(&self) -> impl Iterator<Item = &Member>, len()}`, `Member { id, presence, last_seen }`, `Applied { Inserted, Updated, Refreshed, Stale }`.

- [ ] **Step 1: Dependencies and manifest**

Root `Cargo.toml` `[workspace.dependencies]`:

```toml
brp-room = { path = "crates/room" }
iroh-gossip = "0.101"
n0-future = "0.3"
```

`crates/room/Cargo.toml`:

```toml
[package]
name = "brp-room"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true

[dependencies]
brp-proto.workspace = true
brp-net.workspace = true
brp-pipeline.workspace = true
brp-codec.workspace = true
brp-capture.workspace = true
iroh.workspace = true
iroh-gossip.workspace = true
n0-future.workspace = true
tokio.workspace = true
bytes.workspace = true
thiserror.workspace = true
tracing.workspace = true

[dev-dependencies]
tokio = { workspace = true, features = ["rt-multi-thread", "macros", "time"] }
```

- [ ] **Step 2: Write the failing membership tests**

`crates/room/src/membership.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use brp_proto::constants::PROTOCOL_VERSION;
    use iroh::SecretKey;

    use super::*;

    fn presence(ts: u64, nickname: &str) -> Presence {
        Presence { version: PROTOCOL_VERSION, ts_unix_ms: ts, nickname: nickname.into(), lives: vec![] }
    }

    fn key(seed: u8) -> PublicKey {
        SecretKey::from_bytes(&[seed; 32]).public()
    }

    #[test]
    fn insert_update_refresh_and_stale_are_told_apart() {
        let mut m = Membership::new(Duration::from_secs(20));
        let t = Instant::now();
        assert_eq!(m.apply(key(1), presence(10, "a"), t), Applied::Inserted);
        assert_eq!(m.apply(key(1), presence(11, "a"), t), Applied::Refreshed);
        assert_eq!(m.apply(key(1), presence(12, "b"), t), Applied::Updated);
        assert_eq!(m.apply(key(1), presence(12, "c"), t), Applied::Stale, "equal timestamp is not newer");
        assert_eq!(m.apply(key(1), presence(5, "c"), t), Applied::Stale);
        assert_eq!(m.get(&key(1)).unwrap().presence.nickname, "b");
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn silent_members_expire_and_others_stay() {
        let mut m = Membership::new(Duration::from_secs(20));
        let t = Instant::now();
        m.apply(key(1), presence(1, "a"), t);
        m.apply(key(2), presence(1, "b"), t + Duration::from_secs(15));
        assert_eq!(m.expire(t + Duration::from_secs(19)), vec![]);
        assert_eq!(m.expire(t + Duration::from_secs(20)), vec![key(1)]);
        assert!(!m.is_member(&key(1)) && m.is_member(&key(2)));
    }

    #[test]
    fn refresh_updates_last_seen() {
        let mut m = Membership::new(Duration::from_secs(20));
        let t = Instant::now();
        m.apply(key(1), presence(1, "a"), t);
        m.apply(key(1), presence(2, "a"), t + Duration::from_secs(19));
        assert!(m.expire(t + Duration::from_secs(21)).is_empty());
    }
}
```

- [ ] **Step 3: Run to verify they fail**

Run: `cargo test -p brp-room`
Expected: compile errors, crate has no modules yet.

- [ ] **Step 4: Implement**

`crates/room/src/error.rs`:

```rust
use iroh::EndpointId;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RoomError {
    #[error(transparent)]
    Net(#[from] brp_net::NetError),
    #[error(transparent)]
    Capture(#[from] brp_capture::CaptureError),
    #[error(transparent)]
    Codec(#[from] brp_codec::CodecError),
    #[error(transparent)]
    Proto(#[from] brp_proto::ProtoError),
    #[error("gossip failed: {0}")]
    Gossip(String),
    #[error("no room member answered within the join timeout")]
    JoinTimeout,
    #[error("{0} is not a member of the room")]
    UnknownMember(EndpointId),
    #[error("unknown live {0}")]
    UnknownLive(u32),
    #[error("unknown preset {0}")]
    UnknownPreset(u32),
    #[error("the room limit of lives per participant is reached")]
    TooManyLives,
    #[error("not watching that live")]
    NotWatching,
}
```

`crates/room/src/membership.rs` (above its tests):

```rust
//! Who is in the room, derived from verified presence messages. The gossip overlay's neighbour
//! events describe the transport, not the room, so they never touch this state.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use brp_proto::Presence;
use iroh::PublicKey;

#[derive(Debug, Clone)]
pub struct Member {
    pub id: PublicKey,
    pub presence: Presence,
    pub last_seen: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applied {
    Inserted,
    Updated,
    Refreshed,
    Stale,
}

pub struct Membership {
    members: HashMap<PublicKey, Member>,
    expiry: Duration,
}

impl Membership {
    pub fn new(expiry: Duration) -> Self {
        Self { members: HashMap::new(), expiry }
    }

    /// The caller has already verified the signature and validated the presence.
    pub fn apply(&mut self, author: PublicKey, presence: Presence, now: Instant) -> Applied {
        match self.members.get_mut(&author) {
            None => {
                self.members.insert(author, Member { id: author, presence, last_seen: now });
                Applied::Inserted
            }
            Some(existing) if presence.ts_unix_ms <= existing.presence.ts_unix_ms => Applied::Stale,
            Some(existing) => {
                let changed = existing.presence.nickname != presence.nickname || existing.presence.lives != presence.lives;
                existing.presence = presence;
                existing.last_seen = now;
                if changed { Applied::Updated } else { Applied::Refreshed }
            }
        }
    }

    pub fn expire(&mut self, now: Instant) -> Vec<PublicKey> {
        let expiry = self.expiry;
        let expired: Vec<PublicKey> = self.members.values().filter(|m| now.duration_since(m.last_seen) >= expiry).map(|m| m.id).collect();
        for id in &expired {
            self.members.remove(id);
        }
        expired
    }

    pub fn is_member(&self, id: &PublicKey) -> bool {
        self.members.contains_key(id)
    }

    pub fn get(&self, id: &PublicKey) -> Option<&Member> {
        self.members.get(id)
    }

    pub fn members(&self) -> impl Iterator<Item = &Member> {
        self.members.values()
    }

    pub fn len(&self) -> usize {
        self.members.len()
    }

    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }
}
```

`crates/room/src/lib.rs`:

```rust
//! A room: membership over signed gossip presence, published lives with lazy encoders, and watches.

pub mod error;
pub mod membership;

pub use error::RoomError;
```

- [ ] **Step 5: Run, lint, commit**

Run: `cargo test -p brp-room`

```bash
git add Cargo.toml Cargo.lock crates/room
git commit -m "feat: add room crate with presence-based membership"
```

### Task 8: Codec factories

**Files:**
- Create: `crates/room/src/codecs.rs`
- Modify: `crates/room/src/lib.rs`

**Interfaces:**
- Produces: `brp_room::codecs::{EncoderFactory, DecoderFactory, EncoderParts { converter: Box<dyn FrameConverter>, encoder: Box<dyn VideoEncoder> }, FfmpegCodecs, fake::FakeCodecs}`. `EncoderFactory::open(&self, source: SourceInfo, source_format: PixelFormat, preset: &Preset) -> Result<EncoderParts, CodecError>`; `DecoderFactory::open(&self, params: &CodecParams) -> Result<Box<dyn VideoDecoder>, CodecError>`.

- [ ] **Step 1: Write the failing test**

`crates/room/src/codecs.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use brp_capture::SourceInfo;
    use brp_codec::RawFrame;
    use brp_proto::{Codec, PixelFormat, Preset};

    use super::*;

    #[test]
    fn fake_factory_builds_a_working_pair_for_the_preset() {
        let preset = Preset { id: 2, name: "720p".into(), width: 1280, height: 720, fps: 30, bitrate_kbps: 5_000, codec: Codec::Av1 };
        let parts = fake::FakeCodecs.open(SourceInfo { width: 1920, height: 1080, fps: 60 }, PixelFormat::Bgra, &preset).unwrap();
        let mut encoder = parts.encoder;
        let params = encoder.params();
        assert_eq!((params.width, params.height, params.fps, params.codec), (1280, 720, 30, Codec::Av1));
        let packets = encoder.encode(&RawFrame::black(1280, 720, 9), false).unwrap();
        let mut decoder = DecoderFactory::open(&fake::FakeCodecs, &params).unwrap();
        assert_eq!(decoder.decode(&packets[0]).unwrap()[0].capture_ts_us, 9);
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p brp-room codecs`
Expected: module missing.

- [ ] **Step 3: Implement**

`crates/room/src/codecs.rs` (above its tests):

```rust
//! Where encoders and decoders come from. The room only knows these traits, so tests swap in fakes.

use brp_capture::SourceInfo;
use brp_codec::{CodecError, EncoderConfig, FrameConverter, VideoDecoder, VideoEncoder, open_decoder, open_encoder};
use brp_codec::ffmpeg::SwsConverter;
use brp_proto::{CodecParams, PixelFormat, Preset};

pub struct EncoderParts {
    pub converter: Box<dyn FrameConverter>,
    pub encoder: Box<dyn VideoEncoder>,
}

pub trait EncoderFactory: Send + Sync + 'static {
    fn open(&self, source: SourceInfo, source_format: PixelFormat, preset: &Preset) -> Result<EncoderParts, CodecError>;
}

pub trait DecoderFactory: Send + Sync + 'static {
    fn open(&self, params: &CodecParams) -> Result<Box<dyn VideoDecoder>, CodecError>;
}

fn config_for(preset: &Preset) -> EncoderConfig {
    EncoderConfig { width: preset.width, height: preset.height, fps: preset.fps, bitrate_kbps: preset.bitrate_kbps, codec: preset.codec }
}

/// The production factory: swscale for conversion, the spec's probe order for encoders,
/// hardware-first decoding.
#[derive(Debug, Clone, Copy, Default)]
pub struct FfmpegCodecs;

impl EncoderFactory for FfmpegCodecs {
    fn open(&self, source: SourceInfo, source_format: PixelFormat, preset: &Preset) -> Result<EncoderParts, CodecError> {
        let converter = SwsConverter::new(source.width, source.height, source_format, preset.width, preset.height)?;
        let encoder = open_encoder(&config_for(preset))?;
        Ok(EncoderParts { converter: Box::new(converter), encoder })
    }
}

impl DecoderFactory for FfmpegCodecs {
    fn open(&self, params: &CodecParams) -> Result<Box<dyn VideoDecoder>, CodecError> {
        open_decoder(params)
    }
}

pub mod fake {
    use brp_codec::fake::{FakeDecoder, FakeEncoder, SolidConverter};

    use super::*;

    /// Keyframe every 30 frames, like a real encoder asked for periodic refresh.
    const FAKE_KEYFRAME_INTERVAL: u32 = 30;

    #[derive(Debug, Clone, Copy, Default)]
    pub struct FakeCodecs;

    impl EncoderFactory for FakeCodecs {
        fn open(&self, _source: SourceInfo, _format: PixelFormat, preset: &Preset) -> Result<EncoderParts, CodecError> {
            Ok(EncoderParts {
                converter: Box::new(SolidConverter::new(preset.width, preset.height)),
                encoder: Box::new(FakeEncoder::new(config_for(preset), FAKE_KEYFRAME_INTERVAL)),
            })
        }
    }

    impl DecoderFactory for FakeCodecs {
        fn open(&self, _params: &CodecParams) -> Result<Box<dyn VideoDecoder>, CodecError> {
            Ok(Box::new(FakeDecoder))
        }
    }
}
```

Add `pub mod codecs;` to `crates/room/src/lib.rs`.

- [ ] **Step 4: Run, lint, commit**

Run: `cargo test -p brp-room`

```bash
git add crates/room
git commit -m "feat: add encoder and decoder factories with fakes for tests"
```

### Task 9: Live registry with capture fan and lazy encoders

**Files:**
- Create: `crates/room/src/registry.rs`, `crates/room/src/snapshot.rs`, `crates/room/tests/registry.rs`
- Modify: `crates/room/src/codecs.rs` (add `preferred_codec`), `crates/room/src/lib.rs`, `crates/net/src/source.rs` (add `SubscribeRejected::EncoderFailed`)

**Interfaces:**
- Consumes: `Publisher::start(live_id, preset_id, slot, converter, encoder)`, `Publisher::{subscribe, request_keyframe, subscriber_count, stop, stats, encoder_name, frames_dropped_at_input}`, `EncoderFactory`, `LatestSlot`, `CaptureSession`.
- Produces: `brp_room::registry::{CaptureFan, LiveRegistry}`. `CaptureFan::default()`, `push(&self, CaptureFrame)`, `attach(&self) -> Arc<LatestSlot<Arc<CaptureFrame>>>`, `detach(&self, &Arc<LatestSlot<..>>)`, `format(&self) -> PixelFormat`. `LiveRegistry::new(encoders: Arc<dyn EncoderFactory>, grace: Duration, on_change: ChangeNotify) -> Arc<Self>`, `add_live(&self, title: String, kind: SourceKind, session: Box<dyn CaptureSession>, fan: Arc<CaptureFan>, presets: Vec<Preset>) -> Result<u32, RoomError>`, `remove_live(&self, live_id) -> Result<(), RoomError>`, `set_presets(&self, live_id, presets: Vec<Preset>) -> Result<(), RoomError>`, `live_infos(&self) -> Vec<LiveInfo>`, `views(&self) -> Vec<OwnLiveView>`, `housekeeping(&self, now: Instant)`, `stop_all(&self)`; implements `brp_net::LiveSource`. `brp_room::ChangeNotify = Arc<dyn Fn() + Send + Sync>`. Snapshot types `OwnLiveView { info: LiveInfo, presets: Vec<PresetView> }`, `PresetView { preset: Preset, encoder: Option<EncoderView>, last_error: Option<String> }`, `EncoderView { name: &'static str, subscribers: usize, frames_encoded: u64, bytes_encoded: u64, dropped_at_input: u64 }`. `EncoderFactory::preferred_codec(&self) -> Codec`.

- [ ] **Step 1: Small additions to net and codecs**

In `crates/net/src/source.rs` add to `SubscribeRejected`:

```rust
    #[error("encoder could not start: {0}")]
    EncoderFailed(String),
```

In `crates/room/src/codecs.rs` add to the `EncoderFactory` trait:

```rust
    /// The codec new lives default to. The real factory probes the GPU once; the spec prefers HEVC,
    /// then H.264, then the software AV1 fallback.
    fn preferred_codec(&self) -> Codec;
```

with `use brp_proto::Codec;` and these implementations:

```rust
// in impl EncoderFactory for FfmpegCodecs
    fn preferred_codec(&self) -> Codec {
        let probe = EncoderConfig { width: 64, height: 64, fps: 30, bitrate_kbps: 1_000, codec: Codec::Hevc };
        brp_codec::open_encoder_auto(probe, None).map(|e| e.params().codec).unwrap_or(Codec::Av1)
    }

// in impl EncoderFactory for fake::FakeCodecs
    fn preferred_codec(&self) -> Codec {
        Codec::H264
    }
```

- [ ] **Step 2: Snapshot types**

`crates/room/src/snapshot.rs`:

```rust
//! Read-only views the window renders. Cloned out of the room on every version bump.

use brp_net::PathKind;
use brp_proto::{LiveInfo, Preset};
use iroh::PublicKey;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncoderView {
    pub name: &'static str,
    pub subscribers: usize,
    pub frames_encoded: u64,
    pub bytes_encoded: u64,
    pub dropped_at_input: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetView {
    pub preset: Preset,
    pub encoder: Option<EncoderView>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnLiveView {
    pub info: LiveInfo,
    pub presets: Vec<PresetView>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberView {
    pub id: PublicKey,
    pub nickname: String,
    pub lives: Vec<LiveInfo>,
    pub seen_ago_ms: u64,
    pub path: PathKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchState {
    Connecting,
    Live,
    Reconnecting,
    Ended,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchView {
    pub publisher: PublicKey,
    pub live_id: u32,
    pub preset_id: u32,
    pub state: WatchState,
    pub frames_decoded: u64,
    pub keyframe_requests: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomSnapshot {
    pub me: PublicKey,
    pub nickname: String,
    pub version: u64,
    pub members: Vec<MemberView>,
    pub own_lives: Vec<OwnLiveView>,
    pub watches: Vec<WatchView>,
}
```

- [ ] **Step 3: Write the failing registry tests**

`crates/room/tests/registry.rs`:

```rust
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use brp_capture::{CaptureBackend, CaptureSession, SourceInfo, SourceRequest, SyntheticSource};
use brp_net::{LiveSource, SubscribeRejected};
use brp_proto::constants::{MAX_LIVES_PER_PARTICIPANT, SOURCE_PRESET_ID};
use brp_proto::{Codec, SourceKind, template_presets};
use brp_room::codecs::fake::FakeCodecs;
use brp_room::registry::{CaptureFan, LiveRegistry};

const GRACE: Duration = Duration::from_millis(300);

struct DummySession;

impl CaptureSession for DummySession {
    fn info(&self) -> SourceInfo {
        SourceInfo { width: 64, height: 32, fps: 30 }
    }
    fn stop(self: Box<Self>) {}
}

async fn synthetic_live(registry: &LiveRegistry, title: &str) -> u32 {
    let fan = Arc::new(CaptureFan::default());
    let sink = fan.clone();
    let session = SyntheticSource { width: 64, height: 32, fps: 60 }
        .start(SourceRequest { kind: SourceKind::Monitor, target_fps: 60 }, Box::new(move |f| sink.push(f)))
        .await
        .unwrap();
    let presets = template_presets(64, 32, 60, Codec::H264);
    registry.add_live(title.into(), SourceKind::Monitor, session, fan, presets).unwrap()
}

#[tokio::test]
async fn encoders_start_on_first_subscription_and_stop_after_the_grace() {
    let changes = Arc::new(AtomicUsize::new(0));
    let counter = changes.clone();
    let registry = LiveRegistry::new(Arc::new(FakeCodecs), GRACE, Arc::new(move || {
        counter.fetch_add(1, Ordering::SeqCst);
    }));
    let live = synthetic_live(&registry, "desk").await;
    assert_eq!(registry.live_infos()[0].presets.len(), 1, "64x32 has no smaller template");
    assert!(registry.views()[0].presets[0].encoder.is_none());

    let mut sub = registry.subscribe(live, SOURCE_PRESET_ID).unwrap();
    let encoder = registry.views()[0].presets[0].encoder.clone().expect("encoder started lazily");
    assert_eq!((encoder.name, encoder.subscribers), ("fake", 1));
    let first = tokio::time::timeout(Duration::from_secs(2), sub.frames.recv()).await.unwrap().unwrap();
    assert!(first.keyframe);

    drop(sub);
    let t = Instant::now();
    registry.housekeeping(t);
    assert!(registry.views()[0].presets[0].encoder.is_some(), "still inside the grace");
    registry.housekeeping(t + GRACE);
    assert!(registry.views()[0].presets[0].encoder.is_none(), "stopped after the grace");
    assert!(changes.load(Ordering::SeqCst) >= 3, "add, start, stop each notified");

    assert_eq!(registry.subscribe(99, 1).unwrap_err(), SubscribeRejected::UnknownLive(99));
    assert_eq!(registry.subscribe(live, 99).unwrap_err(), SubscribeRejected::UnknownPreset(99));
    registry.remove_live(live).unwrap();
    assert!(registry.live_infos().is_empty());
}

#[tokio::test]
async fn removing_a_preset_stops_its_encoder_and_ends_its_subscription() {
    let registry = LiveRegistry::new(Arc::new(FakeCodecs), GRACE, Arc::new(|| {}));
    let live = synthetic_live(&registry, "desk").await;
    let mut presets = template_presets(64, 32, 60, Codec::H264);
    presets.push(brp_proto::Preset { id: 2, name: "tiny".into(), width: 32, height: 16, fps: 30, bitrate_kbps: 1_000, codec: Codec::H264 });
    registry.set_presets(live, presets.clone()).unwrap();
    let mut sub = registry.subscribe(live, 2).unwrap();
    let frame = tokio::time::timeout(Duration::from_secs(2), sub.frames.recv()).await.unwrap().unwrap();
    assert!(frame.keyframe);

    registry.set_presets(live, presets[..1].to_vec()).unwrap();
    assert_eq!(registry.live_infos()[0].presets.len(), 1);
    let ended = tokio::time::timeout(Duration::from_secs(2), async {
        while sub.frames.recv().await.is_some() {}
    })
    .await;
    assert!(ended.is_ok(), "frame channel closes when the preset's encoder stops");
    registry.stop_all();
}

#[test]
fn live_limit_and_preset_validation_are_enforced() {
    let registry = LiveRegistry::new(Arc::new(FakeCodecs), GRACE, Arc::new(|| {}));
    for i in 0..MAX_LIVES_PER_PARTICIPANT {
        registry.add_live(format!("l{i}"), SourceKind::Window, Box::new(DummySession), Arc::new(CaptureFan::default()), template_presets(64, 32, 30, Codec::H264)).unwrap();
    }
    let over = registry.add_live("too many".into(), SourceKind::Window, Box::new(DummySession), Arc::new(CaptureFan::default()), vec![]);
    assert!(matches!(over, Err(brp_room::RoomError::TooManyLives)));
    let bad = vec![brp_proto::Preset { id: 1, name: "huge".into(), width: 4096, height: 2160, fps: 30, bitrate_kbps: 5_000, codec: Codec::H264 }];
    assert!(matches!(registry.set_presets(1, bad), Err(brp_room::RoomError::Proto(_))));
}
```

- [ ] **Step 4: Run to verify they fail**

Run: `cargo test -p brp-room --test registry`
Expected: modules unresolved.

- [ ] **Step 5: Implement**

`crates/room/src/registry.rs`:

```rust
//! Lives this participant publishes. Encoders exist only while someone is subscribed.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use brp_capture::{CaptureFrame, CaptureSession};
use brp_net::{LiveSource, SubscribeRejected, Subscription};
use brp_pipeline::{LatestSlot, Publisher};
use brp_proto::constants::{MAX_LIVES_PER_PARTICIPANT, MAX_PRESETS_PER_LIVE};
use brp_proto::{LiveInfo, PixelFormat, Preset, ProtoError, SourceKind};

use crate::codecs::EncoderFactory;
use crate::error::RoomError;
use crate::snapshot::{EncoderView, OwnLiveView, PresetView};

pub type ChangeNotify = Arc<dyn Fn() + Send + Sync>;

type CaptureSlot = Arc<LatestSlot<Arc<CaptureFrame>>>;

/// Delivers each captured frame to every running encoder of one live without copying pixels.
#[derive(Default)]
pub struct CaptureFan {
    slots: Mutex<Vec<CaptureSlot>>,
    last_format: Mutex<Option<PixelFormat>>,
}

impl CaptureFan {
    pub fn push(&self, frame: CaptureFrame) {
        *lock(&self.last_format) = Some(frame.format);
        let frame = Arc::new(frame);
        for slot in lock(&self.slots).iter() {
            slot.put(frame.clone());
        }
    }

    pub fn attach(&self) -> CaptureSlot {
        let slot = LatestSlot::new();
        lock(&self.slots).push(slot.clone());
        slot
    }

    pub fn detach(&self, slot: &CaptureSlot) {
        lock(&self.slots).retain(|s| !Arc::ptr_eq(s, slot));
    }

    /// The compositor's pixel order, known after the first frame. The converter rebuilds itself if
    /// this guess is wrong, so a default before the first frame costs nothing.
    pub fn format(&self) -> PixelFormat {
        lock(&self.last_format).unwrap_or(PixelFormat::Bgrx)
    }
}

struct RunningEncoder {
    publisher: Publisher,
    slot: CaptureSlot,
    idle_since: Option<Instant>,
}

struct PresetState {
    preset: Preset,
    running: Option<RunningEncoder>,
    last_error: Option<String>,
}

struct OwnLive {
    info: LiveInfo,
    session: Option<Box<dyn CaptureSession>>,
    fan: Arc<CaptureFan>,
    presets: BTreeMap<u32, PresetState>,
}

struct Inner {
    lives: BTreeMap<u32, OwnLive>,
    next_live_id: u32,
}

pub struct LiveRegistry {
    inner: Mutex<Inner>,
    encoders: Arc<dyn EncoderFactory>,
    grace: Duration,
    on_change: ChangeNotify,
}

impl LiveRegistry {
    pub fn new(encoders: Arc<dyn EncoderFactory>, grace: Duration, on_change: ChangeNotify) -> Arc<Self> {
        Arc::new(Self { inner: Mutex::new(Inner { lives: BTreeMap::new(), next_live_id: 1 }), encoders, grace, on_change })
    }

    pub fn add_live(&self, title: String, kind: SourceKind, session: Box<dyn CaptureSession>, fan: Arc<CaptureFan>, presets: Vec<Preset>) -> Result<u32, RoomError> {
        let source = session.info();
        validate_presets(&presets, source.width, source.height, source.fps)?;
        let mut inner = lock(&self.inner);
        if inner.lives.len() >= MAX_LIVES_PER_PARTICIPANT {
            return Err(RoomError::TooManyLives);
        }
        let id = inner.next_live_id;
        inner.next_live_id += 1;
        let info = LiveInfo {
            id,
            title,
            kind,
            source_width: source.width,
            source_height: source.height,
            source_fps: source.fps,
            has_audio: false,
            presets: presets.clone(),
        };
        let presets = presets.into_iter().map(|p| (p.id, PresetState { preset: p, running: None, last_error: None })).collect();
        inner.lives.insert(id, OwnLive { info, session: Some(session), fan, presets });
        drop(inner);
        (self.on_change)();
        Ok(id)
    }

    pub fn remove_live(&self, live_id: u32) -> Result<(), RoomError> {
        let mut inner = lock(&self.inner);
        let mut live = inner.lives.remove(&live_id).ok_or(RoomError::UnknownLive(live_id))?;
        drop(inner);
        for state in live.presets.values_mut() {
            stop_encoder(&live.fan, state);
        }
        if let Some(session) = live.session.take() {
            session.stop();
        }
        (self.on_change)();
        Ok(())
    }

    /// Replaces the preset list. Running encoders whose preset is unchanged keep running; removed or
    /// edited presets stop, which ends their subscriptions with live-ended.
    pub fn set_presets(&self, live_id: u32, presets: Vec<Preset>) -> Result<(), RoomError> {
        let mut inner = lock(&self.inner);
        let live = inner.lives.get_mut(&live_id).ok_or(RoomError::UnknownLive(live_id))?;
        validate_presets(&presets, live.info.source_width, live.info.source_height, live.info.source_fps)?;
        let mut old = std::mem::take(&mut live.presets);
        for preset in presets.iter() {
            let state = match old.remove(&preset.id) {
                Some(mut state) if state.preset == *preset => {
                    state.last_error = None;
                    state
                }
                Some(mut state) => {
                    stop_encoder(&live.fan, &mut state);
                    PresetState { preset: preset.clone(), running: None, last_error: None }
                }
                None => PresetState { preset: preset.clone(), running: None, last_error: None },
            };
            live.presets.insert(preset.id, state);
        }
        for mut removed in old.into_values() {
            stop_encoder(&live.fan, &mut removed);
        }
        live.info.presets = presets;
        drop(inner);
        (self.on_change)();
        Ok(())
    }

    pub fn live_infos(&self) -> Vec<LiveInfo> {
        lock(&self.inner).lives.values().map(|l| l.info.clone()).collect()
    }

    pub fn views(&self) -> Vec<OwnLiveView> {
        lock(&self.inner)
            .lives
            .values()
            .map(|live| OwnLiveView {
                info: live.info.clone(),
                presets: live
                    .presets
                    .values()
                    .map(|state| PresetView {
                        preset: state.preset.clone(),
                        encoder: state.running.as_ref().map(|r| EncoderView {
                            name: r.publisher.encoder_name(),
                            subscribers: r.publisher.subscriber_count(),
                            frames_encoded: r.publisher.stats().frames_encoded.load(Ordering::Relaxed),
                            bytes_encoded: r.publisher.stats().bytes_encoded.load(Ordering::Relaxed),
                            dropped_at_input: r.publisher.frames_dropped_at_input(),
                        }),
                        last_error: state.last_error.clone(),
                    })
                    .collect(),
            })
            .collect()
    }

    /// Stops encoders that have had no subscriber for the whole grace period.
    pub fn housekeeping(&self, now: Instant) {
        let mut inner = lock(&self.inner);
        let mut stopped_any = false;
        for live in inner.lives.values_mut() {
            for state in live.presets.values_mut() {
                let idle_for = match state.running.as_mut() {
                    Some(running) if running.publisher.subscriber_count() == 0 => now.duration_since(*running.idle_since.get_or_insert(now)),
                    Some(running) => {
                        running.idle_since = None;
                        continue;
                    }
                    None => continue,
                };
                if idle_for >= self.grace {
                    stop_encoder(&live.fan, state);
                    stopped_any = true;
                }
            }
        }
        drop(inner);
        if stopped_any {
            (self.on_change)();
        }
    }

    pub fn stop_all(&self) {
        let ids: Vec<u32> = lock(&self.inner).lives.keys().copied().collect();
        for id in ids {
            let _ = self.remove_live(id);
        }
    }
}

impl LiveSource for LiveRegistry {
    fn subscribe(&self, live_id: u32, preset_id: u32) -> Result<Subscription, SubscribeRejected> {
        let mut inner = lock(&self.inner);
        let live = inner.lives.get_mut(&live_id).ok_or(SubscribeRejected::UnknownLive(live_id))?;
        let source = brp_capture::SourceInfo { width: live.info.source_width, height: live.info.source_height, fps: live.info.source_fps };
        let format = live.fan.format();
        let fan = live.fan.clone();
        let state = live.presets.get_mut(&preset_id).ok_or(SubscribeRejected::UnknownPreset(preset_id))?;
        let mut started = false;
        if state.running.is_none() {
            match self.encoders.open(source, format, &state.preset) {
                Ok(parts) => {
                    let slot = fan.attach();
                    let publisher = Publisher::start(live_id, preset_id, slot.clone(), parts.converter, parts.encoder);
                    state.running = Some(RunningEncoder { publisher, slot, idle_since: None });
                    state.last_error = None;
                    started = true;
                }
                Err(error) => {
                    state.last_error = Some(error.to_string());
                    drop(inner);
                    (self.on_change)();
                    return Err(SubscribeRejected::EncoderFailed(error.to_string()));
                }
            }
        }
        let running = state.running.as_mut().expect("set above");
        running.idle_since = None;
        let subscription = running.publisher.subscribe(live_id, preset_id);
        drop(inner);
        if started {
            (self.on_change)();
        }
        subscription
    }

    fn request_keyframe(&self, live_id: u32, preset_id: u32) {
        if let Some(running) = lock(&self.inner).lives.get(&live_id).and_then(|l| l.presets.get(&preset_id)).and_then(|s| s.running.as_ref()) {
            running.publisher.request_keyframe(live_id, preset_id);
        }
    }
}

fn stop_encoder(fan: &CaptureFan, state: &mut PresetState) {
    if let Some(running) = state.running.take() {
        running.publisher.stop();
        fan.detach(&running.slot);
    }
}

fn validate_presets(presets: &[Preset], width: u32, height: u32, fps: u32) -> Result<(), RoomError> {
    if presets.len() > MAX_PRESETS_PER_LIVE {
        return Err(RoomError::Proto(ProtoError::Invalid("too many presets".into())));
    }
    for preset in presets {
        preset.validate(width, height, fps).map_err(|e| RoomError::Proto(ProtoError::Invalid(format!("preset {}: {e}", preset.id))))?;
    }
    Ok(())
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}
```

`Publisher::stop` closes the slot and joins the encoder thread; the fan-out inside drops its senders when the publisher's `Inner` is dropped, which closes every subscriber's frame receiver, so the server sends live-ended on those control streams.

Add to `crates/room/src/lib.rs`: `pub mod registry; pub mod snapshot; pub use registry::ChangeNotify;`.

- [ ] **Step 6: Run, lint, commit**

Run: `cargo test -p brp-room && cargo clippy --workspace --all-targets -- -D warnings`
Expected: all three registry tests pass. The second test depends on the subscriber's frame receiver closing when its publisher stops; if it hangs, the `Publisher` still holds a clone of the fan-out somewhere and that is the bug to fix.

```bash
git add crates/net/src/source.rs crates/room
git commit -m "feat: add live registry with shared capture fan and lazy encoders"
```

### Task 10: Gossip presence loop and the Room

**Files:**
- Create: `crates/room/src/gossip.rs`, `crates/room/src/room.rs`, `crates/room/tests/two_rooms.rs`
- Modify: `crates/room/src/lib.rs`

**Interfaces:**
- Consumes: `Membership`, `LiveRegistry`, `CaptureFan`, `EncoderFactory`, `DecoderFactory`, `bind_endpoint`, `MediaServer`, `Signed`, `Presence`, `template_presets`.
- Produces: `brp_room::{Room, RoomConfig, RoomTimings}`. `RoomConfig { secret: SecretKey, relay: RelaySetting, nickname: String, target_fps: u32, capture: Arc<dyn CaptureBackend>, encoders: Arc<dyn EncoderFactory>, decoders: Arc<dyn DecoderFactory>, on_change: ChangeNotify, on_frame: FrameNotify, timings: RoomTimings }`; `RoomTimings { heartbeat, expiry, housekeeping, encoder_grace, join_timeout }` with `Default` from the constants; `Room::create(config).await`, `Room::join(config, ticket).await`, `id()`, `nickname()`, `version()`, `ticket()`, `snapshot()`, `start_live(kind, title).await -> u32`, `stop_live(live_id)`, `set_presets(live_id, presets)`, `leave().await`. Task 11 adds `watch`, `unwatch`, and fills the snapshot's watches and paths.

Verified: `Gossip::builder().spawn(endpoint)`; `Router::builder(ep).accept(MEDIA_ALPN, server).accept(iroh_gossip::ALPN, gossip.clone()).spawn()`; `gossip.subscribe(TopicId::from_bytes(bytes), Vec<EndpointId>).await`; `topic.joined().await`; `topic.split()`; `sender.broadcast(Bytes).await`; `receiver.next().await` through `n0_future::StreamExt` yielding `Result<Event, ApiError>`; `Event::Received(Message { content, .. })`; `EndpointAddr.id`; `impl From<EndpointId> for EndpointAddr`.

- [ ] **Step 1: Write the failing integration tests**

`crates/room/tests/two_rooms.rs`:

```rust
use std::sync::Arc;
use std::time::{Duration, Instant};

use brp_capture::SyntheticSource;
use brp_net::RelaySetting;
use brp_proto::SourceKind;
use brp_room::codecs::fake::FakeCodecs;
use brp_room::{Room, RoomConfig, RoomTimings};
use iroh::SecretKey;

pub fn config(nickname: &str) -> RoomConfig {
    RoomConfig {
        secret: SecretKey::generate(),
        relay: RelaySetting::Disabled,
        nickname: nickname.into(),
        target_fps: 30,
        capture: Arc::new(SyntheticSource { width: 64, height: 32, fps: 30 }),
        encoders: Arc::new(FakeCodecs),
        decoders: Arc::new(FakeCodecs),
        on_change: Arc::new(|| {}),
        on_frame: Arc::new(|| {}),
        timings: RoomTimings {
            heartbeat: Duration::from_millis(200),
            expiry: Duration::from_secs(1),
            housekeeping: Duration::from_millis(100),
            encoder_grace: Duration::from_millis(300),
            join_timeout: Duration::from_secs(5),
        },
    }
}

pub async fn wait_until(what: &str, timeout: Duration, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn two_rooms_see_each_other_and_the_catalog_propagates() {
    let a = Room::create(config("alice")).await.unwrap();
    let b = Room::join(config("bob"), a.ticket()).await.unwrap();

    wait_until("mutual presence", Duration::from_secs(5), || a.snapshot().members.len() == 1 && b.snapshot().members.len() == 1).await;
    assert_eq!(a.snapshot().members[0].nickname, "bob");
    assert_eq!(b.snapshot().members[0].nickname, "alice");
    assert_eq!(b.snapshot().members[0].id, a.id());
    assert!(a.version() > 0);

    let live = a.start_live(SourceKind::Monitor, "desk".into()).await.unwrap();
    wait_until("catalog", Duration::from_secs(5), || b.snapshot().members[0].lives.iter().any(|l| l.id == live && l.title == "desk")).await;
    let seen = b.snapshot().members[0].lives[0].clone();
    assert_eq!((seen.source_width, seen.source_height, seen.presets.len()), (64, 32, 1));
    assert!(a.snapshot().own_lives[0].presets[0].encoder.is_none(), "nobody watches yet");

    a.stop_live(live).unwrap();
    wait_until("live removed", Duration::from_secs(5), || b.snapshot().members[0].lives.is_empty()).await;

    b.leave().await;
    a.leave().await;
}

#[tokio::test]
async fn a_bad_ticket_times_out_instead_of_hanging() {
    let a = Room::create(config("alice")).await.unwrap();
    let mut ticket = a.ticket();
    // Point the bootstrap at an endpoint id nobody runs.
    ticket.bootstrap[0].id = SecretKey::generate().public();
    let mut cfg = config("bob");
    cfg.timings.join_timeout = Duration::from_millis(500);
    let started = Instant::now();
    let joined = Room::join(cfg, ticket).await;
    assert!(matches!(joined, Err(brp_room::RoomError::JoinTimeout)));
    assert!(started.elapsed() < Duration::from_secs(5));
    a.leave().await;
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p brp-room --test two_rooms`
Expected: `Room` unresolved.

- [ ] **Step 3: Implement the gossip loop**

`crates/room/src/gossip.rs`:

```rust
//! Joins the room topic and keeps membership current with signed presence.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use brp_proto::constants::PROTOCOL_VERSION;
use brp_proto::{Presence, Signed, decode, encode};
use bytes::Bytes;
use iroh::{EndpointId, PublicKey, SecretKey};
use iroh_gossip::api::{Event, GossipReceiver, GossipSender};
use iroh_gossip::net::Gossip;
use iroh_gossip::proto::TopicId;
use n0_future::StreamExt;
use tokio::sync::{Notify, mpsc};

use crate::error::RoomError;
use crate::membership::{Applied, Membership};
use crate::registry::{ChangeNotify, LiveRegistry};

pub(crate) async fn join(gossip: &Gossip, topic: TopicId, bootstrap: Vec<EndpointId>, timeout: Duration) -> Result<(GossipSender, GossipReceiver), RoomError> {
    let mut topic = gossip.subscribe(topic, bootstrap.clone()).await.map_err(|e| RoomError::Gossip(e.to_string()))?;
    // A creator has nobody to join; everyone else must reach a neighbour or the ticket is dead.
    if !bootstrap.is_empty() {
        tokio::time::timeout(timeout, topic.joined()).await.map_err(|_| RoomError::JoinTimeout)?.map_err(|e| RoomError::Gossip(e.to_string()))?;
    }
    Ok(topic.split())
}

pub(crate) struct PresenceLoop {
    pub secret: SecretKey,
    pub nickname: String,
    pub sender: GossipSender,
    pub receiver: GossipReceiver,
    pub membership: Arc<Mutex<Membership>>,
    pub registry: Arc<LiveRegistry>,
    pub dirty: Arc<Notify>,
    pub heartbeat: Duration,
    pub on_change: ChangeNotify,
    pub expired: mpsc::Sender<PublicKey>,
}

impl PresenceLoop {
    pub async fn run(mut self) {
        let me = self.secret.public();
        // The first tick fires immediately, which doubles as the join announcement.
        let mut heartbeat = tokio::time::interval(self.heartbeat);
        loop {
            tokio::select! {
                _ = heartbeat.tick() => {
                    self.broadcast().await;
                    self.expire().await;
                }
                _ = self.dirty.notified() => self.broadcast().await,
                event = self.receiver.next() => match event {
                    Some(Ok(Event::Received(message))) => self.receive(me, &message.content),
                    Some(Ok(Event::Lagged)) => tracing::warn!("gossip lagged; the next heartbeat repairs the catalog"),
                    Some(Ok(other)) => tracing::debug!(?other, "gossip neighbour event"),
                    Some(Err(error)) => {
                        tracing::error!(%error, "gossip receiver failed");
                        break;
                    }
                    None => break,
                },
            }
        }
    }

    async fn broadcast(&self) {
        let presence = Presence { version: PROTOCOL_VERSION, ts_unix_ms: unix_ms(), nickname: self.nickname.clone(), lives: self.registry.live_infos() };
        let bytes = match Signed::sign(&self.secret, &presence).and_then(|signed| encode(&signed)) {
            Ok(bytes) => bytes,
            Err(error) => {
                tracing::error!(%error, "could not sign presence");
                return;
            }
        };
        if let Err(error) = self.sender.broadcast(Bytes::from(bytes)).await {
            tracing::warn!(%error, "presence broadcast failed");
        }
    }

    async fn expire(&self) {
        let expired = lock(&self.membership).expire(Instant::now());
        if expired.is_empty() {
            return;
        }
        (self.on_change)();
        for id in expired {
            let _ = self.expired.send(id).await;
        }
    }

    fn receive(&self, me: PublicKey, content: &[u8]) {
        let signed: Signed = match decode(content) {
            Ok(signed) => signed,
            Err(error) => {
                tracing::debug!(%error, "dropping undecodable gossip message");
                return;
            }
        };
        let presence: Presence = match signed.verify() {
            Ok(presence) => presence,
            Err(error) => {
                tracing::debug!(author = %signed.author.fmt_short(), %error, "dropping presence");
                return;
            }
        };
        if let Err(error) = presence.validate() {
            tracing::debug!(author = %signed.author.fmt_short(), %error, "dropping invalid presence");
            return;
        }
        if signed.author == me {
            return;
        }
        match lock(&self.membership).apply(signed.author, presence, Instant::now()) {
            Applied::Inserted | Applied::Updated => (self.on_change)(),
            Applied::Refreshed | Applied::Stale => {}
        }
    }
}

pub(crate) fn unix_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}
```

- [ ] **Step 4: Implement the Room**

`crates/room/src/room.rs`:

```rust
//! Wires endpoint, gossip, media server, registry, and watcher together behind one handle.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use brp_capture::{CaptureBackend, SourceRequest};
use brp_net::{MediaServer, PathKind, RelaySetting, bind_endpoint};
use brp_pipeline::FrameNotify;
use brp_proto::constants::{ENCODER_IDLE_STOP_GRACE, JOIN_TIMEOUT, MEDIA_ALPN, MEMBER_EXPIRY, NICKNAME_MAX_LEN, PRESENCE_HEARTBEAT, REGISTRY_HOUSEKEEPING};
use brp_proto::{Preset, RoomTicket, SourceKind, template_presets};
use iroh::protocol::Router;
use iroh::{Endpoint, EndpointAddr, EndpointId, PublicKey, SecretKey};
use iroh_gossip::net::Gossip;
use iroh_gossip::proto::TopicId;
use tokio::sync::{Notify, mpsc};
use tokio::task::JoinHandle;

use crate::codecs::{DecoderFactory, EncoderFactory};
use crate::error::RoomError;
use crate::gossip::{self, PresenceLoop, lock};
use crate::membership::Membership;
use crate::registry::{CaptureFan, ChangeNotify, LiveRegistry};
use crate::snapshot::{MemberView, RoomSnapshot};

#[derive(Debug, Clone, Copy)]
pub struct RoomTimings {
    pub heartbeat: Duration,
    pub expiry: Duration,
    pub housekeeping: Duration,
    pub encoder_grace: Duration,
    pub join_timeout: Duration,
}

impl Default for RoomTimings {
    fn default() -> Self {
        Self { heartbeat: PRESENCE_HEARTBEAT, expiry: MEMBER_EXPIRY, housekeeping: REGISTRY_HOUSEKEEPING, encoder_grace: ENCODER_IDLE_STOP_GRACE, join_timeout: JOIN_TIMEOUT }
    }
}

pub struct RoomConfig {
    pub secret: SecretKey,
    pub relay: RelaySetting,
    pub nickname: String,
    pub target_fps: u32,
    pub capture: Arc<dyn CaptureBackend>,
    pub encoders: Arc<dyn EncoderFactory>,
    pub decoders: Arc<dyn DecoderFactory>,
    pub on_change: ChangeNotify,
    pub on_frame: FrameNotify,
    pub timings: RoomTimings,
}

pub struct Room {
    me: PublicKey,
    nickname: String,
    topic: [u8; 32],
    endpoint: Endpoint,
    router: Router,
    membership: Arc<Mutex<Membership>>,
    registry: Arc<LiveRegistry>,
    capture: Arc<dyn CaptureBackend>,
    encoders: Arc<dyn EncoderFactory>,
    target_fps: u32,
    version: Arc<AtomicU64>,
    tasks: Vec<JoinHandle<()>>,
}

impl Room {
    pub async fn create(config: RoomConfig) -> Result<Self, RoomError> {
        Self::start(config, RoomTicket::random_topic(), Vec::new()).await
    }

    pub async fn join(config: RoomConfig, ticket: RoomTicket) -> Result<Self, RoomError> {
        Self::start(config, ticket.topic, ticket.bootstrap).await
    }

    async fn start(config: RoomConfig, topic: [u8; 32], bootstrap: Vec<EndpointAddr>) -> Result<Self, RoomError> {
        let nickname: String = config.nickname.chars().take(NICKNAME_MAX_LEN).collect();
        let me = config.secret.public();
        let endpoint = bind_endpoint(config.secret.clone(), config.relay, bootstrap.clone()).await?;

        let version = Arc::new(AtomicU64::new(0));
        let dirty = Arc::new(Notify::new());
        let notify: ChangeNotify = {
            let version = version.clone();
            let callback = config.on_change.clone();
            Arc::new(move || {
                version.fetch_add(1, Ordering::Relaxed);
                callback();
            })
        };
        // Registry changes alter our presence, so they also wake the broadcaster.
        let registry_notify: ChangeNotify = {
            let notify = notify.clone();
            let dirty = dirty.clone();
            Arc::new(move || {
                notify();
                dirty.notify_one();
            })
        };
        let membership = Arc::new(Mutex::new(Membership::new(config.timings.expiry)));
        let registry = LiveRegistry::new(config.encoders.clone(), config.timings.encoder_grace, registry_notify);
        let policy = {
            let membership = membership.clone();
            Arc::new(move |peer: EndpointId| lock(&membership).is_member(&peer))
        };

        let gossip = Gossip::builder().spawn(endpoint.clone());
        let router = Router::builder(endpoint.clone())
            .accept(MEDIA_ALPN, MediaServer::new(registry.clone(), policy))
            .accept(iroh_gossip::ALPN, gossip.clone())
            .spawn();

        let bootstrap_ids: Vec<EndpointId> = bootstrap.iter().map(|addr| addr.id).collect();
        let (sender, receiver) = gossip::join(&gossip, TopicId::from_bytes(topic), bootstrap_ids, config.timings.join_timeout).await?;

        let (expired_tx, _expired_rx) = mpsc::channel::<PublicKey>(16);
        let presence = PresenceLoop {
            secret: config.secret,
            nickname: nickname.clone(),
            sender,
            receiver,
            membership: membership.clone(),
            registry: registry.clone(),
            dirty,
            heartbeat: config.timings.heartbeat,
            on_change: notify.clone(),
            expired: expired_tx,
        };
        let housekeeping = {
            let registry = registry.clone();
            let every = config.timings.housekeeping;
            async move {
                let mut tick = tokio::time::interval(every);
                loop {
                    tick.tick().await;
                    registry.housekeeping(Instant::now());
                }
            }
        };
        let tasks = vec![tokio::spawn(presence.run()), tokio::spawn(housekeeping)];

        Ok(Self { me, nickname, topic, endpoint, router, membership, registry, capture: config.capture, encoders: config.encoders, target_fps: config.target_fps, version, tasks })
    }

    pub fn id(&self) -> PublicKey {
        self.me
    }

    pub fn nickname(&self) -> &str {
        &self.nickname
    }

    pub fn version(&self) -> u64 {
        self.version.load(Ordering::Relaxed)
    }

    /// A ticket listing this participant as bootstrap, so anyone online can invite.
    pub fn ticket(&self) -> RoomTicket {
        RoomTicket::new(self.topic, vec![self.endpoint.addr()])
    }

    pub fn snapshot(&self) -> RoomSnapshot {
        let now = Instant::now();
        let members = lock(&self.membership)
            .members()
            .map(|m| MemberView {
                id: m.id,
                nickname: m.presence.nickname.clone(),
                lives: m.presence.lives.clone(),
                seen_ago_ms: now.duration_since(m.last_seen).as_millis() as u64,
                path: PathKind::Unknown,
            })
            .collect();
        RoomSnapshot { me: self.me, nickname: self.nickname.clone(), version: self.version(), members, own_lives: self.registry.views(), watches: Vec::new() }
    }

    pub async fn start_live(&self, kind: SourceKind, title: String) -> Result<u32, RoomError> {
        let fan = Arc::new(CaptureFan::default());
        let sink = fan.clone();
        let session = self.capture.start(SourceRequest { kind, target_fps: self.target_fps }, Box::new(move |frame| sink.push(frame))).await?;
        let info = session.info();
        let fps = info.fps.min(self.target_fps).max(1);
        let presets = template_presets(info.width, info.height, fps, self.encoders.preferred_codec());
        self.registry.add_live(title, kind, session, fan, presets)
    }

    pub fn stop_live(&self, live_id: u32) -> Result<(), RoomError> {
        self.registry.remove_live(live_id)
    }

    pub fn set_presets(&self, live_id: u32, presets: Vec<Preset>) -> Result<(), RoomError> {
        self.registry.set_presets(live_id, presets)
    }

    pub async fn leave(mut self) {
        for task in self.tasks.drain(..) {
            task.abort();
        }
        self.registry.stop_all();
        if let Err(error) = self.router.shutdown().await {
            tracing::warn!(%error, "router shutdown");
        }
        self.endpoint.close().await;
    }
}
```

`crates/room/src/lib.rs` becomes:

```rust
//! A room: membership over signed gossip presence, published lives with lazy encoders, and watches.

pub mod codecs;
pub mod error;
mod gossip;
pub mod membership;
pub mod registry;
mod room;
pub mod snapshot;

pub use error::RoomError;
pub use registry::ChangeNotify;
pub use room::{Room, RoomConfig, RoomTimings};
pub use snapshot::{EncoderView, MemberView, OwnLiveView, PresetView, RoomSnapshot, WatchState, WatchView};
```

- [ ] **Step 5: Run, lint, commit**

Run: `cargo test -p brp-room && cargo clippy --workspace --all-targets -- -D warnings`
Expected: both integration tests pass. The first should finish in about a second; the second in about half a second.

```bash
git add crates/room
git commit -m "feat: add room with gossip presence, membership gating, and lives"
```

### Task 11: Watcher with reconnection and preset fallback

**Files:**
- Create: `crates/room/src/watcher.rs`
- Modify: `crates/room/src/room.rs`, `crates/room/src/lib.rs`, `crates/room/tests/two_rooms.rs`

**Interfaces:**
- Consumes: `MediaClient::{connect, subscribe, path_kind}`, `ViewerSubscription`, `Viewer::start(runtime, frames, control, decoder, ViewerSink)`, `DecoderFactory`, `Membership`.
- Produces: `brp_room::{WatchHandle { slot: Arc<LatestSlot<RawFrame>>, stats: Arc<ViewerStats> }}`; `Room::watch(&self, publisher: PublicKey, live_id: u32, preset_id: u32) -> Result<WatchHandle, RoomError>`, `Room::unwatch(&self, publisher, live_id) -> Result<(), RoomError>`; snapshot `watches` and member `path` populated.

- [ ] **Step 1: Add the failing tests**

Append to `crates/room/tests/two_rooms.rs`:

```rust
use brp_net::{MediaClient, RelaySetting as Relay, bind_endpoint};
use brp_proto::constants::SOURCE_PRESET_ID;
use brp_proto::{Codec, Preset};
use brp_room::WatchState;
use std::sync::atomic::Ordering;

async fn joined_pair() -> (Room, Room) {
    let a = Room::create(config("alice")).await.unwrap();
    let b = Room::join(config("bob"), a.ticket()).await.unwrap();
    wait_until("mutual presence", Duration::from_secs(5), || a.snapshot().members.len() == 1 && b.snapshot().members.len() == 1).await;
    (a, b)
}

#[tokio::test]
async fn watching_starts_the_encoder_and_unwatching_stops_it_after_the_grace() {
    let (a, b) = joined_pair().await;
    let live = a.start_live(SourceKind::Monitor, "desk".into()).await.unwrap();
    wait_until("catalog", Duration::from_secs(5), || !b.snapshot().members[0].lives.is_empty()).await;

    let handle = b.watch(a.id(), live, SOURCE_PRESET_ID).unwrap();
    wait_until("decoded frames", Duration::from_secs(5), || handle.stats.frames_decoded.load(Ordering::Relaxed) >= 3).await;
    let frame = handle.slot.try_take().expect("a frame is waiting for the renderer");
    assert_eq!((frame.width, frame.height), (64, 32));
    assert_eq!(b.snapshot().watches[0].state, WatchState::Live);
    let encoder = a.snapshot().own_lives[0].presets[0].encoder.clone().expect("encoder started for the watcher");
    assert_eq!(encoder.subscribers, 1);
    assert_ne!(b.snapshot().members[0].path, brp_net::PathKind::Unknown);

    b.unwatch(a.id(), live).unwrap();
    assert!(b.snapshot().watches.is_empty());
    wait_until("encoder stopped", Duration::from_secs(5), || a.snapshot().own_lives[0].presets[0].encoder.is_none()).await;

    b.leave().await;
    a.leave().await;
}

#[tokio::test]
async fn a_stranger_is_refused_by_the_media_server() {
    let a = Room::create(config("alice")).await.unwrap();
    let stranger = bind_endpoint(SecretKey::generate(), Relay::Disabled, vec![]).await.unwrap();
    let client = MediaClient::connect(&stranger, a.ticket().bootstrap[0].clone()).await.unwrap();
    let refused = tokio::time::timeout(Duration::from_secs(5), client.subscribe(1, SOURCE_PRESET_ID)).await.expect("refusal is prompt");
    assert!(refused.is_err());
    stranger.close().await;
    a.leave().await;
}

#[tokio::test]
async fn stopping_the_live_ends_the_watch_and_leaving_expires_the_member() {
    let (a, b) = joined_pair().await;
    let live = a.start_live(SourceKind::Window, "game".into()).await.unwrap();
    wait_until("catalog", Duration::from_secs(5), || !b.snapshot().members[0].lives.is_empty()).await;
    let handle = b.watch(a.id(), live, SOURCE_PRESET_ID).unwrap();
    wait_until("live", Duration::from_secs(5), || handle.stats.frames_decoded.load(Ordering::Relaxed) >= 1).await;

    a.stop_live(live).unwrap();
    wait_until("watch ended", Duration::from_secs(5), || b.snapshot().watches.first().is_some_and(|w| w.state == WatchState::Ended)).await;

    a.leave().await;
    wait_until("member expired", Duration::from_secs(5), || b.snapshot().members.is_empty() && b.snapshot().watches.is_empty()).await;
    b.leave().await;
}

#[tokio::test]
async fn preset_changes_propagate_and_a_removed_preset_falls_back_to_source() {
    let (a, b) = joined_pair().await;
    let live = a.start_live(SourceKind::Monitor, "desk".into()).await.unwrap();
    let mut presets = a.snapshot().own_lives[0].info.presets.clone();
    presets.push(Preset { id: 2, name: "tiny".into(), width: 32, height: 16, fps: 30, bitrate_kbps: 1_000, codec: Codec::H264 });
    a.set_presets(live, presets.clone()).unwrap();
    wait_until("two presets", Duration::from_secs(5), || b.snapshot().members[0].lives.first().is_some_and(|l| l.presets.len() == 2)).await;

    let handle = b.watch(a.id(), live, 2).unwrap();
    wait_until("tiny frames", Duration::from_secs(5), || handle.stats.frames_decoded.load(Ordering::Relaxed) >= 1).await;
    assert_eq!(handle.slot.try_take().map(|f| (f.width, f.height)), Some((32, 16)));

    a.set_presets(live, presets[..1].to_vec()).unwrap();
    wait_until("fallback to source", Duration::from_secs(10), || b.snapshot().watches.first().is_some_and(|w| w.preset_id == SOURCE_PRESET_ID && w.state == WatchState::Live)).await;
    wait_until("source frames", Duration::from_secs(5), || handle.slot.try_take().is_some_and(|f| f.width == 64)).await;

    b.leave().await;
    a.leave().await;
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p brp-room --test two_rooms`
Expected: `watch`, `unwatch`, `WatchHandle` unresolved.

- [ ] **Step 3: Implement the watcher**

`crates/room/src/watcher.rs`:

```rust
//! Remote lives this participant watches: one media connection per publisher, one decode pipeline
//! per watch, reconnection with backoff while the publisher stays a member.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::sync::atomic::Ordering;

use brp_codec::RawFrame;
use brp_net::{MediaClient, NetError, PathKind};
use brp_pipeline::{FrameNotify, LatestSlot, Viewer, ViewerSink, ViewerStats};
use brp_proto::constants::{RESUBSCRIBE_BACKOFF_INITIAL, RESUBSCRIBE_BACKOFF_MAX, SOURCE_PRESET_ID};
use brp_proto::{PublisherMessage, ViewerMessage};
use iroh::{Endpoint, EndpointAddr, PublicKey};
use tokio::runtime::Handle;
use tokio::sync::oneshot;

use crate::codecs::DecoderFactory;
use crate::error::RoomError;
use crate::gossip::lock;
use crate::membership::Membership;
use crate::registry::ChangeNotify;
use crate::snapshot::{WatchState, WatchView};

/// What a tile renders from. Stable across reconnects of the same watch.
#[derive(Clone)]
pub struct WatchHandle {
    pub slot: Arc<LatestSlot<RawFrame>>,
    pub stats: Arc<ViewerStats>,
}

type WatchKey = (PublicKey, u32);

struct WatchEntry {
    preset_id: u32,
    state: WatchState,
    handle: WatchHandle,
    /// Dropping this ends the watch task. Replacing a watch drops the old one, which is how a
    /// preset switch is an unsubscribe followed by a subscribe.
    _cancel: oneshot::Sender<()>,
}

#[derive(Default)]
struct Inner {
    clients: HashMap<PublicKey, Arc<MediaClient>>,
    watches: HashMap<WatchKey, WatchEntry>,
}

pub struct Watcher {
    endpoint: Endpoint,
    runtime: Handle,
    decoders: Arc<dyn DecoderFactory>,
    membership: Arc<Mutex<Membership>>,
    on_change: ChangeNotify,
    on_frame: FrameNotify,
    inner: Mutex<Inner>,
}

enum Outcome {
    Cancelled,
    Ended,
    Lost,
}

impl Watcher {
    pub fn new(endpoint: Endpoint, runtime: Handle, decoders: Arc<dyn DecoderFactory>, membership: Arc<Mutex<Membership>>, on_change: ChangeNotify, on_frame: FrameNotify) -> Arc<Self> {
        Arc::new(Self { endpoint, runtime, decoders, membership, on_change, on_frame, inner: Mutex::default() })
    }

    pub fn watch(self: &Arc<Self>, publisher: PublicKey, live_id: u32, preset_id: u32) -> Result<WatchHandle, RoomError> {
        if !lock(&self.membership).is_member(&publisher) {
            return Err(RoomError::UnknownMember(publisher));
        }
        let handle = WatchHandle { slot: LatestSlot::new(), stats: Arc::new(ViewerStats::default()) };
        let (cancel_tx, cancel_rx) = oneshot::channel();
        lock(&self.inner).watches.insert((publisher, live_id), WatchEntry { preset_id, state: WatchState::Connecting, handle: handle.clone(), _cancel: cancel_tx });
        tokio::spawn(self.clone().run_watch(publisher, live_id, preset_id, handle.clone(), cancel_rx));
        (self.on_change)();
        Ok(handle)
    }

    pub fn unwatch(&self, publisher: PublicKey, live_id: u32) -> Result<(), RoomError> {
        lock(&self.inner).watches.remove(&(publisher, live_id)).ok_or(RoomError::NotWatching)?;
        (self.on_change)();
        Ok(())
    }

    pub fn member_left(&self, id: PublicKey) {
        let mut inner = lock(&self.inner);
        inner.watches.retain(|(publisher, _), _| *publisher != id);
        inner.clients.remove(&id);
        drop(inner);
        (self.on_change)();
    }

    pub fn path_kind(&self, publisher: &PublicKey) -> PathKind {
        lock(&self.inner).clients.get(publisher).map(|c| c.path_kind()).unwrap_or(PathKind::Unknown)
    }

    pub fn views(&self) -> Vec<WatchView> {
        lock(&self.inner)
            .watches
            .iter()
            .map(|((publisher, live_id), entry)| WatchView {
                publisher: *publisher,
                live_id: *live_id,
                preset_id: entry.preset_id,
                state: entry.state,
                frames_decoded: entry.handle.stats.frames_decoded.load(Ordering::Relaxed),
                keyframe_requests: entry.handle.stats.keyframe_requests.load(Ordering::Relaxed),
            })
            .collect()
    }

    pub fn stop_all(&self) {
        let mut inner = lock(&self.inner);
        inner.watches.clear();
        inner.clients.clear();
    }

    /// Returns false when the watch was removed meanwhile, which tells the task to stop.
    fn set_state(&self, key: WatchKey, preset_id: u32, state: WatchState) -> bool {
        let mut inner = lock(&self.inner);
        let Some(entry) = inner.watches.get_mut(&key) else { return false };
        entry.state = state;
        entry.preset_id = preset_id;
        drop(inner);
        (self.on_change)();
        true
    }

    async fn client_for(&self, publisher: PublicKey) -> Result<Arc<MediaClient>, RoomError> {
        if let Some(client) = lock(&self.inner).clients.get(&publisher).cloned() {
            return Ok(client);
        }
        // Address resolution goes through the endpoint's lookups: the ticket's bootstrap list and
        // the addresses gossip learned while joining.
        let client = Arc::new(MediaClient::connect(&self.endpoint, EndpointAddr::from(publisher)).await?);
        lock(&self.inner).clients.insert(publisher, client.clone());
        Ok(client)
    }

    fn forget_client(&self, publisher: PublicKey) {
        lock(&self.inner).clients.remove(&publisher);
    }

    /// Spec 6.6: a watched preset the publisher removed falls back to Source while the live remains.
    fn fallback_preset(&self, publisher: PublicKey, live_id: u32, preset_id: u32) -> Option<u32> {
        let membership = lock(&self.membership);
        let live = membership.get(&publisher)?.presence.lives.iter().find(|l| l.id == live_id)?;
        let still_offered = live.presets.iter().any(|p| p.id == preset_id);
        if !still_offered && preset_id != SOURCE_PRESET_ID { Some(SOURCE_PRESET_ID) } else { None }
    }

    fn live_exists(&self, publisher: PublicKey, live_id: u32) -> bool {
        lock(&self.membership).get(&publisher).is_some_and(|m| m.presence.lives.iter().any(|l| l.id == live_id))
    }

    async fn run_watch(self: Arc<Self>, publisher: PublicKey, live_id: u32, mut preset_id: u32, handle: WatchHandle, mut cancel: oneshot::Receiver<()>) {
        let key = (publisher, live_id);
        let mut backoff = RESUBSCRIBE_BACKOFF_INITIAL;
        loop {
            if !lock(&self.membership).is_member(&publisher) {
                self.set_state(key, preset_id, WatchState::Ended);
                return;
            }
            let attempt = async {
                let client = self.client_for(publisher).await?;
                client.subscribe(live_id, preset_id).await.map_err(RoomError::from)
            };
            let subscription = tokio::select! {
                _ = &mut cancel => return,
                result = attempt => result,
            };
            let subscription = match subscription {
                Ok(subscription) => subscription,
                Err(RoomError::Net(NetError::Rejected(reason))) => {
                    tracing::info!(%reason, live_id, preset_id, "subscription rejected");
                    if let Some(fallback) = self.fallback_preset(publisher, live_id, preset_id) {
                        preset_id = fallback;
                    } else if !self.live_exists(publisher, live_id) {
                        self.set_state(key, preset_id, WatchState::Ended);
                        return;
                    }
                    if !self.wait_before_retry(key, preset_id, &mut backoff, &mut cancel).await {
                        return;
                    }
                    continue;
                }
                Err(error) => {
                    tracing::debug!(%error, "watch attempt failed");
                    self.forget_client(publisher);
                    if !self.wait_before_retry(key, preset_id, &mut backoff, &mut cancel).await {
                        return;
                    }
                    continue;
                }
            };
            backoff = RESUBSCRIBE_BACKOFF_INITIAL;

            let decoder = match self.decoders.open(&subscription.params) {
                Ok(decoder) => decoder,
                Err(error) => {
                    tracing::error!(%error, "no decoder for this live");
                    let _ = subscription.control.send(ViewerMessage::Unsubscribe).await;
                    self.set_state(key, preset_id, WatchState::Ended);
                    return;
                }
            };
            let sink = ViewerSink { slot: handle.slot.clone(), stats: handle.stats.clone(), notify: self.on_frame.clone() };
            let viewer = Viewer::start(self.runtime.clone(), subscription.frames, subscription.control.clone(), decoder, sink);
            if !self.set_state(key, preset_id, WatchState::Live) {
                stop_viewer(viewer).await;
                return;
            }

            let mut events = subscription.events;
            let outcome = loop {
                tokio::select! {
                    _ = &mut cancel => break Outcome::Cancelled,
                    event = events.recv() => match event {
                        Some(PublisherMessage::LiveEnded) => break Outcome::Ended,
                        Some(_) => continue,
                        None => break Outcome::Lost,
                    },
                }
            };
            stop_viewer(viewer).await;
            match outcome {
                Outcome::Cancelled => {
                    let _ = subscription.control.send(ViewerMessage::Unsubscribe).await;
                    return;
                }
                Outcome::Ended => {
                    // The publisher removed or restarted this preset. Presence may lag behind, in which
                    // case the retry is rejected and handled above.
                    if let Some(fallback) = self.fallback_preset(publisher, live_id, preset_id) {
                        preset_id = fallback;
                    } else if !self.live_exists(publisher, live_id) {
                        self.set_state(key, preset_id, WatchState::Ended);
                        return;
                    }
                }
                Outcome::Lost => self.forget_client(publisher),
            }
            if !self.wait_before_retry(key, preset_id, &mut backoff, &mut cancel).await {
                return;
            }
        }
    }

    /// Marks the watch reconnecting and sleeps the current backoff. False means the watch is gone.
    async fn wait_before_retry(&self, key: WatchKey, preset_id: u32, backoff: &mut std::time::Duration, cancel: &mut oneshot::Receiver<()>) -> bool {
        if !self.set_state(key, preset_id, WatchState::Reconnecting) {
            return false;
        }
        tokio::select! {
            _ = cancel => return false,
            _ = tokio::time::sleep(*backoff) => {}
        }
        *backoff = (*backoff * 2).min(RESUBSCRIBE_BACKOFF_MAX);
        true
    }
}

/// `Viewer::stop` joins the decode thread, so it runs off the async executor.
async fn stop_viewer(viewer: Viewer) {
    let _ = tokio::task::spawn_blocking(move || viewer.stop()).await;
}
```

- [ ] **Step 4: Wire the watcher into the Room**

In `crates/room/src/room.rs`:

- Add field `watcher: Arc<Watcher>` and import `crate::watcher::{WatchHandle, Watcher}`.
- In `start`, after the presence loop is built, replace the `let (expired_tx, _expired_rx)` line with `let (expired_tx, mut expired_rx) = mpsc::channel::<PublicKey>(16);`, create the watcher, and spawn the expiry consumer:

```rust
        let watcher = Watcher::new(endpoint.clone(), tokio::runtime::Handle::current(), config.decoders.clone(), membership.clone(), notify.clone(), config.on_frame.clone());
        let expiry_consumer = {
            let watcher = watcher.clone();
            async move {
                while let Some(id) = expired_rx.recv().await {
                    watcher.member_left(id);
                }
            }
        };
        let tasks = vec![tokio::spawn(presence.run()), tokio::spawn(housekeeping), tokio::spawn(expiry_consumer)];
```

- In `snapshot`, set `path: self.watcher.path_kind(&m.id)` and `watches: self.watcher.views()`.
- Add the commands:

```rust
    pub fn watch(&self, publisher: PublicKey, live_id: u32, preset_id: u32) -> Result<WatchHandle, RoomError> {
        self.watcher.watch(publisher, live_id, preset_id)
    }

    pub fn unwatch(&self, publisher: PublicKey, live_id: u32) -> Result<(), RoomError> {
        self.watcher.unwatch(publisher, live_id)
    }
```

- In `leave`, call `self.watcher.stop_all();` before `self.registry.stop_all();`.

In `crates/room/src/lib.rs` add `mod watcher;` and `pub use watcher::WatchHandle;`.

- [ ] **Step 5: Run, lint, commit**

Run: `cargo test -p brp-room && cargo clippy --workspace --all-targets -- -D warnings`
Expected: all six integration tests pass. The fallback test takes the longest because it waits for a heartbeat and a backoff; it must finish under ten seconds.

```bash
git add crates/room
git commit -m "feat: add watcher with reconnection, member expiry, and preset fallback"
```

### Task 12: `publish` and `watch` on the Room

**Files:**
- Modify: `crates/app/Cargo.toml`, `crates/app/src/cli.rs`, `crates/app/src/error.rs`, `crates/app/src/publish.rs`, `crates/app/src/watch.rs`, `crates/room/src/room.rs`

**Interfaces:**
- Consumes: `Room`, `RoomConfig`, `RoomTimings`, `WatchHandle`, `FfmpegCodecs`, `PortalCapture`, `identity::load_or_create`, `App::new(title, description, slot, stats)`.
- Produces: `brp publish [--ticket T] [--nickname N] ...` creates or joins a room and shares one live; `brp watch <ticket> [--nickname N] [--no-relay]` joins the room and watches live 1 of the ticket's bootstrap peer at Source; `Room::online(&self, timeout: Duration) -> bool`; `AppError::Room`.

- [ ] **Step 1: Manifest, CLI, error**

`crates/app/Cargo.toml`: add `brp-room.workspace = true`.

`crates/app/src/cli.rs`: add to `PublishArgs`

```rust
    /// Join this room instead of creating a new one.
    #[arg(long)]
    pub ticket: Option<String>,
    /// Shown to other participants. Defaults to the short peer id.
    #[arg(long)]
    pub nickname: Option<String>,
```

and to `WatchArgs`

```rust
    #[arg(long)]
    pub nickname: Option<String>,
```

`crates/app/src/error.rs`: add `#[error(transparent)] Room(#[from] brp_room::RoomError),`.

`crates/room/src/room.rs`: add

```rust
    /// Waits for relay registration so the ticket carries a relay address. Always bounded, because
    /// with relays disabled the transport never reports online.
    pub async fn online(&self, timeout: Duration) -> bool {
        tokio::time::timeout(timeout, self.endpoint.online()).await.is_ok()
    }
```

- [ ] **Step 2: Rewrite publish**

`crates/app/src/publish.rs`:

```rust
use std::str::FromStr;
use std::sync::Arc;

use brp_capture::PortalCapture;
use brp_net::RelaySetting;
use brp_proto::constants::{RELAY_ONLINE_TIMEOUT, SOURCE_PRESET_ID, STATS_LOG_INTERVAL};
use brp_proto::{RoomTicket, SourceKind};
use brp_room::codecs::FfmpegCodecs;
use brp_room::{Room, RoomConfig, RoomTimings};

use crate::cli::PublishArgs;
use crate::error::AppError;
use crate::identity;

pub async fn run(args: PublishArgs) -> Result<(), AppError> {
    let relay = if args.no_relay { RelaySetting::Disabled } else { RelaySetting::Default };
    let secret = identity::load_or_create()?;
    let nickname = args.nickname.clone().unwrap_or_else(|| secret.public().fmt_short().to_string());
    let config = RoomConfig {
        secret,
        relay,
        nickname,
        target_fps: args.fps,
        capture: Arc::new(PortalCapture),
        encoders: Arc::new(FfmpegCodecs),
        decoders: Arc::new(FfmpegCodecs),
        on_change: Arc::new(|| {}),
        on_frame: Arc::new(|| {}),
        timings: RoomTimings::default(),
    };
    let room = match &args.ticket {
        Some(ticket) => Room::join(config, RoomTicket::from_str(ticket)?).await?,
        None => Room::create(config).await?,
    };

    let kind: SourceKind = args.source.into();
    let title = match kind {
        SourceKind::Monitor => "Monitor 1",
        SourceKind::Window => "Window 1",
    };
    let live = room.start_live(kind, title.into()).await?;
    if args.bitrate_kbps.is_some() || args.codec.is_some() {
        let mut presets = room.snapshot().own_lives.iter().find(|l| l.info.id == live).map(|l| l.info.presets.clone()).unwrap_or_default();
        for preset in &mut presets {
            if let (Some(bitrate), true) = (args.bitrate_kbps, preset.id == SOURCE_PRESET_ID) {
                preset.bitrate_kbps = bitrate;
            }
            if let Some(codec) = args.codec {
                preset.codec = codec.into();
            }
        }
        room.set_presets(live, presets)?;
    }

    if relay == RelaySetting::Default && !room.online(RELAY_ONLINE_TIMEOUT).await {
        tracing::warn!("relay registration timed out; the ticket may only work on the local network");
    }
    let snapshot = room.snapshot();
    let own = &snapshot.own_lives[0];
    println!("Nickname: {}  Live: {} ({}x{} @ {} fps, {} presets)", snapshot.nickname, own.info.title, own.info.source_width, own.info.source_height, own.info.source_fps, own.presets.len());
    println!("Ticket:\n{}\n\nShare it: brp watch <ticket>. Press Ctrl-C to stop.", room.ticket());

    let mut ticker = tokio::time::interval(STATS_LOG_INTERVAL);
    let mut last_bytes = 0u64;
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            _ = ticker.tick() => {
                let snapshot = room.snapshot();
                let bytes: u64 = snapshot.own_lives.iter().flat_map(|l| l.presets.iter()).filter_map(|p| p.encoder.as_ref()).map(|e| e.bytes_encoded).sum();
                let kbps = bytes.saturating_sub(last_bytes) * 8 / 1000 / STATS_LOG_INTERVAL.as_secs().max(1);
                last_bytes = bytes;
                let running: Vec<String> = snapshot.own_lives.iter().flat_map(|l| l.presets.iter()).filter_map(|p| p.encoder.as_ref().map(|e| format!("{}:{}x{}", e.name, p.preset.width, p.preset.height))).collect();
                tracing::info!(members = snapshot.members.len(), encoders = ?running, kbps, "publishing");
            }
        }
    }
    room.leave().await;
    Ok(())
}
```

- [ ] **Step 3: Rewrite watch as a room member**

`crates/app/src/watch.rs`:

```rust
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use brp_capture::PortalCapture;
use brp_net::RelaySetting;
use brp_proto::RoomTicket;
use brp_proto::constants::{JOIN_TIMEOUT, SOURCE_PRESET_ID};
use brp_room::codecs::FfmpegCodecs;
use brp_room::{Room, RoomConfig, RoomError, RoomTimings, WatchState};
use tokio::runtime::Runtime;
use winit::event_loop::EventLoop;

use crate::cli::WatchArgs;
use crate::error::AppError;
use crate::identity;
use crate::window::{App, AppEvent};

const LIVE_ID: u32 = 1;
/// How often the status line follows the watch state until plan 2b renders snapshots directly.
const STATUS_POLL: Duration = Duration::from_millis(500);

pub fn run(runtime: &Runtime, args: WatchArgs) -> Result<(), AppError> {
    let ticket = RoomTicket::from_str(&args.ticket)?;
    let publisher = ticket.bootstrap.first().ok_or(AppError::EmptyTicket)?.id;
    let relay = if args.no_relay { RelaySetting::Disabled } else { RelaySetting::Default };

    let event_loop = EventLoop::<AppEvent>::with_user_event().build().map_err(|e| AppError::Window(e.to_string()))?;
    let proxy = event_loop.create_proxy();
    let frame_proxy = proxy.clone();

    let (room, handle, description) = runtime.block_on(async {
        let secret = identity::load_or_create()?;
        let nickname = args.nickname.clone().unwrap_or_else(|| secret.public().fmt_short().to_string());
        let config = RoomConfig {
            secret,
            relay,
            nickname,
            target_fps: 60,
            capture: Arc::new(PortalCapture),
            encoders: Arc::new(FfmpegCodecs),
            decoders: Arc::new(FfmpegCodecs),
            on_change: Arc::new(|| {}),
            on_frame: Arc::new(move || {
                let _ = frame_proxy.send_event(AppEvent::NewFrame);
            }),
            timings: RoomTimings::default(),
        };
        let room = Room::join(config, ticket).await?;
        let deadline = Instant::now() + JOIN_TIMEOUT;
        let live = loop {
            let found = room.snapshot().members.into_iter().find(|m| m.id == publisher).and_then(|m| m.lives.into_iter().find(|l| l.id == LIVE_ID));
            if let Some(live) = found {
                break live;
            }
            if Instant::now() > deadline {
                return Err(AppError::Room(RoomError::UnknownLive(LIVE_ID)));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        };
        let handle = room.watch(publisher, LIVE_ID, SOURCE_PRESET_ID)?;
        let description = format!("{} {}x{} @ {} fps from {}", live.title, live.source_width, live.source_height, live.source_fps, publisher.fmt_short());
        Ok::<_, AppError>((Arc::new(room), handle, description))
    })?;
    println!("Watching: {description}");

    let poller = runtime.spawn({
        let room = room.clone();
        let mut last = String::new();
        async move {
            let mut tick = tokio::time::interval(STATUS_POLL);
            loop {
                tick.tick().await;
                let snapshot = room.snapshot();
                let status = match snapshot.watches.first() {
                    Some(w) if w.state == WatchState::Live => format!("live, {:?} path", snapshot.members.iter().find(|m| m.id == w.publisher).map(|m| m.path)),
                    Some(w) => format!("{:?}", w.state),
                    None => "publisher left the room".to_string(),
                };
                if status != last && proxy.send_event(AppEvent::Status(status.clone())).is_ok() {
                    last = status;
                }
            }
        }
    });

    let mut app = App::new(format!("brp: {}", publisher.fmt_short()), description, handle.slot.clone(), handle.stats.clone());
    let outcome = event_loop.run_app(&mut app).map_err(|e| AppError::Window(e.to_string()));

    poller.abort();
    drop(handle);
    if let Ok(room) = Arc::try_unwrap(room) {
        runtime.block_on(room.leave());
    }
    outcome
}
```

- [ ] **Step 4: Build, lint, test**

Run: `cargo build --release && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: clean.

- [ ] **Step 5: Manual check with two headless publishers and one viewer**

1. Terminal A: `cargo run --release -- publish --no-relay --nickname alice`. Pick a monitor. Copy ticket A.
2. Terminal B: `cargo run --release -- publish --no-relay --nickname bob --ticket <ticket A>`. Pick a monitor. Expected within a heartbeat: both terminals log `members = 1`.
3. Terminal C: `cargo run --release -- watch --no-relay <ticket A>`. Expected: a window showing alice's monitor, the status line `live, Some(Direct) path`, and terminal A logging one running encoder with a non-zero bitrate.
4. Terminal D: `cargo run --release -- watch --no-relay <ticket printed by B>`. Expected: bob's monitor, and terminal B now logging an encoder.
5. Close window C. Expected within about six seconds: terminal A logs `encoders = []` as the idle encoder stops.
6. Ctrl-C terminal B. Expected: window D shows `Ended` or `publisher left the room` within the twenty second expiry; terminal A logs `members = 0`.
7. Restart without `--no-relay` on two machines if available and repeat steps 1 to 3.

- [ ] **Step 6: Commit**

```bash
git add crates/app crates/room/src/room.rs
git commit -m "feat: run publish and watch as room participants"
```

### Task 13: README for rooms

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Update usage**

Replace the Usage section of `README.md` with:

````markdown
## Usage

```
cargo build --release

# Create a room, share one monitor, print the ticket
./target/release/brp publish --nickname alice [--fps 60] [--bitrate-kbps N] [--codec hevc|h264|av1] [--source monitor|window] [--no-relay]

# Join that room and share too
./target/release/brp publish --nickname bob --ticket <ticket>

# Watch the ticket's publisher (the full participant window arrives in the next slice)
./target/release/brp watch <ticket> [--nickname N] [--no-relay]
```

A ticket names the room and one member already in it. Anyone in the room can hand out a ticket.
Media connections are accepted only from room members. Encoders run only while someone watches.

`--no-relay` skips the public relay servers; use it on a LAN.
````

Add to the Development section:

```
cargo test -p brp-room              # two rooms in one process, fake codecs
```

- [ ] **Step 2: Commit**

```bash
git add README.md
git commit -m "docs: describe rooms in the README"
```

## Plan self-review notes

Spec coverage: membership and presence (Tasks 5, 7, 10); bootstrap through the in-memory lookup (Task 6); membership gating with the refusal code (Tasks 6, 10); lazy encoders, capture fan, housekeeping (Task 9); templates and preset validation (Tasks 5, 9); watcher with backoff, expiry, and Source fallback (Task 11); snapshot and version counter (Tasks 9, 10, 11); headless commands (Task 12); constants of spec section 9 (Task 5); tests of spec section 10 minus the manual two-machine check, which is Task 12 step 5 in reduced form and plan 2b's final check in full.

Deliberate scope notes: `MemberView.path` reads the media connection when one exists and reports Unknown otherwise; gossip-only peers therefore show Unknown until watched, which the window in 2b labels as such. The `watch` command's status poller is a bridge until 2b renders snapshots. The single unverified library call is `PathList` iteration in Task 6.

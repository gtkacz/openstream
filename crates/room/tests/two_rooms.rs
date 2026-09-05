use std::sync::Arc;
use std::time::{Duration, Instant};

use brp_capture::{CaptureBackend, FrameSink, SourceRequest, StartFuture, SyntheticSource};
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
        capture: Arc::new(SyntheticSource {
            width: 64,
            height: 32,
            fps: 30,
        }),
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

    wait_until("mutual presence", Duration::from_secs(5), || {
        a.snapshot().members.len() == 1 && b.snapshot().members.len() == 1
    })
    .await;
    assert_eq!(a.snapshot().members[0].nickname, "bob");
    assert_eq!(b.snapshot().members[0].nickname, "alice");
    assert_eq!(b.snapshot().members[0].id, a.id());
    assert!(a.version() > 0);

    let live = a
        .start_live(SourceKind::Monitor, "desk".into())
        .await
        .unwrap();
    wait_until("catalog", Duration::from_secs(5), || {
        b.snapshot().members[0]
            .lives
            .iter()
            .any(|l| l.id == live && l.title == "desk")
    })
    .await;
    let seen = b.snapshot().members[0].lives[0].clone();
    assert_eq!(
        (seen.source_width, seen.source_height, seen.presets.len()),
        (64, 32, 1)
    );
    assert!(
        a.snapshot().own_lives[0].presets[0].encoder.is_none(),
        "nobody watches yet"
    );

    a.stop_live(live).unwrap();
    wait_until("live removed", Duration::from_secs(5), || {
        b.snapshot().members[0].lives.is_empty()
    })
    .await;

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

use brp_net::{MediaClient, RelaySetting as Relay, bind_endpoint};
use brp_proto::constants::{MAX_LIVES_PER_PARTICIPANT, SOURCE_PRESET_ID};
use brp_proto::{Codec, Preset};
use brp_room::WatchState;
use std::sync::atomic::{AtomicUsize, Ordering};

async fn joined_pair() -> (Room, Room) {
    let a = Room::create(config("alice")).await.unwrap();
    let b = Room::join(config("bob"), a.ticket()).await.unwrap();
    wait_until("mutual presence", Duration::from_secs(5), || {
        a.snapshot().members.len() == 1 && b.snapshot().members.len() == 1
    })
    .await;
    (a, b)
}

#[tokio::test]
async fn watching_starts_the_encoder_and_unwatching_stops_it_after_the_grace() {
    let (a, b) = joined_pair().await;
    let live = a
        .start_live(SourceKind::Monitor, "desk".into())
        .await
        .unwrap();
    wait_until("catalog", Duration::from_secs(5), || {
        !b.snapshot().members[0].lives.is_empty()
    })
    .await;

    let handle = b.watch(a.id(), live, SOURCE_PRESET_ID).unwrap();
    wait_until("decoded frames", Duration::from_secs(5), || {
        handle.stats.frames_decoded.load(Ordering::Relaxed) >= 3
    })
    .await;
    let frame = handle
        .slot
        .try_take()
        .expect("a frame is waiting for the renderer");
    assert_eq!((frame.width, frame.height), (64, 32));
    assert_eq!(b.snapshot().watches[0].state, WatchState::Live);
    let encoder = a.snapshot().own_lives[0].presets[0]
        .encoder
        .clone()
        .expect("encoder started for the watcher");
    assert_eq!(encoder.subscribers, 1);
    assert_ne!(b.snapshot().members[0].path, brp_net::PathKind::Unknown);

    b.unwatch(a.id(), live).unwrap();
    assert!(b.snapshot().watches.is_empty());
    wait_until("encoder stopped", Duration::from_secs(5), || {
        a.snapshot().own_lives[0].presets[0].encoder.is_none()
    })
    .await;

    b.leave().await;
    a.leave().await;
}

#[tokio::test]
async fn a_stranger_is_refused_by_the_media_server() {
    let a = Room::create(config("alice")).await.unwrap();
    let stranger = bind_endpoint(SecretKey::generate(), Relay::Disabled, vec![])
        .await
        .unwrap();
    let client = MediaClient::connect(&stranger, a.ticket().bootstrap[0].clone())
        .await
        .unwrap();
    let refused = tokio::time::timeout(
        Duration::from_secs(5),
        client.subscribe(1, SOURCE_PRESET_ID),
    )
    .await
    .expect("refusal is prompt");
    assert!(refused.is_err());
    stranger.close().await;
    a.leave().await;
}

#[tokio::test]
async fn stopping_the_live_ends_the_watch_and_leaving_expires_the_member() {
    let (a, b) = joined_pair().await;
    let live = a
        .start_live(SourceKind::Window, "game".into())
        .await
        .unwrap();
    wait_until("catalog", Duration::from_secs(5), || {
        !b.snapshot().members[0].lives.is_empty()
    })
    .await;
    let handle = b.watch(a.id(), live, SOURCE_PRESET_ID).unwrap();
    wait_until("live", Duration::from_secs(5), || {
        handle.stats.frames_decoded.load(Ordering::Relaxed) >= 1
    })
    .await;

    a.stop_live(live).unwrap();
    wait_until("watch ended", Duration::from_secs(5), || {
        b.snapshot()
            .watches
            .first()
            .is_some_and(|w| w.state == WatchState::Ended)
    })
    .await;

    a.leave().await;
    wait_until("member expired", Duration::from_secs(5), || {
        b.snapshot().members.is_empty() && b.snapshot().watches.is_empty()
    })
    .await;
    b.leave().await;
}

#[tokio::test]
async fn preset_changes_propagate_and_a_removed_preset_falls_back_to_source() {
    let (a, b) = joined_pair().await;
    let live = a
        .start_live(SourceKind::Monitor, "desk".into())
        .await
        .unwrap();
    let mut presets = a.snapshot().own_lives[0].info.presets.clone();
    presets.push(Preset {
        id: 2,
        name: "tiny".into(),
        width: 32,
        height: 16,
        fps: 30,
        bitrate_kbps: 1_000,
        codec: Codec::H264,
    });
    a.set_presets(live, presets.clone()).unwrap();
    wait_until("two presets", Duration::from_secs(5), || {
        b.snapshot().members[0]
            .lives
            .first()
            .is_some_and(|l| l.presets.len() == 2)
    })
    .await;

    let handle = b.watch(a.id(), live, 2).unwrap();
    wait_until("tiny frames", Duration::from_secs(5), || {
        handle.stats.frames_decoded.load(Ordering::Relaxed) >= 1
    })
    .await;
    assert_eq!(
        handle.slot.try_take().map(|f| (f.width, f.height)),
        Some((32, 16))
    );

    a.set_presets(live, presets[..1].to_vec()).unwrap();
    wait_until("fallback to source", Duration::from_secs(10), || {
        b.snapshot()
            .watches
            .first()
            .is_some_and(|w| w.preset_id == SOURCE_PRESET_ID && w.state == WatchState::Live)
    })
    .await;
    wait_until("source frames", Duration::from_secs(5), || {
        handle.slot.try_take().is_some_and(|f| f.width == 64)
    })
    .await;

    b.leave().await;
    a.leave().await;
}

/// Counts capture sessions actually opened, so the test can prove the ninth `start_live` never
/// touches capture (which for real users is a desktop portal permission dialog).
struct CountingCapture {
    opened: Arc<AtomicUsize>,
    inner: SyntheticSource,
}

impl CaptureBackend for CountingCapture {
    fn start(&self, request: SourceRequest, sink: FrameSink) -> StartFuture<'_> {
        self.opened.fetch_add(1, Ordering::SeqCst);
        self.inner.start(request, sink)
    }
}

#[tokio::test]
async fn the_ninth_live_is_refused_before_capture_opens_a_session() {
    let opened = Arc::new(AtomicUsize::new(0));
    let mut cfg = config("alice");
    cfg.capture = Arc::new(CountingCapture {
        opened: opened.clone(),
        inner: SyntheticSource {
            width: 64,
            height: 32,
            fps: 30,
        },
    });
    let room = Room::create(cfg).await.unwrap();

    for i in 0..MAX_LIVES_PER_PARTICIPANT {
        room.start_live(SourceKind::Monitor, format!("l{i}"))
            .await
            .unwrap();
    }
    let refused = room
        .start_live(SourceKind::Monitor, "one too many".into())
        .await;
    assert!(matches!(refused, Err(brp_room::RoomError::TooManyLives)));
    assert_eq!(opened.load(Ordering::SeqCst), MAX_LIVES_PER_PARTICIPANT);

    room.leave().await;
}

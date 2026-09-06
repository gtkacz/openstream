use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use brp_audio::{FakeOutput, FakeOutputHandle, SyntheticTone};
use brp_capture::{
    CaptureBackend, CaptureError, FrameSink, SourceDescriptor, SourceId, SourceListing,
    SourceRequest, StartFuture, SyntheticSource,
};
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
        audio_capture: Arc::new(SyntheticTone {
            frequency_hz: 440.0,
            amplitude: 0.5,
        }),
        audio_output: Arc::new(FakeOutput::new().0),
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
        .start_live(SourceKind::Monitor, None, "desk".into())
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
        .start_live(SourceKind::Monitor, None, "desk".into())
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
        client.subscribe(1, SOURCE_PRESET_ID, false),
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
        .start_live(SourceKind::Window, None, "game".into())
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
        .start_live(SourceKind::Monitor, None, "desk".into())
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
        room.start_live(SourceKind::Monitor, None, format!("l{i}"))
            .await
            .unwrap();
    }
    let refused = room
        .start_live(SourceKind::Monitor, None, "one too many".into())
        .await;
    assert!(matches!(refused, Err(brp_room::RoomError::TooManyLives)));
    assert_eq!(opened.load(Ordering::SeqCst), MAX_LIVES_PER_PARTICIPANT);

    room.leave().await;
}

/// Records the request it was started with and answers `sources` with one fixed choice, so the
/// tests can see what the room passes through without a real platform.
struct RecordingCapture {
    seen: Arc<Mutex<Option<SourceRequest>>>,
    inner: SyntheticSource,
}

impl CaptureBackend for RecordingCapture {
    fn sources(&self, kind: SourceKind) -> Result<SourceListing, CaptureError> {
        Ok(SourceListing::Choices(vec![SourceDescriptor {
            id: SourceId(42),
            kind,
            name: "Fake display".into(),
            width: 64,
            height: 32,
        }]))
    }

    fn start(&self, request: SourceRequest, sink: FrameSink) -> StartFuture<'_> {
        *self.seen.lock().unwrap() = Some(request);
        self.inner.start(request, sink)
    }
}

fn recording_config(seen: Arc<Mutex<Option<SourceRequest>>>) -> RoomConfig {
    let mut cfg = config("alice");
    cfg.capture = Arc::new(RecordingCapture {
        seen,
        inner: SyntheticSource {
            width: 64,
            height: 32,
            fps: 30,
        },
    });
    cfg
}

#[tokio::test]
async fn start_live_passes_the_source_id_into_the_capture_request() {
    let seen = Arc::new(Mutex::new(None));
    let room = Room::create(recording_config(seen.clone())).await.unwrap();

    room.start_live(SourceKind::Window, Some(SourceId(42)), "game".into())
        .await
        .unwrap();

    let request = (*seen.lock().unwrap()).expect("capture was started");
    assert_eq!(request.kind, SourceKind::Window);
    assert_eq!(request.source, Some(SourceId(42)));
    assert_eq!(request.target_fps, 30);
    room.leave().await;
}

#[tokio::test]
async fn sources_passes_the_backend_listing_through() {
    let room = Room::create(recording_config(Arc::default()))
        .await
        .unwrap();

    let listing = room.sources(SourceKind::Monitor).unwrap();

    match listing {
        SourceListing::Choices(choices) => {
            assert_eq!(choices.len(), 1);
            assert_eq!(choices[0].id, SourceId(42));
            assert_eq!(choices[0].kind, SourceKind::Monitor);
        }
        SourceListing::PlatformPicker => panic!("the fake lists a choice"),
    }
    room.leave().await;
}

fn config_with_output(nickname: &str) -> (RoomConfig, FakeOutputHandle) {
    let (output, handle) = FakeOutput::new();
    let mut cfg = config(nickname);
    cfg.audio_output = Arc::new(output);
    (cfg, handle)
}

fn is_audible(samples: &[f32]) -> bool {
    samples.iter().any(|s| s.abs() > 0.05)
}

#[tokio::test]
async fn a_watch_carries_audio_to_the_output_and_a_second_watch_of_the_same_publisher_does_not() {
    let a = Room::create(config("alice")).await.unwrap();
    let (bob_cfg, output) = config_with_output("bob");
    let b = Room::join(bob_cfg, a.ticket()).await.unwrap();
    wait_until("mutual presence", Duration::from_secs(5), || {
        a.snapshot().members.len() == 1 && b.snapshot().members.len() == 1
    })
    .await;
    let desk = a
        .start_live(SourceKind::Monitor, None, "desk".into())
        .await
        .unwrap();
    let game = a
        .start_live(SourceKind::Window, None, "game".into())
        .await
        .unwrap();
    wait_until("catalog with audio", Duration::from_secs(5), || {
        let members = b.snapshot().members;
        members[0].lives.len() == 2 && members[0].has_audio
    })
    .await;

    b.watch(a.id(), desk, SOURCE_PRESET_ID).unwrap();
    b.watch(a.id(), game, SOURCE_PRESET_ID).unwrap();
    wait_until("both live", Duration::from_secs(5), || {
        b.snapshot()
            .watches
            .iter()
            .filter(|w| w.state == WatchState::Live)
            .count()
            == 2
    })
    .await;
    let carriers: Vec<u32> = b
        .snapshot()
        .watches
        .iter()
        .filter(|w| w.audio)
        .map(|w| w.live_id)
        .collect();
    assert_eq!(carriers, vec![desk], "only the first watch carries audio");
    assert_eq!(a.snapshot().own_audio.subscribers, 1);
    wait_until("audible output", Duration::from_secs(5), || {
        is_audible(&output.render(1024))
    })
    .await;

    b.unwatch(a.id(), desk).unwrap();
    wait_until(
        "audio moved to the game tile",
        Duration::from_secs(10),
        || {
            b.snapshot()
                .watches
                .iter()
                .any(|w| w.live_id == game && w.audio && w.state == WatchState::Live)
        },
    )
    .await;
    wait_until("still audible", Duration::from_secs(5), || {
        is_audible(&output.render(1024))
    })
    .await;

    b.set_volume(a.id(), 0.0);
    assert_eq!(b.snapshot().members[0].gain, 0.0);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        !is_audible(&output.render(1024)),
        "gain zero silences the publisher"
    );

    b.leave().await;
    a.leave().await;
}

#[tokio::test]
async fn turning_share_audio_off_ends_the_packets_and_the_flag_in_presence() {
    let a = Room::create(config("alice")).await.unwrap();
    let (bob_cfg, output) = config_with_output("bob");
    let b = Room::join(bob_cfg, a.ticket()).await.unwrap();
    wait_until("mutual presence", Duration::from_secs(5), || {
        a.snapshot().members.len() == 1 && b.snapshot().members.len() == 1
    })
    .await;
    let live = a
        .start_live(SourceKind::Monitor, None, "desk".into())
        .await
        .unwrap();
    wait_until("catalog", Duration::from_secs(5), || {
        b.snapshot().members[0].lives.len() == 1
    })
    .await;
    b.watch(a.id(), live, SOURCE_PRESET_ID).unwrap();
    wait_until("audible", Duration::from_secs(5), || {
        is_audible(&output.render(1024))
    })
    .await;

    a.set_audio(false);
    wait_until("flag cleared", Duration::from_secs(5), || {
        !b.snapshot().members[0].has_audio && a.snapshot().own_audio.subscribers == 0
    })
    .await;
    // The jitter buffer can still hold real packets right when the flag clears, so wait for
    // them to drain rather than assuming a fixed render count empties it.
    wait_until("silence", Duration::from_secs(5), || {
        !is_audible(&output.render(1024))
    })
    .await;
    let renders: Vec<bool> = {
        let mut results = Vec::with_capacity(10);
        for _ in 0..10 {
            results.push(is_audible(&output.render(1024)));
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        results
    };
    assert!(
        renders.iter().all(|audible| !audible),
        "expected 10 consecutive silent renders, got {renders:?}"
    );

    b.leave().await;
    a.leave().await;
}

/// Spec 9 sells the share-audio toggle as the retry path: turning it off must end the packets and
/// turning it back on must get the carrier resubscribed without touching the viewer.
#[tokio::test]
async fn toggling_share_audio_off_and_on_gets_the_carrier_back() {
    let a = Room::create(config("alice")).await.unwrap();
    let (bob_cfg, output) = config_with_output("bob");
    let b = Room::join(bob_cfg, a.ticket()).await.unwrap();
    wait_until("mutual presence", Duration::from_secs(5), || {
        a.snapshot().members.len() == 1 && b.snapshot().members.len() == 1
    })
    .await;
    let live = a
        .start_live(SourceKind::Monitor, None, "desk".into())
        .await
        .unwrap();
    wait_until("catalog", Duration::from_secs(5), || {
        b.snapshot().members[0].lives.len() == 1
    })
    .await;
    b.watch(a.id(), live, SOURCE_PRESET_ID).unwrap();
    wait_until("audible", Duration::from_secs(5), || {
        is_audible(&output.render(1024))
    })
    .await;

    a.set_audio(false);
    wait_until(
        "the carrier lost its audio",
        Duration::from_secs(10),
        || !b.snapshot().members[0].has_audio && b.snapshot().watches.iter().all(|w| !w.audio),
    )
    .await;
    wait_until("silence", Duration::from_secs(5), || {
        !is_audible(&output.render(1024))
    })
    .await;

    a.set_audio(true);
    wait_until("the carrier came back", Duration::from_secs(10), || {
        b.snapshot()
            .watches
            .iter()
            .any(|w| w.audio && w.state == WatchState::Live)
    })
    .await;
    wait_until("audible again", Duration::from_secs(10), || {
        is_audible(&output.render(1024))
    })
    .await;

    b.leave().await;
    a.leave().await;
}

#[tokio::test]
async fn master_mute_and_a_broken_output_are_reported_in_the_snapshot() {
    struct BrokenOutput;
    impl brp_audio::AudioOutput for BrokenOutput {
        fn start(
            &self,
            _render: brp_audio::RenderFn,
        ) -> Result<Box<dyn brp_audio::AudioOutputSession>, brp_audio::AudioError> {
            Err(brp_audio::AudioError::Device("no sound card".into()))
        }
    }
    let mut cfg = config("alice");
    cfg.audio_output = Arc::new(BrokenOutput);
    let a = Room::create(cfg).await.unwrap();
    assert_eq!(
        a.snapshot().audio_output_error.as_deref(),
        Some("audio device: no sound card")
    );
    assert!(!a.snapshot().master_mute);
    a.set_master_mute(true);
    assert!(a.snapshot().master_mute);
    a.leave().await;
}

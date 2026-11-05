//! Publishers that go quiet. A dead one stops its runtime, so, as with `kill -9` or a cut cable, no
//! packet ever says goodbye. A still one stays alive and simply has nothing new to send.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use brp_audio::{AudioSelection, FakeOutput, SyntheticTone};
use brp_capture::{CaptureBackend, FrameSink, SourceRequest, StartFuture, SyntheticSource};
use brp_net::RelaySetting;
use brp_proto::SourceKind;
use brp_proto::constants::{MEDIA_IDLE_TIMEOUT, SIGNAL_LOST_AFTER, SOURCE_PRESET_ID};
use brp_room::codecs::fake::FakeCodecs;
use brp_room::{Room, RoomConfig, RoomTimings, WatchState};
use iroh::SecretKey;

const SOURCE: SyntheticSource = SyntheticSource {
    width: 64,
    height: 32,
    fps: 30,
};

fn config(nickname: &str, capture: Arc<dyn CaptureBackend>) -> RoomConfig {
    RoomConfig {
        secret: SecretKey::generate(),
        relay: RelaySetting::Disabled,
        nickname: nickname.into(),
        target_fps: 30,
        capture,
        audio_capture: Arc::new(SyntheticTone {
            frequency_hz: 440.0,
            amplitude: 0.5,
        }),
        audio_output: Arc::new(FakeOutput::new().0),
        audio_applications: AudioSelection::All,
        encoders: Arc::new(FakeCodecs),
        decoders: Arc::new(FakeCodecs),
        on_change: Arc::new(|| {}),
        on_frame: Arc::new(|_publisher, _live_id| {}),
        timings: RoomTimings {
            heartbeat: Duration::from_millis(200),
            // The real value, so these measure the order things happen in for real users.
            expiry: brp_proto::constants::MEMBER_EXPIRY,
            housekeeping: Duration::from_millis(100),
            encoder_grace: Duration::from_millis(300),
            join_timeout: Duration::from_secs(5),
        },
    }
}

async fn wait_until(what: &str, timeout: Duration, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Delivers frames until told to stop, then nothing, as a compositor does for a screen nobody touches.
struct StillScreen(Arc<AtomicBool>);

impl CaptureBackend for StillScreen {
    fn start(&self, request: SourceRequest, mut sink: FrameSink) -> StartFuture<'_> {
        let still = self.0.clone();
        SOURCE.start(
            request,
            Box::new(move |frame| {
                if !still.load(Ordering::Relaxed) {
                    sink(frame);
                }
            }),
        )
    }
}

fn state_of(room: &Room) -> WatchState {
    room.snapshot().watches[0].state
}

#[tokio::test]
async fn a_dead_publisher_loses_its_signal_then_reconnects_before_it_expires() {
    let publisher = tokio::runtime::Runtime::new().unwrap();
    let source = Arc::new(SOURCE);
    let (a, live, ticket) = publisher
        .spawn(async move {
            let a = Room::create(config("alice", source)).await.unwrap();
            let live = a
                .start_live(SourceKind::Monitor, None, "desk".into())
                .await
                .unwrap();
            let ticket = a.ticket();
            (a, live, ticket)
        })
        .await
        .unwrap();
    let a_id = a.id();

    let b = Room::join(config("bob", Arc::new(SOURCE)), ticket)
        .await
        .unwrap();
    wait_until("the catalog", Duration::from_secs(10), || {
        b.snapshot()
            .members
            .first()
            .is_some_and(|m| !m.lives.is_empty())
    })
    .await;
    let handle = b.watch(a_id, live, SOURCE_PRESET_ID).unwrap();
    wait_until("decoded frames", Duration::from_secs(5), || {
        handle.stats.frames_decoded.load(Ordering::Relaxed) >= 3
    })
    .await;
    assert_eq!(state_of(&b), WatchState::Live);

    let died = Instant::now();
    publisher.shutdown_background();
    std::mem::forget(a);

    wait_until(
        "the signal to read as lost",
        SIGNAL_LOST_AFTER + Duration::from_secs(1),
        || state_of(&b) == WatchState::SignalLost,
    )
    .await;
    let lost = died.elapsed();
    assert!(
        lost >= SIGNAL_LOST_AFTER - Duration::from_secs(1),
        "signal read as lost after only {lost:?}"
    );

    wait_until(
        "the watch to reconnect",
        MEDIA_IDLE_TIMEOUT + Duration::from_secs(2),
        || state_of(&b) == WatchState::Reconnecting,
    )
    .await;
    assert!(
        b.snapshot().members.iter().any(|m| m.id == a_id),
        "the publisher must still be a member when the watch reaches Reconnecting"
    );

    b.leave().await;
}

#[tokio::test]
async fn a_still_screen_is_not_mistaken_for_a_dead_publisher() {
    let still = Arc::new(AtomicBool::new(false));
    let a = Room::create(config("alice", Arc::new(StillScreen(still.clone()))))
        .await
        .unwrap();
    let live = a
        .start_live(SourceKind::Monitor, None, "desk".into())
        .await
        .unwrap();
    let b = Room::join(config("bob", Arc::new(SOURCE)), a.ticket())
        .await
        .unwrap();
    wait_until("the catalog", Duration::from_secs(10), || {
        b.snapshot()
            .members
            .first()
            .is_some_and(|m| !m.lives.is_empty())
    })
    .await;
    let handle = b.watch(a.id(), live, SOURCE_PRESET_ID).unwrap();
    wait_until("first frames", Duration::from_secs(5), || {
        handle.stats.frames_decoded.load(Ordering::Relaxed) >= 3
    })
    .await;
    still.store(true, Ordering::Relaxed);

    let until = Instant::now() + MEDIA_IDLE_TIMEOUT + Duration::from_secs(2);
    while Instant::now() < until {
        assert_eq!(state_of(&b), WatchState::Live);
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    a.leave().await;
    b.leave().await;
}

#[tokio::test]
async fn a_member_nobody_watches_still_shows_their_delay_and_their_loss() {
    let publisher = tokio::runtime::Runtime::new().unwrap();
    let source = Arc::new(SOURCE);
    let (a, ticket) = publisher
        .spawn(async move {
            let a = Room::create(config("alice", source)).await.unwrap();
            let ticket = a.ticket();
            (a, ticket)
        })
        .await
        .unwrap();

    let b = Room::join(config("bob", Arc::new(SOURCE)), ticket)
        .await
        .unwrap();
    wait_until(
        "a round trip to the member",
        Duration::from_secs(10),
        || {
            b.snapshot()
                .members
                .first()
                .is_some_and(|m| m.rtt_ms.is_some())
        },
    )
    .await;
    assert!(!b.snapshot().members[0].signal_lost);
    assert!(b.snapshot().watches.is_empty());

    publisher.shutdown_background();
    std::mem::forget(a);

    wait_until(
        "the member's signal to read as lost",
        SIGNAL_LOST_AFTER + Duration::from_secs(1),
        || b.snapshot().members[0].signal_lost,
    )
    .await;

    b.leave().await;
}

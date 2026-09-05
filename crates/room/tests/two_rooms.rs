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

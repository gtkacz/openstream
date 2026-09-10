//! Probe: a peer that leaves and comes straight back, the way the in-app update relaunches it.

use std::sync::Arc;
use std::time::{Duration, Instant};

use brp_audio::{FakeOutput, SyntheticTone};
use brp_capture::SyntheticSource;
use brp_net::RelaySetting;
use brp_proto::RoomTicket;
use brp_room::codecs::fake::FakeCodecs;
use brp_room::{Room, RoomConfig, RoomTimings};
use iroh::SecretKey;

const JOIN_TIMEOUT: Duration = Duration::from_secs(5);

fn config(nickname: &str, secret: SecretKey) -> RoomConfig {
    RoomConfig {
        secret,
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
        audio_applications: brp_audio::AudioSelection::All,
        encoders: Arc::new(FakeCodecs),
        decoders: Arc::new(FakeCodecs),
        on_change: Arc::new(|| {}),
        on_frame: Arc::new(|_publisher, _live_id| {}),
        timings: RoomTimings {
            heartbeat: Duration::from_millis(200),
            expiry: Duration::from_secs(1),
            housekeeping: Duration::from_millis(100),
            encoder_grace: Duration::from_millis(300),
            join_timeout: JOIN_TIMEOUT,
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

/// Spec 15: the relaunch after an in-room update carries `rejoin_ticket()`. `ticket()` names the
/// process that is leaving, so the new one would wait out the join timeout and land on the start
/// screen instead of back in the room.
#[tokio::test]
async fn a_rejoin_ticket_bootstraps_through_the_other_members() {
    let secret = SecretKey::generate();
    let a = Room::create(config("alice", secret.clone())).await.unwrap();
    let b = Room::join(config("bob", SecretKey::generate()), a.ticket())
        .await
        .unwrap();
    wait_until("mutual presence", Duration::from_secs(10), || {
        a.snapshot().members.len() == 1 && b.snapshot().members.len() == 1
    })
    .await;

    let rejoin = a.rejoin_ticket().await;
    assert_eq!(
        rejoin
            .bootstrap
            .iter()
            .map(|addr| addr.id)
            .collect::<Vec<_>>(),
        vec![b.id()],
        "the members that stay are the way back in"
    );
    a.leave().await;

    let a2 = Room::join(config("alice", secret), rejoin).await.unwrap();

    wait_until("alice sees bob again", Duration::from_secs(10), || {
        a2.snapshot().members.iter().any(|m| m.nickname == "bob")
    })
    .await;
    a2.leave().await;
    b.leave().await;
}

/// Alone in the room the rejoin ticket has nobody to name but this node. It must still encode,
/// since the relaunched process parses it back, and joining it must not wait out the timeout for
/// a neighbour that could only be ourselves.
#[tokio::test]
async fn rejoining_an_empty_room_keeps_the_topic_and_does_not_wait() {
    let secret = SecretKey::generate();
    let a = Room::create(config("alice", secret.clone())).await.unwrap();
    let topic = a.ticket().topic;

    let rejoin: RoomTicket = a
        .rejoin_ticket()
        .await
        .to_string()
        .parse()
        .expect("the relaunched process parses the ticket back");
    a.leave().await;

    let started = Instant::now();
    let a2 = Room::join(config("alice", secret), rejoin).await.unwrap();

    assert_eq!(a2.ticket().topic, topic, "the room is the same room");
    assert!(started.elapsed() < JOIN_TIMEOUT, "{:?}", started.elapsed());
    a2.leave().await;
}

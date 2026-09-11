//! Probe: three peers bootstrapped through the creator, then the creator leaves.

use std::sync::Arc;
use std::time::{Duration, Instant};

use brp_audio::{FakeOutput, SyntheticTone};
use brp_capture::SyntheticSource;
use brp_net::RelaySetting;
use brp_room::codecs::fake::FakeCodecs;
use brp_room::{Room, RoomConfig, RoomTimings};
use iroh::SecretKey;

fn config(nickname: &str) -> RoomConfig {
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

#[tokio::test]
async fn the_others_keep_seeing_each_other_after_the_creator_leaves() {
    let a = Room::create(config("alice")).await.unwrap();
    let b = Room::join(config("bob"), a.ticket()).await.unwrap();
    let c = Room::join(config("carol"), a.ticket()).await.unwrap();

    wait_until("everyone sees everyone", Duration::from_secs(10), || {
        a.snapshot().members.len() == 2
            && b.snapshot().members.len() == 2
            && c.snapshot().members.len() == 2
    })
    .await;

    let a_id = a.id();
    a.leave().await;

    wait_until("the creator expires", Duration::from_secs(10), || {
        !b.snapshot().members.iter().any(|m| m.id == a_id)
            && !c.snapshot().members.iter().any(|m| m.id == a_id)
    })
    .await;

    // Well past one expiry: presence must still be flowing between the survivors.
    tokio::time::sleep(Duration::from_secs(3)).await;
    let b_members: Vec<String> = b
        .snapshot()
        .members
        .iter()
        .map(|m| m.nickname.clone())
        .collect();
    let c_members: Vec<String> = c
        .snapshot()
        .members
        .iter()
        .map(|m| m.nickname.clone())
        .collect();
    assert_eq!(b_members, vec!["carol".to_string()], "bob lost carol");
    assert_eq!(c_members, vec!["bob".to_string()], "carol lost bob");

    b.leave().await;
    c.leave().await;
}

#[tokio::test]
async fn a_survivor_can_still_invite_after_the_creator_leaves() {
    let a = Room::create(config("alice")).await.unwrap();
    let b = Room::join(config("bob"), a.ticket()).await.unwrap();
    wait_until("mutual presence", Duration::from_secs(10), || {
        a.snapshot().members.len() == 1 && b.snapshot().members.len() == 1
    })
    .await;
    a.leave().await;

    let d = Room::join(config("dave"), b.ticket()).await.unwrap();
    wait_until("dave joins through bob", Duration::from_secs(10), || {
        b.snapshot().members.iter().any(|m| m.nickname == "dave")
            && d.snapshot().members.iter().any(|m| m.nickname == "bob")
    })
    .await;

    d.leave().await;
    b.leave().await;
}

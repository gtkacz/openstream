use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use brp_audio::{
    AudioError, AudioOutput, AudioOutputSession, AudioSelection, CpalInput, FakeOutput,
    FakeOutputHandle, RenderFn, SyntheticTone, VoiceCapture,
};
use brp_capture::SyntheticSource;
use brp_net::RelaySetting;
use brp_proto::VoiceState;
use brp_room::codecs::fake::FakeCodecs;
use brp_room::{Room, RoomConfig, RoomTimings};
use iroh::SecretKey;

const WAIT: Duration = Duration::from_secs(15);

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
        audio_applications: AudioSelection::All,
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

fn is_audible(samples: &[f32]) -> bool {
    samples.iter().any(|s| s.abs() > 0.05)
}

fn mic() -> Arc<dyn VoiceCapture> {
    Arc::new(SyntheticTone {
        frequency_hz: 440.0,
        amplitude: 0.5,
    })
}

/// A voice output of its own per person, as the app gives each room one.
fn voice_output() -> (Arc<dyn AudioOutput>, FakeOutputHandle) {
    let (output, handle) = FakeOutput::new();
    (Arc::new(output), handle)
}

struct Person {
    room: Room,
    voice: FakeOutputHandle,
}

/// Rooms of the given names, all seeing one another, none yet in voice.
async fn room_of(names: &[&str]) -> Vec<Person> {
    let first = Room::create(config(names[0])).await.unwrap();
    let ticket = first.ticket();
    let mut rooms = vec![first];
    for name in &names[1..] {
        rooms.push(Room::join(config(name), ticket.clone()).await.unwrap());
    }
    let count = rooms.len() - 1;
    wait_until("mutual presence", WAIT, || {
        rooms.iter().all(|r| r.snapshot().members.len() == count)
    })
    .await;
    rooms
        .into_iter()
        .map(|room| Person {
            room,
            voice: voice_output().1,
        })
        .collect()
}

fn join(person: &mut Person) {
    let (output, handle) = voice_output();
    person.room.join_voice(mic(), output).unwrap();
    person.voice = handle;
}

fn voice_of(viewer: &Room, id: iroh::PublicKey) -> Option<VoiceState> {
    viewer
        .snapshot()
        .members
        .iter()
        .find(|m| m.id == id)
        .and_then(|m| m.voice)
}

async fn hears(listener: &Person) {
    wait_until("audible voice", WAIT, || {
        is_audible(&listener.voice.render(960))
    })
    .await;
}

async fn goes_silent(listener: &Person) {
    wait_until("silent voice", WAIT, || {
        !is_audible(&listener.voice.render(960))
    })
    .await;
}

#[tokio::test]
async fn voice_reaches_the_other_person_in_voice() {
    let mut people = room_of(&["alice", "bob"]).await;
    join(&mut people[0]);
    join(&mut people[1]);
    hears(&people[1]).await;
    hears(&people[0]).await;
}

#[tokio::test]
async fn being_in_the_room_is_not_being_in_voice() {
    let mut people = room_of(&["alice", "bob", "carol"]).await;
    join(&mut people[0]);
    join(&mut people[1]);
    let (a, b) = (people[0].room.id(), people[1].room.id());
    let carol = &people[2].room;
    wait_until("both in voice, as carol sees it", WAIT, || {
        voice_of(carol, a).is_some() && voice_of(carol, b).is_some()
    })
    .await;
    assert!(!carol.snapshot().voice.joined);
}

#[tokio::test]
async fn muting_closes_the_microphone_and_the_room_sees_it() {
    let mut people = room_of(&["alice", "bob"]).await;
    join(&mut people[0]);
    join(&mut people[1]);
    hears(&people[1]).await;

    people[0].room.set_voice_muted(true);

    let view = people[0].room.snapshot().voice;
    assert!(view.joined && view.muted && !view.transmitting);
    let a = people[0].room.id();
    wait_until("bob sees alice muted", WAIT, || {
        voice_of(&people[1].room, a)
            == Some(VoiceState {
                muted: true,
                deafened: false,
            })
    })
    .await;
    goes_silent(&people[1]).await;
}

#[tokio::test]
async fn deafening_silences_voice_and_undeafening_restores_the_mute() {
    let mut people = room_of(&["alice", "bob"]).await;
    join(&mut people[0]);
    join(&mut people[1]);
    hears(&people[1]).await;
    let (a, b) = (people[0].room.id(), people[1].room.id());

    people[1].room.set_voice_muted(true);
    people[1].room.set_voice_deafened(true);
    let view = people[1].room.snapshot().voice;
    assert!(view.muted && view.deafened && !view.transmitting);
    wait_until("alice sees bob deafened", WAIT, || {
        voice_of(&people[0].room, b).is_some_and(|v| v.deafened)
    })
    .await;
    goes_silent(&people[1]).await;

    people[1].room.set_voice_deafened(false);
    let view = people[1].room.snapshot().voice;
    assert!(view.muted && !view.deafened, "the earlier mute comes back");
    wait_until("alice sees bob hearing again", WAIT, || {
        voice_of(&people[0].room, b).is_some_and(|v| !v.deafened)
    })
    .await;
    hears(&people[1]).await;
    assert!(voice_of(&people[1].room, a).is_some());
}

#[tokio::test]
async fn leaving_voice_keeps_you_in_the_room() {
    let mut people = room_of(&["alice", "bob"]).await;
    join(&mut people[0]);
    join(&mut people[1]);
    let b = people[1].room.id();
    wait_until("alice sees bob in voice", WAIT, || {
        voice_of(&people[0].room, b).is_some()
    })
    .await;

    people[1].room.leave_voice();

    assert!(!people[1].room.snapshot().voice.joined);
    wait_until("alice sees bob out of voice", WAIT, || {
        voice_of(&people[0].room, b).is_none()
    })
    .await;
    assert!(people[0].room.snapshot().members.iter().any(|m| m.id == b));
}

#[tokio::test]
async fn the_master_mute_leaves_voice_audible() {
    let mut people = room_of(&["alice", "bob"]).await;
    join(&mut people[0]);
    join(&mut people[1]);
    people[1].room.set_master_mute(true);
    hears(&people[1]).await;
}

struct CountingOutput {
    starts: Arc<AtomicUsize>,
    inner: FakeOutput,
}

impl AudioOutput for CountingOutput {
    fn start(&self, render: RenderFn) -> Result<Box<dyn AudioOutputSession>, AudioError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        self.inner.start(render)
    }
}

#[tokio::test]
async fn joining_twice_opens_one_output() {
    let people = room_of(&["alice"]).await;
    let starts = Arc::new(AtomicUsize::new(0));
    let output: Arc<dyn AudioOutput> = Arc::new(CountingOutput {
        starts: starts.clone(),
        inner: FakeOutput::new().0,
    });
    people[0].room.join_voice(mic(), output.clone()).unwrap();
    let before = people[0].room.snapshot().voice;
    people[0].room.join_voice(mic(), output).unwrap();
    assert_eq!(starts.load(Ordering::SeqCst), 1);
    assert_eq!(people[0].room.snapshot().voice, before);
}

fn broken_mic() -> Arc<dyn VoiceCapture> {
    Arc::new(CpalInput::new(Some("nonsense".into())))
}

#[tokio::test]
async fn a_missing_microphone_leaves_you_in_voice_listening_and_muted() {
    let mut people = room_of(&["alice", "bob"]).await;
    join(&mut people[1]);
    let (output, handle) = voice_output();
    people[0].room.join_voice(broken_mic(), output).unwrap();
    people[0].voice = handle;

    let view = people[0].room.snapshot().voice;
    assert!(view.joined && view.muted && !view.transmitting);
    assert!(view.error.is_some());
    hears(&people[0]).await;
}

#[tokio::test]
async fn unmuting_with_a_broken_microphone_stays_muted() {
    let people = room_of(&["alice"]).await;
    let (output, _) = voice_output();
    people[0].room.join_voice(broken_mic(), output).unwrap();

    people[0].room.set_voice_muted(false);

    let view = people[0].room.snapshot().voice;
    assert!(view.muted && !view.transmitting);
    assert!(view.error.is_some());
}

fn member_of(viewer: &Room, id: iroh::PublicKey) -> brp_room::MemberView {
    viewer
        .snapshot()
        .members
        .into_iter()
        .find(|m| m.id == id)
        .unwrap()
}

fn bytes_from(viewer: &Room, id: iroh::PublicKey) -> u64 {
    viewer
        .voice_levels()
        .members
        .get(&id)
        .map_or(0, |sample| sample.bytes)
}

#[tokio::test]
async fn a_person_you_mute_goes_silent_and_stays_subscribed() {
    let mut people = room_of(&["alice", "bob"]).await;
    join(&mut people[0]);
    join(&mut people[1]);
    hears(&people[1]).await;
    let a = people[0].room.id();

    people[1].room.set_hearing(a, false);

    assert!(!member_of(&people[1].room, a).hearing);
    goes_silent(&people[1]).await;
    let before = bytes_from(&people[1].room, a);
    wait_until("bytes still rising", WAIT, || {
        bytes_from(&people[1].room, a) > before
    })
    .await;

    people[1].room.set_hearing(a, true);
    hears(&people[1]).await;
}

#[tokio::test]
async fn a_person_you_deafen_hears_silence_while_others_hear_you() {
    let mut people = room_of(&["alice", "bob", "carol"]).await;
    for person in &mut people {
        join(person);
    }
    hears(&people[1]).await;
    hears(&people[2]).await;
    let b = people[1].room.id();
    // Only alice talks, so what bob and carol render is alice's.
    people[1].room.set_voice_muted(true);
    people[2].room.set_voice_muted(true);

    people[0].room.set_sending(b, false);

    assert!(!member_of(&people[0].room, b).sending);
    goes_silent(&people[1]).await;
    hears(&people[2]).await;
}

#[tokio::test]
async fn voice_volume_scales_only_that_person() {
    let mut people = room_of(&["alice", "bob"]).await;
    join(&mut people[0]);
    join(&mut people[1]);
    hears(&people[1]).await;
    let a = people[0].room.id();

    people[1].room.set_voice_volume(a, 0.0);
    goes_silent(&people[1]).await;
    people[1].room.set_voice_volume(a, 1.0);
    hears(&people[1]).await;

    let member = member_of(&people[1].room, a);
    assert_eq!(member.voice_gain, 1.0);
    assert_eq!(member.gain, 1.0);
    people[1].room.set_voice_volume(a, 7.0);
    assert_eq!(member_of(&people[1].room, a).voice_gain, 1.0);
}

#[tokio::test]
async fn per_person_controls_survive_leaving_and_rejoining_voice() {
    let mut people = room_of(&["alice", "bob"]).await;
    join(&mut people[0]);
    join(&mut people[1]);
    let a = people[0].room.id();
    people[1].room.set_hearing(a, false);
    people[1].room.set_sending(a, false);
    people[1].room.set_voice_volume(a, 0.4);

    people[1].room.leave_voice();
    join(&mut people[1]);

    let member = member_of(&people[1].room, a);
    assert!(!member.hearing && !member.sending);
    assert_eq!(member.voice_gain, 0.4);
    goes_silent(&people[1]).await;
}

#[tokio::test]
async fn a_member_who_leaves_the_room_leaves_no_listener_behind() {
    let mut people = room_of(&["alice", "bob", "carol"]).await;
    for person in &mut people {
        join(person);
    }
    let (b, c) = (people[1].room.id(), people[2].room.id());
    wait_until("alice listens to both", WAIT, || {
        let members = people[0].room.voice_levels().members;
        members.contains_key(&b) && members.contains_key(&c)
    })
    .await;
    people[0].room.set_voice_volume(c, 0.3);

    people.pop().unwrap().room.leave().await;

    wait_until("carol's listener is gone", WAIT, || {
        !people[0].room.voice_levels().members.contains_key(&c)
    })
    .await;
    wait_until("carol's prefs are gone", WAIT, || {
        people[0].room.snapshot().members.iter().all(|m| m.id != c)
    })
    .await;
}

#[tokio::test]
async fn levels_rise_while_someone_talks() {
    let mut people = room_of(&["alice", "bob"]).await;
    join(&mut people[0]);
    join(&mut people[1]);
    let a = people[0].room.id();
    wait_until("bob's meter for alice", WAIT, || {
        // Rendering pulls the track through the mixer, which is what updates its level.
        people[1].voice.render(960);
        people[1]
            .room
            .voice_levels()
            .members
            .get(&a)
            .is_some_and(|sample| sample.level > 0.0)
    })
    .await;
    wait_until("alice's own meter", WAIT, || {
        let own = people[0].room.voice_levels().own;
        own.level > 0.0 && own.bytes > 0
    })
    .await;
}

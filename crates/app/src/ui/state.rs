//! State the snapshot cannot carry, and the ordering and rate helpers every panel shares.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use brp_audio::{AudioSource, SendVolumeSupport};
use brp_capture::{SourceDescriptor, SourceId};
use brp_proto::SourceKind;
use brp_proto::constants::STATS_LOG_INTERVAL;
use brp_room::{MemberView, RoomSnapshot, WatchState, WatchView};

use super::applications::ApplicationPicker;
use crate::commands::RoomCommand;
use crate::render::tiles::TileKey;
use crate::settings::AudioApplications;

/// The source list the user is choosing from, on platforms without a picker of their own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourcePicker {
    pub kind: SourceKind,
    pub choices: Vec<SourceDescriptor>,
}

/// Which sheet a compact window shows above its dock.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum CompactSheet {
    #[default]
    Room,
    Share,
}

/// What a tile needs to go on showing once its watch has gone.
#[derive(Debug, Clone)]
pub struct DrawnTile {
    pub rect: egui::Rect,
    pub chip: String,
    /// It had no picture of its own to fade: the state wash was covering it.
    pub washed: bool,
}

/// Window-local state the room snapshot cannot carry: pending edits, per-tile choices, and the
/// status line.
#[derive(Debug, Default)]
pub struct UiState {
    /// What each watched stream has carried, sampled as the window draws.
    pub traffic: HashMap<TileKey, StreamTraffic>,
    /// The room's events, newest last.
    pub ledger: Ledger,
    /// This participant's streams with their controls unfolded in the rail.
    pub own_open: HashSet<u32>,
    /// Kept while the sheet slides shut, so its content does not change on the way out.
    pub sheet: CompactSheet,
    pub sheet_open: bool,
    /// Last error or notice, shown in the dock.
    pub status: String,
    /// True from the share click until the live started or failed: the portal dialog on Linux,
    /// capture start with its fallback on Windows.
    pub share_pending: bool,
    /// Open while the user picks a source from the platform's list.
    pub picker: Option<SourcePicker>,
    /// Open while the user edits which applications the room hears.
    pub applications: Option<ApplicationPicker>,
    monitor_shares: u32,
    window_shares: u32,
    /// Preset picked for a remote live before it is watched.
    pub preset_choice: HashMap<TileKey, u32>,
    /// How many tiles the grid last held and in how many columns, kept so a window resized near a
    /// tie does not flip the layout. A different number of tiles is a fresh choice.
    pub grid_columns: Option<(usize, usize)>,
    /// What the grid drew on its last pass.
    pub drawn_tiles: HashMap<TileKey, DrawnTile>,
    /// Tiles fading out where they stood.
    pub leaving_tiles: HashMap<TileKey, DrawnTile>,
    /// Tiles that finished leaving, whose video the window can drop. Kept here rather than in a
    /// pass's output because egui can discard the pass that finished them.
    pub gone_tiles: Vec<TileKey>,
    /// Tiles with the stats overlay open.
    pub stats_visible: HashSet<TileKey>,
    /// This participant's lives with their quality details unfolded.
    pub quality_open: HashSet<u32>,
    /// Bitrate being edited per (live id, preset id), committed when the widget is released.
    pub bitrate_edits: HashMap<(u32, u32), u32>,
    /// Frame rate being edited per live id, committed when the widget is released.
    pub fps_edits: HashMap<u32, u32>,
    /// Aggregate upload rate across every running encoder, in kilobits per second.
    pub upload_kbps: u64,
    upload_meter: BitrateMeter,
    /// One meter per running encoder, keyed by (live id, preset id); dropped when the encoder stops.
    preset_meters: HashMap<(u32, u32), BitrateMeter>,
}

/// One bar of the traffic strip: how much arrived in a moment, and whether a keyframe did.
#[derive(Debug, Clone, Copy)]
pub struct TrafficBar {
    pub megabits_per_second: f32,
    pub keyframe: bool,
}

/// Real bytes off the wire. No frames means no bars, which is what a still screen looks like.
#[derive(Debug, Default)]
pub struct StreamTraffic {
    last_bytes: u64,
    last_keyframes: u64,
    last_at: Option<Instant>,
    pub bytes_per_second: f64,
    pub bars: std::collections::VecDeque<TrafficBar>,
}

impl StreamTraffic {
    const BARS: usize = 50;
    const EVERY: f64 = 1.0 / 12.0;

    pub fn sample(&mut self, bytes: u64, keyframes: u64, now: Instant) {
        let Some(at) = self.last_at else {
            self.last_at = Some(now);
            self.last_bytes = bytes;
            self.last_keyframes = keyframes;
            return;
        };
        let elapsed = now.duration_since(at).as_secs_f64();
        if elapsed < Self::EVERY {
            return;
        }
        let rate = bytes.saturating_sub(self.last_bytes) as f64 / elapsed;
        self.bytes_per_second = self.bytes_per_second * 0.85 + rate * 0.15;
        if self.bytes_per_second < 1_000.0 {
            self.bytes_per_second = 0.0;
        }
        self.bars.push_back(TrafficBar {
            megabits_per_second: (rate * 8.0 / 1e6) as f32,
            keyframe: keyframes > self.last_keyframes,
        });
        if self.bars.len() > Self::BARS {
            self.bars.pop_front();
        }
        self.last_at = Some(now);
        self.last_bytes = bytes;
        self.last_keyframes = keyframes;
    }
}

/// A round trip in words. Peers on one network often take under a millisecond, which is not zero.
pub fn delay_text(rtt_ms: u32) -> String {
    if rtt_ms == 0 {
        "<1 ms".to_string()
    } else {
        format!("{rtt_ms} ms")
    }
}

/// Whether a watched stream has stopped arriving.
pub fn signal_lost(state: WatchState) -> bool {
    matches!(state, WatchState::SignalLost | WatchState::Reconnecting)
}

/// How an event in the ledger went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Good,
    Neutral,
    Bad,
}

/// Who joined, how they connect, and what failed, as it happens. Timed from when the room
/// opened, like a mission clock, which needs no time zone and says how long the room has run.
#[derive(Debug, Default)]
pub struct Ledger {
    started: Option<Instant>,
    pub lines: std::collections::VecDeque<(String, Outcome)>,
    newest_at: Option<Instant>,
    seen: HashMap<iroh::PublicKey, (brp_net::PathKind, bool, String)>,
    watching: HashSet<TileKey>,
    primed: bool,
}

impl Ledger {
    const KEEP: usize = 5;
    /// How long the newest line stays lit before it is only history.
    const FRESH: std::time::Duration = std::time::Duration::from_secs(8);

    pub fn newest_is_fresh(&self) -> bool {
        self.newest_at.is_some_and(|at| at.elapsed() < Self::FRESH)
    }

    fn add(&mut self, message: String, outcome: Outcome) {
        let started = *self.started.get_or_insert_with(Instant::now);
        let seconds = started.elapsed().as_secs();
        self.lines.push_back((
            format!("{:02}:{:02} {message}", seconds / 60, seconds % 60),
            outcome,
        ));
        self.newest_at = Some(Instant::now());
        if self.lines.len() > Self::KEEP {
            self.lines.pop_front();
        }
    }

    /// Compares the snapshot with what was last seen. Safe to run twice in a frame, because a second
    /// run sees no difference.
    pub fn note(&mut self, snapshot: &RoomSnapshot) {
        use brp_net::PathKind;
        let path_words = |path: PathKind, rtt: Option<u32>| match (path, rtt) {
            (PathKind::Direct, Some(rtt)) => format!("direct {}", delay_text(rtt)),
            (PathKind::Relayed, Some(rtt)) => format!("relayed {}", delay_text(rtt)),
            (PathKind::Direct, None) => "direct".to_string(),
            (PathKind::Relayed, None) => "relayed".to_string(),
            (PathKind::Unknown, _) => "finding a path".to_string(),
        };
        if !self.primed {
            self.primed = true;
            self.add("Room open".to_string(), Outcome::Good);
        }
        let mut present = HashSet::new();
        for member in &snapshot.members {
            present.insert(member.id);
            let lost = member.signal_lost;
            match self.seen.get(&member.id).cloned() {
                None => {
                    let path = path_words(member.path, member.rtt_ms);
                    self.add(
                        format!("{} joined · {path}", member.nickname),
                        Outcome::Good,
                    );
                }
                Some((path, was_lost, _)) => {
                    if lost && !was_lost {
                        self.add(format!("{}: no signal", member.nickname), Outcome::Bad);
                    } else if was_lost && !lost {
                        self.add(format!("{} reconnected", member.nickname), Outcome::Good);
                    } else if path != member.path && member.path != PathKind::Unknown {
                        let path = path_words(member.path, member.rtt_ms);
                        self.add(format!("{} · {path}", member.nickname), Outcome::Neutral);
                    }
                }
            }
            self.seen
                .insert(member.id, (member.path, lost, member.nickname.clone()));
        }
        let gone: Vec<_> = self
            .seen
            .keys()
            .filter(|id| !present.contains(*id))
            .copied()
            .collect();
        for id in gone {
            if let Some((_, _, name)) = self.seen.remove(&id) {
                self.add(format!("{name} left"), Outcome::Neutral);
            }
        }
        let watching: HashSet<TileKey> = snapshot
            .watches
            .iter()
            .map(|w| (w.publisher, w.live_id))
            .collect();
        for key in watching.difference(&self.watching.clone()) {
            let name = snapshot
                .members
                .iter()
                .find(|m| m.id == key.0)
                .map_or("Someone", |m| m.nickname.as_str());
            let title = name.to_string();
            self.add(format!("Watching {title}"), Outcome::Good);
        }
        self.watching = watching;
    }
}

impl UiState {
    /// Creates an empty state for a freshly opened window.
    pub fn new() -> Self {
        Self::default()
    }

    /// New lives are titled by kind and ordinal; the ordinal never repeats within a session.
    pub fn next_title(&mut self, kind: SourceKind) -> String {
        let counter = match kind {
            SourceKind::Monitor => &mut self.monitor_shares,
            SourceKind::Window => &mut self.window_shares,
        };
        *counter += 1;
        let name = match kind {
            SourceKind::Monitor => "Monitor",
            SourceKind::Window => "Window",
        };
        format!("{name} {counter}")
    }

    /// Whether this tile is still on screen, so its video has to outlive its watch until it has left.
    pub fn keeps_picture(&self, key: &TileKey) -> bool {
        self.drawn_tiles.contains_key(key) || self.leaving_tiles.contains_key(key)
    }

    /// Opens the picker. Refuses while a share is pending or a picker is already open, so a
    /// second click cannot discard an in-progress choice. Returns whether it opened.
    pub fn open_picker(&mut self, kind: SourceKind, choices: Vec<SourceDescriptor>) -> bool {
        if self.share_pending || self.picker.is_some() {
            return false;
        }
        self.picker = Some(SourcePicker { kind, choices });
        true
    }

    /// The share command for a picked source, closing the picker. `None` when no picker is open
    /// or the id is not one of its choices.
    pub fn pick_source(&mut self, id: SourceId) -> Option<RoomCommand> {
        let picker = self.picker.as_ref()?;
        if !picker.choices.iter().any(|choice| choice.id == id) {
            return None;
        }
        let kind = picker.kind;
        self.picker = None;
        Some(RoomCommand::Share {
            kind,
            source: Some(id),
        })
    }

    /// Closes the picker without sharing; a no-op when none is open.
    pub fn cancel_picker(&mut self) {
        self.picker = None;
    }

    /// Opens the application picker, or replaces the list of one already open — Refresh takes the
    /// same path, and the draft must survive it.
    pub fn open_applications(
        &mut self,
        reported: Vec<AudioSource>,
        stored: &AudioApplications,
        volume_support: SendVolumeSupport,
    ) {
        match &mut self.applications {
            Some(picker) => picker.refresh(reported),
            None => {
                let mut picker = ApplicationPicker::new(reported, stored);
                picker.volume_support = volume_support;
                self.applications = Some(picker);
            }
        }
    }

    /// Closes it, discarding the draft.
    pub fn cancel_applications(&mut self) {
        self.applications = None;
    }

    /// Feeds the cumulative byte counters of a snapshot into the upload and per-encoder meters.
    /// Meters of encoders that are no longer running are forgotten, so a restarted encoder starts
    /// a fresh measurement instead of inheriting a stale one.
    pub fn refresh_rates(&mut self, snapshot: &RoomSnapshot, now: Instant) {
        self.upload_kbps = self.upload_meter.update(total_encoded_bytes(snapshot), now);
        let running: HashMap<(u32, u32), u64> = snapshot
            .own_lives
            .iter()
            .flat_map(|live| {
                live.presets.iter().filter_map(move |preset| {
                    preset
                        .encoder
                        .as_ref()
                        .map(|encoder| ((live.info.id, preset.preset.id), encoder.bytes_encoded))
                })
            })
            .collect();
        self.preset_meters
            .retain(|key, _| running.contains_key(key));
        for (key, bytes) in running {
            self.preset_meters
                .entry(key)
                .or_default()
                .update(bytes, now);
        }
    }

    /// Measured encode rate of one running encoder, or `None` while it is not running.
    pub fn preset_kbps(&self, live_id: u32, preset_id: u32) -> Option<u64> {
        self.preset_meters
            .get(&(live_id, preset_id))
            .map(BitrateMeter::kbps)
    }
}

/// Aggregate encode rate from the cumulative byte counters in successive snapshots.
#[derive(Debug, Default)]
pub struct BitrateMeter {
    last_bytes: u64,
    last_at: Option<Instant>,
    kbps: u64,
}

impl BitrateMeter {
    /// Recomputes once per stats interval; in between it returns the previous rate.
    pub fn update(&mut self, total_bytes: u64, now: Instant) -> u64 {
        let Some(last_at) = self.last_at else {
            self.last_at = Some(now);
            self.last_bytes = total_bytes;
            return self.kbps;
        };
        let elapsed = now.duration_since(last_at);
        if elapsed < STATS_LOG_INTERVAL {
            return self.kbps;
        }
        let bits = total_bytes.saturating_sub(self.last_bytes) * 8;
        self.kbps = bits / 1000 / elapsed.as_secs().max(1);
        self.last_bytes = total_bytes;
        self.last_at = Some(now);
        self.kbps
    }

    pub fn kbps(&self) -> u64 {
        self.kbps
    }
}

/// Sums encoded bytes across every preset of every own live, for the upload meter.
pub fn total_encoded_bytes(snapshot: &RoomSnapshot) -> u64 {
    snapshot
        .own_lives
        .iter()
        .flat_map(|live| live.presets.iter())
        .filter_map(|preset| preset.encoder.as_ref())
        .map(|encoder| encoder.bytes_encoded)
        .sum()
}

/// Members by nickname then id, so the panel does not reorder between snapshots.
pub fn ordered_members(snapshot: &RoomSnapshot) -> Vec<&MemberView> {
    let mut members: Vec<&MemberView> = snapshot.members.iter().collect();
    members.sort_by(|a, b| {
        a.nickname
            .cmp(&b.nickname)
            .then_with(|| a.id.as_bytes().cmp(b.id.as_bytes()))
    });
    members
}

/// Watches by publisher then live id, so tiles keep their grid position.
pub fn ordered_watches(snapshot: &RoomSnapshot) -> Vec<&WatchView> {
    let mut watches: Vec<&WatchView> = snapshot.watches.iter().collect();
    watches.sort_by(|a, b| {
        a.publisher
            .as_bytes()
            .cmp(b.publisher.as_bytes())
            .then(a.live_id.cmp(&b.live_id))
    });
    watches
}

/// The watches the grid draws: [`ordered_watches`] minus the keys shown in a pop-out window.
pub fn visible_watches<'a>(
    snapshot: &'a RoomSnapshot,
    popped: &HashSet<TileKey>,
) -> Vec<&'a WatchView> {
    ordered_watches(snapshot)
        .into_iter()
        .filter(|w| w.state != WatchState::Ended)
        .filter(|w| !popped.contains(&(w.publisher, w.live_id)))
        .collect()
}

/// No stream anywhere in the room, so sharing is the thing to do next and the canvas offers it.
pub fn nobody_is_sharing(snapshot: &RoomSnapshot) -> bool {
    snapshot.own_lives.is_empty() && snapshot.members.iter().all(|m| m.lives.is_empty())
}

/// The live's title as its publisher advertises it, or `None` once the publisher is gone.
pub fn live_title(snapshot: &RoomSnapshot, key: TileKey) -> Option<String> {
    snapshot
        .members
        .iter()
        .find(|m| m.id == key.0)
        .and_then(|m| m.lives.iter().find(|l| l.id == key.1))
        .map(|l| l.title.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use brp_audio::AudioSelection;

    #[test]
    fn the_ledger_remembers_how_an_event_went_and_how_recent_the_newest_is() {
        let mut ledger = Ledger::default();
        assert!(!ledger.newest_is_fresh(), "nothing has happened yet");
        ledger.add("Sam joined".to_string(), Outcome::Good);
        assert!(ledger.newest_is_fresh());
        assert_eq!(ledger.lines[0].1, Outcome::Good);
        ledger.newest_at = Some(Instant::now() - Ledger::FRESH);
        assert!(
            !ledger.newest_is_fresh(),
            "it cools rather than staying lit"
        );
    }
    use brp_capture::{SourceDescriptor, SourceId};
    use brp_net::PathKind;
    use brp_proto::{Codec, LiveInfo, Preset};
    use brp_room::{
        AudioCaptureState, EncoderView, OwnAudioView, OwnLiveView, PresetView, WatchState,
    };
    use iroh::SecretKey;
    use std::time::Duration;

    use crate::commands::RoomCommand;

    #[test]
    fn titles_count_per_kind_and_never_repeat() {
        let mut state = UiState::new();
        assert_eq!(state.next_title(SourceKind::Monitor), "Monitor 1");
        assert_eq!(state.next_title(SourceKind::Window), "Window 1");
        assert_eq!(state.next_title(SourceKind::Monitor), "Monitor 2");
    }

    #[test]
    fn meter_reports_kilobits_per_second_once_per_interval() {
        let start = Instant::now();
        let mut meter = BitrateMeter::default();
        assert_eq!(meter.update(0, start), 0);
        assert_eq!(meter.update(1_000, start + Duration::from_millis(100)), 0);
        let bytes = 250_000 * STATS_LOG_INTERVAL.as_secs();
        assert_eq!(meter.update(bytes, start + STATS_LOG_INTERVAL), 2_000);
        assert_eq!(
            meter.update(bytes, start + STATS_LOG_INTERVAL + Duration::from_millis(1)),
            2_000
        );
    }

    fn own_live_with_encoder(bytes: Option<u64>) -> OwnLiveView {
        let preset = Preset {
            id: 1,
            name: "Source".into(),
            width: 64,
            height: 32,
            fps: 30,
            bitrate_kbps: 5_000,
            codec: Codec::H264,
        };
        OwnLiveView {
            full_chroma: true,
            info: LiveInfo {
                id: 7,
                title: "desk".into(),
                kind: SourceKind::Monitor,
                source_width: 64,
                source_height: 32,
                source_fps: 30,
                has_audio: false,
                presets: vec![preset.clone()],
            },
            presets: vec![PresetView {
                preset,
                encoder: bytes.map(|bytes_encoded| EncoderView {
                    name: "fake",
                    subscribers: 1,
                    frames_encoded: 10,
                    bytes_encoded,
                    dropped_at_input: 0,
                }),
                last_error: None,
            }],
        }
    }

    fn snapshot_with(own_lives: Vec<OwnLiveView>) -> RoomSnapshot {
        RoomSnapshot {
            voice: Default::default(),
            me: SecretKey::generate().public(),
            nickname: "me".into(),
            version: 1,
            members: Vec::new(),
            own_lives,
            watches: Vec::new(),
            own_audio: OwnAudioView {
                enabled: true,
                state: AudioCaptureState::Idle,
                selection: AudioSelection::All,
                subscribers: 0,
                packets_encoded: 0,
            },
            audio_output_error: None,
            master_mute: false,
        }
    }

    #[test]
    fn per_encoder_rates_follow_the_snapshot_and_vanish_when_the_encoder_stops() {
        let start = Instant::now();
        let mut state = UiState::new();
        state.refresh_rates(&snapshot_with(vec![own_live_with_encoder(Some(0))]), start);
        assert_eq!(state.preset_kbps(7, 1), Some(0));
        let bytes = 250_000 * STATS_LOG_INTERVAL.as_secs();
        state.refresh_rates(
            &snapshot_with(vec![own_live_with_encoder(Some(bytes))]),
            start + STATS_LOG_INTERVAL,
        );
        assert_eq!(state.preset_kbps(7, 1), Some(2_000));
        assert_eq!(state.upload_kbps, 2_000);
        state.refresh_rates(
            &snapshot_with(vec![own_live_with_encoder(None)]),
            start + 2 * STATS_LOG_INTERVAL,
        );
        assert_eq!(state.preset_kbps(7, 1), None);
    }

    fn member(nickname: &str) -> MemberView {
        MemberView {
            voice: None,
            hearing: true,
            sending: true,
            voice_gain: 1.0,
            id: SecretKey::generate().public(),
            nickname: nickname.into(),
            lives: Vec::new(),
            seen_ago_ms: 0,
            path: PathKind::Unknown,
            rtt_ms: None,
            signal_lost: false,
            watching_me: false,
            has_audio: false,
            audio_mix: None,
            gain: 1.0,
        }
    }

    #[test]
    fn members_sort_by_nickname() {
        let me = SecretKey::generate().public();
        let snapshot = RoomSnapshot {
            voice: Default::default(),
            me,
            nickname: "me".into(),
            version: 1,
            members: vec![member("zed"), member("amy"), member("kim")],
            own_lives: Vec::new(),
            watches: Vec::new(),
            own_audio: OwnAudioView {
                enabled: true,
                state: AudioCaptureState::Idle,
                selection: AudioSelection::All,
                subscribers: 0,
                packets_encoded: 0,
            },
            audio_output_error: None,
            master_mute: false,
        };
        let names: Vec<&str> = ordered_members(&snapshot)
            .iter()
            .map(|m| m.nickname.as_str())
            .collect();
        assert_eq!(names, ["amy", "kim", "zed"]);
    }

    fn descriptor(id: u64) -> SourceDescriptor {
        SourceDescriptor {
            id: SourceId(id),
            kind: SourceKind::Monitor,
            name: format!("Monitor {id}"),
            width: 1920,
            height: 1080,
        }
    }

    #[test]
    fn picking_a_listed_source_issues_the_share_and_closes_the_picker() {
        let mut state = UiState::new();
        assert!(state.open_picker(SourceKind::Monitor, vec![descriptor(1), descriptor(2)]));
        assert_eq!(
            state.pick_source(SourceId(2)),
            Some(RoomCommand::Share {
                kind: SourceKind::Monitor,
                source: Some(SourceId(2)),
            })
        );
        assert!(state.picker.is_none());
    }

    #[test]
    fn an_unlisted_id_or_a_closed_picker_yields_no_command() {
        let mut state = UiState::new();
        assert_eq!(state.pick_source(SourceId(1)), None);
        assert!(state.open_picker(SourceKind::Window, vec![descriptor(1)]));
        assert_eq!(state.pick_source(SourceId(9)), None);
        assert!(state.picker.is_some());
        state.cancel_picker();
        assert!(state.picker.is_none());
    }

    #[test]
    fn the_picker_does_not_open_while_a_share_is_pending() {
        let mut state = UiState::new();
        state.share_pending = true;
        assert!(!state.open_picker(SourceKind::Monitor, vec![descriptor(1)]));
        assert!(state.picker.is_none());
    }

    #[test]
    fn the_picker_does_not_reopen_while_one_is_open() {
        let mut state = UiState::new();
        assert!(state.open_picker(SourceKind::Monitor, vec![descriptor(1)]));
        assert!(!state.open_picker(SourceKind::Window, vec![descriptor(2)]));
        assert_eq!(
            state.picker,
            Some(SourcePicker {
                kind: SourceKind::Monitor,
                choices: vec![descriptor(1)],
            })
        );
    }

    fn watch(publisher: iroh::PublicKey, live_id: u32) -> WatchView {
        WatchView {
            publisher,
            live_id,
            preset_id: 1,
            state: WatchState::Live,
            frames_decoded: 0,
            keyframe_requests: 0,
            audio: false,
        }
    }

    #[test]
    fn ended_watches_leave_the_drawer_but_reconnecting_watches_remain() {
        let publisher = SecretKey::from_bytes(&[1; 32]).public();
        let mut snapshot = snapshot_with(Vec::new());
        let mut ended = watch(publisher, 1);
        ended.state = WatchState::Ended;
        let mut reconnecting = watch(publisher, 2);
        reconnecting.state = WatchState::Reconnecting;
        snapshot.watches = vec![ended, reconnecting];
        let visible = visible_watches(&snapshot, &HashSet::new());
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].live_id, 2);
    }

    #[test]
    fn visible_watches_skip_popped_keys_and_keep_the_order() {
        let a = SecretKey::from_bytes(&[1u8; 32]).public();
        let b = SecretKey::from_bytes(&[2u8; 32]).public();
        let mut snapshot = snapshot_with(Vec::new());
        snapshot.watches = vec![watch(b, 1), watch(a, 2), watch(a, 1)];
        let popped = HashSet::from([(a, 2)]);
        let keys: Vec<TileKey> = visible_watches(&snapshot, &popped)
            .iter()
            .map(|w| (w.publisher, w.live_id))
            .collect();
        // Key order follows the derived public key bytes, not the seed bytes, so sort the
        // expectation the same way `ordered_watches` does.
        let mut expected = vec![(a, 1), (b, 1)];
        expected.sort_by(|x, y| x.0.as_bytes().cmp(y.0.as_bytes()).then(x.1.cmp(&y.1)));
        assert_eq!(keys, expected);
        assert_eq!(visible_watches(&snapshot, &HashSet::new()).len(), 3);
    }

    #[test]
    fn live_title_follows_the_publisher_and_vanishes_with_it() {
        let mut publisher = member("bob");
        publisher.lives.push(LiveInfo {
            id: 4,
            title: "desk".into(),
            kind: SourceKind::Monitor,
            source_width: 64,
            source_height: 32,
            source_fps: 30,
            has_audio: false,
            presets: Vec::new(),
        });
        let id = publisher.id;
        let mut snapshot = snapshot_with(Vec::new());
        snapshot.members = vec![publisher];
        assert_eq!(live_title(&snapshot, (id, 4)), Some("desk".to_string()));
        assert_eq!(live_title(&snapshot, (id, 5)), None);
        snapshot.members.clear();
        assert_eq!(live_title(&snapshot, (id, 4)), None);
    }

    #[test]
    fn opening_the_application_picker_twice_refreshes_it_without_discarding_the_draft() {
        use crate::settings::AudioApplications;
        use brp_audio::{AppKey, AudioSource};

        let listed = |key: &str| {
            vec![AudioSource {
                key: AppKey::new(key),
                label: key.to_string(),
            }]
        };
        let mut state = UiState::new();
        state.open_applications(
            listed("firefox"),
            &AudioApplications::default(),
            SendVolumeSupport::Never,
        );
        let picker = state.applications.as_mut().expect("opened");
        picker.only = true;
        picker.toggle(&AppKey::new("firefox"), true);

        state.open_applications(
            listed("spotify"),
            &AudioApplications::default(),
            SendVolumeSupport::Never,
        );
        let picker = state.applications.as_ref().expect("still open");
        assert!(picker.only, "a refresh must not discard the edit");
        assert!(picker.is_chosen(&AppKey::new("firefox")));
        assert_eq!(picker.rows().len(), 2);

        state.cancel_applications();
        assert!(state.applications.is_none());
    }
}

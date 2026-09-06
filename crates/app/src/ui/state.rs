//! State the snapshot cannot carry, and the ordering and rate helpers every panel shares.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use brp_capture::{SourceDescriptor, SourceId};
use brp_proto::SourceKind;
use brp_proto::constants::STATS_LOG_INTERVAL;
use brp_room::{MemberView, RoomSnapshot, WatchView};

use crate::commands::RoomCommand;
use crate::render::tiles::TileKey;

/// The source list the user is choosing from, on platforms without a picker of their own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourcePicker {
    pub kind: SourceKind,
    pub choices: Vec<SourceDescriptor>,
}

/// Window-local state the room snapshot cannot carry: pending edits, per-tile choices, and the
/// status line.
#[derive(Debug, Default)]
pub struct UiState {
    /// Last error or notice, shown in the status bar.
    pub status: String,
    /// True from the share click until the live started or failed: the portal dialog on Linux,
    /// capture start with its fallback on Windows.
    pub share_pending: bool,
    /// Open while the user picks a source from the platform's list.
    pub picker: Option<SourcePicker>,
    monitor_shares: u32,
    window_shares: u32,
    /// Preset picked for a remote live before it is watched.
    pub preset_choice: HashMap<TileKey, u32>,
    /// Tiles with the stats overlay open.
    pub stats_visible: HashSet<TileKey>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use brp_capture::{SourceDescriptor, SourceId};
    use brp_net::PathKind;
    use brp_proto::{Codec, LiveInfo, Preset};
    use brp_room::{EncoderView, OwnLiveView, PresetView};
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
            me: SecretKey::generate().public(),
            nickname: "me".into(),
            version: 1,
            members: Vec::new(),
            own_lives,
            watches: Vec::new(),
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
            id: SecretKey::generate().public(),
            nickname: nickname.into(),
            lives: Vec::new(),
            seen_ago_ms: 0,
            path: PathKind::Unknown,
        }
    }

    #[test]
    fn members_sort_by_nickname() {
        let me = SecretKey::generate().public();
        let snapshot = RoomSnapshot {
            me,
            nickname: "me".into(),
            version: 1,
            members: vec![member("zed"), member("amy"), member("kim")],
            own_lives: Vec::new(),
            watches: Vec::new(),
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
}

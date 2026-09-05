//! State the snapshot cannot carry, and the ordering and rate helpers every panel shares.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use brp_proto::SourceKind;
use brp_proto::constants::STATS_LOG_INTERVAL;
use brp_room::{MemberView, RoomSnapshot, WatchView};

use crate::render::tiles::TileKey;

/// Window-local state the room snapshot cannot carry: pending edits, per-tile choices, and the
/// status line.
#[derive(Debug, Default)]
pub struct UiState {
    /// Last error or notice, shown in the status bar.
    pub status: String,
    /// True while the portal picker is open for a new live.
    pub share_pending: bool,
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
    /// Upload rate reported by the bitrate meter, in kilobits per second.
    pub upload_kbps: u64,
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
    use brp_net::PathKind;
    use iroh::SecretKey;
    use std::time::Duration;

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
}

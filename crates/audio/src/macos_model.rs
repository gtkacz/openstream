//! The decisions the macOS backend makes about what CoreAudio and ScreenCaptureKit report, kept
//! apart from the system calls so every runner tests them.

// Compiled into tests everywhere and used for real only on macOS.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use std::collections::{BTreeMap, BTreeSet};

use brp_proto::constants::{EXECUTABLE_NAME, MACOS_BUNDLE_ID};

use crate::selection::{AppKey, AudioSelection, AudioSource};

/// Whether a process is OpenStream: this one, any other bundled OpenStream (a link opened while a
/// room is open starts a second window, which plays the room's audio), or an unbundled build,
/// which has no bundle identifier and is known by its executable. Capturing any of them would
/// send the room its own audio back.
pub(crate) fn is_openstream(
    pid: i32,
    bundle_id: Option<&str>,
    name: Option<&str>,
    own_pid: i32,
) -> bool {
    let bundle_id = bundle_id.map(str::trim).filter(|id| !id.is_empty());
    pid == own_pid
        || bundle_id.is_some_and(|id| id.eq_ignore_ascii_case(MACOS_BUNDLE_ID))
        || (bundle_id.is_none() && name == Some(EXECUTABLE_NAME))
}

/// A running application as ScreenCaptureKit lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AppIds {
    pub pid: i32,
    pub bundle_id: String,
    pub name: String,
}

impl AppIds {
    fn is_openstream(&self, own_pid: i32) -> bool {
        is_openstream(self.pid, Some(&self.bundle_id), Some(&self.name), own_pid)
    }
}

/// The pids an `All` capture leaves out: every OpenStream process.
pub(crate) fn excluded_pids(running: &[AppIds], own_pid: i32) -> BTreeSet<i32> {
    running
        .iter()
        .filter(|app| app.is_openstream(own_pid))
        .map(|app| app.pid)
        .collect()
}

/// The pids the stream's filter names for `selection`: the OpenStream processes an `All` capture
/// leaves out, or the applications an `Only` capture takes. Re-read on every poll, since either
/// set changes as applications start and quit.
pub(crate) fn target_pids(
    running: &[AppIds],
    selection: &AudioSelection,
    own_pid: i32,
) -> BTreeSet<i32> {
    match selection {
        AudioSelection::All => excluded_pids(running, own_pid),
        AudioSelection::Only(selected) => selected_pids(running, selected, own_pid),
    }
}

/// Which set of pids the stream's filter last took. A set counts as applied only once
/// ScreenCaptureKit confirmed it, so a refused update is tried again on the next poll.
#[derive(Debug, Default)]
pub(crate) struct AppliedTarget {
    applied: Option<BTreeSet<i32>>,
}

impl AppliedTarget {
    pub(crate) fn wants(&self, pids: &BTreeSet<i32>) -> bool {
        self.applied.as_ref() != Some(pids)
    }

    pub(crate) fn record(&mut self, pids: BTreeSet<i32>, applied: bool) {
        if applied {
            self.applied = Some(pids);
        }
    }
}

/// Stereo from one buffer per channel: left and right from the first two, a lone channel on both
/// sides. Interleaves only as many frames as every used plane holds.
pub(crate) fn planar_to_stereo(planes: &[&[f32]]) -> Vec<f32> {
    match planes {
        [] => Vec::new(),
        [mono] => mono.iter().flat_map(|&s| [s, s]).collect(),
        [left, right, ..] => left
            .iter()
            .zip(right.iter())
            .flat_map(|(&l, &r)| [l, r])
            .collect(),
    }
}

/// Stereo from frames of `channels` interleaved samples, keeping the first two channels (or
/// doubling a single one). A trailing partial frame is dropped.
pub(crate) fn interleaved_to_stereo(samples: &[f32], channels: usize) -> Vec<f32> {
    match channels {
        0 => Vec::new(),
        1 => samples.iter().flat_map(|&s| [s, s]).collect(),
        2 => samples[..samples.len() - samples.len() % 2].to_vec(),
        _ => samples
            .chunks_exact(channels)
            .flat_map(|frame| [frame[0], frame[1]])
            .collect(),
    }
}

/// One process CoreAudio knows about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AudioProcess {
    pub pid: i32,
    pub bundle_id: Option<String>,
    /// The application's display name, when the system has one.
    pub label: Option<String>,
    /// Whether it is playing through an output device right now.
    pub running_output: bool,
}

/// The applications audible right now, once each by identity and never OpenStream, sorted by
/// label. A process without a bundle identifier has no stable identity to select it by and is
/// left out.
pub(crate) fn audible_sources(processes: Vec<AudioProcess>, own_pid: i32) -> Vec<AudioSource> {
    let mut by_key: BTreeMap<AppKey, String> = BTreeMap::new();
    for process in processes {
        if !process.running_output
            || is_openstream(
                process.pid,
                process.bundle_id.as_deref(),
                process.label.as_deref(),
                own_pid,
            )
        {
            continue;
        }
        let Some(bundle) = process.bundle_id.filter(|b| !b.trim().is_empty()) else {
            continue;
        };
        let label = process
            .label
            .filter(|l| !l.trim().is_empty())
            .unwrap_or_else(|| bundle.clone());
        by_key.entry(AppKey::bundle(&bundle)).or_insert(label);
    }
    let mut sources: Vec<AudioSource> = by_key
        .into_iter()
        .map(|(key, label)| AudioSource { key, label })
        .collect();
    sources.sort_by_key(|source| source.label.to_lowercase());
    sources
}

/// The pids among `running` whose application the selection names, never OpenStream's.
pub(crate) fn selected_pids(
    running: &[AppIds],
    selected: &BTreeSet<AppKey>,
    own_pid: i32,
) -> BTreeSet<i32> {
    running
        .iter()
        .filter(|app| {
            !app.is_openstream(own_pid) && selected.contains(&AppKey::bundle(&app.bundle_id))
        })
        .map(|app| app.pid)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::selection::AppKey;

    #[test]
    fn two_planes_interleave_into_left_right_pairs() {
        let left = [0.1, 0.2, 0.3];
        let right = [-0.1, -0.2, -0.3];
        assert_eq!(
            planar_to_stereo(&[&left, &right]),
            vec![0.1, -0.1, 0.2, -0.2, 0.3, -0.3]
        );
    }

    #[test]
    fn one_plane_plays_on_both_sides_and_extra_planes_are_dropped() {
        assert_eq!(
            planar_to_stereo(&[&[0.5, 0.25]]),
            vec![0.5, 0.5, 0.25, 0.25]
        );
        assert_eq!(
            planar_to_stereo(&[&[1.0], &[2.0], &[3.0]]),
            vec![1.0, 2.0],
            "a third channel is not part of stereo"
        );
        assert!(planar_to_stereo(&[]).is_empty());
    }

    #[test]
    fn uneven_planes_stop_at_the_shorter_one() {
        assert_eq!(
            planar_to_stereo(&[&[1.0, 2.0, 3.0], &[4.0]]),
            vec![1.0, 4.0]
        );
    }

    #[test]
    fn interleaved_input_keeps_its_first_two_channels() {
        assert_eq!(
            interleaved_to_stereo(&[1.0, 2.0, 3.0, 4.0], 2),
            vec![1.0, 2.0, 3.0, 4.0]
        );
        assert_eq!(
            interleaved_to_stereo(&[1.0, 2.0], 1),
            vec![1.0, 1.0, 2.0, 2.0]
        );
        assert_eq!(
            interleaved_to_stereo(&[1.0, 2.0, 9.0, 3.0, 4.0, 9.0, 5.0], 3),
            vec![1.0, 2.0, 3.0, 4.0],
            "a trailing partial frame is dropped"
        );
        assert!(interleaved_to_stereo(&[1.0], 0).is_empty());
    }

    const OWN_BUNDLE: &str = brp_proto::constants::MACOS_BUNDLE_ID;

    fn app(pid: i32, bundle: &str, name: &str) -> AppIds {
        AppIds {
            pid,
            bundle_id: bundle.to_string(),
            name: name.to_string(),
        }
    }

    #[test]
    fn openstream_is_recognised_by_pid_bundle_identifier_or_bare_executable() {
        let own = 10;
        assert!(is_openstream(
            own,
            Some("com.example.tool"),
            Some("Tool"),
            own
        ));
        assert!(is_openstream(11, Some(OWN_BUNDLE), Some("OpenStream"), own));
        assert!(
            is_openstream(11, Some(&OWN_BUNDLE.to_uppercase()), None, own),
            "bundle identifiers compare without case"
        );
        assert!(
            is_openstream(12, None, Some("brp"), own),
            "an unbundled build has no bundle identifier, only its executable"
        );
        assert!(is_openstream(12, Some(""), Some("brp"), own));
        assert!(!is_openstream(
            13,
            Some("com.example.brp"),
            Some("brp"),
            own
        ));
        assert!(!is_openstream(
            14,
            Some("com.spotify.client"),
            Some("Spotify"),
            own
        ));
    }

    #[test]
    fn every_openstream_process_is_excluded_from_everything() {
        let own = 10;
        let running = vec![
            app(own, OWN_BUNDLE, "OpenStream"),
            app(11, OWN_BUNDLE, "OpenStream"),
            app(12, "", "brp"),
            app(13, "com.spotify.client", "Spotify"),
        ];
        assert_eq!(excluded_pids(&running, own), BTreeSet::from([own, 11, 12]));
    }

    #[test]
    fn the_filter_follows_openstream_windows_that_open_after_the_start() {
        let own = 10;
        let before = vec![
            app(own, OWN_BUNDLE, "OpenStream"),
            app(13, "com.spotify.client", "Spotify"),
        ];
        let after = vec![
            app(own, OWN_BUNDLE, "OpenStream"),
            app(13, "com.spotify.client", "Spotify"),
            app(20, OWN_BUNDLE, "OpenStream"),
        ];
        let all = AudioSelection::All;
        assert_eq!(target_pids(&before, &all, own), BTreeSet::from([own]));
        assert_eq!(
            target_pids(&after, &all, own),
            BTreeSet::from([own, 20]),
            "a window opened later is excluded too"
        );
        let only = AudioSelection::Only(BTreeSet::from([AppKey::bundle("com.spotify.client")]));
        assert_eq!(target_pids(&after, &only, own), BTreeSet::from([13]));
    }

    #[test]
    fn a_failed_filter_update_is_tried_again_and_a_successful_one_is_not() {
        let mut target = AppliedTarget::default();
        let first = BTreeSet::from([1, 2]);
        assert!(target.wants(&first), "nothing applied yet");
        target.record(first.clone(), false);
        assert!(target.wants(&first), "a failure leaves the set to retry");
        target.record(first.clone(), true);
        assert!(!target.wants(&first));
        assert!(
            target.wants(&BTreeSet::from([1])),
            "a different set is applied"
        );
    }

    fn process(pid: i32, bundle: &str, label: Option<&str>, running: bool) -> AudioProcess {
        AudioProcess {
            pid,
            bundle_id: (!bundle.is_empty()).then(|| bundle.to_string()),
            label: label.map(str::to_string),
            running_output: running,
        }
    }

    #[test]
    fn the_audible_list_is_other_processes_playing_now_once_each_by_name() {
        let own = 10;
        let sources = audible_sources(
            vec![
                process(1, "com.spotify.client", Some("Spotify"), true),
                process(2, "org.mozilla.firefox", Some("Firefox"), true),
                process(3, "org.mozilla.firefox", Some("Firefox"), true),
                process(4, "com.apple.Music", Some("Music"), false),
                process(own, OWN_BUNDLE, Some("OpenStream"), true),
                process(6, OWN_BUNDLE, Some("OpenStream"), true),
                process(5, "", Some("nameless"), true),
            ],
            own,
        );
        let labels: Vec<&str> = sources.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, ["Firefox", "Spotify"]);
        assert_eq!(sources[0].key, AppKey::bundle("org.mozilla.firefox"));
    }

    #[test]
    fn an_unnamed_process_is_labelled_by_its_bundle_id() {
        let sources = audible_sources(vec![process(1, "com.example.Tool", None, true)], 99);
        assert_eq!(sources[0].label, "com.example.Tool");
        assert_eq!(sources[0].key, AppKey::bundle("com.example.tool"));
    }

    #[test]
    fn only_running_applications_named_in_the_selection_are_captured() {
        let own = 10;
        let running = vec![
            app(1, "com.spotify.client", "Spotify"),
            app(2, "org.mozilla.firefox", "Firefox"),
            app(own, OWN_BUNDLE, "OpenStream"),
            app(11, OWN_BUNDLE, "OpenStream"),
        ];
        let wanted = BTreeSet::from([
            AppKey::bundle("COM.SPOTIFY.CLIENT"),
            AppKey::bundle(OWN_BUNDLE),
            AppKey::bundle("com.apple.music"),
        ]);
        assert_eq!(selected_pids(&running, &wanted, own), BTreeSet::from([1]));
        assert!(selected_pids(&running, &BTreeSet::new(), own).is_empty());
    }
}

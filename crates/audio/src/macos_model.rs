//! The decisions the macOS backend makes about what CoreAudio and ScreenCaptureKit report, kept
//! apart from the system calls so every runner tests them.

// Compiled into tests everywhere and used for real only on macOS.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use std::collections::{BTreeMap, BTreeSet};

use crate::selection::{AppKey, AudioSource};

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

/// The applications audible right now, once each by identity and never brp itself, sorted by
/// label. A process without a bundle identifier has no stable identity to select it by and is
/// left out.
pub(crate) fn audible_sources(processes: Vec<AudioProcess>, own_pid: i32) -> Vec<AudioSource> {
    let mut by_key: BTreeMap<AppKey, String> = BTreeMap::new();
    for process in processes {
        if !process.running_output || process.pid == own_pid {
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

/// The pids among `running` (pid and bundle identifier) whose application the selection names,
/// never brp's own.
pub(crate) fn selected_pids(
    running: &[(i32, String)],
    selected: &BTreeSet<AppKey>,
    own_pid: i32,
) -> BTreeSet<i32> {
    running
        .iter()
        .filter(|(pid, bundle)| *pid != own_pid && selected.contains(&AppKey::bundle(bundle)))
        .map(|(pid, _)| *pid)
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
                process(own, "io.github.gtkacz.openstream", Some("OpenStream"), true),
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
            (1, "com.spotify.client".to_string()),
            (2, "org.mozilla.firefox".to_string()),
            (own, "io.github.gtkacz.openstream".to_string()),
        ];
        let wanted = BTreeSet::from([
            AppKey::bundle("COM.SPOTIFY.CLIENT"),
            AppKey::bundle("io.github.gtkacz.openstream"),
            AppKey::bundle("com.apple.music"),
        ]);
        assert_eq!(selected_pids(&running, &wanted, own), BTreeSet::from([1]));
        assert!(selected_pids(&running, &BTreeSet::new(), own).is_empty());
    }
}

//! The application picker: which applications the room hears. A draft edited in a window of its
//! own and applied on Done, because every apply swaps the capture session and gaps the room for
//! the length of one backend start.

use std::collections::BTreeSet;

use brp_audio::{AppKey, AudioSelection, AudioSource};

use crate::settings::{AudioApplications, AudioMode};

/// A row in the picker: an identity, what to call it, and whether the platform reports it playing
/// audio now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationRow {
    pub key: AppKey,
    pub label: String,
    pub playing: bool,
}

/// What the platform last reported, and the draft the user is editing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationPicker {
    reported: Vec<AudioSource>,
    /// Draft mode; `false` is every application except brp.
    pub only: bool,
    /// Draft set, kept in both modes so flipping to every application and back loses nothing.
    pub chosen: BTreeSet<AppKey>,
}

impl ApplicationPicker {
    /// Opens with what the platform reports and the stored choice as the draft. The stored choice,
    /// not the selection in force: under `all` only the settings still hold the names.
    pub fn new(reported: Vec<AudioSource>, stored: &AudioApplications) -> Self {
        Self {
            reported,
            only: stored.mode == AudioMode::Only,
            chosen: stored.names.iter().map(|name| AppKey::new(name)).collect(),
        }
    }

    /// Replaces the reported list and keeps the draft: a refresh mid-edit must not discard the edit.
    pub fn refresh(&mut self, reported: Vec<AudioSource>) {
        self.reported = reported;
    }

    /// The rows to draw: what is playing first, then what is chosen but absent, each group
    /// alphabetical by label so a refresh does not reshuffle the list.
    pub fn rows(&self) -> Vec<ApplicationRow> {
        let mut rows: Vec<ApplicationRow> = self
            .reported
            .iter()
            .map(|source| ApplicationRow {
                key: source.key.clone(),
                label: source.label.clone(),
                playing: true,
            })
            .collect();
        rows.sort_by(|a, b| a.label.cmp(&b.label).then_with(|| a.key.cmp(&b.key)));
        // A closed application has no friendly name to report, so its stored identity is the label.
        let mut absent: Vec<ApplicationRow> = self
            .chosen
            .iter()
            .filter(|key| !self.reported.iter().any(|source| &source.key == *key))
            .map(|key| ApplicationRow {
                key: key.clone(),
                label: key.as_str().to_string(),
                playing: false,
            })
            .collect();
        absent.sort_by(|a, b| a.label.cmp(&b.label));
        rows.append(&mut absent);
        rows
    }

    pub fn is_chosen(&self, key: &AppKey) -> bool {
        self.chosen.contains(key)
    }

    pub fn toggle(&mut self, key: &AppKey, chosen: bool) {
        if chosen {
            self.chosen.insert(key.clone());
        } else {
            self.chosen.remove(key);
        }
    }

    /// What Done applies to the room and stores in the settings.
    pub fn applied(&self) -> AudioApplications {
        AudioApplications {
            mode: if self.only {
                AudioMode::Only
            } else {
                AudioMode::All
            },
            names: self
                .chosen
                .iter()
                .map(|key| key.as_str().to_string())
                .collect(),
        }
    }
}

/// The one-line summary beside the button: what the room hears.
pub fn selection_summary(selection: &AudioSelection) -> String {
    match selection {
        AudioSelection::All => "all applications".to_string(),
        AudioSelection::Only(keys) if keys.is_empty() => "no applications selected".to_string(),
        AudioSelection::Only(keys) => {
            let plural = if keys.len() == 1 { "" } else { "s" };
            format!("{} application{plural}", keys.len())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(key: &str, label: &str) -> AudioSource {
        AudioSource {
            key: AppKey::new(key),
            label: label.into(),
        }
    }

    fn stored(mode: AudioMode, names: [&str; 1]) -> AudioApplications {
        AudioApplications {
            mode,
            names: names.iter().map(|n| n.to_string()).collect(),
        }
    }

    #[test]
    fn the_draft_starts_from_the_stored_choice_in_both_modes() {
        let all = ApplicationPicker::new(Vec::new(), &stored(AudioMode::All, ["firefox"]));
        assert!(!all.only);
        assert!(
            all.is_chosen(&AppKey::new("firefox")),
            "under all the chosen set is still visible, so a flip back loses nothing"
        );
        let only = ApplicationPicker::new(Vec::new(), &stored(AudioMode::Only, ["firefox"]));
        assert!(only.only);
    }

    #[test]
    fn rows_list_what_is_playing_first_then_the_selected_but_absent() {
        let picker = ApplicationPicker::new(
            vec![source("spotify", "Spotify"), source("firefox", "Firefox")],
            &AudioApplications {
                mode: AudioMode::Only,
                names: vec!["game.exe".into(), "another.exe".into(), "firefox".into()],
            },
        );
        let rows: Vec<(String, bool)> = picker
            .rows()
            .into_iter()
            .map(|row| (row.label, row.playing))
            .collect();
        assert_eq!(
            rows,
            [
                ("Firefox".to_string(), true),
                ("Spotify".to_string(), true),
                ("another.exe".to_string(), false),
                ("game.exe".to_string(), false),
            ],
            "each group alphabetical by label, so a refresh does not reshuffle; an absent row is \
             labelled with its stored key"
        );
    }

    #[test]
    fn a_refresh_replaces_the_list_and_keeps_the_draft() {
        let mut picker = ApplicationPicker::new(
            vec![source("firefox", "Firefox")],
            &AudioApplications::default(),
        );
        picker.only = true;
        picker.toggle(&AppKey::new("firefox"), true);
        picker.refresh(vec![source("spotify", "Spotify")]);
        assert!(picker.only);
        assert!(picker.is_chosen(&AppKey::new("firefox")));
        let rows: Vec<(String, bool)> = picker
            .rows()
            .into_iter()
            .map(|row| (row.label, row.playing))
            .collect();
        assert_eq!(
            rows,
            [
                ("Spotify".to_string(), true),
                ("firefox".to_string(), false),
            ]
        );
    }

    #[test]
    fn what_done_applies_keeps_the_names_in_both_modes() {
        let mut picker = ApplicationPicker::new(
            vec![source("firefox", "Firefox")],
            &AudioApplications::default(),
        );
        picker.only = true;
        picker.toggle(&AppKey::new("firefox"), true);
        picker.toggle(&AppKey::new("spotify"), true);
        picker.toggle(&AppKey::new("spotify"), false);
        let applied = picker.applied();
        assert_eq!(
            applied,
            AudioApplications {
                mode: AudioMode::Only,
                names: vec!["firefox".into()],
            }
        );
        picker.only = false;
        assert_eq!(
            picker.applied(),
            AudioApplications {
                mode: AudioMode::All,
                names: vec!["firefox".into()],
            }
        );
    }

    #[test]
    fn the_summary_names_the_mode_and_counts_the_set() {
        assert_eq!(selection_summary(&AudioSelection::All), "all applications");
        assert_eq!(
            selection_summary(&AudioSelection::Only(BTreeSet::new())),
            "no applications selected"
        );
        assert_eq!(
            selection_summary(&AudioSelection::Only(BTreeSet::from([AppKey::new("a")]))),
            "1 application"
        );
        assert_eq!(
            selection_summary(&AudioSelection::Only(BTreeSet::from([
                AppKey::new("a"),
                AppKey::new("b"),
                AppKey::new("c"),
            ]))),
            "3 applications"
        );
    }
}

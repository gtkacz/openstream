//! The application picker: which applications the room hears. A draft edited in a window of its
//! own and applied on Done, because every apply swaps the capture session and gaps the room for
//! the length of one backend start.

use std::collections::{BTreeMap, BTreeSet};

use brp_audio::{
    AppKey, AudioSelection, AudioSource, MAX_SEPARATE_APPLICATIONS, SendVolumeSupport,
};

use super::state::UiState;
use super::theme;
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
    /// Draft volumes in percent; an application not named here is at full.
    pub volumes: BTreeMap<AppKey, u8>,
    /// What this platform's capture can do with them.
    pub volume_support: SendVolumeSupport,
}

impl ApplicationPicker {
    /// Opens with what the platform reports and the stored choice as the draft. The stored choice,
    /// not the selection in force: under `all` only the settings still hold the names.
    pub fn new(reported: Vec<AudioSource>, stored: &AudioApplications) -> Self {
        Self {
            reported,
            only: stored.mode == AudioMode::Only,
            chosen: stored.names.iter().map(|name| AppKey::new(name)).collect(),
            volumes: stored.to_volumes(),
            volume_support: SendVolumeSupport::Never,
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

    /// Whether this row's volume can be edited in the draft's mode: an application the room will
    /// not hear has nothing to turn down, and a backend mixing everything cannot tell them apart.
    pub fn volume_editable(&self, key: &AppKey) -> bool {
        match self.volume_support {
            SendVolumeSupport::Never => false,
            SendVolumeSupport::OnlySelected => self.only && self.chosen.contains(key),
            SendVolumeSupport::Always => !self.only || self.chosen.contains(key),
        }
    }

    /// Whether the room hears more applications than the capture can keep apart, so some sliders
    /// may have no effect. Which ones is up to the order they started in, so only the count is known.
    pub fn too_many_for_volumes(&self) -> bool {
        let heard = self
            .reported
            .iter()
            .filter(|source| !self.only || self.chosen.contains(&source.key))
            .count();
        self.volume_support == SendVolumeSupport::Always && heard > MAX_SEPARATE_APPLICATIONS
    }

    pub fn volume(&self, key: &AppKey) -> u8 {
        self.volumes
            .get(key)
            .copied()
            .unwrap_or(FULL_VOLUME)
            .min(FULL_VOLUME)
    }

    pub fn set_volume(&mut self, key: &AppKey, percent: u8) {
        self.volumes.insert(key.clone(), percent.min(FULL_VOLUME));
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
            // Full is what an absent name means, so it is not written out.
            volumes: self
                .volumes
                .iter()
                .filter(|(_, percent)| **percent < FULL_VOLUME)
                .map(|(key, percent)| (key.as_str().to_string(), *percent))
                .collect(),
        }
    }
}

/// Whether Done has to swap the capture session. A swap gaps the room and a volume needs none, so a
/// Done that only moved a slider skips it. A Done that changed nothing still swaps: that is the
/// retry path after a capture failure.
pub fn swaps_capture(before: &AudioApplications, after: &AudioApplications) -> bool {
    after.to_selection() != before.to_selection() || after.volumes == before.volumes
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

const FULL_VOLUME: u8 = 100;
const VOLUME_FIGURE_WIDTH: f32 = 32.0;
const MAX_LIST_HEIGHT: f32 = 400.0;
const ROW_HEIGHT: f32 = 32.0;
/// A radio's width and the gap after it.
const RADIO_LABEL_INSET: f32 = 24.0;

/// What one pass of the picker produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerOutcome {
    /// Refresh was clicked: enumerate again and merge into the open draft.
    Refresh,
    /// Done was clicked: apply this to the room and store it.
    Applied(AudioApplications),
}

/// Draws the picker when one is open. Esc, Cancel, and the title bar's close button all discard
/// the draft; the main window has no other Esc consumer, since the fullscreen rule lives in the
/// pop-out windows and each has an egui context of its own.
pub fn draw(ctx: &egui::Context, state: &mut UiState) -> Option<PickerOutcome> {
    // Kept after the picker closes, so the dialog can leave still showing what it showed.
    let leaving_id = egui::Id::new("application-picker-content");
    let open = state.applications.is_some();
    if let Some(picker) = &state.applications {
        ctx.data_mut(|data| data.insert_temp(leaving_id, picker.clone()));
    }
    let mut picker = ctx.data(|data| data.get_temp::<ApplicationPicker>(leaving_id))?;
    let rows = picker.rows();
    let (mut done, mut cancelled, mut refresh) = (false, false, false);
    let (_, dismissed) = theme::dialog(
        ctx,
        "application-picker",
        "Applications the room hears",
        open,
        |ui| {
            ui.spacing_mut().interact_size.y = ROW_HEIGHT;
            ui.spacing_mut().item_spacing.y = 0.0;
            theme::radio(
                ui,
                &mut picker.only,
                false,
                "All applications (except OpenStream)",
            );
            theme::radio(ui, &mut picker.only, true, "Only the applications I select");
            let only = picker.only;
            ui.horizontal(|ui| {
                // Starts on the radio labels' edge, since this list belongs to the radio above it.
                ui.add_space(RADIO_LABEL_INSET);
                ui.vertical(|ui| {
                    if rows.is_empty() {
                        ui.add_space(4.0);
                        ui.weak("No applications are playing audio");
                    }
                    egui::ScrollArea::vertical()
                        .max_height(
                            (ctx.content_rect().height() - 280.0).clamp(120.0, MAX_LIST_HEIGHT),
                        )
                        .show(ui, |ui| {
                            for row in &rows {
                                let mut chosen = picker.is_chosen(&row.key);
                                let label = if row.playing {
                                    row.label.clone()
                                } else {
                                    format!("{} (not playing)", row.label)
                                };
                                // Under "all" the list stays visible but disabled: what was chosen
                                // is still there and a flip back loses nothing.
                                let check =
                                    |ui: &mut egui::Ui| theme::check(ui, &mut chosen, &label);
                                let mut toggled = false;
                                let mut percent = picker.volume(&row.key);
                                let editable = picker.volume_editable(&row.key);
                                theme::split_row(
                                    ui,
                                    ROW_HEIGHT,
                                    |ui| toggled = ui.add_enabled(only, check).changed(),
                                    |ui| {
                                        if editable {
                                            // Right to left, so the figure comes first. A fixed
                                            // width keeps the slider from shifting as it changes.
                                            ui.add_sized(
                                                [VOLUME_FIGURE_WIDTH, ROW_HEIGHT],
                                                egui::Label::new(
                                                    egui::RichText::new(format!("{percent}%"))
                                                        .small()
                                                        .color(theme::SECONDARY),
                                                ),
                                            );
                                            // The thumb is sized from the row's height otherwise.
                                            ui.spacing_mut().interact_size.y = 18.0;
                                            ui.visuals_mut().selection.bg_fill = theme::ACCENT;
                                            ui.add(
                                                egui::Slider::new(&mut percent, 0..=FULL_VOLUME)
                                                    .show_value(false),
                                            )
                                            .on_hover_text("How loud the room hears it");
                                        }
                                    },
                                );
                                if toggled {
                                    picker.toggle(&row.key, chosen);
                                }
                                if percent != picker.volume(&row.key) {
                                    picker.set_volume(&row.key, percent);
                                }
                            }
                        });
                });
            });
            if picker.too_many_for_volumes() {
                ui.add_space(8.0);
                ui.colored_label(
                    theme::WARNING,
                    format!(
                        "Only {MAX_SEPARATE_APPLICATIONS} applications can have their own volume \
                         at once. The rest stay at full."
                    ),
                );
            }
            theme::dialog_footer(
                ui,
                |ui| {
                    refresh = theme::action(ui, theme::icons::REFRESH, "Refresh", true).clicked();
                },
                |ui| {
                    done = theme::primary_button(ui, "Done", true).clicked();
                    cancelled = theme::text_button(ui, "Cancel", true).clicked();
                },
            );
        },
    )?;
    if !open {
        return None;
    }
    if done {
        let applied = picker.applied();
        state.cancel_applications();
        return Some(PickerOutcome::Applied(applied));
    }
    if cancelled || dismissed {
        state.cancel_applications();
        return None;
    }
    state.applications = Some(picker);
    refresh.then_some(PickerOutcome::Refresh)
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
            ..Default::default()
        }
    }

    #[test]
    fn a_volume_only_applies_where_the_capture_can_tell_applications_apart() {
        let key = AppKey::new("firefox");
        let mut picker = ApplicationPicker::new(Vec::new(), &AudioApplications::default());

        assert!(
            !picker.volume_editable(&key),
            "the default promises nothing"
        );

        picker.volume_support = SendVolumeSupport::Always;
        assert!(picker.volume_editable(&key), "every application is heard");
        picker.only = true;
        assert!(!picker.volume_editable(&key), "not chosen, so not heard");
        picker.toggle(&key, true);
        assert!(picker.volume_editable(&key));

        picker.volume_support = SendVolumeSupport::OnlySelected;
        assert!(picker.volume_editable(&key));
        picker.only = false;
        assert!(!picker.volume_editable(&key), "the platform's finished mix");
    }

    #[test]
    fn only_a_volume_below_full_is_stored_and_a_stored_one_comes_back() {
        let mut picker = ApplicationPicker::new(Vec::new(), &AudioApplications::default());
        let (game, music) = (AppKey::new("game"), AppKey::new("music"));
        picker.set_volume(&game, 25);
        picker.set_volume(&music, 100);
        let applied = picker.applied();
        assert_eq!(applied.volumes, BTreeMap::from([("game".to_string(), 25)]));

        let reopened = ApplicationPicker::new(Vec::new(), &applied);
        assert_eq!(reopened.volume(&game), 25);
        assert_eq!(reopened.volume(&music), 100);
    }

    #[test]
    fn done_swaps_the_capture_unless_only_a_volume_moved() {
        let before = AudioApplications::default();
        let quieter = AudioApplications {
            volumes: BTreeMap::from([("game".to_string(), 25)]),
            ..before.clone()
        };
        assert!(!swaps_capture(&before, &quieter), "no gap for a slider");
        assert!(swaps_capture(&before, &before), "retry path");
        let only = AudioApplications {
            mode: AudioMode::Only,
            ..quieter.clone()
        };
        assert!(swaps_capture(&before, &only), "the selection changed too");
    }

    #[test]
    fn the_picker_warns_when_the_room_hears_more_than_the_capture_keeps_apart() {
        let many: Vec<AudioSource> = (0..=MAX_SEPARATE_APPLICATIONS)
            .map(|index| source(&format!("app{index}"), "App"))
            .collect();
        let mut picker = ApplicationPicker::new(many, &AudioApplications::default());
        assert!(
            !picker.too_many_for_volumes(),
            "this capture has no sliders"
        );

        picker.volume_support = SendVolumeSupport::Always;
        assert!(picker.too_many_for_volumes());

        picker.only = true;
        assert!(!picker.too_many_for_volumes(), "none of them is chosen");
        for index in 0..=MAX_SEPARATE_APPLICATIONS {
            picker.toggle(&AppKey::new(&format!("app{index}")), true);
        }
        assert!(picker.too_many_for_volumes());
    }

    #[test]
    fn a_refresh_keeps_the_volumes_being_edited() {
        let mut picker = ApplicationPicker::new(Vec::new(), &AudioApplications::default());
        picker.set_volume(&AppKey::new("game"), 25);
        picker.refresh(vec![source("spotify", "Spotify")]);
        assert_eq!(picker.volume(&AppKey::new("game")), 25);
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
                ..Default::default()
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
                ..Default::default()
            }
        );
        picker.only = false;
        assert_eq!(
            picker.applied(),
            AudioApplications {
                mode: AudioMode::All,
                names: vec!["firefox".into()],
                ..Default::default()
            }
        );
    }

    #[test]
    fn the_outcome_of_done_carries_the_choice_and_its_selection() {
        let mut picker = ApplicationPicker::new(Vec::new(), &AudioApplications::default());
        picker.only = true;
        picker.toggle(&AppKey::new("firefox"), true);
        let outcome = PickerOutcome::Applied(picker.applied());
        let PickerOutcome::Applied(applied) = outcome else {
            panic!("Done applies a choice");
        };
        assert_eq!(applied.mode, AudioMode::Only);
        assert_eq!(
            applied.to_selection(),
            AudioSelection::Only(BTreeSet::from([AppKey::new("firefox")]))
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

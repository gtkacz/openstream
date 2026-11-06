//! The Settings dialog: edits a draft copy of the saved settings and hands it back on Save.
//! Changes apply when the next room opens; the dialog says so while a room is open.

use brp_audio::AudioDevice;

use super::{motion, theme};
use crate::settings::{NoiseSuppression, RelayChoice, Settings, normalised_nickname};

const ROW_GAP: f32 = 12.0;
/// The frame rates offered as one click each, and the choice to type any other.
const FPS_CHOICES: [(Option<u32>, &str); 3] =
    [(Some(30), "30 fps"), (Some(60), "60 fps"), (None, "Custom")];

const SUPPRESSION_CHOICES: [(NoiseSuppression, &str); 2] = [
    (NoiseSuppression::Off, "Off"),
    (NoiseSuppression::Light, "Light"),
];

/// Dialog state: whether it is open, the draft being edited, the devices listed when it opened,
/// and the last validation or save error.
#[derive(Debug, Default)]
pub struct SettingsDialog {
    pub open: bool,
    pub draft: Settings,
    /// The nickname field's raw text. Held apart from `draft.nickname` so the text edit keeps
    /// whatever the user is typing, spaces included, instead of a normalised value rebuilt from
    /// it every frame.
    pub nickname_text: String,
    pub devices: Vec<AudioDevice>,
    pub devices_error: Option<String>,
    pub inputs: Vec<AudioDevice>,
    pub inputs_error: Option<String>,
    pub error: String,
    /// The frame rate is being typed rather than picked. Held apart from the value because 60 can be
    /// a typed number too, and choosing Custom must not change it.
    pub custom_fps: bool,
}

impl SettingsDialog {
    /// Opens with a fresh draft of `settings` and the device lists as enumerated now.
    pub fn open_with(
        &mut self,
        settings: &Settings,
        devices: Result<Vec<AudioDevice>, String>,
        inputs: Result<Vec<AudioDevice>, String>,
    ) {
        self.draft = settings.clone();
        self.nickname_text = settings.nickname.clone().unwrap_or_default();
        match devices {
            Ok(devices) => {
                self.devices = devices;
                self.devices_error = None;
            }
            Err(error) => {
                self.devices = Vec::new();
                self.devices_error = Some(error);
            }
        }
        match inputs {
            Ok(inputs) => {
                self.inputs = inputs;
                self.inputs_error = None;
            }
            Err(error) => {
                self.inputs = Vec::new();
                self.inputs_error = Some(error);
            }
        }
        self.error.clear();
        self.custom_fps = !FPS_CHOICES
            .iter()
            .any(|(fps, _)| *fps == Some(settings.fps));
        self.open = true;
    }
}

/// The three radio buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayKind {
    Default,
    Custom,
    Disabled,
}

impl RelayKind {
    pub fn of(choice: &RelayChoice) -> Self {
        match choice {
            RelayChoice::Default => Self::Default,
            RelayChoice::Custom(_) => Self::Custom,
            RelayChoice::Disabled => Self::Disabled,
        }
    }
}

/// The choice after a radio click. Switching to custom keeps a URL that was already typed and
/// starts empty otherwise; switching away keeps nothing, since the draft is discarded on Cancel.
pub fn switch_relay(current: &RelayChoice, kind: RelayKind) -> RelayChoice {
    match (kind, current) {
        (RelayKind::Custom, RelayChoice::Custom(url)) => RelayChoice::Custom(url.clone()),
        (RelayKind::Custom, _) => RelayChoice::Custom(String::new()),
        (RelayKind::Default, _) => RelayChoice::Default,
        (RelayKind::Disabled, _) => RelayChoice::Disabled,
    }
}

/// What the combo box shows for the draft's device: its name when listed, the id with a note when
/// it is saved but absent, or the default.
pub fn device_label(selected: Option<&str>, devices: &[AudioDevice]) -> String {
    match selected {
        None => "System default".to_string(),
        Some(id) => devices
            .iter()
            .find(|d| d.id == id)
            .map(|d| d.name.clone())
            .unwrap_or_else(|| format!("{id} (not present)")),
    }
}

/// A dropdown of `System default` and `devices` bound to `selected`, with `error` under it.
fn device_picker(
    ui: &mut egui::Ui,
    id_salt: &str,
    selected: &mut Option<String>,
    devices: &[AudioDevice],
    error: Option<String>,
) {
    let label = device_label(selected.as_deref(), devices);
    let width = ui.available_width();
    theme::dropdown(ui, id_salt, label, Some(width), |ui| {
        ui.selectable_value(selected, None, "System default");
        for device in devices {
            ui.selectable_value(selected, Some(device.id.clone()), &device.name);
        }
    });
    if let Some(error) = error {
        ui.colored_label(theme::error(), error);
    }
}

/// Draws the dialog when open. Returns the settings to persist when Save was clicked and the
/// draft validates; the caller saves and reopens the dialog with the error if that fails.
pub fn draw(ctx: &egui::Context, dialog: &mut SettingsDialog, room_open: bool) -> Option<Settings> {
    let mut saved = None;
    let visible = dialog.open;
    let (_, dismissed) = theme::dialog(ctx, "settings", "Settings", visible, |ui| {
        egui::ScrollArea::vertical()
            .max_height((ctx.content_rect().height() - 200.0).clamp(120.0, 560.0))
            .auto_shrink([false, true])
            .show(ui, |ui| {
                theme::form_row(ui, "Your name", |ui| {
                    theme::field(
                        ui,
                        egui::TextEdit::singleline(&mut dialog.nickname_text),
                        theme::CONTROL_HEIGHT,
                    );
                });
                ui.add_space(ROW_GAP);
                theme::form_row(ui, "Connection", |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let mut kind = RelayKind::of(&dialog.draft.relay);
                    let before = kind;
                    theme::radio(ui, &mut kind, RelayKind::Default, "Public relays");
                    theme::radio(ui, &mut kind, RelayKind::Custom, "Custom relay URL");
                    theme::radio(ui, &mut kind, RelayKind::Disabled, "No relay (LAN only)");
                    if kind != before {
                        dialog.draft.relay = switch_relay(&dialog.draft.relay, kind);
                    }
                    if let RelayChoice::Custom(url) = &mut dialog.draft.relay {
                        ui.add_space(4.0);
                        theme::field(
                            ui,
                            egui::TextEdit::singleline(url).hint_text("https://relay.example.com/"),
                            theme::CONTROL_HEIGHT,
                        );
                    }
                });
                ui.add_space(ROW_GAP);
                theme::form_row(ui, "Invite links", |ui| {
                    theme::switch(ui, &mut dialog.draft.auto_join_links, "Join without asking");
                    ui.label(
                        egui::RichText::new("Any link you open can then put you in a room")
                            .small()
                            .color(theme::secondary()),
                    );
                });
                ui.add_space(ROW_GAP);
                theme::form_row(ui, "Voice", |ui| {
                    theme::switch(ui, &mut dialog.draft.auto_join_voice, "Join voice automatically");
                    ui.label(
                        egui::RichText::new("Your microphone goes live in every room you enter")
                            .small()
                            .color(theme::secondary()),
                    );
                });
                ui.add_space(ROW_GAP);
                theme::form_row(ui, "Frame rate ceiling", |ui| {
                    let selected = (!dialog.custom_fps).then_some(dialog.draft.fps);
                    let choice = theme::segmented(
                        ui,
                        "settings-fps",
                        &FPS_CHOICES,
                        selected,
                        ui.available_width() / 3.0,
                    );
                    match choice {
                        Some(Some(fps)) => {
                            dialog.draft.fps = fps;
                            dialog.custom_fps = false;
                        }
                        Some(None) => dialog.custom_fps = true,
                        None => {}
                    }
                    motion::unfold(ui, "settings-custom-fps", dialog.custom_fps, |ui| {
                        ui.add_space(4.0);
                        theme::drag_value(
                            ui,
                            egui::DragValue::new(&mut dialog.draft.fps)
                                .range(1..=u32::MAX)
                                .suffix(" fps"),
                        );
                    });
                });
                ui.add_space(ROW_GAP);
                theme::form_row(ui, "Microphone", |ui| {
                    let error = dialog.inputs_error.as_ref().map(|error| {
                        format!(
                            "Could not list microphones. Check your audio system, then reopen \
                 Settings. ({error})"
                        )
                    });
                    device_picker(
                        ui,
                        "settings-input-device",
                        &mut dialog.draft.audio.input_device,
                        &dialog.inputs,
                        error,
                    );
                });
                ui.add_space(ROW_GAP);
                theme::form_row(ui, "Echo cancellation", |ui| {
                    theme::switch(
                        ui,
                        &mut dialog.draft.audio.echo_cancellation,
                        "Cancel what your speakers play",
                    );
                    ui.label(
                        egui::RichText::new(
                            "Not needed with headphones, or when your system already cancels echo",
                        )
                        .small()
                        .color(theme::secondary()),
                    );
                });
                ui.add_space(ROW_GAP);
                theme::form_row(ui, "Noise suppression", |ui| {
                    let selected = dialog.draft.audio.noise_suppression;
                    if let Some(choice) = theme::segmented(
                        ui,
                        "settings-noise-suppression",
                        &SUPPRESSION_CHOICES,
                        selected,
                        ui.available_width() / 3.0,
                    ) {
                        dialog.draft.audio.noise_suppression = choice;
                    }
                    let hint = match dialog.draft.audio.noise_suppression {
                        NoiseSuppression::Off => "Your microphone as it is",
                        NoiseSuppression::Light => {
                            "Removes steady noise such as fans and hum, at almost no CPU cost"
                        }
                    };
                    ui.label(egui::RichText::new(hint).small().color(theme::secondary()));
                });
                ui.add_space(ROW_GAP);
                theme::form_row(ui, "Automatic gain", |ui| {
                    theme::switch(
                        ui,
                        &mut dialog.draft.audio.automatic_gain,
                        "Even out your volume",
                    );
                });
                ui.add_space(ROW_GAP);
                theme::form_row(ui, "Audio output", |ui| {
                    let label =
                        device_label(dialog.draft.audio.output_device.as_deref(), &dialog.devices);
                    let width = ui.available_width();
                    theme::dropdown(ui, "settings-output-device", label, Some(width), |ui| {
                        ui.selectable_value(
                            &mut dialog.draft.audio.output_device,
                            None,
                            "System default",
                        );
                        for device in &dialog.devices {
                            ui.selectable_value(
                                &mut dialog.draft.audio.output_device,
                                Some(device.id.clone()),
                                &device.name,
                            );
                        }
                    });
                    if let Some(error) = &dialog.devices_error {
                        ui.colored_label(
                            theme::error(),
                            format!("Could not list output devices. Check your audio system, then reopen \
                 Settings. ({error})"),
                        );
                    }
                });
                ui.add_space(ROW_GAP);
                theme::form_row(ui, "Voice output", |ui| {
                    let mut same = !dialog.draft.audio.separate_voice_output;
                    theme::switch(ui, &mut same, "Same as audio output");
                    dialog.draft.audio.separate_voice_output = !same;
                    motion::unfold(
                        ui,
                        "settings-voice-output",
                        dialog.draft.audio.separate_voice_output,
                        |ui| {
                            ui.add_space(4.0);
                            device_picker(
                                ui,
                                "settings-voice-output-device",
                                &mut dialog.draft.audio.voice_output_device,
                                &dialog.devices,
                                None,
                            );
                        },
                    );
                });
                ui.add_space(ROW_GAP);
                theme::form_row(ui, "Updates", |ui| {
                    theme::switch(
                        ui,
                        &mut dialog.draft.check_updates,
                        "Check for updates at launch",
                    );
                    ui.label(
                        egui::RichText::new("Takes effect the next time OpenStream starts")
                            .small()
                            .color(theme::secondary()),
                    );
                });
                if room_open {
                    ui.add_space(ROW_GAP);
                    ui.weak(
                        "Changes apply to the next room you open. Voice devices and processing \
                         apply the next time you join voice.",
                    );
                }
            });
        // Outside the scroll area, because the row Save rejects can be scrolled out of view.
        if !dialog.error.is_empty() {
            ui.add_space(ROW_GAP);
            ui.colored_label(theme::error(), &dialog.error);
        }
        theme::dialog_footer(
            ui,
            |_| {},
            |ui| {
                if theme::primary_button(ui, "Save changes", true).clicked() {
                    dialog.draft.nickname = normalised_nickname(&dialog.nickname_text);
                    match dialog.draft.validate() {
                        Ok(()) => {
                            saved = Some(dialog.draft.clone());
                            dialog.open = false;
                        }
                        Err(error) => dialog.error = error,
                    }
                }
                if theme::text_button(ui, "Cancel", true).clicked() {
                    dialog.open = false;
                }
            },
        );
    })?;
    // The title bar's close button and Esc behave like Cancel.
    dialog.open &= !dismissed;
    saved
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switching_relay_kinds_keeps_a_typed_url_only_while_custom() {
        let typed = RelayChoice::Custom("https://r.example/".into());
        assert_eq!(switch_relay(&typed, RelayKind::Custom), typed);
        assert_eq!(
            switch_relay(&typed, RelayKind::Disabled),
            RelayChoice::Disabled
        );
        assert_eq!(
            switch_relay(&RelayChoice::Default, RelayKind::Custom),
            RelayChoice::Custom(String::new())
        );
        assert_eq!(RelayKind::of(&typed), RelayKind::Custom);
    }

    #[test]
    fn the_device_label_names_listed_devices_and_flags_absent_ones() {
        let devices = vec![AudioDevice {
            id: "host:abc".into(),
            name: "Speakers".into(),
        }];
        assert_eq!(device_label(None, &devices), "System default");
        assert_eq!(device_label(Some("host:abc"), &devices), "Speakers");
        assert_eq!(
            device_label(Some("host:gone"), &devices),
            "host:gone (not present)"
        );
    }

    #[test]
    fn a_saved_frame_rate_outside_the_choices_opens_as_custom() {
        let mut dialog = SettingsDialog::default();
        let mut settings = Settings {
            fps: 144,
            ..Settings::default()
        };
        dialog.open_with(&settings, Ok(Vec::new()), Ok(Vec::new()));
        assert!(dialog.custom_fps);
        settings.fps = 30;
        dialog.open_with(&settings, Ok(Vec::new()), Ok(Vec::new()));
        assert!(!dialog.custom_fps);
    }

    #[test]
    fn opening_takes_a_fresh_draft_and_clears_old_errors() {
        let mut dialog = SettingsDialog {
            error: "old".into(),
            ..Default::default()
        };
        let settings = Settings {
            fps: 24,
            ..Settings::default()
        };
        dialog.open_with(&settings, Err("no host".into()), Ok(Vec::new()));
        assert!(dialog.open);
        assert_eq!(dialog.draft, settings);
        assert!(dialog.error.is_empty());
        assert_eq!(dialog.devices_error.as_deref(), Some("no host"));
        assert!(dialog.devices.is_empty());
    }

    /// The text drawn while the dialog is scrolled from the top to the bottom: every string in the
    /// order first seen, and the strings of the last frame (the bottom of the dialog).
    fn shown_text(dialog: &mut SettingsDialog, room_open: bool) -> (Vec<String>, Vec<String>) {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut seen: Vec<String> = Vec::new();
        let mut last = Vec::new();
        for frame in 0..120 {
            let scroll = frame >= 60;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(900.0, 1400.0),
                    )),
                    time: Some(f64::from(frame) / 60.0),
                    events: if scroll {
                        vec![
                            egui::Event::PointerMoved(egui::pos2(450.0, 700.0)),
                            egui::Event::MouseWheel {
                                unit: egui::MouseWheelUnit::Point,
                                delta: egui::vec2(0.0, -400.0),
                                phase: egui::TouchPhase::Move,
                                modifiers: egui::Modifiers::NONE,
                            },
                        ]
                    } else {
                        Vec::new()
                    },
                    ..Default::default()
                },
                |ui| {
                    draw(ui.ctx(), dialog, room_open);
                },
            );
            output.textures_delta.clear();
            last = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) => Some(text.galley.text().to_string()),
                    _ => None,
                })
                .collect();
            for text in &last {
                if !seen.contains(text) {
                    seen.push(text.clone());
                }
            }
        }
        (seen, last)
    }

    fn opened() -> SettingsDialog {
        let mut dialog = SettingsDialog::default();
        dialog.open_with(&Settings::default(), Ok(Vec::new()), Ok(Vec::new()));
        dialog
    }

    #[test]
    fn the_rows_come_in_the_specs_order() {
        let (seen, _) = shown_text(&mut opened(), false);
        let labels = [
            "Your name",
            "Connection",
            "Invite links",
            "Voice",
            "Frame rate ceiling",
            "Microphone",
            "Echo cancellation",
            "Noise suppression",
            "Automatic gain",
            "Audio output",
            "Voice output",
            "Updates",
        ];
        let positions: Vec<Option<usize>> = labels
            .iter()
            .map(|label| seen.iter().position(|t| t == label))
            .collect();
        assert!(positions.iter().all(Option::is_some), "{positions:?}");
        assert!(positions.is_sorted(), "{positions:?}");
    }

    #[test]
    fn the_second_output_picker_appears_only_when_voice_is_separated() {
        let pickers = |dialog: &mut SettingsDialog| {
            let (_, last) = shown_text(dialog, false);
            let from = last.iter().position(|t| t == "Voice output").unwrap();
            let to = last.iter().position(|t| t == "Updates").unwrap();
            last[from..to]
                .iter()
                .filter(|t| *t == "System default")
                .count()
        };
        let mut dialog = opened();
        assert_eq!(pickers(&mut dialog), 0);
        dialog.draft.audio.separate_voice_output = true;
        assert_eq!(pickers(&mut dialog), 1);
    }

    #[test]
    fn the_footnote_says_voice_devices_apply_on_the_next_join() {
        let has_note = |room_open| {
            shown_text(&mut opened(), room_open).1.iter().any(|t| {
                t.contains("Voice devices and processing apply the next time you join voice.")
            })
        };
        assert!(has_note(true));
        assert!(!has_note(false));
    }

    #[test]
    fn the_suppression_hint_follows_the_choice() {
        let hints = |choice| {
            let mut dialog = opened();
            dialog.draft.audio.noise_suppression = choice;
            shown_text(&mut dialog, false).0
        };
        let off = hints(NoiseSuppression::Off);
        assert!(off.iter().any(|t| t == "Your microphone as it is"));
        assert!(!off.iter().any(|t| t.starts_with("Removes steady noise")));
        let light = hints(NoiseSuppression::Light);
        assert!(
            light
                .iter()
                .any(|t| t == "Removes steady noise such as fans and hum, at almost no CPU cost")
        );
        assert!(!light.iter().any(|t| t == "Your microphone as it is"));
    }
}

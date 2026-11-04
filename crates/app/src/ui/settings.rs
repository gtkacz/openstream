//! The Settings dialog: edits a draft copy of the saved settings and hands it back on Save.
//! Changes apply when the next room opens; the dialog says so while a room is open.

use brp_audio::OutputDevice;

use crate::settings::{RelayChoice, Settings, normalised_nickname};

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
    pub devices: Vec<OutputDevice>,
    pub devices_error: Option<String>,
    pub error: String,
}

impl SettingsDialog {
    /// Opens with a fresh draft of `settings` and the device list as enumerated now.
    pub fn open_with(&mut self, settings: &Settings, devices: Result<Vec<OutputDevice>, String>) {
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
        self.error.clear();
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
pub fn device_label(selected: Option<&str>, devices: &[OutputDevice]) -> String {
    match selected {
        None => "System default".to_string(),
        Some(id) => devices
            .iter()
            .find(|d| d.id == id)
            .map(|d| d.name.clone())
            .unwrap_or_else(|| format!("{id} (not present)")),
    }
}

/// Draws the dialog when open. Returns the settings to persist when Save was clicked and the
/// draft validates; the caller saves and reopens the dialog with the error if that fails.
pub fn draw(ctx: &egui::Context, dialog: &mut SettingsDialog, room_open: bool) -> Option<Settings> {
    if !dialog.open {
        return None;
    }
    let mut saved = None;
    let (_, dismissed) = super::theme::dialog(ctx, "settings", "Settings", |ui| {
        egui::ScrollArea::vertical()
            .max_height((ctx.content_rect().height() - 200.0).clamp(120.0, 560.0))
            .auto_shrink([false, true])
            .show(ui, |ui| {
                super::theme::section_heading(ui, "Your name");
                ui.add(
                    egui::TextEdit::singleline(&mut dialog.nickname_text)
                        .margin(egui::vec2(12.0, 10.0))
                        .desired_width(f32::INFINITY),
                );
                ui.add_space(16.0);
                super::theme::section_heading(ui, "Connection");
                let mut kind = RelayKind::of(&dialog.draft.relay);
                let before = kind;
                ui.radio_value(&mut kind, RelayKind::Default, "Public relays");
                ui.radio_value(&mut kind, RelayKind::Custom, "Custom relay URL");
                ui.radio_value(&mut kind, RelayKind::Disabled, "No relay (LAN only)");
                if kind != before {
                    dialog.draft.relay = switch_relay(&dialog.draft.relay, kind);
                }
                if let RelayChoice::Custom(url) = &mut dialog.draft.relay {
                    ui.add(
                        egui::TextEdit::singleline(url)
                            .hint_text("https://relay.example.com/")
                            .margin(egui::vec2(12.0, 10.0))
                            .desired_width(f32::INFINITY),
                    );
                }
                ui.add_space(16.0);
                super::theme::section_heading(ui, "Frame rate ceiling");
                ui.horizontal_wrapped(|ui| {
                    super::theme::uniform_controls(ui);
                    ui.add(
                        egui::DragValue::new(&mut dialog.draft.fps)
                            .range(1..=u32::MAX)
                            .suffix(" fps"),
                    );
                    for fps in [30, 60] {
                        if ui
                            .add(
                                egui::Button::new(format!("{fps} fps"))
                                    .selected(dialog.draft.fps == fps),
                            )
                            .clicked()
                        {
                            dialog.draft.fps = fps;
                        }
                    }
                });
                ui.add_space(16.0);
                super::theme::section_heading(ui, "Audio output");
                let label =
                    device_label(dialog.draft.audio.output_device.as_deref(), &dialog.devices);
                egui::ComboBox::from_id_salt("settings-output-device")
                    .selected_text(label)
                    .width(ui.available_width())
                    .icon(super::theme::chevron)
                    .show_ui(ui, |ui| {
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
                ui.add_space(16.0);
                super::theme::section_heading(ui, "Updates");
                super::theme::switch(
                    ui,
                    &mut dialog.draft.check_updates,
                    "Check for updates at launch",
                );
                ui.weak("Takes effect the next time OpenStream starts");
                if let Some(error) = &dialog.devices_error {
                    ui.colored_label(
                        super::theme::ERROR,
                        format!("Could not list output devices: {error}"),
                    );
                }
                if room_open {
                    ui.weak("Changes apply to the next room you open");
                }
                if !dialog.error.is_empty() {
                    ui.colored_label(super::theme::ERROR, &dialog.error);
                }
            });
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if super::theme::primary_button(ui, "Save changes", true).clicked() {
                dialog.draft.nickname = normalised_nickname(&dialog.nickname_text);
                match dialog.draft.validate() {
                    Ok(()) => {
                        saved = Some(dialog.draft.clone());
                        dialog.open = false;
                    }
                    Err(error) => dialog.error = error,
                }
            }
            if ui
                .add(egui::Button::new("Cancel").min_size(egui::vec2(0.0, 36.0)))
                .clicked()
            {
                dialog.open = false;
            }
        });
    });
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
        let devices = vec![OutputDevice {
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
    fn opening_takes_a_fresh_draft_and_clears_old_errors() {
        let mut dialog = SettingsDialog {
            error: "old".into(),
            ..Default::default()
        };
        let settings = Settings {
            fps: 24,
            ..Settings::default()
        };
        dialog.open_with(&settings, Err("no host".into()));
        assert!(dialog.open);
        assert_eq!(dialog.draft, settings);
        assert!(dialog.error.is_empty());
        assert_eq!(dialog.devices_error.as_deref(), Some("no host"));
        assert!(dialog.devices.is_empty());
    }
}

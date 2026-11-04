//! The dock under the video: the share and audio controls, upload rate, master mute, Settings, and
//! whatever notice or error needs saying. In a compact window it also opens the Room and Share
//! sheets, which hold what the rail holds in a wide one.

use brp_room::RoomSnapshot;

use super::own_lives;
use super::state::{CompactSheet, UiState};
use super::theme::{self, icons};
use super::update::{self, UpdateState};
use crate::commands::RoomCommand;

/// Draws the dock. The master mute toggle emits `SetMasterMute`; the Settings button sets
/// `open_settings`; the Update button sets `update_clicked`.
// One parameter per thing the bar shows; a struct would be built and unpacked in one place.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    state: &mut UiState,
    commands: &mut Vec<RoomCommand>,
    open_settings: &mut bool,
    update_state: &UpdateState,
    update_clicked: &mut bool,
) {
    let compact = theme::is_compact(ui.ctx());
    egui::Panel::bottom("status")
        .frame(
            egui::Frame::NONE
                .fill(theme::GROUND)
                .inner_margin(egui::Margin::symmetric(12, 10)),
        )
        .show(ui, |ui| {
            notices(ui, snapshot, state, update_state, update_clicked);
            if compact {
                compact_row(ui, snapshot, state, commands, open_settings);
            } else {
                wide_rows(ui, snapshot, state, commands, open_settings);
            }
        });
}

fn compact_row(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    state: &mut UiState,
    commands: &mut Vec<RoomCommand>,
    open_settings: &mut bool,
) {
    theme::split_row(
        ui,
        theme::CONTROL_HEIGHT,
        |ui| {
            sheet_button(ui, state, CompactSheet::Room, icons::ROOM, "Room");
            sheet_button(
                ui,
                state,
                CompactSheet::Share,
                icons::SHARE_MONITOR,
                "Share",
            );
            // The Share sheet that says this may be closed, and the portal takes a moment to open.
            if state.share_pending {
                theme::spinner(ui, 16.0);
            }
        },
        |ui| {
            settings_button(ui, open_settings);
            master_mute(ui, snapshot, commands, false);
        },
    );
}

/// One row when everything fits, otherwise the audio controls drop to a second row. The widths are
/// the labelled controls measured with Chakra Petch, with the Stop sharing button showing.
fn wide_rows(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    state: &UiState,
    commands: &mut Vec<RoomCommand>,
    open_settings: &mut bool,
) {
    const ONE_ROW_WIDTH: f32 = 1200.0;
    const ONE_ROW_WITHOUT_STATUS_WIDTH: f32 = 960.0;
    const LABELLED_ROW_WIDTH: f32 = 640.0;
    let width = ui.available_width();
    let one_row = width >= ONE_ROW_WITHOUT_STATUS_WIDTH;
    let labelled = width >= LABELLED_ROW_WIDTH;
    let show_audio_status = width >= ONE_ROW_WIDTH || !one_row;
    let mut mute_commands = Vec::new();
    theme::split_row(
        ui,
        theme::CONTROL_HEIGHT,
        |ui| {
            own_lives::share_buttons(ui, snapshot, state, commands, labelled);
            if one_row {
                ui.add_space(8.0);
                own_lives::audio_controls(ui, snapshot, commands, show_audio_status);
            }
        },
        |ui| {
            settings_button(ui, open_settings);
            master_mute(ui, snapshot, &mut mute_commands, labelled);
            if one_row {
                upload(ui, state);
            }
        },
    );
    commands.extend(mute_commands);
    if !one_row {
        theme::split_row(
            ui,
            theme::CONTROL_HEIGHT,
            |ui| own_lives::audio_controls(ui, snapshot, commands, show_audio_status),
            |ui| upload(ui, state),
        );
    }
}

fn settings_button(ui: &mut egui::Ui, open_settings: &mut bool) {
    if theme::icon_button(ui, icons::SETTINGS, "Settings", true).clicked() {
        *open_settings = true;
    }
}

fn notices(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    state: &UiState,
    update_state: &UpdateState,
    update_clicked: &mut bool,
) {
    let has_notice = !state.status.is_empty()
        || update_state.available.is_some()
        || snapshot.audio_output_error.is_some();
    if !has_notice {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        if !state.status.is_empty() {
            ui.weak(state.status.as_str());
        }
        if update::draw(ui, update_state) {
            *update_clicked = true;
        }
        if let Some(error) = &snapshot.audio_output_error {
            ui.colored_label(theme::ERROR, format!("No audio output: {error}"));
        }
    });
    ui.add_space(4.0);
}

fn master_mute(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    commands: &mut Vec<RoomCommand>,
    labelled: bool,
) {
    let mut muted = snapshot.master_mute;
    let glyph = if muted { icons::MUTED } else { icons::VOLUME };
    let response = if labelled {
        theme::toggle(ui, &mut muted, glyph, "Mute all")
    } else {
        theme::icon_toggle(ui, &mut muted, glyph, "Mute all")
    };
    if response.changed() {
        commands.push(RoomCommand::SetMasterMute(muted));
    }
}

fn upload(ui: &mut egui::Ui, state: &UiState) {
    ui.label(theme::figure(format!("{} kbps", state.upload_kbps)))
        .on_hover_text("What you are sending to the room");
    ui.label(theme::icon(icons::UPLOAD).color(theme::MUTED));
}

/// Clicking the open sheet's button closes it; clicking the other's switches straight to it.
fn sheet_button(
    ui: &mut egui::Ui,
    state: &mut UiState,
    sheet: CompactSheet,
    glyph: &str,
    text: &str,
) {
    let mut selected = state.sheet_open && state.sheet == sheet;
    if theme::toggle(ui, &mut selected, glyph, text).changed() {
        state.sheet = sheet;
        state.sheet_open = selected;
    }
}

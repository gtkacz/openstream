//! Bottom status bar: ticket and link, member count, upload rate, identity, last notice.

use brp_room::RoomSnapshot;

use super::state::UiState;
use super::update::{self, UpdateState};
use crate::commands::RoomCommand;
use crate::link;

/// Draws the bottom status bar. The two copy buttons write to the clipboard directly rather than
/// queueing a `RoomCommand`; the master mute toggle emits `SetMasterMute`; the Settings button
/// sets `open_settings`; the Update button sets `update_clicked`.
// One parameter per thing the bar shows; a struct would be built and unpacked in one place.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    ticket: &str,
    state: &UiState,
    commands: &mut Vec<RoomCommand>,
    open_settings: &mut bool,
    update_state: &UpdateState,
    update_clicked: &mut bool,
) {
    egui::Panel::bottom("status")
        .frame(
            egui::Frame::NONE
                .fill(super::theme::GROUND)
                .inner_margin(egui::Margin::symmetric(16, 8)),
        )
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                copy_button(ui, "Copy link", "Link copied", || link::share_link(ticket));
                copy_button(ui, "Copy ticket", "Ticket copied", || ticket.to_string());
                ui.add_space(8.0);
                // `snapshot.members` excludes this participant, so count it in.
                let members = snapshot.members.len() + 1;
                let plural = if members == 1 { "" } else { "s" };
                ui.label(format!("{members} member{plural}"));
                ui.add_space(8.0);
                ui.weak(format!("Upload {} kbps", state.upload_kbps));
                ui.add_space(8.0);
                ui.weak(format!(
                    "{} · {}",
                    snapshot.nickname,
                    snapshot.me.fmt_short()
                ));
                if !state.status.is_empty() {
                    ui.add_space(8.0);
                    ui.weak(state.status.as_str());
                }
                ui.add_space(8.0);
                let mut muted = snapshot.master_mute;
                if ui.toggle_value(&mut muted, "Mute all").changed() {
                    commands.push(RoomCommand::SetMasterMute(muted));
                }
                ui.add_space(8.0);
                if ui.button("Settings").clicked() {
                    *open_settings = true;
                }
                if update_state.available.is_some() {
                    ui.add_space(8.0);
                    if update::draw(ui, update_state) {
                        *update_clicked = true;
                    }
                }
                if let Some(error) = &snapshot.audio_output_error {
                    ui.add_space(8.0);
                    ui.colored_label(super::theme::ERROR, format!("no audio output: {error}"));
                }
            });
        });
}

fn copy_button(ui: &mut egui::Ui, label: &str, copied: &str, text: impl FnOnce() -> String) {
    let id = ui.make_persistent_id(("clipboard-feedback", label));
    let now = ui.input(|input| input.time);
    let until = ui
        .ctx()
        .data(|data| data.get_temp::<f64>(id))
        .unwrap_or(0.0);
    let remaining = until - now;
    let response = ui.add(
        egui::Button::new(if remaining > 0.0 { copied } else { label })
            .min_size(egui::vec2(112.0, 32.0)),
    );
    if response.clicked() {
        ui.ctx().copy_text(text());
        ui.ctx().data_mut(|data| data.insert_temp(id, now + 1.5));
        ui.ctx().request_repaint();
    } else if remaining > 0.0 {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs_f64(remaining));
    }
}

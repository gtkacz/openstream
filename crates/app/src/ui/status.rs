//! Bottom status bar: ticket, member count, upload rate, identity, last notice.

use brp_room::RoomSnapshot;

use super::state::UiState;

/// Draws the bottom status bar. Reads only; the ticket copy button is the sole side effect,
/// applied directly to the clipboard rather than queued as a `RoomCommand`.
pub fn draw(ui: &mut egui::Ui, snapshot: &RoomSnapshot, ticket: &str, state: &UiState) {
    egui::Panel::bottom("status").show(ui, |ui| {
        ui.horizontal(|ui| {
            if ui.button("Copy ticket").clicked() {
                ui.ctx().copy_text(ticket.to_string());
            }
            ui.separator();
            // `snapshot.members` excludes this participant, so count it in.
            let members = snapshot.members.len() + 1;
            let plural = if members == 1 { "" } else { "s" };
            ui.label(format!("{members} member{plural}"));
            ui.separator();
            ui.label(format!("up {} kbps", state.upload_kbps));
            ui.separator();
            ui.label(format!(
                "{} ({})",
                snapshot.nickname,
                snapshot.me.fmt_short()
            ));
            if !state.status.is_empty() {
                ui.separator();
                ui.weak(state.status.as_str());
            }
        });
    });
}

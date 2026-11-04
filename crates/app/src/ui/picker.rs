//! The source picker: a centred window listing what the platform can share, drawn only on
//! platforms without a picker of their own.

use brp_proto::SourceKind;

use super::state::{SourcePicker, UiState};
use crate::commands::RoomCommand;

const MAX_LIST_HEIGHT: f32 = 400.0;

/// Draws the picker when one is open and pushes the share command for a chosen source.
pub fn draw(ctx: &egui::Context, state: &mut UiState, commands: &mut Vec<RoomCommand>) {
    // Kept after the picker closes, so the dialog can leave still showing what it showed.
    let leaving_id = egui::Id::new("source-picker-content");
    let open = state.picker.is_some();
    if let Some(picker) = &state.picker {
        ctx.data_mut(|data| data.insert_temp(leaving_id, picker.clone()));
    }
    let Some(picker) = ctx.data(|data| data.get_temp::<SourcePicker>(leaving_id)) else {
        return;
    };
    let title = match picker.kind {
        SourceKind::Monitor => "Share a monitor",
        SourceKind::Window => "Share a window",
    };
    let mut picked = None;
    let Some((cancelled, dismissed)) =
        super::theme::dialog(ctx, "source-picker", title, open, |ui| {
            if picker.choices.is_empty() {
                ui.weak("No screens or windows are available to share");
            }
            egui::ScrollArea::vertical()
                .max_height((ctx.content_rect().height() - 200.0).clamp(120.0, MAX_LIST_HEIGHT))
                .show(ui, |ui| {
                    for choice in &picker.choices {
                        let label =
                            format!("{} ({} × {})", choice.name, choice.width, choice.height);
                        let size = egui::vec2(ui.available_width(), super::theme::CONTROL_HEIGHT);
                        let row = super::theme::tactile(
                            ui,
                            super::theme::Tone::Quiet,
                            false,
                            true,
                            |ui| ui.add(egui::Button::new(label).min_size(size)),
                        );
                        if row.clicked() {
                            picked = Some(choice.id);
                        }
                    }
                });
            ui.add_space(8.0);
            super::theme::text_button(ui, "Cancel", true).clicked()
        })
    else {
        return;
    };
    if !open {
        return;
    }
    if let Some(id) = picked {
        commands.extend(state.pick_source(id));
    } else if cancelled || dismissed {
        state.cancel_picker();
    }
}

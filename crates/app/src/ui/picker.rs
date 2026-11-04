//! The source picker: a centred window listing what the platform can share, drawn only on
//! platforms without a picker of their own.

use brp_proto::SourceKind;

use super::state::UiState;
use crate::commands::RoomCommand;

const MAX_LIST_HEIGHT: f32 = 400.0;

/// Draws the picker when one is open and pushes the share command for a chosen source.
pub fn draw(ctx: &egui::Context, state: &mut UiState, commands: &mut Vec<RoomCommand>) {
    // Cloned so the window closure does not borrow `state` while it draws.
    let Some(picker) = state.picker.clone() else {
        return;
    };
    let title = match picker.kind {
        SourceKind::Monitor => "Share a monitor",
        SourceKind::Window => "Share a window",
    };
    let mut picked = None;
    let (cancelled, dismissed) = super::theme::dialog(ctx, "source-picker", title, |ui| {
        if picker.choices.is_empty() {
            ui.weak("No screens or windows are available to share");
        }
        egui::ScrollArea::vertical()
            .max_height((ctx.content_rect().height() - 200.0).clamp(120.0, MAX_LIST_HEIGHT))
            .show(ui, |ui| {
                for choice in &picker.choices {
                    let label = format!("{} ({} × {})", choice.name, choice.width, choice.height);
                    if ui
                        .add(
                            egui::Button::selectable(false, label)
                                .min_size(egui::vec2(ui.available_width(), 36.0)),
                        )
                        .clicked()
                    {
                        picked = Some(choice.id);
                    }
                }
            });
        ui.add_space(8.0);
        ui.button("Cancel").clicked()
    });
    let cancelled = cancelled || dismissed;
    if let Some(id) = picked {
        commands.extend(state.pick_source(id));
    } else if cancelled {
        state.cancel_picker();
    }
}

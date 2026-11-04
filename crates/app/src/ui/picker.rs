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
    let mut cancelled = false;
    egui::Window::new(title)
        .collapsible(false)
        .resizable(false)
        .default_width(400.0)
        .max_width((ctx.content_rect().width() - 64.0).max(160.0))
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            if picker.choices.is_empty() {
                ui.weak("No screens or windows are available to share");
            }
            egui::ScrollArea::vertical()
                .max_height(MAX_LIST_HEIGHT)
                .show(ui, |ui| {
                    for choice in &picker.choices {
                        let label = format!("{} ({}x{})", choice.name, choice.width, choice.height);
                        if ui.selectable_label(false, label).clicked() {
                            picked = Some(choice.id);
                        }
                    }
                });
            ui.add_space(16.0);
            if ui.button("Cancel").clicked() {
                cancelled = true;
            }
        });
    if let Some(id) = picked {
        commands.extend(state.pick_source(id));
    } else if cancelled {
        state.cancel_picker();
    }
}

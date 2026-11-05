//! The source picker: a centred window listing what the platform can share, drawn only on
//! platforms without a picker of their own.

use brp_proto::SourceKind;

use super::state::{SourcePicker, UiState};
use super::theme;
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
    let (title, nothing_to_share) = match picker.kind {
        SourceKind::Monitor => (
            "Share a monitor",
            "No monitors are available to share. Check that one is connected, then try again.",
        ),
        SourceKind::Window => (
            "Share a window",
            "No windows are available to share. Open the one you want, then try again.",
        ),
    };
    let glyph = match picker.kind {
        SourceKind::Monitor => theme::icons::SHARE_MONITOR,
        SourceKind::Window => theme::icons::SHARE_WINDOW,
    };
    let mut picked = None;
    let mut cancelled = false;
    let Some((_, dismissed)) = theme::dialog(ctx, "source-picker", title, open, |ui| {
        if picker.choices.is_empty() {
            ui.weak(nothing_to_share);
        }
        egui::ScrollArea::vertical()
            .max_height((ctx.content_rect().height() - 200.0).clamp(120.0, MAX_LIST_HEIGHT))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                for choice in &picker.choices {
                    let size = egui::vec2(ui.available_width(), theme::CONTROL_HEIGHT);
                    let dimensions =
                        egui::RichText::new(format!("{} × {}", choice.width, choice.height))
                            .small()
                            .color(theme::secondary());
                    let row = theme::tactile(ui, theme::Tone::Quiet, false, true, |ui| {
                        ui.add(
                            egui::Button::new((
                                theme::icon(glyph),
                                choice.name.as_str(),
                                egui::Atom::grow(),
                                dimensions,
                            ))
                            .min_size(size),
                        )
                    });
                    if row.clicked() {
                        picked = Some(choice.id);
                    }
                }
            });
        theme::dialog_footer(
            ui,
            |_| {},
            |ui| {
                cancelled = theme::text_button(ui, "Cancel", true).clicked();
            },
        );
    }) else {
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

//! Central panel: reserves one rect per watched live for the video renderer and draws the hover
//! overlay and status text on top. The panel frame is transparent so the tiles show through. The
//! overlay is shared with pop-out windows, which place one live over their whole panel.

use std::collections::HashSet;

use brp_room::{RoomSnapshot, WatchState, WatchView};

use super::members::{offered_preset, preset_selector};
use super::state::{UiState, live_title, visible_watches};
use crate::commands::{RoomCommand, WindowCommand};
use crate::render::grid;
use crate::render::tiles::TileKey;

/// Where an overlay is drawn, which decides its window buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// A cell of the main window's grid: offers pop out and fullscreen.
    Grid,
    /// A pop-out window: offers the fullscreen toggle and a return to the grid.
    PopOut { fullscreen: bool },
}

/// Draws the tile grid without the popped-out lives, returning where the video renderer should
/// place each drawn live's frame, in egui points.
pub fn draw(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    state: &mut UiState,
    popped: &HashSet<TileKey>,
    commands: &mut Vec<RoomCommand>,
    window_commands: &mut Vec<WindowCommand>,
) -> Vec<(TileKey, egui::Rect)> {
    let mut placements = Vec::new();
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            let watches = visible_watches(snapshot, popped);
            if watches.is_empty() {
                let hint = if popped.is_empty() {
                    "Choose a stream in the room panel to start watching"
                } else {
                    "Your streams are open in their own windows"
                };
                let center = ui.max_rect().center();
                ui.painter().text(
                    center - egui::vec2(0.0, 16.0),
                    egui::Align2::CENTER_CENTER,
                    if popped.is_empty() {
                        "Ready to watch"
                    } else {
                        "Watching in pop-outs"
                    },
                    egui::FontId::proportional(20.0),
                    super::theme::TEXT,
                );
                ui.scope_builder(
                    egui::UiBuilder::new().max_rect(egui::Rect::from_center_size(
                        center + egui::vec2(0.0, 32.0),
                        egui::vec2((ui.available_width() - 32.0).clamp(32.0, 360.0), 72.0),
                    )),
                    |ui| {
                        ui.vertical_centered(|ui| {
                            ui.weak(hint);
                        });
                    },
                );
                return;
            }
            let rects = grid::layout(ui.max_rect(), watches.len());
            for (watch, rect) in watches.iter().zip(rects) {
                let key = (watch.publisher, watch.live_id);
                placements.push((key, rect));
                tile_overlay(
                    ui,
                    snapshot,
                    state,
                    commands,
                    window_commands,
                    watch,
                    key,
                    rect,
                    Placement::Grid,
                );
            }
        });
    placements
}

/// Status text, the stats readout, and the hover bar for one live drawn in `rect`.
#[allow(clippy::too_many_arguments)]
pub fn tile_overlay(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    state: &mut UiState,
    commands: &mut Vec<RoomCommand>,
    window_commands: &mut Vec<WindowCommand>,
    watch: &WatchView,
    key: TileKey,
    rect: egui::Rect,
    placement: Placement,
) {
    let response = ui.allocate_rect(rect, egui::Sense::hover());
    let live = snapshot
        .members
        .iter()
        .find(|m| m.id == key.0)
        .and_then(|m| m.lives.iter().find(|l| l.id == key.1));
    let title = live_title(snapshot, key).unwrap_or_else(|| "publisher left".to_string());
    let status = match watch.state {
        WatchState::Connecting => Some("connecting"),
        WatchState::Reconnecting => Some("reconnecting"),
        WatchState::Ended => Some("ended"),
        WatchState::Live => None,
    };
    if let Some(status) = status {
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            format!("{title}: {status}"),
            egui::FontId::proportional(16.0),
            egui::Color32::WHITE,
        );
    }
    if state.stats_visible.contains(&key) {
        let preset = live
            .and_then(|l| l.presets.iter().find(|p| p.id == watch.preset_id))
            .map(|p| p.name.as_str())
            .unwrap_or("?");
        ui.painter().text(
            rect.left_bottom() + egui::vec2(8.0, -8.0),
            egui::Align2::LEFT_BOTTOM,
            format!(
                "decoded {}  keyframe requests {}  preset {preset}",
                watch.frames_decoded, watch.keyframe_requests
            ),
            egui::FontId::proportional(12.0),
            egui::Color32::WHITE,
        );
    }
    // A preset dropdown extends past the tile; keeping overlays up while any popup is open stops
    // the dropdown from vanishing under the pointer.
    let show_controls = response.contains_pointer()
        || ui.input(|input| input.key_pressed(egui::Key::Tab))
        || egui::Popup::is_any_open(ui.ctx())
        || ui.ctx().memory(|memory| memory.focused().is_some());
    let opacity = ui.ctx().animate_bool_with_time(
        ui.id().with(("stream-controls", key)),
        show_controls,
        0.12,
    );
    if opacity <= 0.0 {
        return;
    }
    let bar = egui::Rect::from_min_size(
        rect.left_top() + egui::vec2(12.0, 12.0),
        egui::vec2(
            (rect.width() - 24.0).max(0.0),
            88.0_f32.min(rect.height() - 24.0).max(0.0),
        ),
    );
    ui.painter()
        .rect_filled(bar, 12.0, super::theme::SURFACE.gamma_multiply(opacity));
    ui.scope_builder(
        egui::UiBuilder::new()
            .id_salt(("stream-controls", key))
            .max_rect(bar.shrink(8.0))
            .layout(egui::Layout::top_down(egui::Align::Min)),
        |ui| {
            ui.set_clip_rect(ui.clip_rect().intersect(rect));
            ui.set_opacity(opacity);
            // Fading controls stop taking input as soon as the pointer leaves.
            if !show_controls {
                ui.disable();
            }
            ui.add(egui::Label::new(egui::RichText::new(&title).size(16.0)).truncate())
                .on_hover_text(&title);
            egui::ScrollArea::horizontal()
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                .id_salt(("tile-toolbar", key))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if let Some(live) = live {
                            let Some(mut preset_id) = offered_preset(live, Some(watch.preset_id))
                            else {
                                return;
                            };
                            preset_selector(
                                ui,
                                ("tile-preset", key),
                                &live.presets,
                                &mut preset_id,
                            );
                            if preset_id != watch.preset_id {
                                state.preset_choice.insert(key, preset_id);
                                commands.push(RoomCommand::Watch { key, preset_id });
                            }
                        }
                        if watch.audio {
                            let gain = snapshot
                                .members
                                .iter()
                                .find(|m| m.id == key.0)
                                .map(|m| m.gain)
                                .unwrap_or(1.0);
                            if let Some(gain) =
                                super::volume_control(ui, ("tile-volume", key), gain)
                            {
                                commands.push(RoomCommand::SetVolume {
                                    publisher: key.0,
                                    gain,
                                });
                            }
                        }
                        let mut stats = state.stats_visible.contains(&key);
                        if ui.toggle_value(&mut stats, "Stats").changed() {
                            if stats {
                                state.stats_visible.insert(key);
                            } else {
                                state.stats_visible.remove(&key);
                            }
                        }
                        match placement {
                            Placement::Grid => {
                                if ui.button("Pop out").clicked() {
                                    window_commands.push(WindowCommand::PopOut(key));
                                }
                                if ui.button("Fullscreen").clicked() {
                                    window_commands.push(WindowCommand::PopOutFullscreen(key));
                                }
                            }
                            Placement::PopOut { fullscreen } => {
                                let label = if fullscreen { "Windowed" } else { "Fullscreen" };
                                if ui.button(label).clicked() {
                                    window_commands.push(WindowCommand::ToggleFullscreen(key));
                                }
                                if ui.button("Back to grid").clicked() {
                                    window_commands.push(WindowCommand::ReturnToGrid(key));
                                }
                            }
                        }
                        if ui.button("Close").clicked() {
                            commands.push(RoomCommand::Unwatch(key));
                        }
                    });
                });
        },
    );
}

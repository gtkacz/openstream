//! Central panel: reserves one rect per watched live for the video renderer and draws the hover
//! overlay and status text on top. The panel frame is transparent so the tiles show through.

use brp_room::{RoomSnapshot, WatchState, WatchView};

use super::members::{offered_preset, preset_selector};
use super::state::{UiState, ordered_watches};
use crate::commands::RoomCommand;
use crate::render::grid;
use crate::render::tiles::TileKey;

/// Draws the tile grid, returning where the video renderer should place each watched live's
/// frame, in egui points.
pub fn draw(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    state: &mut UiState,
    commands: &mut Vec<RoomCommand>,
) -> Vec<(TileKey, egui::Rect)> {
    let mut placements = Vec::new();
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            let watches = ordered_watches(snapshot);
            if watches.is_empty() {
                ui.centered_and_justified(|ui| ui.weak("Tick a live on the left to watch it"));
                return;
            }
            let rects = grid::layout(ui.max_rect(), watches.len());
            for (watch, rect) in watches.iter().zip(rects) {
                let key = (watch.publisher, watch.live_id);
                placements.push((key, rect));
                overlay(ui, snapshot, state, commands, watch, key, rect);
            }
        });
    placements
}

fn overlay(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    state: &mut UiState,
    commands: &mut Vec<RoomCommand>,
    watch: &WatchView,
    key: TileKey,
    rect: egui::Rect,
) {
    let response = ui.allocate_rect(rect, egui::Sense::hover());
    let live = snapshot
        .members
        .iter()
        .find(|m| m.id == key.0)
        .and_then(|m| m.lives.iter().find(|l| l.id == key.1));
    let title = live
        .map(|l| l.title.clone())
        .unwrap_or_else(|| "publisher left".to_string());
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
            egui::FontId::monospace(13.0),
            egui::Color32::WHITE,
        );
    }
    // A preset dropdown extends past the tile; keeping overlays up while any popup is open stops
    // the dropdown from vanishing under the pointer.
    if !response.contains_pointer() && !egui::Popup::is_any_open(ui.ctx()) {
        return;
    }
    let bar = egui::Rect::from_min_size(
        rect.left_top() + egui::vec2(8.0, 8.0),
        egui::vec2(rect.width() - 16.0, 28.0),
    );
    ui.painter()
        .rect_filled(bar, 4.0, egui::Color32::from_black_alpha(160));
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(bar.shrink(4.0))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.strong(title.as_str());
            if let Some(live) = live {
                let Some(mut preset_id) = offered_preset(live, Some(watch.preset_id)) else {
                    return;
                };
                preset_selector(ui, ("tile-preset", key), &live.presets, &mut preset_id);
                if preset_id != watch.preset_id {
                    state.preset_choice.insert(key, preset_id);
                    commands.push(RoomCommand::Watch { key, preset_id });
                }
            }
            let mut stats = state.stats_visible.contains(&key);
            if ui.toggle_value(&mut stats, "stats").changed() {
                if stats {
                    state.stats_visible.insert(key);
                } else {
                    state.stats_visible.remove(&key);
                }
            }
            if ui.small_button("close").clicked() {
                commands.push(RoomCommand::Unwatch(key));
            }
        },
    );
}

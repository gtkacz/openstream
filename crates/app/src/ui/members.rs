//! Left panel: every member with a path badge, and each of their lives with a watch checkbox and
//! a preset selector.

use std::fmt::Debug;
use std::hash::Hash;

use brp_net::PathKind;
use brp_proto::Preset;
use brp_proto::constants::SOURCE_PRESET_ID;
use brp_room::RoomSnapshot;

use super::state::{UiState, ordered_members};
use crate::commands::RoomCommand;

/// Draws the members panel and pushes `Watch`/`Unwatch` commands for checkbox and preset changes.
pub fn draw(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    state: &mut UiState,
    commands: &mut Vec<RoomCommand>,
) {
    egui::Panel::left("members").resizable(true).show(ui, |ui| {
        ui.heading("Room");
        if snapshot.members.is_empty() {
            ui.weak("nobody else yet");
        }
        egui::ScrollArea::vertical().show(ui, |ui| {
            for member in ordered_members(snapshot) {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.strong(member.nickname.as_str());
                    ui.weak(member.id.fmt_short().to_string());
                    ui.weak(path_badge(member.path));
                });
                for live in &member.lives {
                    let key = (member.id, live.id);
                    let watch = snapshot
                        .watches
                        .iter()
                        .find(|w| w.publisher == key.0 && w.live_id == key.1);
                    let mut watched = watch.is_some();
                    let mut preset_id = watch
                        .map(|w| w.preset_id)
                        .or_else(|| state.preset_choice.get(&key).copied())
                        .unwrap_or(SOURCE_PRESET_ID);
                    // A remembered choice the publisher has since removed falls back to Source.
                    if !live.presets.iter().any(|p| p.id == preset_id) {
                        preset_id = SOURCE_PRESET_ID;
                    }
                    ui.horizontal(|ui| {
                        ui.add_space(12.0);
                        let label = format!(
                            "{} {}x{}",
                            live.title, live.source_width, live.source_height
                        );
                        if ui.checkbox(&mut watched, label).changed() {
                            commands.push(if watched {
                                RoomCommand::Watch { key, preset_id }
                            } else {
                                RoomCommand::Unwatch(key)
                            });
                        }
                        let before = preset_id;
                        preset_selector(ui, ("member-preset", key), &live.presets, &mut preset_id);
                        if preset_id != before {
                            state.preset_choice.insert(key, preset_id);
                            if watched {
                                commands.push(RoomCommand::Watch { key, preset_id });
                            }
                        }
                    });
                }
            }
        });
    });
}

/// Human-readable label for a member's connection path.
pub fn path_badge(path: PathKind) -> &'static str {
    match path {
        PathKind::Direct => "direct",
        PathKind::Relayed => "relayed",
        PathKind::Unknown => "path unknown",
    }
}

/// Draws a combo box over `presets` and writes the chosen id into `selected` when it changes.
/// Shared by the members panel and the tile overlay.
pub fn preset_selector(
    ui: &mut egui::Ui,
    id_salt: impl Hash + Debug,
    presets: &[Preset],
    selected: &mut u32,
) {
    let text = presets
        .iter()
        .find(|p| p.id == *selected)
        .map(|p| p.name.clone())
        .unwrap_or_default();
    egui::ComboBox::from_id_salt(id_salt)
        .selected_text(text)
        .show_ui(ui, |ui| {
            for preset in presets {
                ui.selectable_value(
                    selected,
                    preset.id,
                    format!("{} {}x{}", preset.name, preset.width, preset.height),
                );
            }
        });
}

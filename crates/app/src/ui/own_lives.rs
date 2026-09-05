//! Bottom panel: this participant's lives with per-preset encoder state and the controls that edit
//! presets, plus the share buttons.

use brp_proto::constants::{MAX_BITRATE_KBPS, MAX_LIVES_PER_PARTICIPANT, MIN_BITRATE_KBPS};
use brp_proto::{Codec, SourceKind};
use brp_room::{OwnLiveView, RoomSnapshot};

use super::state::UiState;
use crate::commands::RoomCommand;
use crate::presets;

/// Draws the own-lives panel: share buttons plus one row group per own live, pushing
/// `Share`/`StopLive`/`SetPresets` commands for user edits.
pub fn draw(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    state: &mut UiState,
    commands: &mut Vec<RoomCommand>,
) {
    egui::Panel::bottom("own-lives")
        .resizable(true)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("My lives");
                let can_share =
                    !state.share_pending && snapshot.own_lives.len() < MAX_LIVES_PER_PARTICIPANT;
                if ui
                    .add_enabled(can_share, egui::Button::new("Share monitor"))
                    .clicked()
                {
                    commands.push(RoomCommand::Share(SourceKind::Monitor));
                }
                if ui
                    .add_enabled(can_share, egui::Button::new("Share window"))
                    .clicked()
                {
                    commands.push(RoomCommand::Share(SourceKind::Window));
                }
                if state.share_pending {
                    ui.weak("waiting for the picker");
                }
            });
            egui::ScrollArea::vertical().show(ui, |ui| {
                for live in &snapshot.own_lives {
                    live_rows(ui, live, state, commands);
                }
            });
        });
}

fn live_rows(
    ui: &mut egui::Ui,
    live: &OwnLiveView,
    state: &mut UiState,
    commands: &mut Vec<RoomCommand>,
) {
    let info = &live.info;
    let current_fps = info
        .presets
        .first()
        .map(|p| p.fps)
        .unwrap_or(info.source_fps);
    let current_codec = info.presets.first().map(|p| p.codec).unwrap_or(Codec::H264);
    ui.separator();
    ui.horizontal(|ui| {
        ui.strong(info.title.as_str());
        ui.weak(format!(
            "{}x{} @ {} fps",
            info.source_width, info.source_height, info.source_fps
        ));

        let fps = state.fps_edits.entry(info.id).or_insert(current_fps);
        let response = ui.add(
            egui::DragValue::new(fps)
                .range(1..=info.source_fps.max(1))
                .suffix(" fps"),
        );
        // Commit on release rather than on every change: each commit restarts the encoders.
        if !response.dragged() && !response.has_focus() {
            let value = *fps;
            state.fps_edits.remove(&info.id);
            if value != current_fps {
                commands.push(RoomCommand::SetPresets {
                    live_id: info.id,
                    presets: presets::with_fps(info, value),
                });
            }
        }

        let mut codec = current_codec;
        egui::ComboBox::from_id_salt(("codec", info.id))
            .selected_text(codec_name(codec))
            .show_ui(ui, |ui| {
                for candidate in [Codec::Hevc, Codec::H264, Codec::Av1] {
                    ui.selectable_value(&mut codec, candidate, codec_name(candidate));
                }
            });
        if codec != current_codec {
            commands.push(RoomCommand::SetPresets {
                live_id: info.id,
                presets: presets::with_codec(info, codec),
            });
        }

        if ui.button("Stop").clicked() {
            commands.push(RoomCommand::StopLive(info.id));
        }
    });

    ui.horizontal(|ui| {
        ui.add_space(12.0);
        ui.label("Templates:");
        for template in presets::templates_for(info) {
            let mut enabled = info.presets.iter().any(|p| p.id == template.id);
            if ui.checkbox(&mut enabled, template.name.as_str()).changed() {
                commands.push(RoomCommand::SetPresets {
                    live_id: info.id,
                    presets: presets::toggle_template(info, template.id),
                });
            }
        }
    });

    for view in &live.presets {
        let preset = &view.preset;
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.monospace(format!(
                "{:<7} {}x{}",
                preset.name, preset.width, preset.height
            ));
            let key = (info.id, preset.id);
            let kbps = state
                .bitrate_edits
                .entry(key)
                .or_insert(preset.bitrate_kbps);
            let response = ui.add(
                egui::DragValue::new(kbps)
                    .range(MIN_BITRATE_KBPS..=MAX_BITRATE_KBPS)
                    .speed(100.0)
                    .suffix(" kbps"),
            );
            if !response.dragged() && !response.has_focus() {
                let value = *kbps;
                state.bitrate_edits.remove(&key);
                if value != preset.bitrate_kbps {
                    commands.push(RoomCommand::SetPresets {
                        live_id: info.id,
                        presets: presets::with_bitrate(info, preset.id, value),
                    });
                }
            }
            match (&view.encoder, &view.last_error) {
                (Some(encoder), _) => {
                    let plural = if encoder.subscribers == 1 { "" } else { "s" };
                    let measured = state.preset_kbps(info.id, preset.id).unwrap_or(0);
                    ui.label(format!(
                        "{} · {measured} kbps · {} viewer{plural} · {} frames",
                        encoder.name, encoder.subscribers, encoder.frames_encoded
                    ))
                }
                (None, Some(error)) => {
                    ui.colored_label(egui::Color32::LIGHT_RED, format!("failed: {error}"))
                }
                (None, None) => ui.weak("idle"),
            };
        });
    }
}

fn codec_name(codec: Codec) -> &'static str {
    match codec {
        Codec::H264 => "H.264",
        Codec::Hevc => "HEVC",
        Codec::Av1 => "AV1",
    }
}

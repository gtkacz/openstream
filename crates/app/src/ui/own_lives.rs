//! This participant's side of the room: the share and audio controls for the dock, and the card
//! for each of their streams with the controls that edit its presets.

use brp_audio::AudioSelection;
use brp_proto::constants::{MAX_BITRATE_KBPS, MAX_LIVES_PER_PARTICIPANT, MIN_BITRATE_KBPS};
use brp_proto::{Codec, SourceKind};
use brp_room::{AudioCaptureState, OwnLiveView, RoomSnapshot};

use super::applications::selection_summary;
use super::state::UiState;
use super::theme::{self, icons};
use crate::commands::RoomCommand;
use crate::presets;

/// Share and stop buttons, pushing `Share`/`StopLive` commands.
pub fn share_buttons(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    state: &UiState,
    commands: &mut Vec<RoomCommand>,
    labelled: bool,
) {
    let can_share = !state.share_pending
        && state.picker.is_none()
        && snapshot.own_lives.len() < MAX_LIVES_PER_PARTICIPANT;
    if theme::primary_action(ui, icons::SHARE_MONITOR, "Share monitor", can_share).clicked() {
        commands.push(RoomCommand::Share {
            kind: SourceKind::Monitor,
            source: None,
        });
    }
    let tool = |ui: &mut egui::Ui, glyph: &str, name: &str, enabled: bool| {
        if labelled {
            theme::action(ui, glyph, name, enabled).clicked()
        } else {
            theme::icon_button(ui, glyph, name, enabled).clicked()
        }
    };
    if tool(ui, icons::SHARE_WINDOW, "Share window", can_share) {
        commands.push(RoomCommand::Share {
            kind: SourceKind::Window,
            source: None,
        });
    }
    // Always present, so starting a stream cannot shove the rest of the dock sideways.
    if tool(
        ui,
        icons::STOP,
        "Stop sharing",
        !snapshot.own_lives.is_empty(),
    ) {
        commands.extend(
            snapshot
                .own_lives
                .iter()
                .map(|live| RoomCommand::StopLive(live.info.id)),
        );
    }
    if state.share_pending {
        theme::spinner(ui, 16.0);
        ui.weak("Starting your stream…");
    }
}

/// The share-audio switch, the application chooser, and what the room hears right now. Without
/// room for `show_status` that moves to the chooser's tooltip; a failure always shows.
pub fn audio_controls(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    commands: &mut Vec<RoomCommand>,
    show_status: bool,
) {
    let mut share_audio = snapshot.own_audio.enabled;
    if theme::switch(ui, &mut share_audio, "Share audio").changed() {
        commands.push(RoomCommand::SetAudio(share_audio));
    }
    let status = audio_status(snapshot);
    let selection = selection_summary(&snapshot.own_audio.selection);
    let tooltip = match (&status, show_status) {
        (Some(status), false) => format!("{selection} · {status}"),
        _ => selection,
    };
    if theme::text_button(ui, "Applications…", true)
        .on_hover_text(tooltip)
        .clicked()
    {
        commands.push(RoomCommand::ChooseApplications);
    }
    if let (true, Some(status)) = (show_status, &status) {
        ui.weak(status);
    }
    if let AudioCaptureState::Failed(error) = &snapshot.own_audio.state {
        ui.colored_label(theme::ERROR, format!("Audio failed: {error}"));
    }
}

fn audio_status(snapshot: &RoomSnapshot) -> Option<String> {
    match &snapshot.own_audio.state {
        AudioCaptureState::Off | AudioCaptureState::Failed(_) => None,
        AudioCaptureState::Idle => Some("Waiting for a listener".to_string()),
        AudioCaptureState::Capturing => match &snapshot.own_audio.selection {
            AudioSelection::Only(keys) if keys.is_empty() => {
                Some("No applications selected".to_string())
            }
            _ => {
                let n = snapshot.own_audio.subscribers;
                let plural = if n == 1 { "" } else { "s" };
                Some(format!("{n} listener{plural}"))
            }
        },
    }
}

/// The compact window's Share sheet: what the dock holds in a wide one.
pub fn share_sheet(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    state: &UiState,
    commands: &mut Vec<RoomCommand>,
) {
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            theme::section_heading(ui, "Share");
            ui.horizontal_wrapped(|ui| {
                theme::uniform_controls(ui);
                share_buttons(ui, snapshot, state, commands, true);
            });
            ui.add_space(12.0);
            theme::section_heading(ui, "Audio");
            ui.horizontal_wrapped(|ui| {
                theme::uniform_controls(ui);
                audio_controls(ui, snapshot, commands, true);
            });
            ui.add_space(12.0);
            theme::section_heading(ui, "Upload");
            ui.label(theme::figure(format!("{} kbps", state.upload_kbps)));
        });
}

/// One of this participant's streams, pushing `StopLive`/`SetPresets` commands for user edits.
pub fn live_card(
    ui: &mut egui::Ui,
    live: &OwnLiveView,
    state: &mut UiState,
    commands: &mut Vec<RoomCommand>,
) {
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        live_rows(ui, live, state, commands);
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
    let streaming = live.presets.iter().any(|preset| {
        preset
            .encoder
            .as_ref()
            .is_some_and(|encoder| encoder.subscribers > 0)
    });
    let failing = live
        .presets
        .iter()
        .any(|preset| preset.last_error.is_some());
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), 20.0),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            // Placed first so a long title truncates against it instead of pushing it out.
            if failing {
                ui.colored_label(theme::ERROR, "Needs attention");
            } else if streaming {
                ui.colored_label(theme::LIVE, "Live");
            } else {
                ui.weak("Ready");
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                theme::live_dot(ui, streaming);
                ui.add(
                    egui::Label::new(egui::RichText::new(&info.title).font(theme::medium(14.0)))
                        .truncate(),
                )
                .on_hover_text(&info.title);
            });
        },
    );
    ui.label(
        egui::RichText::new(format!(
            "{} × {} · {} fps",
            info.source_width, info.source_height, info.source_fps
        ))
        .small()
        .color(theme::SECONDARY),
    );
    let mut stop = false;
    let mut edits = Vec::new();
    theme::split_row(
        ui,
        theme::CONTROL_HEIGHT,
        |ui| {
            let fps = state.fps_edits.entry(info.id).or_insert(current_fps);
            let response = theme::drag_value(
                ui,
                egui::DragValue::new(fps)
                    .range(1..=info.source_fps.max(1))
                    .suffix(" fps"),
            );
            // Commit on release rather than on every change: each commit restarts the encoders.
            if !response.dragged() && !response.has_focus() {
                let value = *fps;
                state.fps_edits.remove(&info.id);
                if value != current_fps {
                    edits.push(RoomCommand::SetPresets {
                        live_id: info.id,
                        presets: presets::with_fps(info, value),
                    });
                }
            }

            let mut codec = current_codec;
            theme::dropdown(
                ui,
                ("codec", info.id),
                codec_name(codec),
                Some(72.0),
                |ui| {
                    for candidate in [Codec::Hevc, Codec::H264, Codec::Av1] {
                        ui.selectable_value(&mut codec, candidate, codec_name(candidate));
                    }
                },
            );
            if codec != current_codec {
                edits.push(RoomCommand::SetPresets {
                    live_id: info.id,
                    presets: presets::with_codec(info, codec),
                });
            }
        },
        |ui| {
            stop = theme::icon_button(ui, icons::STOP, "Stop this stream", true).clicked();
        },
    );
    commands.extend(edits);
    if stop {
        commands.push(RoomCommand::StopLive(info.id));
    }

    for preset in &live.presets {
        if let Some(error) = &preset.last_error {
            ui.colored_label(
                theme::ERROR,
                format!("{} failed: {error}", preset.preset.name),
            );
        }
    }
    if live.presets.iter().any(|preset| {
        preset
            .encoder
            .as_ref()
            .is_some_and(|encoder| brp_codec::is_software_encoder(encoder.name))
    }) {
        ui.colored_label(theme::WARNING, "Software encoding · using the CPU");
    }
    egui::CollapsingHeader::new("Quality & encoding")
        .id_salt(("quality", info.id))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("Templates");
                // The last preset cannot be unticked: a live that offers nothing cannot be watched.
                let last_one = info.presets.len() == 1;
                for template in presets::templates_for(info) {
                    let mut enabled = info.presets.iter().any(|p| p.id == template.id);
                    let locked = enabled && last_one;
                    let checkbox = egui::Checkbox::new(&mut enabled, template.name.as_str());
                    if ui.add_enabled(!locked, checkbox).changed() {
                        commands.push(RoomCommand::SetPresets {
                            live_id: info.id,
                            presets: presets::toggle_template(info, template.id),
                        });
                    }
                }
            });

            for view in &live.presets {
                let preset = &view.preset;
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!(
                        "{} · {} × {}",
                        preset.name, preset.width, preset.height
                    ));
                    let key = (info.id, preset.id);
                    let kbps = state
                        .bitrate_edits
                        .entry(key)
                        .or_insert(preset.bitrate_kbps);
                    let response = theme::drag_value(
                        ui,
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
                            ui.label(theme::figure(format!(
                                "{} · {measured} kbps · {} viewer{plural} · {} frames",
                                encoder.name, encoder.subscribers, encoder.frames_encoded
                            )))
                        }
                        (None, Some(error)) => {
                            ui.colored_label(theme::ERROR, format!("failed: {error}"))
                        }
                        (None, None) => ui.weak("idle"),
                    };
                });
            }
        });
}

fn codec_name(codec: Codec) -> &'static str {
    match codec {
        Codec::H264 => "H.264",
        Codec::Hevc => "HEVC",
        Codec::Av1 => "AV1",
    }
}

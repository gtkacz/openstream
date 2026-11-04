//! This participant's side of the room: the share and audio controls for the dock, and the card
//! for each of their streams with the controls that edit its presets.

use brp_audio::AudioSelection;
use brp_proto::constants::{MAX_BITRATE_KBPS, MAX_LIVES_PER_PARTICIPANT, MIN_BITRATE_KBPS};
use brp_proto::{Codec, SourceKind};
use brp_room::{AudioCaptureState, OwnLiveView, RoomSnapshot};

use super::applications::selection_summary;
use super::motion;
use super::state::UiState;
use super::theme::{self, icons};
use crate::commands::RoomCommand;
use crate::presets;

/// How the share buttons are laid out: in the dock's row, saying as much as the width allows, or
/// stacked across a narrow sheet, where a row of them would run off the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShareLayout {
    Row {
        share_labelled: bool,
        tools_labelled: bool,
    },
    Stacked,
}

/// Share and stop buttons, pushing `Share`/`StopLive` commands.
pub fn share_buttons(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    state: &UiState,
    commands: &mut Vec<RoomCommand>,
    layout: ShareLayout,
) {
    let can_share = !state.share_pending
        && state.picker.is_none()
        && snapshot.own_lives.len() < MAX_LIVES_PER_PARTICIPANT;
    let button = |ui: &mut egui::Ui, glyph: &str, name: &str, tone: theme::Tone, enabled: bool| {
        let labelled = match layout {
            ShareLayout::Stacked => {
                let size = egui::vec2(ui.available_width(), theme::CONTROL_HEIGHT);
                return theme::wide_action(ui, glyph, name, tone, enabled, size);
            }
            // The main action keeps its words longest: it is the one a newcomer has to find.
            ShareLayout::Row { share_labelled, .. } if tone == theme::Tone::Primary => {
                share_labelled
            }
            ShareLayout::Row { tools_labelled, .. } => tools_labelled,
        };
        match (labelled, tone) {
            (true, theme::Tone::Primary) => theme::primary_action(ui, glyph, name, enabled),
            (true, _) => theme::action(ui, glyph, name, enabled),
            (false, _) => theme::toned_icon_button(ui, tone, glyph, name, enabled),
        }
    };
    if button(
        ui,
        icons::SHARE_MONITOR,
        "Share monitor",
        theme::Tone::Primary,
        can_share,
    )
    .clicked()
    {
        commands.push(RoomCommand::Share {
            kind: SourceKind::Monitor,
            source: None,
        });
    }
    if button(
        ui,
        icons::SHARE_WINDOW,
        "Share window",
        theme::Tone::Neutral,
        can_share,
    )
    .clicked()
    {
        commands.push(RoomCommand::Share {
            kind: SourceKind::Window,
            source: None,
        });
    }
    // Always present, so starting a stream cannot shove the rest of the dock sideways.
    if button(
        ui,
        icons::STOP,
        "Stop sharing",
        theme::Tone::Neutral,
        !snapshot.own_lives.is_empty(),
    )
    .clicked()
    {
        commands.extend(
            snapshot
                .own_lives
                .iter()
                .map(|live| RoomCommand::StopLive(live.info.id)),
        );
    }
}

/// The share-audio switch, the application chooser, and what the room hears right now. Without
/// room for `show_status` that moves to the chooser's tooltip. A failure is the dock's to say.
pub fn audio_controls(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    commands: &mut Vec<RoomCommand>,
    show_status: bool,
    labelled: bool,
) {
    let mut share_audio = snapshot.own_audio.enabled;
    let switch_text = if labelled { "Share audio" } else { "Audio" };
    if theme::switch(ui, &mut share_audio, switch_text).changed() {
        commands.push(RoomCommand::SetAudio(share_audio));
    }
    let status = audio_status(snapshot);
    let selection = selection_summary(&snapshot.own_audio.selection);
    let tooltip = match (&status, show_status) {
        (Some(status), false) => format!("{selection} · {status}"),
        _ => selection,
    };
    let chooser = if labelled {
        theme::text_button(ui, "Applications…", true).on_hover_text(tooltip)
    } else {
        theme::icon_button(ui, icons::APPLICATIONS, "Applications…", true).on_hover_text(tooltip)
    };
    if chooser.clicked() {
        commands.push(RoomCommand::ChooseApplications);
    }
    if let (true, Some(status)) = (show_status, &status) {
        ui.weak(status);
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
            share_buttons(ui, snapshot, state, commands, ShareLayout::Stacked);
            ui.add_space(12.0);
            theme::section_heading(ui, "Audio");
            ui.horizontal_wrapped(|ui| {
                theme::uniform_controls(ui);
                audio_controls(ui, snapshot, commands, true, true);
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
        ui.spacing_mut().item_spacing.y = 4.0;
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
                ui.colored_label(theme::ERROR, egui::RichText::new("Needs attention").small());
            } else if streaming {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.colored_label(theme::LIVE, egui::RichText::new("Live").small());
                theme::live_dot(ui, true);
            } else {
                ui.label(egui::RichText::new("Ready").small().color(theme::SECONDARY));
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
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
    ui.add_space(4.0);
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
    let open = state.quality_open.contains(&info.id);
    if theme::disclosure(ui, "Quality & encoding", open).clicked() {
        if open {
            state.quality_open.remove(&info.id);
        } else {
            state.quality_open.insert(info.id);
        }
    }
    motion::unfold(ui, ("quality", info.id), open, |ui| {
        quality_details(ui, live, state, commands);
    });
}

fn quality_details(
    ui: &mut egui::Ui,
    live: &OwnLiveView,
    state: &mut UiState,
    commands: &mut Vec<RoomCommand>,
) {
    let info = &live.info;
    ui.label(
        egui::RichText::new("Templates")
            .small()
            .color(theme::SECONDARY),
    );
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 16.0;
        // The last preset cannot be unticked: a live that offers nothing cannot be watched.
        let last_one = info.presets.len() == 1;
        for template in presets::templates_for(info) {
            let mut enabled = info.presets.iter().any(|p| p.id == template.id);
            let locked = enabled && last_one;
            // Not `add_enabled_ui`: its child scope would keep this row from wrapping.
            let check = |ui: &mut egui::Ui| theme::check(ui, &mut enabled, template.name.as_str());
            if ui.add_enabled(!locked, check).changed() {
                commands.push(RoomCommand::SetPresets {
                    live_id: info.id,
                    presets: presets::toggle_template(info, template.id),
                });
            }
        }
    });
    for view in &live.presets {
        let preset = &view.preset;
        let dimensions = format!("{} × {}", preset.width, preset.height);
        ui.add_space(8.0);
        theme::split_row(
            ui,
            theme::CONTROL_HEIGHT,
            |ui| {
                ui.label(preset.name.as_str());
            },
            |ui| {
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
            },
        );
        match (&view.encoder, &view.last_error) {
            (Some(encoder), _) => {
                let plural = if encoder.subscribers == 1 { "" } else { "s" };
                let measured = state.preset_kbps(info.id, preset.id).unwrap_or(0);
                // Two fixed lines, because wrapped as one they break next to a separator.
                ui.label(theme::figure(format!("{dimensions} · {}", encoder.name)));
                ui.label(theme::figure(format!(
                    "{measured} kbps · {} viewer{plural} · {} frames",
                    encoder.subscribers, encoder.frames_encoded
                )))
            }
            (None, Some(error)) => ui.colored_label(theme::ERROR, format!("failed: {error}")),
            (None, None) => ui.label(theme::figure(format!("{dimensions} · idle"))),
        };
    }
}

fn codec_name(codec: Codec) -> &'static str {
    match codec {
        Codec::H264 => "H.264",
        Codec::Hevc => "HEVC",
        Codec::Av1 => "AV1",
    }
}

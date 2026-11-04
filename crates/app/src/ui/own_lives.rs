//! This participant's side of the room: the share and audio controls for the dock, and the card
//! for each of their streams with the controls that edit its presets.

use brp_audio::AudioSelection;
use brp_proto::constants::{MAX_BITRATE_KBPS, MAX_LIVES_PER_PARTICIPANT, MIN_BITRATE_KBPS};
use brp_proto::{Codec, LiveInfo, Preset, SourceKind, VideoQuality};
use brp_room::{AudioCaptureState, OwnLiveView, PresetView, RoomSnapshot};

use super::applications::selection_summary;
use super::motion;
use super::state::{UiState, nobody_is_sharing};
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

/// A click on Share would start something now.
pub fn can_share(snapshot: &RoomSnapshot, state: &UiState) -> bool {
    !state.share_pending
        && state.picker.is_none()
        && snapshot.own_lives.len() < MAX_LIVES_PER_PARTICIPANT
}

/// A rate for reading rather than for machines: one fixed decimal once it reaches megabits, so a
/// figure that changes while it is watched holds its width.
pub fn bitrate(kbps: u64) -> String {
    if kbps < 1000 {
        format!("{kbps} kbps")
    } else {
        format!("{:.1} Mbps", kbps as f64 / 1000.0)
    }
}

/// What was typed into a bitrate, in kilobits. The allowed range in megabits and the allowed range
/// in kilobits do not overlap, so a number with no unit can only mean one of them, and one that fits
/// neither is refused rather than clamped to a limit nobody asked for.
fn parse_bitrate(text: &str) -> Option<f64> {
    let text = text.trim().to_lowercase();
    let kbps = if let Some(number) = text.strip_suffix("mbps") {
        number.trim().parse::<f64>().ok()? * 1000.0
    } else if let Some(number) = text.strip_suffix("kbps") {
        number.trim().parse::<f64>().ok()?
    } else {
        let number = text.parse::<f64>().ok()?;
        if number <= f64::from(MAX_BITRATE_KBPS) / 1000.0 {
            number * 1000.0
        } else if number >= f64::from(MIN_BITRATE_KBPS) {
            number
        } else {
            return None;
        }
    };
    kbps.is_finite().then_some(kbps.round())
}

/// A new bitrate restarts the encoders, so a change has to be one the field can show. Clicking in and
/// out of "4.4 Mbps" re-reads those words as 4400, which must not restart a 4437 kbps preset.
fn visibly_differs(a: u32, b: u32) -> bool {
    bitrate(u64::from(a)) != bitrate(u64::from(b))
}

/// An encoder as a person would name it, without the codec the card already shows beside it.
fn encoder_label(name: &str) -> &str {
    if brp_codec::is_software_encoder(name) {
        return "Software";
    }
    match name.rsplit('_').next() {
        Some("vaapi") => "VAAPI",
        Some("nvenc") => "NVENC",
        Some("amf") => "AMF",
        Some("qsv") => "Quick Sync",
        Some("mf") => "Media Foundation",
        _ => name,
    }
}

/// Share and stop buttons, pushing `Share`/`StopLive` commands.
pub fn share_buttons(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    state: &UiState,
    commands: &mut Vec<RoomCommand>,
    layout: ShareLayout,
) {
    let can_share = can_share(snapshot, state);
    // Accent says what to do next, so it leaves once you are sharing, and sits on the canvas's own
    // Share button while the canvas offers one.
    let share_tone = if snapshot.own_lives.is_empty() && !nobody_is_sharing(snapshot) {
        theme::Tone::Primary
    } else {
        theme::Tone::Neutral
    };
    let button = |ui: &mut egui::Ui,
                  glyph: &str,
                  name: &str,
                  tone: theme::Tone,
                  main: bool,
                  enabled: bool| {
        let labelled = match layout {
            ShareLayout::Stacked => {
                let size = egui::vec2(ui.available_width(), theme::CONTROL_HEIGHT);
                return theme::wide_action(ui, glyph, name, tone, enabled, size);
            }
            // The main action keeps its words longest: it is the one a newcomer has to find.
            ShareLayout::Row { share_labelled, .. } if main => share_labelled,
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
        share_tone,
        true,
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
        false,
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
        false,
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

/// One control for what the room hears: the share-audio switch with its application chooser on the
/// same surface, then what the room hears right now. Without room for `show_status` that moves to
/// the chooser's tooltip. A failure is the dock's to say.
pub fn audio_controls(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    commands: &mut Vec<RoomCommand>,
    show_status: bool,
    labelled: bool,
) {
    const SWITCH_INSET: f32 = 10.0;
    let status = audio_status(snapshot);
    let selection = selection_summary(&snapshot.own_audio.selection);
    let tooltip = match (&status, show_status) {
        (Some(status), false) => format!("{selection} · {status}"),
        _ => selection,
    };
    // Painted under the two controls afterwards rather than framed around them: a frame is a group,
    // which starts at the top of a row instead of its centre and cannot wrap with the row.
    let backing = ui.painter().add(egui::Shape::Noop);
    ui.add_space(SWITCH_INSET);
    let mut share_audio = snapshot.own_audio.enabled;
    let switch_text = if labelled { "Share audio" } else { "Audio" };
    let switch = theme::switch(ui, &mut share_audio, switch_text);
    if switch.changed() {
        commands.push(RoomCommand::SetAudio(share_audio));
    }
    let chooser = theme::toned_icon_button(
        ui,
        theme::Tone::Quiet,
        icons::APPLICATIONS,
        "Applications…",
        true,
    )
    .on_hover_text(tooltip);
    if chooser.clicked() {
        commands.push(RoomCommand::ChooseApplications);
    }
    let surface = egui::Rect::from_min_max(
        egui::pos2(switch.rect.left() - SWITCH_INSET, chooser.rect.top()),
        chooser.rect.max,
    );
    ui.painter().set(
        backing,
        egui::Shape::rect_filled(surface, theme::RADIUS_CONTROL, theme::RAISED),
    );
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

/// The compact window's Share sheet: what the dock holds in a wide one. Returns the height it would
/// like, for the sheet to fit itself to.
pub fn share_sheet(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    state: &UiState,
    commands: &mut Vec<RoomCommand>,
) -> f32 {
    let top = ui.min_rect().top();
    let output = egui::ScrollArea::vertical()
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
            ui.label(theme::figure(bitrate(state.upload_kbps)));
        });
    output.inner_rect.top() - top + output.content_size.y
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

/// Every quality this live could offer, each ticked on its own row with what it costs. The last one
/// on cannot be unticked: a live that offers nothing cannot be watched.
fn quality_details(
    ui: &mut egui::Ui,
    live: &OwnLiveView,
    state: &mut UiState,
    commands: &mut Vec<RoomCommand>,
) {
    let info = &live.info;
    let last_one = info.presets.len() == 1;
    for template in presets::templates_for(info) {
        let view = live.presets.iter().find(|v| v.preset.id == template.id);
        let mut toggled = false;
        ui.add_space(4.0);
        theme::split_row(
            ui,
            theme::CONTROL_HEIGHT,
            |ui| {
                let mut enabled = view.is_some();
                let locked = enabled && last_one;
                let check =
                    |ui: &mut egui::Ui| theme::check(ui, &mut enabled, template.name.as_str());
                toggled = ui.add_enabled(!locked, check).changed();
            },
            |ui| {
                // Lossless has no bitrate to choose: it takes what the picture needs.
                if let Some(view) = view.filter(|v| v.preset.quality() != VideoQuality::Lossless) {
                    bitrate_field(ui, info, &view.preset, state, commands);
                }
            },
        );
        if toggled {
            commands.push(RoomCommand::SetPresets {
                live_id: info.id,
                presets: presets::toggle_template(info, template.id),
            });
        }
        if let Some(view) = view {
            preset_detail(ui, info.id, view, state);
        }
    }
}

fn bitrate_field(
    ui: &mut egui::Ui,
    info: &LiveInfo,
    preset: &Preset,
    state: &mut UiState,
    commands: &mut Vec<RoomCommand>,
) {
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
            .custom_formatter(|value, _| bitrate(value as u64))
            .custom_parser(parse_bitrate),
    );
    // Commit on release rather than on every change: each commit restarts the encoders.
    if !response.dragged() && !response.has_focus() {
        let value = *kbps;
        state.bitrate_edits.remove(&key);
        if visibly_differs(value, preset.bitrate_kbps) {
            commands.push(RoomCommand::SetPresets {
                live_id: info.id,
                presets: presets::with_bitrate(info, preset.id, value),
            });
        }
    }
}

fn preset_detail(ui: &mut egui::Ui, live_id: u32, view: &PresetView, state: &UiState) {
    let preset = &view.preset;
    let dimensions = format!("{} × {}", preset.width, preset.height);
    match (&view.encoder, &view.last_error) {
        (Some(encoder), _) => {
            let plural = if encoder.subscribers == 1 { "" } else { "s" };
            let measured = state.preset_kbps(live_id, preset.id).unwrap_or(0);
            // Two fixed lines, because wrapped as one they break next to a separator.
            ui.label(theme::figure(format!(
                "{dimensions} · {}",
                encoder_label(encoder.name)
            )))
            .on_hover_text(encoder.name);
            ui.label(theme::figure(format!(
                "{} · {} viewer{plural}",
                bitrate(measured),
                encoder.subscribers
            )))
            .on_hover_text(format!("{} frames encoded", encoder.frames_encoded))
        }
        (None, Some(error)) => ui.colored_label(theme::ERROR, format!("failed: {error}")),
        (None, None) => ui.label(theme::figure(format!("{dimensions} · idle"))),
    };
}

fn codec_name(codec: Codec) -> &'static str {
    match codec {
        Codec::H264 => "H.264",
        Codec::Hevc => "HEVC",
        Codec::Av1 => "AV1",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rate_reads_in_kilobits_until_it_reaches_a_megabit_and_holds_one_decimal_after() {
        assert_eq!(bitrate(0), "0 kbps");
        assert_eq!(bitrate(999), "999 kbps");
        assert_eq!(bitrate(1000), "1.0 Mbps");
        assert_eq!(bitrate(4437), "4.4 Mbps");
        assert_eq!(bitrate(40_000), "40.0 Mbps");
    }

    #[test]
    fn a_typed_rate_takes_its_own_unit() {
        assert_eq!(parse_bitrate("4.5 Mbps"), Some(4500.0));
        assert_eq!(parse_bitrate("8000 kbps"), Some(8000.0));
        assert_eq!(parse_bitrate(" 2.5mbps "), Some(2500.0));
    }

    #[test]
    fn a_bare_number_is_whichever_unit_it_could_be_in_range() {
        assert_eq!(parse_bitrate("40"), Some(40_000.0));
        assert_eq!(parse_bitrate("250"), Some(250_000.0));
        assert_eq!(parse_bitrate("8000"), Some(8000.0));
    }

    #[test]
    fn a_number_that_fits_neither_unit_or_is_not_a_number_is_refused() {
        assert_eq!(parse_bitrate("800"), None);
        assert_eq!(parse_bitrate("fast"), None);
        assert_eq!(parse_bitrate("nan"), None);
        assert_eq!(parse_bitrate("inf"), None);
        assert_eq!(parse_bitrate("inf mbps"), None);
    }

    #[test]
    fn typed_megabits_land_on_whole_kilobits_despite_float_rounding() {
        assert_eq!(parse_bitrate("2.3 Mbps"), Some(2300.0));
        assert_eq!(parse_bitrate("1.1"), Some(1100.0));
    }

    #[test]
    fn only_a_change_the_field_can_show_counts_as_a_change() {
        assert!(!visibly_differs(4400, 4437), "clicking in and out");
        assert!(visibly_differs(4500, 4437));
        assert!(visibly_differs(5000, 4437));
    }

    #[test]
    fn an_encoder_is_named_for_a_person_and_an_unknown_one_keeps_its_own_name() {
        assert_eq!(encoder_label("hevc_vaapi"), "VAAPI");
        assert_eq!(encoder_label("av1_nvenc"), "NVENC");
        assert_eq!(encoder_label("h264_qsv"), "Quick Sync");
        assert_eq!(encoder_label("libsvtav1"), "Software");
        assert_eq!(encoder_label("mystery"), "mystery");
    }
}

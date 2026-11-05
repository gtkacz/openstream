//! The dock under the video: the share and audio controls, upload rate, master mute, Settings, and
//! whatever notice or error needs saying. In a compact window it also opens the Room and Share
//! sheets, which hold what the rail holds in a wide one.

use brp_room::{AudioCaptureState, RoomSnapshot};

use super::motion;
use super::own_lives;
use super::state::{CompactSheet, UiState};
use super::theme::{self, icons};
use super::update::{self, UpdateState};
use crate::commands::RoomCommand;

const GROUP_GAP: f32 = 8.0;

/// Draws the dock. The master mute toggle emits `SetMasterMute`; the Settings button sets
/// `open_settings`; the Update button sets `update_clicked`.
// One parameter per thing the bar shows; a struct would be built and unpacked in one place.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    state: &mut UiState,
    commands: &mut Vec<RoomCommand>,
    open_settings: &mut bool,
    update_state: &UpdateState,
    update_clicked: &mut bool,
) {
    let compact = theme::is_compact(ui.ctx());
    let height_id = ui.id().with("status-height");
    let height = ui
        .ctx()
        .data(|data| data.get_temp::<f32>(height_id))
        .unwrap_or(theme::CONTROL_HEIGHT + 25.0);
    egui::Panel::bottom("status")
        .exact_size(height)
        .frame(
            egui::Frame::NONE
                .fill(theme::ground())
                .inner_margin(egui::Margin::symmetric(12, 12)),
        )
        .show(ui, |ui| {
            theme::paint_material(ui, ui.max_rect().expand2(egui::vec2(12.0, 12.0)));
            let content = ui.scope(|ui| {
                notices(ui, snapshot, state, update_state, update_clicked);
                if compact {
                    compact_row(ui, snapshot, state, commands, open_settings);
                } else {
                    wide_row(ui, snapshot, state, commands, open_settings);
                }
            });
            let needed = content.response.rect.height() + 25.0;
            if (needed - height).abs() > 0.5 {
                ui.ctx()
                    .data_mut(|data| data.insert_temp(height_id, needed));
                ui.ctx().request_discard("dock content height changed");
            }
        });
}

fn compact_row(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    state: &mut UiState,
    commands: &mut Vec<RoomCommand>,
    open_settings: &mut bool,
) {
    theme::split_row(
        ui,
        theme::CONTROL_HEIGHT,
        |ui| {
            sheet_button(ui, state, CompactSheet::Room, icons::ROOM, "Room");
            sheet_button(
                ui,
                state,
                CompactSheet::Share,
                icons::SHARE_MONITOR,
                "Share",
            );
        },
        |ui| {
            settings_button(ui, open_settings);
            master_mute(ui, snapshot, commands, false);
        },
    );
}

/// What the dock can afford to say at this width. It sheds words rather than taking a second row,
/// because that height would come out of the video. The least useful go first, and the main action's
/// label last. The widths are the controls measured in IBM Plex Mono, with Stop sharing showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DockFit {
    audio_status: bool,
    tools_labelled: bool,
    audio_labelled: bool,
    upload: bool,
    share_labelled: bool,
}

impl DockFit {
    fn for_width(width: f32) -> Self {
        Self {
            audio_status: width >= 1240.0,
            tools_labelled: width >= 900.0,
            audio_labelled: width >= 720.0,
            upload: width >= 600.0,
            share_labelled: width >= 480.0,
        }
    }
}

fn wide_row(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    state: &UiState,
    commands: &mut Vec<RoomCommand>,
    open_settings: &mut bool,
) {
    let fit = DockFit::for_width(ui.available_width());
    let mut mute_commands = Vec::new();
    theme::split_row(
        ui,
        theme::CONTROL_HEIGHT,
        |ui| {
            own_lives::share_buttons(
                ui,
                snapshot,
                state,
                commands,
                own_lives::ShareLayout::Row {
                    share_labelled: fit.share_labelled,
                    tools_labelled: fit.tools_labelled,
                },
            );
            ui.add_space(GROUP_GAP);
            own_lives::audio_controls(ui, snapshot, commands, fit.audio_status, fit.audio_labelled);
        },
        |ui| {
            settings_button(ui, open_settings);
            master_mute(ui, snapshot, &mut mute_commands, fit.tools_labelled);
            if fit.upload {
                ui.add_space(GROUP_GAP);
                upload(ui, state);
            }
        },
    );
    commands.extend(mute_commands);
}

fn settings_button(ui: &mut egui::Ui, open_settings: &mut bool) {
    if theme::icon_button(ui, icons::SETTINGS, "Settings", true).clicked() {
        *open_settings = true;
    }
}

/// Whatever needs saying right now, on a line above the controls. It opens and shuts on a spring, so
/// the video above it gives up and takes back that height smoothly.
fn notices(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    state: &UiState,
    update_state: &UpdateState,
    update_clicked: &mut bool,
) {
    let audio_error = match &snapshot.own_audio.state {
        AudioCaptureState::Failed(error) => Some(error),
        _ => None,
    };
    let audio_limit = match &snapshot.own_audio.state {
        AudioCaptureState::Limited(notice) => Some(notice),
        _ => None,
    };
    let has_notice = !state.status.is_empty()
        || state.share_pending
        || update_state.available.is_some()
        || snapshot.audio_output_error.is_some()
        || audio_error.is_some()
        || audio_limit.is_some();
    motion::unfold(ui, "dock-notices", has_notice, |ui| {
        ui.horizontal_wrapped(|ui| {
            if state.share_pending {
                theme::spinner(ui, 16.0);
                ui.weak("Starting your stream…");
            }
            if !state.status.is_empty() {
                ui.weak(state.status.as_str());
            }
            if update::draw(ui, update_state) {
                *update_clicked = true;
            }
            if let Some(error) = &snapshot.audio_output_error {
                ui.colored_label(theme::error(), format!("No audio output: {error}"));
            }
            if let Some(error) = audio_error {
                ui.colored_label(theme::error(), format!("Audio failed: {error}"));
            }
            if let Some(notice) = audio_limit {
                ui.colored_label(theme::warning(), notice);
            }
        });
        ui.add_space(4.0);
    });
}

fn master_mute(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    commands: &mut Vec<RoomCommand>,
    labelled: bool,
) {
    let mut muted = snapshot.master_mute;
    let glyph = if muted { icons::MUTED } else { icons::VOLUME };
    let response = if labelled {
        theme::toggle(ui, &mut muted, glyph, "Mute all")
    } else {
        theme::icon_toggle(ui, &mut muted, glyph, "Mute all")
    };
    if response.changed() {
        commands.push(RoomCommand::SetMasterMute(muted));
    }
}

fn upload(ui: &mut egui::Ui, state: &UiState) {
    ui.spacing_mut().item_spacing.x = 4.0;
    ui.label(theme::figure(format!(
        "↑ {}",
        own_lives::bitrate(state.upload_kbps)
    )))
    .on_hover_text("What you are sending to the room");
}

/// Clicking the open sheet's button closes it; clicking the other's switches straight to it.
fn sheet_button(
    ui: &mut egui::Ui,
    state: &mut UiState,
    sheet: CompactSheet,
    glyph: &str,
    text: &str,
) {
    let mut selected = state.sheet_open && state.sheet == sheet;
    if theme::toggle(ui, &mut selected, glyph, text).changed() {
        state.sheet = sheet;
        state.sheet_open = selected;
    }
}

#[cfg(test)]
mod tests {
    use brp_audio::AudioSelection;
    use brp_room::OwnAudioView;

    use super::*;

    fn room() -> RoomSnapshot {
        RoomSnapshot {
            me: iroh::SecretKey::from_bytes(&[1; 32]).public(),
            nickname: "Alex".into(),
            version: 1,
            members: Vec::new(),
            own_lives: Vec::new(),
            watches: Vec::new(),
            own_audio: OwnAudioView {
                enabled: true,
                state: AudioCaptureState::Capturing,
                // The longest thing the status can say, so the widths hold at the worst case.
                selection: AudioSelection::Only(Default::default()),
                subscribers: 12,
                packets_encoded: 0,
            },
            audio_output_error: None,
            master_mute: false,
        }
    }

    /// Run at the width the dock itself gets, which also covers a rail dragged to its widest.
    fn dock_text(width: f32) -> Vec<(String, egui::Rect)> {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let snapshot = room();
        let mut state = UiState::new();
        state.upload_kbps = 123_456;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 200.0),
                )),
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ui, |ui| {
                        wide_row(ui, &snapshot, &state, &mut Vec::new(), &mut false);
                    });
            },
        );
        output.textures_delta.clear();
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some((
                    text.galley.text().to_string(),
                    text.galley.rect.translate(text.pos.to_vec2()),
                )),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_dock_stays_on_one_row_with_nothing_touching_at_every_width() {
        for width in (396..=1800).step_by(4) {
            let width = width as f32;
            let text = dock_text(width);
            let tops: Vec<f32> = text.iter().map(|(_, rect)| rect.center().y).collect();
            let (lowest, highest) = tops.iter().fold((f32::MAX, f32::MIN), |(low, high), y| {
                (low.min(*y), high.max(*y))
            });
            assert!(
                highest - lowest < 4.0,
                "at {width} it took a second row: {text:?}"
            );
            for (name, rect) in &text {
                assert!(
                    rect.left() >= 0.0 && rect.right() <= width,
                    "at {width} {name:?} leaves the dock: {rect:?}"
                );
            }
            for (index, (name, rect)) in text.iter().enumerate() {
                for (other, other_rect) in &text[index + 1..] {
                    assert!(
                        !rect.intersects(*other_rect),
                        "at {width} {name:?} overlaps {other:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn full_dock_keeps_controls_inside_its_clip() {
        for width in [640.0, 800.0, 960.0, 1160.0, 1600.0] {
            let ctx = egui::Context::default();
            theme::install(&ctx);
            let mut state = UiState::new();
            state.status = "An update is available".into();
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 720.0));
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    ..Default::default()
                },
                |ui| {
                    draw(
                        ui,
                        &room(),
                        &mut state,
                        &mut Vec::new(),
                        &mut false,
                        &UpdateState::new(false),
                        &mut false,
                    );
                },
            );
            output.textures_delta.clear();
            for shape in &output.shapes {
                if let egui::Shape::Text(text) = &shape.shape {
                    if !text.galley.mesh_bounds.is_finite() {
                        continue;
                    }
                    let ink = text.galley.mesh_bounds.translate(text.pos.to_vec2());
                    assert!(
                        shape.clip_rect.contains_rect(ink),
                        "{width}: {:?} clipped ink={ink:?} clip={:?}",
                        text.galley.text(),
                        shape.clip_rect
                    );
                    assert!(screen.contains_rect(ink));
                }
            }
        }
    }

    #[test]
    fn the_main_action_keeps_its_words_longest() {
        for width in (396..=1800).step_by(4) {
            let fit = DockFit::for_width(width as f32);
            assert!(fit.share_labelled || !fit.tools_labelled);
            assert!(fit.tools_labelled || !fit.audio_status);
            assert!(fit.audio_labelled || !fit.tools_labelled);
        }
    }
}

//! Voice in the dock: one switch to be in voice or not, and while in it the microphone and
//! headphones toggles. It emits commands and never touches the room.

use std::collections::VecDeque;

use brp_room::{MemberView, OwnVoiceView, RoomSnapshot};
use egui::emath::GuiRounding;
use egui::{Color32, Rect, pos2};

use super::motion;
use super::state::UiState;
use super::theme::{self, icons};
use crate::commands::RoomCommand;

const SWITCH_INSET: f32 = 10.0;
/// A peak of about -26 dBFS: above an open microphone's noise floor, below quiet speech.
pub const SPEAKING_LEVEL: f32 = 0.05;
const BAR: f32 = 3.0;
const PITCH: f32 = 4.0;
/// Bars on a tile's name chip.
pub const CHIP_BARS: usize = 3;
const DIMMED: f32 = 0.45;
const SHORTEST_BAR: f32 = 0.1;

/// How much of the voice controls the dock has room for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceLayout {
    /// The switch and, while in voice, the toggles, on one surface.
    Surface,
    /// Only the toggles, and only while in voice; the switch lives in the Room sheet.
    IconsOnly,
}

pub fn controls(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    commands: &mut Vec<RoomCommand>,
    layout: VoiceLayout,
) {
    let voice = &snapshot.voice;
    // Painted under the controls afterwards for the same reason as the audio surface.
    let backing = ui.painter().add(egui::Shape::Noop);
    let lit = motion::follow(
        ui.ctx(),
        ui.id().with("voice-on-air"),
        voice.transmitting,
        motion::HOVER,
    );
    let mut switch_rect = None;
    if layout == VoiceLayout::Surface {
        ui.add_space(SWITCH_INSET);
        let mut joined = voice.joined;
        let switch = theme::switch(ui, &mut joined, "Voice");
        if switch.changed() {
            commands.push(if joined {
                RoomCommand::JoinVoice
            } else {
                RoomCommand::LeaveVoice
            });
        }
        switch_rect = Some(switch.rect);
    }
    let last = voice.joined.then(|| toggles(ui, voice, commands));
    if let Some(switch) = switch_rect {
        let right = last.map_or(switch.right() + SWITCH_INSET, |last| last.right());
        let middle = switch.center().y;
        ui.painter().set(
            backing,
            egui::Shape::rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(
                        switch.left() - SWITCH_INSET,
                        middle - theme::CONTROL_HEIGHT / 2.0,
                    ),
                    egui::pos2(right, middle + theme::CONTROL_HEIGHT / 2.0),
                ),
                theme::RADIUS_CONTROL,
                theme::raised().lerp_to_gamma(theme::signal_surface(), lit),
            ),
        );
    }
}

/// Returns the rect of the last toggle, where the surface ends.
fn toggles(ui: &mut egui::Ui, voice: &OwnVoiceView, commands: &mut Vec<RoomCommand>) -> egui::Rect {
    let usual_ink = ui.visuals().widgets.inactive.fg_stroke.color;
    let mut muted = voice.muted;
    let (glyph, name) = if muted {
        (icons::MIC_OFF, "Unmute microphone")
    } else {
        (icons::MIC, "Mute microphone")
    };
    if voice.transmitting {
        ui.visuals_mut().widgets.inactive.fg_stroke.color = theme::signal();
    }
    let microphone = theme::toned_icon_toggle(ui, theme::Tone::Quiet, &mut muted, glyph, name);
    ui.visuals_mut().widgets.inactive.fg_stroke.color = usual_ink;
    if microphone.changed() {
        commands.push(RoomCommand::SetVoiceMuted(muted));
    }
    let mut deafened = voice.deafened;
    let (glyph, name) = if deafened {
        (icons::HEADPHONE_OFF, "Undeafen")
    } else {
        (icons::HEADPHONES, "Deafen")
    };
    let headphones = theme::toned_icon_toggle(ui, theme::Tone::Quiet, &mut deafened, glyph, name);
    if headphones.changed() {
        commands.push(RoomCommand::SetVoiceDeafened(deafened));
    }
    headphones.rect
}

/// The dock's notice for a microphone that is not working.
pub fn microphone_notice(error: &str) -> String {
    format!(
        "No microphone. Check your input device or choose another in Settings, then turn Voice \
         off and on. ({error})"
    )
}

pub fn bar_ink(level: f32) -> Color32 {
    if level > SPEAKING_LEVEL {
        theme::signal()
    } else {
        theme::muted()
    }
}

pub fn is_speaking(levels: Option<&VecDeque<f32>>) -> bool {
    levels
        .and_then(|levels| levels.back())
        .is_some_and(|level| *level > SPEAKING_LEVEL)
}

/// The same bars as `traffic_strip`, newest on the right. With reduced motion the strip would
/// flicker without moving anywhere, so it holds still and says the word instead.
pub fn level_strip(ui: &egui::Ui, rect: Rect, levels: Option<&VecDeque<f32>>, dimmed: bool) {
    if motion::ambient_allowed(ui.ctx()) {
        bars(ui.painter(), rect, levels, dimmed);
    } else if is_speaking(levels) {
        let galley = ui.painter().layout_no_wrap(
            "speaking".to_string(),
            theme::medium(13.0),
            theme::signal(),
        );
        let at = pos2(
            rect.right() - galley.size().x,
            rect.center().y - galley.size().y / 2.0,
        );
        ui.painter().galley(at, galley, theme::signal());
    }
}

/// The tile chip's three bars, which keep drawing under reduced motion because the chip has no
/// room for a word.
pub fn chip_mark(painter: &egui::Painter, rect: Rect, levels: &VecDeque<f32>) {
    bars(painter, rect, Some(levels), false);
}

fn bars(painter: &egui::Painter, rect: Rect, levels: Option<&VecDeque<f32>>, dimmed: bool) {
    let Some(levels) = levels else {
        return;
    };
    let height = rect.height() - 2.0;
    let count = ((rect.width() / PITCH) as usize).min(levels.len());
    for (index, level) in levels.iter().skip(levels.len() - count).enumerate() {
        let x = rect.right() - BAR - (count - 1 - index) as f32 * PITCH;
        let tall = level.clamp(SHORTEST_BAR, 1.0) * height;
        let ink = if dimmed {
            bar_ink(*level).gamma_multiply(DIMMED)
        } else {
            bar_ink(*level)
        };
        let shape = Rect::from_min_max(
            pos2(x, rect.top() + height - tall),
            pos2(x + BAR, rect.top() + height),
        );
        painter.rect_filled(
            shape.round_to_pixels(painter.ctx().pixels_per_point()),
            0,
            ink,
        );
    }
}

/// Voice bytes per second on a person's wire, as (towards them, back from them). The map and the
/// rail share it so the two never disagree about who is sending.
pub fn wire_rates(snapshot: &RoomSnapshot, state: &UiState, member: &MemberView) -> (f64, f64) {
    let rate = |id| {
        state
            .voice
            .get(id)
            .map_or(0.0, |meter| meter.traffic.bytes_per_second)
    };
    let in_voice = member.voice.is_some();
    (
        if in_voice && member.sending {
            rate(&snapshot.me)
        } else {
            0.0
        },
        if member.hearing {
            rate(&member.id)
        } else {
            0.0
        },
    )
}

#[cfg(test)]
mod tests {
    use brp_room::{AudioCaptureState, OwnAudioView};

    use super::*;

    fn pass(
        ctx: &egui::Context,
        snapshot: &RoomSnapshot,
        events: Vec<egui::Event>,
        frame: u32,
    ) -> (Vec<RoomCommand>, Vec<(String, egui::Rect)>) {
        let mut commands = Vec::new();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 200.0),
                )),
                time: Some(f64::from(frame) / 60.0),
                events,
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            theme::uniform_controls(ui);
                            controls(ui, snapshot, &mut commands, VoiceLayout::Surface);
                        });
                    });
            },
        );
        output.textures_delta.clear();
        let text = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some((
                    text.galley.text().to_string(),
                    text.galley.rect.translate(text.pos.to_vec2()),
                )),
                _ => None,
            })
            .collect();
        (commands, text)
    }

    fn click(snapshot: &RoomSnapshot, target: &str) -> Vec<RoomCommand> {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        pass(&ctx, snapshot, Vec::new(), 0);
        let (_, text) = pass(&ctx, snapshot, Vec::new(), 1);
        let at = text
            .iter()
            .find(|(shown, _)| shown.contains(target))
            .unwrap_or_else(|| panic!("no {target:?} in {text:?}"))
            .1
            .center();
        let mut commands = Vec::new();
        for (frame, pressed) in [(2, true), (3, false)] {
            let events = vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ];
            commands.extend(pass(&ctx, snapshot, events, frame).0);
        }
        commands
    }

    fn snapshot(voice: OwnVoiceView) -> RoomSnapshot {
        RoomSnapshot {
            voice,
            me: iroh::SecretKey::from_bytes(&[1; 32]).public(),
            nickname: "Alex".into(),
            version: 1,
            members: Vec::new(),
            own_lives: Vec::new(),
            watches: Vec::new(),
            own_audio: OwnAudioView {
                enabled: false,
                state: AudioCaptureState::Off,
                selection: Default::default(),
                subscribers: 0,
                packets_encoded: 0,
            },
            audio_output_error: None,
            master_mute: false,
        }
    }

    fn joined() -> OwnVoiceView {
        OwnVoiceView {
            joined: true,
            ..Default::default()
        }
    }

    #[test]
    fn the_switch_joins_and_leaves_voice() {
        assert_eq!(
            click(&snapshot(OwnVoiceView::default()), "Voice"),
            [RoomCommand::JoinVoice]
        );
        assert_eq!(
            click(&snapshot(joined()), "Voice"),
            [RoomCommand::LeaveVoice]
        );
    }

    #[test]
    fn the_icons_mute_and_deafen() {
        let snapshot_in_voice = snapshot(joined());
        assert_eq!(
            click(&snapshot_in_voice, icons::MIC),
            [RoomCommand::SetVoiceMuted(true)]
        );
        assert_eq!(
            click(&snapshot_in_voice, icons::HEADPHONES),
            [RoomCommand::SetVoiceDeafened(true)]
        );
        let muted = snapshot(OwnVoiceView {
            muted: true,
            deafened: true,
            ..joined()
        });
        assert_eq!(
            click(&muted, icons::MIC_OFF),
            [RoomCommand::SetVoiceMuted(false)]
        );
        assert_eq!(
            click(&muted, icons::HEADPHONE_OFF),
            [RoomCommand::SetVoiceDeafened(false)]
        );
    }

    fn painted(reduced_motion: bool, draw: impl Fn(&egui::Ui)) -> Vec<egui::Shape> {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        motion::set_less_motion(&ctx, reduced_motion);
        let mut shapes = Vec::new();
        for frame in 0..2 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(200.0, 100.0),
                    )),
                    time: Some(f64::from(frame) / 60.0),
                    ..Default::default()
                },
                |ui| draw(ui),
            );
            output.textures_delta.clear();
            shapes = output
                .shapes
                .into_iter()
                .map(|clipped| clipped.shape)
                .collect();
        }
        shapes
    }

    fn fills(shapes: &[egui::Shape]) -> Vec<(Rect, Color32)> {
        shapes
            .iter()
            .filter_map(|shape| match shape {
                egui::Shape::Rect(rect) => Some((rect.rect, rect.fill)),
                _ => None,
            })
            .collect()
    }

    fn levels(values: &[f32]) -> VecDeque<f32> {
        values.iter().copied().collect()
    }

    #[test]
    fn loud_bars_are_signal_and_quiet_ones_are_muted() {
        let history = levels(&[0.01, 0.5]);
        let shapes = painted(false, |ui| {
            level_strip(
                ui,
                Rect::from_min_size(pos2(10.0, 10.0), egui::vec2(72.0, 12.0)),
                Some(&history),
                false,
            );
        });
        let bars = fills(&shapes);
        assert_eq!(bars.len(), 2);
        assert_eq!(bars[0].1, theme::muted());
        assert_eq!(bars[1].1, theme::signal());
        assert!(bars[1].0.left() > bars[0].0.left(), "newest on the right");
        assert!(bars[1].0.height() > bars[0].0.height());
    }

    #[test]
    fn with_reduced_motion_a_speaker_reads_speaking() {
        let rect = Rect::from_min_size(pos2(10.0, 10.0), egui::vec2(72.0, 12.0));
        let words = |history: VecDeque<f32>| {
            let shapes = painted(true, |ui| level_strip(ui, rect, Some(&history), false));
            assert!(fills(&shapes).is_empty(), "the bars hold still");
            shapes
                .iter()
                .filter_map(|shape| match shape {
                    egui::Shape::Text(text) => Some(text.galley.text().to_string()),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(words(levels(&[0.5])), ["speaking"]);
        assert!(words(levels(&[0.5, 0.0])).is_empty());
    }

    #[test]
    fn a_chip_mark_shows_the_last_three_levels() {
        let history = levels(&[1.0, 1.0, 0.2, 0.4, 0.6]);
        let shapes = painted(true, |ui| {
            chip_mark(
                ui.painter(),
                Rect::from_min_size(pos2(10.0, 10.0), egui::vec2(PITCH * CHIP_BARS as f32, 12.0)),
                &history,
            );
        });
        let heights: Vec<f32> = fills(&shapes)
            .iter()
            .map(|(rect, _)| rect.height())
            .collect();
        assert_eq!(heights.len(), CHIP_BARS);
        assert!(
            heights[0] < heights[1] && heights[1] < heights[2],
            "{heights:?}"
        );
    }
}

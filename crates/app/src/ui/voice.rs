//! Voice in the dock: one switch to be in voice or not, and while in it the microphone and
//! headphones toggles. It emits commands and never touches the room.

use std::collections::VecDeque;

use brp_room::{MemberView, OwnVoiceView, RoomSnapshot};
use egui::emath::GuiRounding;
use egui::{Color32, Rect, pos2};

use super::members::{self, PEER_INDENT, ROW_HEIGHT, STREAM_INDENT, STRIP_HEIGHT, right_text};
use super::motion;
use super::state::UiState;
use super::theme::{self, icons};
use crate::commands::RoomCommand;

const SWITCH_INSET: f32 = 10.0;
/// A peak of about -26 dBFS: above an open microphone's noise floor, below quiet speech.
pub const SPEAKING_LEVEL: f32 = 0.05;
/// The same bar width and pitch as `traffic_strip`, so the two meters read as one family.
const BAR: f32 = 3.0;
pub const PITCH: f32 = 4.0;
/// Enough bars to show a trend on a chip without crowding the name.
pub const CHIP_BARS: usize = 3;
/// Someone you cannot hear is shown, but quieter than someone you can.
const DIMMED: f32 = 0.45;
/// A silent moment still leaves a visible tick, so the strip never looks absent.
const SHORTEST_BAR: f32 = 0.1;
/// The room between a row's word, its strip and what stands beside them.
const LABEL_GAP: f32 = 8.0;

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

/// The word for your own voice state and the colour it is drawn in, shared by every place that
/// shows it so the words always match.
pub fn own_state(snapshot: &RoomSnapshot) -> Option<(&'static str, Color32)> {
    let own = &snapshot.voice;
    if !own.joined {
        None
    } else if own.error.is_some() {
        Some(("no microphone", theme::error()))
    } else if own.deafened {
        Some(("deafened", theme::secondary()))
    } else if own.muted {
        Some(("muted", theme::secondary()))
    } else if own.transmitting {
        Some(("on air", theme::signal()))
    } else {
        None
    }
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
pub fn level_strip(
    painter: &egui::Painter,
    rect: Rect,
    levels: Option<&VecDeque<f32>>,
    dimmed: bool,
) {
    if motion::ambient_allowed(painter.ctx()) {
        bars(painter, rect, levels, dimmed);
    } else if is_speaking(levels) {
        let galley =
            painter.layout_no_wrap("speaking".to_string(), theme::medium(13.0), theme::signal());
        let at = pos2(
            rect.right() - galley.size().x,
            rect.center().y - galley.size().y / 2.0,
        );
        painter.galley(at, galley, theme::signal());
    }
}

/// The tile chip's bars. Under reduced motion they hold one height and only their colour says
/// whether the person is speaking, because the chip has no room for a word.
pub fn chip_mark(painter: &egui::Painter, rect: Rect, levels: &VecDeque<f32>) {
    if motion::ambient_allowed(painter.ctx()) {
        bars(painter, rect, Some(levels), false);
        return;
    }
    let still: VecDeque<f32> = levels
        .back()
        .map_or_else(VecDeque::new, |newest| [*newest; CHIP_BARS].into());
    bars(painter, rect, Some(&still), false);
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

/// Your own voice row: the strip, and the word for how you are heard. Returns the row's centre y,
/// for its wire.
pub fn own_row(ui: &mut egui::Ui, snapshot: &RoomSnapshot, state: &UiState) -> f32 {
    let (rect, row) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ROW_HEIGHT),
        egui::Sense::hover(),
    );
    row.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, "Voice"));
    let painter = ui.painter();
    let right = own_state(snapshot).map_or(rect.right(), |(word, ink)| {
        right_text(painter, rect.right_center(), word, ink)
    });
    let label_end = voice_label(painter, rect, rect.left() + PEER_INDENT, theme::text());
    let strip = Rect::from_min_max(
        pos2(label_end + LABEL_GAP, rect.center().y - STRIP_HEIGHT / 2.0),
        pos2(right - LABEL_GAP, rect.center().y + STRIP_HEIGHT / 2.0),
    );
    if strip.width() > 0.0 {
        let levels = state.voice.get(&snapshot.me).map(|meter| &meter.levels);
        level_strip(painter, strip, levels, false);
    }
    rect.center().y
}

/// Someone else's voice row and its control line. Returns the row's centre y, for its wire.
pub fn member_rows(
    ui: &mut egui::Ui,
    member: &MemberView,
    state: &UiState,
    commands: &mut Vec<RoomCommand>,
) -> f32 {
    let dimmed = !member.hearing;
    let fade = |ink: Color32| {
        if dimmed {
            ink.gamma_multiply(DIMMED)
        } else {
            ink
        }
    };
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ROW_HEIGHT),
        egui::Sense::hover(),
    );
    let hear = members::tick(
        ui,
        rect,
        ui.id().with(("hear", member.id)),
        member.hearing,
        &format!("Hear {}", member.nickname),
        fade(theme::text()),
    )
    .on_hover_text(format!("Hear {}", member.nickname));
    let painter = ui.painter();
    if hear.clicked() {
        commands.push(RoomCommand::SetHearing {
            peer: member.id,
            hearing: !member.hearing,
        });
    }
    let silenced = member.voice.as_ref().and_then(|voice| {
        if voice.deafened {
            Some("deafened")
        } else if voice.muted {
            Some("muted")
        } else {
            None
        }
    });
    // The tick's own rect, inside the wider area it listens on.
    let right = hear.rect.left() + 2.0 - LABEL_GAP;
    let label_end = voice_label(
        painter,
        rect,
        rect.left() + STREAM_INDENT,
        fade(theme::text()),
    );
    if let Some(word) = silenced {
        right_text(
            painter,
            pos2(right, rect.center().y),
            word,
            fade(theme::secondary()),
        );
    } else {
        let strip = Rect::from_min_max(
            pos2(label_end + LABEL_GAP, rect.center().y - STRIP_HEIGHT / 2.0),
            pos2(right, rect.center().y + STRIP_HEIGHT / 2.0),
        );
        if strip.width() > 0.0 {
            let levels = state.voice.get(&member.id).map(|meter| &meter.levels);
            level_strip(painter, strip, levels, dimmed);
        }
    }
    ui.horizontal(|ui| {
        ui.add_space(STREAM_INDENT);
        let deafen = ui
            .painter()
            .layout_no_wrap("Deafen".into(), theme::medium(14.0), Color32::PLACEHOLDER)
            .mesh_bounds
            .width()
            + 2.0 * ui.spacing().button_padding.x
            + ui.spacing().item_spacing.x;
        ui.spacing_mut().slider_width = (ui.available_width() - deafen).max(0.0);
        let mut gain = member.voice_gain;
        ui.push_id(("voice-volume", member.id), |ui| {
            super::volume_slider(ui, &mut gain);
        });
        if gain != member.voice_gain {
            commands.push(RoomCommand::SetVoiceVolume {
                peer: member.id,
                gain,
            });
        }
        let mut deafened = !member.sending;
        let toggle = theme::quiet_toggle(ui, &mut deafened, "Deafen")
            .on_hover_text(format!("{} stops hearing you", member.nickname));
        if toggle.changed() {
            commands.push(RoomCommand::SetSending {
                peer: member.id,
                sending: !deafened,
            });
        }
    });
    rect.center().y
}

/// Returns where the word ends.
fn voice_label(painter: &egui::Painter, row: Rect, left: f32, ink: Color32) -> f32 {
    let galley = painter.layout_no_wrap("Voice".into(), theme::medium(14.0), ink);
    let end = left + galley.size().x;
    painter.galley(
        pos2(left, row.center().y - galley.size().y / 2.0),
        galley,
        ink,
    );
    end
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
                ui.painter(),
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
            let shapes = painted(true, |ui| {
                level_strip(ui.painter(), rect, Some(&history), false)
            });
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

    fn chip_heights(reduced_motion: bool, history: &VecDeque<f32>) -> Vec<(f32, Color32)> {
        let shapes = painted(reduced_motion, |ui| {
            chip_mark(
                ui.painter(),
                Rect::from_min_size(pos2(10.0, 10.0), egui::vec2(PITCH * CHIP_BARS as f32, 12.0)),
                history,
            );
        });
        fills(&shapes)
            .iter()
            .map(|(rect, ink)| (rect.height(), *ink))
            .collect()
    }

    #[test]
    fn a_chip_mark_shows_the_last_three_levels() {
        let bars = chip_heights(false, &levels(&[1.0, 1.0, 0.2, 0.4, 0.6]));
        assert_eq!(bars.len(), CHIP_BARS);
        assert!(bars[0].0 < bars[1].0 && bars[1].0 < bars[2].0, "{bars:?}");
    }

    #[test]
    fn with_reduced_motion_a_chip_mark_holds_still_and_keeps_its_colour() {
        let loud = chip_heights(true, &levels(&[0.2, 0.4, 0.6]));
        assert_eq!(loud.len(), CHIP_BARS);
        assert!(
            loud.iter()
                .all(|bar| bar.0 == loud[0].0 && bar.1 == theme::signal())
        );
        let quiet = chip_heights(true, &levels(&[0.6, 0.0]));
        assert!(quiet.iter().all(|bar| bar.1 == theme::muted()));
    }

    fn person(voice: Option<brp_proto::VoiceState>) -> MemberView {
        MemberView {
            voice,
            hearing: true,
            sending: true,
            voice_gain: 1.0,
            rtt_ms: None,
            signal_lost: false,
            watching_me: false,
            id: iroh::SecretKey::from_bytes(&[2; 32]).public(),
            nickname: "Morgan".into(),
            lives: Vec::new(),
            seen_ago_ms: 0,
            path: brp_net::PathKind::Direct,
            has_audio: false,
            audio_mix: None,
            gain: 1.0,
        }
    }

    fn row_pass(
        ctx: &egui::Context,
        member: &MemberView,
        events: Vec<egui::Event>,
        frame: u32,
    ) -> (Vec<RoomCommand>, Vec<(String, Rect)>) {
        let state = UiState::new();
        let mut commands = Vec::new();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 200.0),
                )),
                time: Some(f64::from(frame) / 60.0),
                events,
                ..Default::default()
            },
            |ui| {
                theme::uniform_controls(ui);
                member_rows(ui, member, &state, &mut commands);
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

    fn click_row(member: &MemberView, target: &str) -> Vec<RoomCommand> {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        row_pass(&ctx, member, Vec::new(), 0);
        let (_, text) = row_pass(&ctx, member, Vec::new(), 1);
        let at = text
            .iter()
            .find(|(shown, _)| shown == target)
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
            commands.extend(row_pass(&ctx, member, events, frame).0);
        }
        commands
    }

    #[test]
    fn unchecking_hear_mutes_that_person() {
        let member = person(Some(Default::default()));
        assert_eq!(
            click_row(&member, "[x]"),
            [RoomCommand::SetHearing {
                peer: member.id,
                hearing: false
            }]
        );
    }

    #[test]
    fn deafen_on_a_person_stops_sending_to_them() {
        let member = person(Some(Default::default()));
        assert_eq!(
            click_row(&member, "Deafen"),
            [RoomCommand::SetSending {
                peer: member.id,
                sending: false
            }]
        );
    }

    #[test]
    fn a_self_muted_person_reads_muted_instead_of_a_strip() {
        let member = person(Some(brp_proto::VoiceState {
            muted: true,
            deafened: false,
        }));
        let ctx = egui::Context::default();
        theme::install(&ctx);
        row_pass(&ctx, &member, Vec::new(), 0);
        let (_, text) = row_pass(&ctx, &member, Vec::new(), 1);
        assert!(text.iter().any(|(shown, _)| shown == "muted"), "{text:?}");
    }

    #[test]
    fn your_row_has_no_count_of_listeners() {
        let mut snapshot = snapshot(OwnVoiceView {
            transmitting: true,
            ..joined()
        });
        snapshot.members.push(person(Some(Default::default())));
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut output = ctx.run_ui(Default::default(), |ui| {
            own_row(ui, &snapshot, &UiState::new());
        });
        output.textures_delta.clear();
        let shapes: Vec<egui::Shape> = output.shapes.into_iter().map(|c| c.shape).collect();
        let words: Vec<String> = shapes
            .iter()
            .filter_map(|shape| match shape {
                egui::Shape::Text(text) => Some(text.galley.text().to_string()),
                _ => None,
            })
            .collect();
        assert!(words.iter().any(|word| word == "Voice"), "{words:?}");
        assert!(
            words
                .iter()
                .all(|word| !word.chars().any(|c| c.is_ascii_digit())),
            "{words:?}"
        );
    }
}

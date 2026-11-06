//! Voice in the dock: one switch to be in voice or not, and while in it the microphone and
//! headphones toggles. It emits commands and never touches the room.

use brp_room::{OwnVoiceView, RoomSnapshot};

use super::motion;
use super::theme::{self, icons};
use crate::commands::RoomCommand;

const SWITCH_INSET: f32 = 10.0;

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
}

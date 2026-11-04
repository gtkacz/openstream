//! The room rail: invite actions, then every person in the room with the streams they offer. You
//! come first, in the same shape as everyone else, with the controls for your own streams.

use std::fmt::Debug;
use std::hash::Hash;

use brp_net::PathKind;
use brp_proto::constants::SOURCE_PRESET_ID;
use brp_proto::{LiveInfo, Preset};
use brp_room::{MemberView, RoomSnapshot, WatchState};

use super::motion;
use super::own_lives;
use super::state::{UiState, ordered_members};
use super::theme::{self, icons};
use crate::commands::RoomCommand;
use crate::link;

const AVATAR_SIZE: f32 = 28.0;

/// Draws the rail and pushes `Watch`/`Unwatch` commands for switch and preset changes.
pub fn draw(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    ticket: &str,
    state: &mut UiState,
    commands: &mut Vec<RoomCommand>,
) {
    egui::Panel::left("members")
        .default_size(288.0)
        .min_size(280.0)
        .max_size(380.0)
        .resizable(true)
        .frame(theme::rail())
        .show(ui, |ui| {
            theme::brand(ui);
            ui.add_space(12.0);
            contents(ui, snapshot, ticket, state, commands);
        });
}

/// What the rail holds below the brand, shared with the compact window's Room sheet.
pub fn contents(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    ticket: &str,
    state: &mut UiState,
    commands: &mut Vec<RoomCommand>,
) {
    // Two equal columns, so the pair ends on the same edge as the cards below it.
    ui.columns(2, |columns| {
        copy_button(&mut columns[0], icons::LINK, "Copy link", || {
            link::share_link(ticket)
        });
        copy_button(&mut columns[1], icons::TICKET, "Copy ticket", || {
            ticket.to_string()
        });
    });
    ui.add_space(12.0);
    // `snapshot.members` excludes this participant, so count it in.
    let members = snapshot.members.len() + 1;
    let plural = if members == 1 { "" } else { "s" };
    theme::section_heading(ui, &format!("{members} member{plural} in this room"));
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            own_block(ui, snapshot, state, commands);
            if snapshot.members.is_empty() {
                ui.add_space(12.0);
                ui.weak("You’re the first here");
                ui.weak("Copy an invite link to bring someone in");
            }
            for member in ordered_members(snapshot) {
                motion::reveal(ui, member.id, |ui| {
                    ui.add_space(16.0);
                    member_block(ui, snapshot, member, state, commands);
                });
            }
        });
}

fn own_block(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    state: &mut UiState,
    commands: &mut Vec<RoomCommand>,
) {
    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
        avatar(ui, snapshot.me.as_bytes(), &snapshot.nickname);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.add(
                egui::Label::new(egui::RichText::new(&snapshot.nickname).font(theme::medium(14.0)))
                    .truncate(),
            )
            .on_hover_text(snapshot.me.fmt_short().to_string());
            ui.label(egui::RichText::new("You").small().color(theme::SECONDARY));
        });
    });
    for live in &snapshot.own_lives {
        motion::reveal(ui, ("own-live", live.info.id), |ui| {
            ui.add_space(8.0);
            own_lives::live_card(ui, live, state, commands);
        });
    }
}

fn member_block(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    member: &MemberView,
    state: &mut UiState,
    commands: &mut Vec<RoomCommand>,
) {
    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
        avatar(ui, member.id.as_bytes(), &member.nickname);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.add(
                egui::Label::new(egui::RichText::new(&member.nickname).font(theme::medium(14.0)))
                    .truncate(),
            )
            .on_hover_text(format!("{} · {}", member.nickname, member.id.fmt_short()));
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                path_glyph(ui, member.path);
                ui.label(
                    egui::RichText::new(path_badge(member.path))
                        .small()
                        .color(theme::SECONDARY),
                );
            });
        });
    });
    if member.has_audio {
        ui.horizontal_wrapped(|ui| {
            if let Some(gain) = super::volume_control(ui, ("member-volume", member.id), member.gain)
            {
                commands.push(RoomCommand::SetVolume {
                    publisher: member.id,
                    gain,
                });
            }
        });
    }
    for live in &member.lives {
        motion::reveal(ui, ("member-live", live.id), |ui| {
            ui.add_space(8.0);
            live_row(ui, snapshot, member, live, state, commands);
        });
    }
}

fn live_row(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    member: &MemberView,
    live: &LiveInfo,
    state: &mut UiState,
    commands: &mut Vec<RoomCommand>,
) {
    let key = (member.id, live.id);
    let watch = snapshot
        .watches
        .iter()
        .find(|w| w.publisher == key.0 && w.live_id == key.1);
    let mut watched = watch.is_some();
    let wanted = watch
        .map(|w| w.preset_id)
        .or_else(|| state.preset_choice.get(&key).copied());
    let raised = motion::follow(
        ui.ctx(),
        ui.id().with(("live-watched", key)),
        watched,
        motion::TRAVEL,
    );
    theme::card()
        // Stops short of the controls' own fill, so the dropdown stays visible on a watched card.
        .fill(theme::SURFACE.lerp_to_gamma(theme::RAISED, raised * 0.45))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), 20.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    theme::live_dot(ui, true);
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(&live.title).font(theme::medium(14.0)),
                        )
                        .truncate(),
                    )
                    .on_hover_text(&live.title);
                },
            );
            let detail = match watch.map(|w| &w.state) {
                Some(WatchState::Connecting) => "Connecting…".to_string(),
                Some(WatchState::Reconnecting) => "Reconnecting…".to_string(),
                Some(WatchState::Ended) => "Ended".to_string(),
                Some(WatchState::Live) | None => format!(
                    "{} × {} · {} fps",
                    live.source_width, live.source_height, live.source_fps
                ),
            };
            ui.label(egui::RichText::new(detail).small().color(theme::SECONDARY));
            let Some(mut preset_id) = offered_preset(live, wanted) else {
                // The publisher advertises no presets at all; nothing to watch yet.
                ui.add_enabled_ui(false, |ui| theme::switch(ui, &mut watched, "Watch"));
                return;
            };
            let before = preset_id;
            let mut flipped = false;
            theme::split_row(
                ui,
                theme::CONTROL_HEIGHT,
                |ui| {
                    flipped = theme::switch(ui, &mut watched, "Watch")
                        .on_hover_text(&live.title)
                        .changed();
                },
                |ui| {
                    preset_selector(ui, ("member-preset", key), &live.presets, &mut preset_id);
                },
            );
            if flipped {
                commands.push(if watched {
                    RoomCommand::Watch { key, preset_id }
                } else {
                    RoomCommand::Unwatch(key)
                });
            }
            if preset_id != before {
                state.preset_choice.insert(key, preset_id);
                if watched {
                    commands.push(RoomCommand::Watch { key, preset_id });
                }
            }
        });
}

/// A person's initial on a dim tint of their own. The hue comes from their key, so a person keeps
/// their colour across rooms and across every other member's window.
fn avatar(ui: &mut egui::Ui, id: &[u8], nickname: &str) {
    let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(AVATAR_SIZE), egui::Sense::hover());
    let hue = person_hue(id);
    let fill = egui::ecolor::HsvaGamma {
        h: hue,
        s: 0.45,
        v: 0.32,
        a: 1.0,
    };
    let ink = egui::ecolor::HsvaGamma {
        h: hue,
        s: 0.28,
        v: 0.98,
        a: 1.0,
    };
    ui.painter()
        .rect_filled(rect, 6.0, egui::Color32::from(fill));
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        initial(nickname),
        theme::semibold(14.0),
        ink.into(),
    );
}

/// Hue as a fraction of the wheel, quantised to six steps so two people are either the same colour
/// or clearly different ones.
fn person_hue(id: &[u8]) -> f32 {
    const STEPS: u8 = 6;
    f32::from(id.first().copied().unwrap_or(0) % STEPS) / f32::from(STEPS)
}

fn initial(nickname: &str) -> String {
    nickname
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "?".to_string())
}

/// How this person's video reaches you: a straight line when peer to peer, a node in the middle
/// when a relay carries it, dashes while the path is unknown.
pub fn path_glyph(ui: &mut egui::Ui, path: PathKind) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(20.0, 12.0), egui::Sense::hover());
    let painter = ui.painter();
    let (left, right) = (
        rect.left_center() + egui::vec2(2.0, 0.0),
        rect.right_center() - egui::vec2(2.0, 0.0),
    );
    let ink = match path {
        PathKind::Direct | PathKind::Relayed => theme::SECONDARY,
        PathKind::Unknown => theme::MUTED,
    };
    let line = egui::Stroke::new(1.0, ink);
    match path {
        PathKind::Direct => {
            painter.line_segment([left, right], line);
        }
        PathKind::Relayed => {
            painter.line_segment([left, right], line);
            painter.circle(rect.center(), 2.5, theme::GROUND, line);
        }
        PathKind::Unknown => {
            painter.extend(egui::Shape::dashed_line(&[left, right], line, 2.0, 2.0));
        }
    }
    painter.circle_filled(left, 2.0, ink);
    painter.circle_filled(right, 2.0, ink);
}

/// Human-readable label for a member's connection path.
pub fn path_badge(path: PathKind) -> &'static str {
    match path {
        PathKind::Direct => "Direct",
        PathKind::Relayed => "Relayed",
        PathKind::Unknown => "Finding a path",
    }
}

fn copy_button(ui: &mut egui::Ui, glyph: &str, label: &str, text: impl FnOnce() -> String) {
    let id = ui.make_persistent_id(("clipboard-feedback", label));
    let now = ui.input(|input| input.time);
    let until = ui
        .ctx()
        .data(|data| data.get_temp::<f64>(id))
        .unwrap_or(0.0);
    let remaining = until - now;
    // One short word for both, so the confirmation fits the column the label did.
    let (glyph, shown) = if remaining > 0.0 {
        (icons::COPIED, "Copied")
    } else {
        (glyph, label)
    };
    let size = egui::vec2(ui.available_width(), theme::CONTROL_HEIGHT);
    let response = theme::wide_action(ui, glyph, shown, theme::Tone::Neutral, true, size);
    if response.clicked() {
        ui.ctx().copy_text(text());
        ui.ctx().data_mut(|data| data.insert_temp(id, now + 1.5));
        ui.ctx().request_repaint();
    } else if remaining > 0.0 {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs_f64(remaining));
    }
}

/// The preset to offer for a remote live: the remembered or watched choice when the live still
/// offers it, else Source, else the first preset the publisher advertises.
pub fn offered_preset(live: &LiveInfo, wanted: Option<u32>) -> Option<u32> {
    let offered = |id: u32| live.presets.iter().any(|p| p.id == id);
    wanted
        .filter(|id| offered(*id))
        .or_else(|| offered(SOURCE_PRESET_ID).then_some(SOURCE_PRESET_ID))
        .or_else(|| live.presets.first().map(|p| p.id))
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
    theme::dropdown(ui, id_salt, text, None, |ui| {
        for preset in presets {
            ui.selectable_value(
                selected,
                preset.id,
                format!("{} {}x{}", preset.name, preset.width, preset.height),
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use brp_proto::{Codec, SourceKind, template_presets};

    #[test]
    fn a_person_is_named_by_their_first_letter_or_a_placeholder() {
        assert_eq!(initial("morgan"), "M");
        assert_eq!(initial("  (Ana)"), "A");
        assert_eq!(initial(""), "?");
        assert_eq!(initial("…"), "?");
    }

    #[test]
    fn hue_is_stable_for_a_key_and_stays_on_the_wheel() {
        for byte in 0..=u8::MAX {
            let hue = person_hue(&[byte, 9]);
            assert_eq!(hue, person_hue(&[byte, 200]));
            assert!((0.0..1.0).contains(&hue));
        }
        assert_eq!(person_hue(&[]), 0.0);
    }

    fn live() -> LiveInfo {
        LiveInfo {
            id: 1,
            title: "desk".into(),
            kind: SourceKind::Monitor,
            source_width: 2560,
            source_height: 1440,
            source_fps: 60,
            has_audio: false,
            presets: template_presets(2560, 1440, 60, Codec::Hevc),
        }
    }

    #[test]
    fn keeps_the_wanted_preset_when_the_live_still_offers_it() {
        let mut info = live();
        // Source plus 1080p only, so the wanted 1080p preset is still offered.
        info.presets.retain(|p| p.id <= 2);
        assert_eq!(offered_preset(&info, Some(2)), Some(2));
    }

    #[test]
    fn falls_back_to_source_when_the_wanted_preset_is_gone() {
        let mut info = live();
        info.presets.retain(|p| p.id <= 2);
        assert_eq!(offered_preset(&info, Some(9)), Some(1));
    }

    #[test]
    fn falls_back_to_source_when_nothing_is_wanted() {
        let mut info = live();
        info.presets.retain(|p| p.id <= 2);
        assert_eq!(offered_preset(&info, None), Some(1));
    }

    #[test]
    fn falls_back_to_the_first_preset_when_source_is_absent() {
        let mut info = live();
        info.presets.retain(|p| p.id == 2 || p.id == 3);
        assert_eq!(offered_preset(&info, None), Some(2));
    }

    #[test]
    fn keeps_the_wanted_preset_even_without_source() {
        let mut info = live();
        info.presets.retain(|p| p.id == 2 || p.id == 3);
        assert_eq!(offered_preset(&info, Some(3)), Some(3));
    }

    #[test]
    fn returns_none_when_the_live_has_no_presets() {
        let mut info = live();
        info.presets.clear();
        assert_eq!(offered_preset(&info, Some(1)), None);
    }
}

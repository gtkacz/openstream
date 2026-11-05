//! The room rail: invite actions, then every person in the room with the streams they offer. You
//! come first, in the same shape as everyone else, with the controls for your own streams.

use std::fmt::Debug;
use std::hash::Hash;

use brp_net::PathKind;
use brp_proto::constants::SOURCE_PRESET_ID;
use brp_proto::{AudioMix, LiveInfo, Preset};
use brp_room::{MemberView, RoomSnapshot, WatchState};

use super::motion;
use super::own_lives;
use super::state::{UiState, ordered_members};
use super::theme::{self, icons};
use crate::commands::RoomCommand;
use crate::link;

const AVATAR_SIZE: f32 = 32.0;
/// Tall enough for a name over its detail, with air the list's own gaps can lean on.
const PERSON_ROW_HEIGHT: f32 = 44.0;
const PERSON_GAP: f32 = 12.0;
const CARD_GAP: f32 = 4.0;

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

/// What the rail holds below the brand, shared with the compact window's Room sheet. Returns the
/// height it would like, for the sheet to fit itself to.
pub fn contents(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    ticket: &str,
    state: &mut UiState,
    commands: &mut Vec<RoomCommand>,
) -> f32 {
    let top = ui.min_rect().top();
    // Two equal columns, so the pair ends on the same edge as the cards below it.
    ui.columns(2, |columns| {
        copy_button(
            &mut columns[0],
            icons::LINK,
            "Copy link",
            theme::CONTROL_HEIGHT,
            || link::share_link(ticket),
        );
        copy_button(
            &mut columns[1],
            icons::TICKET,
            "Copy ticket",
            theme::CONTROL_HEIGHT,
            || ticket.to_string(),
        );
    });
    ui.add_space(12.0);
    // `snapshot.members` excludes this participant, so count it in.
    let members = snapshot.members.len() + 1;
    let plural = if members == 1 { "" } else { "s" };
    theme::section_heading(ui, &format!("{members} member{plural} in this room"));
    let output = egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // Every gap in the list is spelled out, so the rhythm is the same for everyone in it.
            ui.spacing_mut().item_spacing.y = 0.0;
            own_block(ui, snapshot, state, commands);
            for member in ordered_members(snapshot) {
                motion::reveal(ui, member.id, |ui| {
                    ui.add_space(PERSON_GAP);
                    member_block(ui, snapshot, member, state, commands);
                });
            }
        });
    output.inner_rect.top() - top + output.content_size.y
}

fn own_block(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    state: &mut UiState,
    commands: &mut Vec<RoomCommand>,
) {
    person_row(
        ui,
        snapshot.me.as_bytes(),
        &snapshot.nickname,
        snapshot.me.fmt_short().to_string(),
        None,
    );
    for live in &snapshot.own_lives {
        motion::reveal(ui, ("own-live", live.info.id), |ui| {
            ui.add_space(CARD_GAP);
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
    person_row(
        ui,
        member.id.as_bytes(),
        &member.nickname,
        format!("{} · {}", member.nickname, member.id.fmt_short()),
        Some(member.path),
    );
    if member.has_audio {
        ui.add_space(CARD_GAP);
        ui.horizontal(|ui| {
            // The slider takes what the mute button leaves, so it ends on the cards' right edge.
            let slider_width = (ui.available_width() - theme::CONTROL_HEIGHT - 8.0).max(0.0);
            ui.spacing_mut().slider_width = slider_width;
            if let Some(gain) = super::volume_control(ui, ("member-volume", member.id), member.gain)
            {
                commands.push(RoomCommand::SetVolume {
                    publisher: member.id,
                    gain,
                });
            }
        });
        if let Some(mix) = &member.audio_mix {
            let summary = mix_summary(mix);
            ui.add(egui::Label::new(egui::RichText::new(&summary).small().weak()).truncate())
                .on_hover_text(summary);
        }
    }
    for live in &member.lives {
        motion::reveal(ui, ("member-live", live.id), |ui| {
            ui.add_space(CARD_GAP);
            live_row(ui, snapshot, member, live, state, commands);
        });
    }
}

/// What someone's audio carries, in the words the rail shows under their volume.
fn mix_summary(mix: &AudioMix) -> String {
    match mix {
        AudioMix::All => "Sharing all applications".to_string(),
        AudioMix::Only { names, others: 0 } if names.is_empty() => {
            "Sharing no applications".to_string()
        }
        AudioMix::Only { names, others: 0 } => format!("Sharing {}", names.join(", ")),
        AudioMix::Only { names, others } => {
            format!("Sharing {} and {others} more", names.join(", "))
        }
    }
}

/// Someone's line in the rail: their avatar, their name, and how they reach you, or `None` for you.
/// Sized up front and painted at computed positions, because a vertical group inside a row starts at
/// the row's top rather than its centre, which is what used to leave names hanging off their avatars.
fn person_row(
    ui: &mut egui::Ui,
    id: &[u8],
    nickname: &str,
    details: String,
    path: Option<PathKind>,
) {
    const TEXT_GAP: f32 = 10.0;
    const GLYPH: egui::Vec2 = egui::vec2(20.0, 12.0);
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), PERSON_ROW_HEIGHT),
        egui::Sense::hover(),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, nickname));
    response.on_hover_text(details);
    let painter = ui.painter();
    avatar(
        painter,
        rect.left_center() + egui::vec2(AVATAR_SIZE / 2.0, 0.0),
        id,
        nickname,
    );
    let left = rect.left() + AVATAR_SIZE + TEXT_GAP;
    let mut job = egui::text::LayoutJob::simple_singleline(
        nickname.to_string(),
        theme::medium(14.0),
        theme::TEXT,
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width((rect.right() - left).max(0.0));
    let name = painter.layout_job(job);
    let detail_text = path.map_or("You", path_badge);
    let detail = painter.layout_no_wrap(
        detail_text.to_string(),
        egui::FontId::proportional(12.0),
        theme::SECONDARY,
    );
    let top = rect.center().y - (name.size().y + detail.size().y) / 2.0;
    let detail_top = top + name.size().y;
    painter.galley(egui::pos2(left, top), name, theme::TEXT);
    let detail_left = if let Some(path) = path {
        let glyph = egui::Rect::from_min_size(
            egui::pos2(left, detail_top + (detail.size().y - GLYPH.y) / 2.0),
            GLYPH,
        );
        path_glyph(ui, ui.id().with(("path-glyph", id)), glyph, path);
        glyph.right() + 4.0
    } else {
        left
    };
    painter.galley(
        egui::pos2(detail_left, detail_top),
        detail,
        theme::SECONDARY,
    );
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
    let mut switched_on = false;
    let card = theme::card()
        // Stops short of the controls' own fill, so the dropdown stays visible on a watched card.
        .fill(theme::SURFACE.lerp_to_gamma(theme::RAISED, raised * 0.45))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.add(
                egui::Label::new(egui::RichText::new(&live.title).font(theme::medium(14.0)))
                    .truncate(),
            )
            .on_hover_text(&live.title);
            let detail = match watch.map(|w| &w.state) {
                Some(WatchState::Connecting) => "Connecting…".to_string(),
                Some(WatchState::SignalLost) => "No signal".to_string(),
                Some(WatchState::Reconnecting) => "Reconnecting…".to_string(),
                Some(WatchState::Ended) => "Ended".to_string(),
                Some(WatchState::Live) | None => format!(
                    "{} × {} · {} fps",
                    live.source_width, live.source_height, live.source_fps
                ),
            };
            ui.label(egui::RichText::new(detail).small().color(theme::SECONDARY));
            ui.add_space(4.0);
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
                switched_on = watched;
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
    if switched_on {
        state.tile_origins.insert(key, card.response.rect);
    }
}

/// A person's initial on a dim tint of their own. The hue comes from their key, so a person keeps
/// their colour across rooms and across every other member's window.
fn avatar(painter: &egui::Painter, centre: egui::Pos2, id: &[u8], nickname: &str) {
    let hue = person_hue(id);
    let fill = egui::ecolor::HsvaGamma {
        h: hue,
        s: 0.45,
        v: 0.38,
        a: 1.0,
    };
    let ink = egui::ecolor::HsvaGamma {
        h: hue,
        s: 0.22,
        v: 1.0,
        a: 1.0,
    };
    painter.circle_filled(centre, AVATAR_SIZE / 2.0, egui::Color32::from(fill));
    painter.text(
        centre,
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

/// How far a connection line has settled into each of its drawings, as continuous amounts so one
/// can turn into another: dashes closing into a line, a relay's node growing out of it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PathShape {
    /// Nothing for dashes, all the way for a solid line.
    pub solid: f32,
    /// How much of a relay's node shows in the middle.
    pub relay: f32,
}

impl PathShape {
    pub fn of(path: PathKind) -> Self {
        match path {
            PathKind::Direct => Self {
                solid: 1.0,
                relay: 0.0,
            },
            PathKind::Relayed => Self {
                solid: 1.0,
                relay: 1.0,
            },
            PathKind::Unknown => Self {
                solid: 0.0,
                relay: 0.0,
            },
        }
    }

    /// Heads for `path` from wherever it has got to, so a path that resolves can be watched doing it.
    fn eased(ctx: &egui::Context, id: egui::Id, path: PathKind) -> Self {
        let settled = Self::of(path);
        Self {
            solid: motion::glide(ctx, id.with("solid"), settled.solid, motion::TRAVEL),
            relay: motion::glide(ctx, id.with("relay"), settled.relay, motion::TRAVEL),
        }
    }
}

/// How this person's video reaches you: a straight line when peer to peer, a node in the middle
/// when a relay carries it, dashes while the path is unknown.
fn path_glyph(ui: &egui::Ui, id: egui::Id, rect: egui::Rect, path: PathKind) {
    const DOT_RADIUS: f32 = 2.0;
    let shape = PathShape::eased(ui.ctx(), id, path);
    paint_path(
        ui.painter(),
        rect.left_center() + egui::vec2(DOT_RADIUS, 0.0),
        rect.right_center() - egui::vec2(DOT_RADIUS, 0.0),
        shape,
        DOT_RADIUS,
        theme::MUTED.lerp_to_gamma(theme::SECONDARY, shape.solid),
    );
}

/// The connection between two points, in the same drawing at any size and angle.
pub fn paint_path(
    painter: &egui::Painter,
    from: egui::Pos2,
    to: egui::Pos2,
    shape: PathShape,
    dot_radius: f32,
    ink: egui::Color32,
) {
    const DASH: f32 = 2.0;
    let line = egui::Stroke::new(1.0, ink);
    if shape.solid >= 1.0 {
        painter.line_segment([from, to], line);
    } else {
        let gap = DASH * (1.0 - shape.solid);
        painter.extend(egui::Shape::dashed_line(&[from, to], line, DASH, gap));
    }
    if shape.relay > 0.0 {
        let radius = dot_radius * 1.25 * shape.relay;
        painter.circle(from.lerp(to, 0.5), radius, theme::GROUND, line);
    }
    painter.circle_filled(from, dot_radius, ink);
    painter.circle_filled(to, dot_radius, ink);
}

/// Human-readable label for a member's connection path.
pub fn path_badge(path: PathKind) -> &'static str {
    match path {
        PathKind::Direct => "Direct",
        PathKind::Relayed => "Relayed",
        PathKind::Unknown => "Finding a path",
    }
}

pub(super) fn copy_button(
    ui: &mut egui::Ui,
    glyph: &str,
    label: &str,
    height: f32,
    text: impl FnOnce() -> String,
) {
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
    let size = egui::vec2(ui.available_width(), height);
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
                format!("{} · {} × {}", preset.name, preset.width, preset.height),
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mix_summary_names_what_is_shared_and_counts_what_did_not_fit() {
        let only = |names: &[&str], others| AudioMix::Only {
            names: names.iter().map(|n| n.to_string()).collect(),
            others,
        };
        assert_eq!(mix_summary(&AudioMix::All), "Sharing all applications");
        assert_eq!(mix_summary(&only(&[], 0)), "Sharing no applications");
        assert_eq!(
            mix_summary(&only(&["firefox", "game.exe"], 0)),
            "Sharing firefox, game.exe"
        );
        assert_eq!(
            mix_summary(&only(&["firefox"], 3)),
            "Sharing firefox and 3 more"
        );
    }
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

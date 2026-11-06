//! The empty canvas as a map of the room: you at the centre and everyone else as far away as their
//! signal is slow. It answers "why is Rae laggy?" at a glance, and it is the one place where the
//! network itself is the picture. No geography: that would need locating people by IP address.

use brp_net::PathKind;
use brp_proto::{LiveInfo, SourceKind, VoiceState};
use brp_room::{MemberView, RoomSnapshot};
use egui::{Color32, Id, Pos2, Rect, Shape, Stroke, UiBuilder, pos2, vec2};

use super::members::{heartbeat_age, heartbeat_ink, offered_preset, packets, path_badge, wire_ink};
use super::motion;
use super::own_lives;
use super::state::{UiState, delay_text, nobody_is_sharing, ordered_members};
use super::theme::{self, icons};
use super::voice;
use crate::commands::RoomCommand;
use crate::render::backdrop::{self, BackdropKind, BackdropRequest};

const FASTEST_MS: f32 = 5.0;
const SLOWEST_MS: f32 = 300.0;
const RINGS_MS: [f32; 3] = [10.0, 50.0, 200.0];
/// Spread around you in the order people sort, so nobody's place jumps when someone else joins.
const ANGLES: [f32; 6] = [205.0, 335.0, 112.0, 42.0, 270.0, 160.0];
/// Past the fixed places, the golden angle keeps newcomers out of each other's way.
const GOLDEN_ANGLE: f32 = 137.5;
const GLYPH_CELL: f32 = 3.0;
const LINE: f32 = 16.0;
const PING_SECONDS: f64 = 0.5;
const PING_TICK: f64 = 1.0 / 12.0;
/// Below this the labels would run into each other and off the canvas, and a list serves better.
const SMALLEST_CANVAS: egui::Vec2 = vec2(640.0, 440.0);
const MOST_PEOPLE: usize = 8;
const STRIP: egui::Vec2 = vec2(72.0, 12.0);

/// Whether this many people can be drawn on this canvas and still be read.
pub fn fits(canvas: Rect, people: usize) -> bool {
    people <= MOST_PEOPLE
        && canvas.width() >= SMALLEST_CANVAS.x
        && canvas.height() >= SMALLEST_CANVAS.y
}

fn reach(rtt_ms: f32, radius: f32) -> f32 {
    radius * (rtt_ms / FASTEST_MS).ln() / (SLOWEST_MS / FASTEST_MS).ln()
}

fn ring_points(center: Pos2, radius: f32) -> Vec<Pos2> {
    (0..=96)
        .map(|step| {
            let angle = std::f32::consts::TAU * step as f32 / 96.0;
            center + radius * egui::Vec2::angled(angle)
        })
        .collect()
}

/// A black backing that keeps text legible on the characters behind it.
fn plate(painter: &egui::Painter, rect: Rect) {
    painter.rect_filled(rect, 0, Color32::BLACK);
}

/// The light behind the canvas, for screens that want it without the map on top.
pub fn light(ui: &egui::Ui, shown: f32, flowing: bool) {
    // A quiet room's light stays mostly blue, and the signal's colour rises as something flows.
    const QUIET: f32 = 0.35;
    let flow = motion::follow(ui.ctx(), Id::new("map-flow"), flowing, motion::SURFACE);
    let (center, radius) = geometry(ui.max_rect());
    backdrop::request(
        ui.ctx(),
        BackdropRequest {
            kind: BackdropKind::Scope { center, radius },
            shown,
            signal: QUIET + (1.0 - QUIET) * flow,
        },
    );
    if ui.input(|input| input.focused) && motion::ambient_allowed(ui.ctx()) {
        ui.ctx().request_repaint_after(backdrop::FRAME);
    }
}

fn geometry(canvas: Rect) -> (Pos2, f32) {
    let center = pos2(canvas.center().x, canvas.center().y + 28.0);
    let radius = (canvas.width() / 2.0 - 180.0)
        .min(canvas.height() / 2.0 - 72.0)
        .max(120.0);
    (center, radius)
}

pub fn draw(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    state: &mut UiState,
    commands: &mut Vec<RoomCommand>,
    shown: f32,
    active: bool,
) {
    let canvas = ui.max_rect();
    let (center, radius) = geometry(canvas);
    light(ui, shown, state.upload_kbps > 0);
    let mut painter = ui.painter_at(canvas);
    painter.multiply_opacity(shown);
    for ms in RINGS_MS {
        let r = reach(ms, radius);
        let dots = Shape::dotted_line(&ring_points(center, r), theme::secondary(), 6.0, 0.7);
        painter.extend(dots);
        let label = painter.layout_no_wrap(format!("{ms} ms"), theme::medium(13.0), theme::muted());
        let at = Rect::from_center_size(center - vec2(0.0, r), label.size() + vec2(8.0, 0.0));
        plate(&painter, at);
        painter.galley(at.min + vec2(4.0, 0.0), label, theme::muted());
    }
    let now = ui.input(|input| input.time);
    let upload = state.upload_kbps as f64 * 125.0;
    let watchers = snapshot
        .members
        .iter()
        .filter(|m| m.watching_me)
        .count()
        .max(1) as f64;
    let members = ordered_members(snapshot);
    let moving = ui.input(|input| input.focused) && motion::ambient_allowed(ui.ctx());
    let mut lines = Vec::new();
    for (index, member) in members.iter().enumerate() {
        let at = place(member, index, center, radius);
        let lost = member.signal_lost;
        let ink = wire_ink(lost, false, member.watching_me && upload > 0.0);
        let stroke = Stroke::new(1.0, ink);
        if member.path == PathKind::Unknown || lost {
            lines.extend(Shape::dashed_line(&[center, at], stroke, 3.0, 4.0));
        } else {
            lines.push(Shape::line_segment([center, at], stroke));
        }
        if member.path == PathKind::Relayed {
            let middle = center.lerp(at, 0.5);
            lines.push(Shape::circle_filled(middle, 3.5, Color32::BLACK));
            lines.push(Shape::circle_stroke(
                middle,
                3.5,
                Stroke::new(1.25, theme::accent()),
            ));
        }
        if moving && member.watching_me && !lost {
            packets(&mut lines, &[center, at], upload / watchers, false, now);
        }
        if moving && !lost {
            let (forward, back) = voice::wire_rates(snapshot, state, member);
            packets(&mut lines, &[center, at], forward, false, now);
            packets(&mut lines, &[center, at], back, true, now);
        }
    }
    painter.extend(lines);
    for (index, member) in members.iter().enumerate() {
        let at = place(member, index, center, radius);
        node(
            ui, &painter, member, at, center, canvas, state, commands, active, shown,
        );
        ping(ui, &painter, member, at, moving);
    }
    centre(&painter, snapshot, center);
    hero(ui, snapshot, state, commands, canvas, shown, active);
    let legend = painter.layout_no_wrap(
        "Closer means less delay".to_string(),
        theme::medium(13.0),
        theme::muted(),
    );
    let at = canvas.right_bottom() - legend.size() - vec2(16.0, 12.0);
    plate(
        &painter,
        Rect::from_min_size(at, legend.size()).expand2(vec2(4.0, 0.0)),
    );
    painter.galley(at, legend, theme::muted());
}

fn place(member: &MemberView, index: usize, center: Pos2, radius: f32) -> Pos2 {
    let distance = member
        .rtt_ms
        .filter(|_| member.path != PathKind::Unknown)
        .map_or(radius * 1.03, |rtt| {
            reach(rtt.max(1) as f32, radius).clamp(radius * 0.26, radius)
        });
    let degrees = ANGLES
        .get(index)
        .copied()
        .unwrap_or(index as f32 * GOLDEN_ANGLE);
    center + distance * egui::Vec2::angled(degrees.to_radians())
}

/// A ring that flares once when a heartbeat really arrives, in steps.
fn ping(ui: &egui::Ui, painter: &egui::Painter, member: &MemberView, at: Pos2, moving: bool) {
    let age = heartbeat_age(ui, member);
    if moving && age < PING_SECONDS {
        let step = (age / PING_TICK).floor() as f32;
        let fade = 1.0 - (age / PING_SECONDS) as f32;
        painter.circle_stroke(
            at,
            14.0 + step * 6.0,
            Stroke::new(1.0, theme::signal().gamma_multiply(fade)),
        );
    }
}

fn offer_label(live: &LiveInfo) -> String {
    format!("[ {} ]", live.title)
}

#[allow(clippy::too_many_arguments)]
fn node(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    member: &MemberView,
    at: Pos2,
    center: Pos2,
    canvas: Rect,
    state: &mut UiState,
    commands: &mut Vec<RoomCommand>,
    active: bool,
    shown: f32,
) {
    const GAP: f32 = 16.0;
    const BUTTON: f32 = 32.0;
    const WIDEST_OFFER: f32 = 240.0;
    let lost = member.signal_lost;
    let backing = Rect::from_center_size(at, egui::Vec2::splat(24.0));
    plate(painter, backing);
    theme::key_glyph(
        painter,
        at,
        member.id.as_bytes(),
        GLYPH_CELL,
        heartbeat_ink(ui, member),
    );
    let on_left = at.x < center.x;
    let meta = if lost {
        "no signal".to_string()
    } else {
        match (member.path, member.rtt_ms) {
            (PathKind::Unknown, _) => path_badge(member.path).to_lowercase(),
            (path, Some(rtt)) => {
                format!("{} · {}", path_badge(path).to_lowercase(), delay_text(rtt))
            }
            (path, None) => path_badge(path).to_lowercase(),
        }
    };
    let name = painter.layout_no_wrap(member.nickname.clone(), theme::medium(14.0), theme::text());
    let meta_ink = if lost {
        theme::error()
    } else {
        theme::secondary()
    };
    let meta_galley = painter.layout_no_wrap(meta, theme::medium(13.0), meta_ink);
    let offers: Vec<_> = member
        .lives
        .iter()
        .filter_map(|live| {
            let key = (member.id, live.id);
            let wanted = state.preset_choice.get(&key).copied();
            offered_preset(live, wanted).map(|preset| {
                let wide = painter
                    .layout_no_wrap(offer_label(live), theme::medium(14.0), theme::text())
                    .size()
                    .x;
                (live, key, preset, (wide + 16.0).min(WIDEST_OFFER))
            })
        })
        .collect();
    let widest_offer = offers.iter().map(|offer| offer.3).fold(0.0, f32::max);
    let width = name.size().x.max(meta_galley.size().x).max(widest_offer) + 12.0;
    let lines = label_lines(member) as f32;
    let height = lines * LINE + offers.len() as f32 * (BUTTON + 2.0) + 8.0;
    let left = if on_left {
        at.x - GAP - width
    } else {
        at.x + GAP
    };
    // Kept on the canvas: someone with many streams stands taller than the space below them.
    let top = (at.y - 12.0)
        .min(canvas.bottom() - height - 4.0)
        .max(canvas.top() + 4.0);
    let left = left.clamp(canvas.left() + 4.0, (canvas.right() - width - 4.0).max(4.0));
    let area = Rect::from_min_size(pos2(left, top), vec2(width, height));
    plate(painter, area);
    let text_x = |galley_width: f32| {
        if on_left {
            area.right() - 6.0 - galley_width
        } else {
            area.left() + 6.0
        }
    };
    painter.galley(
        pos2(text_x(name.size().x), area.top() + 2.0),
        name,
        theme::text(),
    );
    painter.galley(
        pos2(text_x(meta_galley.size().x), area.top() + 2.0 + LINE),
        meta_galley,
        meta_ink,
    );
    if let Some(voice_state) = member.voice {
        let top = area.top() + 2.0 + 2.0 * LINE;
        match voice_word(voice_state) {
            Some(word) => {
                let galley = painter.layout_no_wrap(
                    word.to_string(),
                    theme::medium(13.0),
                    theme::secondary(),
                );
                painter.galley(
                    pos2(text_x(galley.size().x), top),
                    galley,
                    theme::secondary(),
                );
            }
            None => {
                let strip = Rect::from_min_size(pos2(text_x(STRIP.x), top + 2.0), STRIP);
                let levels = state.voice.get(&member.id).map(|meter| &meter.levels);
                voice::level_strip(ui, strip, levels, !member.hearing);
            }
        }
    }
    let mut y = area.top() + lines * LINE + 6.0;
    for (live, key, preset_id, wide) in offers {
        let size = vec2(wide, BUTTON);
        let rect = Rect::from_min_size(pos2(text_x(wide), y), size);
        let clicked = ui
            .scope_builder(
                UiBuilder::new().id_salt(("map-offer", key)).max_rect(rect),
                |ui| {
                    ui.set_opacity(shown);
                    ui.set_clip_rect(ui.clip_rect().intersect(rect));
                    if !active {
                        ui.disable();
                    }
                    let offer = theme::wide_action(
                        ui,
                        icons::SHARE_MONITOR,
                        &live.title,
                        theme::Tone::Neutral,
                        true,
                        size,
                    );
                    offer.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::Button,
                            active,
                            format!("Watch {} · {}", member.nickname, live.title),
                        )
                    });
                    // Only a title too long for its button needs saying again.
                    if wide >= WIDEST_OFFER {
                        offer.on_hover_text(&live.title).clicked()
                    } else {
                        offer.clicked()
                    }
                },
            )
            .inner;
        if clicked {
            commands.push(RoomCommand::Watch { key, preset_id });
        }
        y += BUTTON + 2.0;
    }
}

/// Name and path, plus a line for voice when the person is in it.
fn label_lines(member: &MemberView) -> usize {
    2 + usize::from(member.voice.is_some())
}

fn voice_word(state: VoiceState) -> Option<&'static str> {
    if state.deafened {
        Some("deafened")
    } else if state.muted {
        Some("muted")
    } else {
        None
    }
}

fn centre(painter: &egui::Painter, snapshot: &RoomSnapshot, center: Pos2) {
    plate(
        painter,
        Rect::from_center_size(center, egui::Vec2::splat(24.0)),
    );
    theme::key_glyph(
        painter,
        center,
        snapshot.me.as_bytes(),
        GLYPH_CELL,
        theme::accent(),
    );
    let watching: usize = snapshot
        .own_lives
        .iter()
        .flat_map(|live| &live.presets)
        .filter_map(|preset| preset.encoder.as_ref())
        .map(|encoder| encoder.subscribers)
        .sum();
    let (text, ink) = match snapshot.own_lives.first() {
        Some(live) if watching > 0 => (
            format!("You · {} live · {watching} watching", live.info.title),
            theme::live(),
        ),
        Some(live) => (format!("You · {} ready", live.info.title), theme::accent()),
        None => ("You".to_string(), theme::accent()),
    };
    let own = &snapshot.voice;
    let word = match () {
        _ if !own.joined => None,
        _ if own.error.is_some() => Some("no microphone"),
        _ if own.deafened => Some("deafened"),
        _ if own.muted => Some("muted"),
        _ if own.transmitting => Some("on air"),
        _ => None,
    };
    let text = word.map_or(text.clone(), |word| format!("{text} · {word}"));
    let label = painter.layout_no_wrap(text, theme::medium(13.0), ink);
    let at = center + vec2(-label.size().x / 2.0, 16.0);
    plate(
        painter,
        Rect::from_min_size(at, label.size()).expand2(vec2(4.0, 1.0)),
    );
    painter.galley(at, label, ink);
}

fn hero(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    state: &UiState,
    commands: &mut Vec<RoomCommand>,
    canvas: Rect,
    shown: f32,
    active: bool,
) {
    let has_offers = snapshot.members.iter().any(|m| !m.lives.is_empty());
    let (title, hint) = if has_offers {
        ("Ready to watch", "Pick a stream to bring it on screen")
    } else {
        (
            "Nothing on screen yet",
            "Streams appear here when someone shares",
        )
    };
    let mut painter = ui.painter_at(canvas);
    painter.multiply_opacity(shown);
    let mid = canvas.center().x;
    let title_galley =
        painter.layout_no_wrap(title.to_string(), theme::semibold(21.0), theme::text());
    let title_at = pos2(mid - title_galley.size().x / 2.0, canvas.top() + 24.0);
    plate(
        &painter,
        Rect::from_min_size(title_at, title_galley.size()).expand2(vec2(6.0, 0.0)),
    );
    let title_bottom = title_at.y + title_galley.size().y;
    painter.galley(title_at, title_galley, theme::text());
    let hint_galley =
        painter.layout_no_wrap(hint.to_string(), theme::medium(14.0), theme::secondary());
    let hint_at = pos2(mid - hint_galley.size().x / 2.0, title_bottom + 4.0);
    plate(
        &painter,
        Rect::from_min_size(hint_at, hint_galley.size()).expand2(vec2(6.0, 0.0)),
    );
    let hint_bottom = hint_at.y + hint_galley.size().y;
    painter.galley(hint_at, hint_galley, theme::secondary());
    if nobody_is_sharing(snapshot) {
        let size = vec2(200.0, theme::CONTROL_HEIGHT);
        let rect = Rect::from_min_size(pos2(mid - size.x / 2.0, hint_bottom + 12.0), size);
        let clicked = ui
            .scope_builder(UiBuilder::new().id_salt("map-share").max_rect(rect), |ui| {
                ui.set_opacity(shown);
                if !active {
                    ui.disable();
                }
                theme::wide_action(
                    ui,
                    icons::SHARE_MONITOR,
                    "Share monitor",
                    theme::Tone::Primary,
                    own_lives::can_share(snapshot, state),
                    size,
                )
                .clicked()
            })
            .inner;
        if clicked {
            commands.push(RoomCommand::Share {
                kind: SourceKind::Monitor,
                source: None,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shorter_round_trip_sits_closer_in() {
        assert!(reach(10.0, 300.0) < reach(50.0, 300.0));
        assert!(reach(50.0, 300.0) < reach(200.0, 300.0));
        assert_eq!(reach(SLOWEST_MS, 300.0), 300.0);
        assert_eq!(reach(FASTEST_MS, 300.0), 0.0);
    }

    #[test]
    fn the_map_gives_way_to_a_list_when_it_could_not_be_read() {
        let roomy = Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 800.0));
        assert!(fits(roomy, 1));
        assert!(fits(roomy, MOST_PEOPLE));
        assert!(!fits(roomy, MOST_PEOPLE + 1));
        assert!(!fits(
            Rect::from_min_size(Pos2::ZERO, vec2(390.0, 800.0)),
            1
        ));
        assert!(!fits(
            Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 300.0)),
            1
        ));
    }

    fn in_voice(voice: Option<VoiceState>) -> MemberView {
        MemberView {
            voice,
            hearing: true,
            sending: true,
            voice_gain: 1.0,
            id: iroh::SecretKey::from_bytes(&[2; 32]).public(),
            nickname: "Rae".into(),
            lives: Vec::new(),
            seen_ago_ms: 0,
            path: PathKind::Unknown,
            rtt_ms: None,
            signal_lost: false,
            watching_me: false,
            has_audio: false,
            audio_mix: None,
            gain: 1.0,
        }
    }

    #[test]
    fn a_person_in_voice_gets_a_third_label_line() {
        assert_eq!(label_lines(&in_voice(Some(VoiceState::default()))), 3);
    }

    #[test]
    fn someone_out_of_voice_keeps_two() {
        assert_eq!(label_lines(&in_voice(None)), 2);
    }
}

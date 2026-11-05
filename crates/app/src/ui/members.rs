//! The room rail: invite actions, then every person in the room with the streams they offer. You
//! come first, in the same shape as everyone else, with the controls for your own streams.

use std::fmt::Debug;
use std::hash::Hash;

use std::time::Duration;

use brp_net::PathKind;
use brp_proto::constants::SOURCE_PRESET_ID;
use brp_proto::{AudioMix, LiveInfo, Preset};
use brp_room::{MemberView, RoomSnapshot, WatchState};
use egui::emath::GuiRounding as _;
use egui::{Color32, Painter, Pos2, Rect, Sense, Shape, Stroke, pos2, vec2};

use super::motion;
use super::own_lives;
use super::state::{UiState, delay_text, ordered_members};
use super::theme::{self, icons};
use crate::commands::RoomCommand;
use crate::link;

const ROW_HEIGHT: f32 = 24.0;
const PERSON_GAP: f32 = 8.0;
const PEER_INDENT: f32 = 40.0;
const STREAM_INDENT: f32 = 72.0;
const GLYPH_BOX: f32 = 24.0;
const GLYPH_CELL: f32 = 3.0;
const STRIP_HEIGHT: f32 = 14.0;
const LEDGER_ROWS: usize = 5;
const LEDGER_LINE: f32 = 16.0;
/// How often network motion steps. Things you touch stay on springs; things the network does tick.
const WIRE_TICK: f64 = 1.0 / 12.0;
const PACKET_SPEED: f32 = 144.0;

/// Draws the rail and pushes `Watch`/`Unwatch` commands for switch and preset changes.
pub fn draw(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    ticket: &str,
    state: &mut UiState,
    commands: &mut Vec<RoomCommand>,
) {
    egui::Panel::left("members")
        .default_size(320.0)
        .min_size(304.0)
        .max_size(420.0)
        .resizable(true)
        .frame(theme::rail())
        .show(ui, |ui| {
            theme::paint_material(ui, ui.max_rect().expand(16.0));
            theme::brand(ui);
            ui.add_space(8.0);
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
    // Two equal columns, so the pair ends on the same edge as the rows below it.
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
    ui.add_space(8.0);
    // `snapshot.members` excludes this participant, so count it in.
    theme::section_heading(ui, &format!("{} in this room", snapshot.members.len() + 1));
    ui.add_space(4.0);
    state.ledger.note(snapshot);
    let ledger_height = LEDGER_ROWS as f32 * LEDGER_LINE + 21.0;
    let list_height = (ui.available_height() - ledger_height).max(48.0);
    let output = egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .max_height(list_height)
        .show(ui, |ui| {
            // Every gap in the list is spelled out, so the rhythm is the same for everyone in it.
            ui.spacing_mut().item_spacing.y = 0.0;
            circuit(ui, snapshot, state, commands);
        });
    ledger(ui, state);
    output.inner_rect.top() - top + output.content_size.y + ledger_height
}

/// The room's last few events, at the foot of the rail.
fn ledger(ui: &mut egui::Ui, state: &UiState) {
    ui.spacing_mut().item_spacing.y = 0.0;
    ui.add_space(10.0);
    let (rule, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rule, 0, theme::rule());
    ui.add_space(10.0);
    let count = state.ledger.lines.len();
    for (index, (line, bad)) in state.ledger.lines.iter().enumerate() {
        let ink = if *bad {
            theme::error()
        } else if index + 1 == count {
            theme::secondary()
        } else {
            theme::muted()
        };
        let (rect, _) =
            ui.allocate_exact_size(vec2(ui.available_width(), LEDGER_LINE), Sense::hover());
        let galley = truncated(ui.painter(), line, theme::medium(13.5), ink, rect.width());
        ui.painter().galley(
            pos2(rect.left(), rect.center().y - galley.size().y / 2.0),
            galley,
            ink,
        );
    }
}

/// One connection in the circuit, and what is flowing along it right now.
struct Wire {
    points: Vec<Pos2>,
    /// How far its dashes have closed into a line, and how much of a relay's node shows.
    shape: PathShape,
    lost: bool,
    relay_at: Option<Pos2>,
    dim: bool,
    /// Bytes per second from the first point towards the last, and back.
    forward: f64,
    back: f64,
}

/// You, your streams, and everyone in the room wired to you. Only your own links are known, so this
/// is a star around you rather than a web.
fn circuit(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    state: &mut UiState,
    commands: &mut Vec<RoomCommand>,
) {
    let slot = ui.painter().add(Shape::Noop);
    let mut wires = Vec::new();
    let (me_rect, me) =
        ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::hover());
    let my_name = format!("{} · you", snapshot.nickname);
    me.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &my_name));
    me.on_hover_text(format!(
        "{} · {}",
        snapshot.nickname,
        snapshot.me.fmt_short()
    ));
    let me_glyph = glyph_rect(me_rect);
    theme::key_glyph(
        ui.painter(),
        me_glyph.center(),
        snapshot.me.as_bytes(),
        GLYPH_CELL,
        theme::accent(),
    );
    let upload_left = right_text(
        ui.painter(),
        me_rect.right_center(),
        &format!("↑ {}", own_lives::bitrate(state.upload_kbps)),
        theme::accent(),
    );
    let my_left = me_glyph.right() + 8.0;
    let name = truncated(
        ui.painter(),
        &my_name,
        theme::medium(14.0),
        theme::text(),
        (upload_left - 8.0 - my_left).max(0.0),
    );
    ui.painter().galley(
        pos2(my_left, me_rect.center().y - name.size().y / 2.0),
        name,
        theme::text(),
    );
    let trunk = pos2(me_glyph.center().x, me_glyph.bottom());
    for live in &snapshot.own_lives {
        let (rect, row) =
            ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
        let failing = live.presets.iter().any(|p| p.last_error.is_some());
        // A failure says what went wrong in the card, so it never stays folded away.
        let open = state.own_open.contains(&live.info.id) || failing;
        let row_rect = rect.with_min_x(rect.left() + PEER_INDENT - 4.0);
        row_hover(ui, &row, row_rect);
        if row.has_focus() {
            ui.painter().rect_stroke(
                row_rect,
                0,
                Stroke::new(1.0, theme::accent()),
                egui::StrokeKind::Inside,
            );
        }
        let hover = row.hovered();
        row.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::CollapsingHeader,
                true,
                open,
                &live.info.title,
            )
        });
        let row = row.on_hover_text("Quality & encoding");
        if row.clicked() {
            if open {
                state.own_open.remove(&live.info.id);
            } else {
                state.own_open.insert(live.info.id);
            }
        }
        let watching: usize = live
            .presets
            .iter()
            .filter_map(|p| p.encoder.as_ref())
            .map(|e| e.subscribers)
            .sum();
        let (word, ink) = if failing {
            ("needs attention".to_string(), theme::error())
        } else if watching > 0 {
            (format!("live · {watching} watching"), theme::accent())
        } else {
            ("ready".to_string(), theme::secondary())
        };
        let on_hover = |ink: Color32| if hover { theme::on_accent() } else { ink };
        let right = right_text(ui.painter(), rect.right_center(), &word, on_hover(ink));
        let title = truncated(
            ui.painter(),
            &live.info.title,
            theme::medium(14.0),
            on_hover(theme::accent()),
            (right - 8.0 - rect.left() - PEER_INDENT).max(0.0),
        );
        ui.painter().galley(
            pos2(
                rect.left() + PEER_INDENT,
                rect.center().y - title.size().y / 2.0,
            ),
            title,
            on_hover(theme::accent()),
        );
        wires.push(Wire {
            points: vec![
                trunk,
                pos2(trunk.x, rect.center().y),
                pos2(rect.left() + PEER_INDENT - 6.0, rect.center().y),
            ],
            shape: PathShape::of(PathKind::Direct),
            lost: false,
            relay_at: None,
            dim: false,
            forward: 0.0,
            back: 0.0,
        });
        motion::unfold(ui, ("own-live", live.info.id), open, |ui| {
            ui.add_space(4.0);
            own_lives::live_card(ui, live, state, commands);
            ui.add_space(4.0);
        });
    }
    let upload = state.upload_kbps as f64 * 125.0;
    let viewers = snapshot
        .members
        .iter()
        .filter(|m| m.watching_me)
        .count()
        .max(1) as f64;
    for member in ordered_members(snapshot) {
        motion::reveal(ui, member.id, |ui| {
            ui.add_space(PERSON_GAP);
            member_block(
                ui,
                snapshot,
                member,
                state,
                commands,
                trunk,
                upload / viewers,
                &mut wires,
            );
        });
    }
    let shapes = paint_wires(ui, &wires);
    ui.painter().set(slot, Shape::Vec(shapes));
    let flowing = !snapshot.watches.is_empty() || snapshot.members.iter().any(|m| m.watching_me);
    if flowing && ui.input(|input| input.focused) && motion::ambient_allowed(ui.ctx()) {
        ui.ctx()
            .request_repaint_after(Duration::from_secs_f64(WIRE_TICK));
    }
}

/// A person dims as their last heartbeat ages, in steps, and dims further once they are lost.
pub fn heartbeat_fade(member: &MemberView) -> f32 {
    let age = member.seen_ago_ms as f32 / 1000.0;
    if member.signal_lost {
        (0.75 - age * 0.03).max(0.25)
    } else {
        (1.0 - age.floor() * 0.09).max(0.55)
    }
}

#[allow(clippy::too_many_arguments)]
fn member_block(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    member: &MemberView,
    state: &mut UiState,
    commands: &mut Vec<RoomCommand>,
    trunk: Pos2,
    out_rate: f64,
    wires: &mut Vec<Wire>,
) {
    let lost = member.signal_lost;
    let (rect, row) =
        ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::hover());
    let path_id = row.id.with("path");
    row.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &member.nickname));
    row.on_hover_text(format!("{} · {}", member.nickname, member.id.fmt_short()));
    let glyph = Rect::from_min_size(
        pos2(rect.left() + PEER_INDENT, rect.center().y - GLYPH_BOX / 2.0),
        egui::Vec2::splat(GLYPH_BOX),
    );
    theme::key_glyph(
        ui.painter(),
        glyph.center(),
        member.id.as_bytes(),
        GLYPH_CELL,
        theme::accent().gamma_multiply(heartbeat_fade(member)),
    );
    let (word, word_ink) = if lost {
        ("no signal".to_string(), theme::error())
    } else {
        (path_badge(member.path).to_lowercase(), theme::secondary())
    };
    let mut right = rect.right();
    if !lost
        && member.path != PathKind::Unknown
        && let Some(rtt) = member.rtt_ms
    {
        right = right_text(
            ui.painter(),
            pos2(right, rect.center().y),
            &delay_text(rtt),
            theme::secondary(),
        ) - 12.0;
    }
    right = right_text(ui.painter(), pos2(right, rect.center().y), &word, word_ink);
    let name_left = glyph.right() + 8.0;
    let name = truncated(
        ui.painter(),
        &member.nickname,
        theme::medium(14.0),
        theme::text(),
        (right - 8.0 - name_left).max(0.0),
    );
    ui.painter().galley(
        pos2(name_left, rect.center().y - name.size().y / 2.0),
        name,
        theme::text(),
    );
    let shape = PathShape::eased(ui.ctx(), path_id, member.path);
    let flowing: f64 = member
        .lives
        .iter()
        .filter_map(|live| state.traffic.get(&(member.id, live.id)))
        .map(|traffic| traffic.bytes_per_second)
        .sum();
    wires.push(Wire {
        points: vec![
            trunk,
            pos2(trunk.x, rect.center().y),
            pos2(glyph.left(), rect.center().y),
        ],
        shape,
        lost,
        relay_at: Some(pos2((trunk.x + glyph.left()) / 2.0, rect.center().y)),
        dim: false,
        forward: if member.watching_me && !lost {
            out_rate
        } else {
            0.0
        },
        back: if lost { 0.0 } else { flowing },
    });
    if member.has_audio {
        ui.horizontal(|ui| {
            ui.add_space(STREAM_INDENT);
            ui.spacing_mut().slider_width =
                (ui.available_width() - theme::CONTROL_HEIGHT - 8.0).max(0.0);
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
            ui.horizontal(|ui| {
                ui.add_space(STREAM_INDENT);
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(&summary)
                            .font(theme::medium(13.0))
                            .color(theme::muted()),
                    )
                    .truncate(),
                )
                .on_hover_text(&summary);
            });
        }
    }
    let branch = pos2(glyph.center().x, glyph.bottom());
    for live in &member.lives {
        let key = (member.id, live.id);
        let at = motion::reveal(ui, ("member-live", key), |ui| {
            stream_row(ui, snapshot, member, live, state, commands)
        });
        let rate = state
            .traffic
            .get(&key)
            .map_or(0.0, |traffic| traffic.bytes_per_second);
        let watched = snapshot
            .watches
            .iter()
            .any(|w| (w.publisher, w.live_id) == key);
        wires.push(Wire {
            points: vec![
                branch,
                pos2(branch.x, at),
                pos2(ui.max_rect().left() + STREAM_INDENT - 6.0, at),
            ],
            shape: PathShape::of(PathKind::Direct),
            lost: lost && watched,
            relay_at: None,
            dim: !watched,
            forward: if watched && !lost { rate } else { 0.0 },
            back: 0.0,
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

/// One stream on offer: its name, what it is carrying, and whether you are watching it. Returns the
/// height of the row's middle, for its wire.
fn stream_row(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    member: &MemberView,
    live: &LiveInfo,
    state: &mut UiState,
    commands: &mut Vec<RoomCommand>,
) -> f32 {
    let key = (member.id, live.id);
    let watch = snapshot
        .watches
        .iter()
        .find(|w| (w.publisher, w.live_id) == key);
    let watched = watch.is_some();
    let wanted = watch
        .map(|w| w.preset_id)
        .or_else(|| state.preset_choice.get(&key).copied());
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::hover());
    let tick = Rect::from_center_size(rect.right_center() - vec2(12.0, 0.0), vec2(24.0, 16.0));
    let toggle = ui.interact(
        tick.expand(2.0),
        ui.id().with(("watch", key)),
        Sense::click(),
    );
    toggle.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, watched, &live.title)
    });
    let hover = motion::follow(
        ui.ctx(),
        toggle.id.with("hover"),
        toggle.hovered(),
        motion::HOVER,
    );
    if hover > 0.0 {
        ui.painter()
            .rect_filled(tick, 0, theme::accent().gamma_multiply(hover));
    }
    ui.painter().text(
        tick.center(),
        egui::Align2::CENTER_CENTER,
        if watched { "[x]" } else { "[ ]" },
        theme::medium(14.0),
        theme::text().lerp_to_gamma(theme::on_accent(), hover),
    );
    if toggle.has_focus() {
        ui.painter().rect_stroke(
            tick.expand(2.0),
            0,
            Stroke::new(1.0, theme::accent()),
            egui::StrokeKind::Inside,
        );
    }
    if toggle.clicked() {
        if watched {
            commands.push(RoomCommand::Unwatch(key));
        } else if let Some(preset_id) = offered_preset(live, wanted) {
            commands.push(RoomCommand::Watch { key, preset_id });
        }
    }
    let figure = watch.map(|w| match w.state {
        WatchState::SignalLost | WatchState::Reconnecting => {
            ("↓ 0 Mbps".to_string(), theme::error())
        }
        WatchState::Connecting => ("connecting".to_string(), theme::secondary()),
        WatchState::Ended => ("ended".to_string(), theme::secondary()),
        WatchState::Live => {
            let rate = state.traffic.get(&key).map_or(0.0, |t| t.bytes_per_second);
            (
                format!("↓ {:.1} Mbps", rate * 8.0 / 1e6),
                theme::secondary(),
            )
        }
    });
    let mut right = tick.left() - 8.0;
    if let Some((figure, ink)) = figure {
        right = right_text(ui.painter(), pos2(right, rect.center().y), &figure, ink) - 8.0;
    }
    let left = rect.left() + STREAM_INDENT;
    let ink = if watched {
        theme::text()
    } else {
        theme::secondary()
    };
    let title = truncated(
        ui.painter(),
        &live.title,
        theme::medium(14.0),
        ink,
        (right - left).max(0.0),
    );
    ui.painter().galley(
        pos2(left, rect.center().y - title.size().y / 2.0),
        title,
        ink,
    );
    preset_line(
        ui,
        member,
        live,
        watch.map(|w| w.preset_id),
        wanted,
        state,
        commands,
    );
    if watched {
        let (strip, _) =
            ui.allocate_exact_size(vec2(ui.available_width(), STRIP_HEIGHT), Sense::hover());
        traffic_strip(ui, strip.with_min_x(left), state.traffic.get(&key));
    }
    rect.center().y
}

/// The quality of a stream, chosen before watching it or changed while watching. A publisher with
/// one preset has nothing to choose, so there is no line for it.
fn preset_line(
    ui: &mut egui::Ui,
    member: &MemberView,
    live: &LiveInfo,
    watching: Option<u32>,
    wanted: Option<u32>,
    state: &mut UiState,
    commands: &mut Vec<RoomCommand>,
) {
    if live.presets.len() < 2 {
        return;
    }
    let key = (member.id, live.id);
    let Some(mut preset_id) = offered_preset(live, wanted) else {
        return;
    };
    let before = preset_id;
    let name = live
        .presets
        .iter()
        .find(|preset| preset.id == preset_id)
        .map_or(String::new(), |preset| preset.name.clone());
    ui.horizontal(|ui| {
        ui.add_space(STREAM_INDENT);
        theme::inline_dropdown(ui, ("member-preset", key), &name, |ui| {
            preset_choices(ui, &live.presets, &mut preset_id);
        });
    });
    if preset_id != before {
        state.preset_choice.insert(key, preset_id);
        if watching.is_some() {
            commands.push(RoomCommand::Watch { key, preset_id });
        }
    }
}

/// Real traffic, newest on the right: height is how much arrived, bright is a keyframe, and a gap is
/// a moment with no frames at all, which is what a still screen looks like.
fn traffic_strip(ui: &egui::Ui, rect: Rect, traffic: Option<&super::state::StreamTraffic>) {
    const BAR: f32 = 3.0;
    const PITCH: f32 = 4.0;
    let Some(traffic) = traffic else {
        return;
    };
    let height = rect.height() - 2.0;
    let count = ((rect.width() / PITCH) as usize).min(traffic.bars.len());
    let bars = traffic.bars.iter().skip(traffic.bars.len() - count);
    for (index, bar) in bars.enumerate() {
        let x = rect.left() + index as f32 * PITCH;
        let (tall, ink) = if bar.keyframe {
            (height, theme::text())
        } else if bar.megabits_per_second <= 0.0 {
            continue;
        } else {
            (
                (bar.megabits_per_second / 16.0).powf(0.6).clamp(0.1, 0.8) * height,
                theme::muted(),
            )
        };
        let shape = Rect::from_min_max(
            pos2(x, rect.top() + height - tall),
            pos2(x + BAR, rect.top() + height),
        );
        ui.painter()
            .rect_filled(shape.round_to_pixels(ui.pixels_per_point()), 0, ink);
    }
}

fn glyph_rect(row: Rect) -> Rect {
    Rect::from_min_size(
        pos2(row.left(), row.center().y - GLYPH_BOX / 2.0),
        egui::Vec2::splat(GLYPH_BOX),
    )
}

fn row_hover(ui: &egui::Ui, row: &egui::Response, rect: Rect) {
    if row.hovered() {
        ui.painter().rect_filled(rect, 0, theme::accent());
    }
}

fn truncated(
    painter: &Painter,
    text: &str,
    font: egui::FontId,
    ink: Color32,
    width: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), font, ink);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width);
    painter.layout_job(job)
}

/// Returns where the text starts, for the next thing leftwards.
fn right_text(painter: &Painter, right_centre: Pos2, text: &str, ink: Color32) -> f32 {
    painter
        .text(
            right_centre,
            egui::Align2::RIGHT_CENTER,
            text,
            theme::medium(14.0),
            ink,
        )
        .left()
}

/// Snapped to pixel centres, so a one-point wire stays one crisp line at every display scale.
fn crisp(ui: &egui::Ui, point: Pos2) -> Pos2 {
    let scale = ui.pixels_per_point();
    pos2(
        ((point.x * scale).floor() + 0.5) / scale,
        ((point.y * scale).floor() + 0.5) / scale,
    )
}

fn paint_wires(ui: &egui::Ui, wires: &[Wire]) -> Vec<Shape> {
    let mut shapes = Vec::new();
    let now = ui.input(|input| input.time);
    let moving = ui.input(|input| input.focused) && motion::ambient_allowed(ui.ctx());
    // Lines first, so packets and relay nodes sit on top of every wire they cross.
    for wire in wires {
        let points: Vec<Pos2> = wire.points.iter().map(|p| crisp(ui, *p)).collect();
        let ink = if wire.lost {
            theme::error()
        } else if wire.dim {
            theme::rule()
        } else {
            theme::wire()
        };
        let stroke = Stroke::new(1.0, ink);
        if wire.lost {
            shapes.extend(Shape::dashed_line(&points, stroke, 3.0, 4.0));
        } else if wire.shape.solid >= 1.0 {
            shapes.push(Shape::line(points, stroke));
        } else {
            let gap = 4.0 * (1.0 - wire.shape.solid);
            shapes.extend(Shape::dashed_line(&points, stroke, 3.0, gap));
        }
    }
    for wire in wires {
        if let Some(relay) = wire.relay_at.filter(|_| wire.shape.relay > 0.0) {
            let radius = 3.5 * wire.shape.relay;
            shapes.push(Shape::circle_filled(relay, radius, theme::ground()));
            shapes.push(Shape::circle_stroke(
                relay,
                radius,
                Stroke::new(1.25, theme::accent()),
            ));
        }
        if moving {
            packets(&mut shapes, &wire.points, wire.forward, false, now);
            packets(&mut shapes, &wire.points, wire.back, true, now);
        }
    }
    shapes
}

/// Squares moving along a wire in whole steps, closer together the more is flowing. Nothing moves on
/// a wire with nothing on it, so a quiet link stays lit and still.
pub fn packets(
    shapes: &mut Vec<Shape>,
    points: &[Pos2],
    bytes_per_second: f64,
    reverse: bool,
    now: f64,
) {
    let megabits = bytes_per_second * 8.0 / 1e6;
    if megabits < 0.02 {
        return;
    }
    let spacing = (220.0 / (1.0 + megabits as f32)).clamp(28.0, 220.0);
    let lengths: Vec<f32> = points
        .windows(2)
        .map(|pair| pair[0].distance(pair[1]))
        .collect();
    let total: f32 = lengths.iter().sum();
    if total < 1.0 {
        return;
    }
    let step = (now / WIRE_TICK).floor() * WIRE_TICK;
    let travelled = (step as f32 * PACKET_SPEED) % spacing;
    let mut along = travelled;
    while along < total {
        let mut left = if reverse { total - along } else { along };
        for (pair, length) in points.windows(2).zip(&lengths) {
            if left <= *length {
                let at = pair[0].lerp(pair[1], left / length.max(0.001));
                let square = Rect::from_center_size(at.round(), egui::Vec2::splat(4.0));
                shapes.push(Shape::rect_filled(square, 0, theme::accent()));
                break;
            }
            left -= length;
        }
        along += spacing;
    }
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
    pub fn eased(ctx: &egui::Context, id: egui::Id, path: PathKind) -> Self {
        let settled = Self::of(path);
        Self {
            solid: motion::glide(ctx, id.with("solid"), settled.solid, motion::TRAVEL),
            relay: motion::glide(ctx, id.with("relay"), settled.relay, motion::TRAVEL),
        }
    }
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
        painter.circle(from.lerp(to, 0.5), radius, theme::ground(), line);
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
        preset_choices(ui, presets, selected);
    });
}

fn preset_choices(ui: &mut egui::Ui, presets: &[Preset], selected: &mut u32) {
    for preset in presets {
        ui.selectable_value(
            selected,
            preset.id,
            format!("{} · {} × {}", preset.name, preset.width, preset.height),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use brp_proto::{Codec, SourceKind, template_presets};

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
}

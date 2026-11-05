//! Central panel: reserves one rect per watched live for the video renderer and draws the hover
//! overlay and status text on top. The panel frame is transparent so the tiles show through. The
//! overlay is shared with pop-out windows, which place one live over their whole panel.

use std::collections::HashSet;

use brp_net::PathKind;
use brp_proto::constants::SIGNAL_LOST_AFTER;
use brp_proto::{LiveInfo, SourceKind};
use brp_room::{RoomSnapshot, WatchState, WatchView};

use super::members::{PathShape, copy_button, offered_preset, paint_path, preset_selector};
use super::motion;
use super::own_lives::can_share;
use super::state::{
    DrawnTile, UiState, delay_text, live_title, nobody_is_sharing, ordered_members, signal_lost,
    visible_watches,
};
use super::theme::{self, icons};
use crate::commands::{RoomCommand, WindowCommand};
use crate::link;
use crate::render::grid;
use crate::render::tiles::{TileFx, TileKey};

/// Where an overlay is drawn, which decides its window buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// A cell of the main window's grid: offers pop out and fullscreen.
    Grid,
    /// A pop-out window: offers the fullscreen toggle and a return to the grid.
    PopOut { fullscreen: bool },
}

/// The gap between tiles, and from a tile to the window's edge.
const TILE_GAP: f32 = 8.0;
/// A stream whose shape nothing has told us yet, such as one whose publisher has left.
const USUAL_ASPECT: f32 = 16.0 / 9.0;
const TILE_RADIUS: u8 = theme::RADIUS_CARD;
/// Below this tile width the toolbar drops its labels, keeping its names as tooltips.
const LABELLED_TOOLBAR_WIDTH: f32 = 430.0;
const TOOLBAR_HEIGHT: f32 = 40.0;
const BAR_PADDING: f32 = 4.0;
const TOOLBAR_MARGIN: f32 = 8.0;
const EMPTY_GAP: f32 = 4.0;
const SHARING_STOPPED: &str = "Sharing stopped";
/// How long a picture takes to reduce to characters, cell by cell, once its signal is gone.
const REDUCE_SECONDS: f32 = 1.5;
const WARM_UP_SECONDS: f32 = 0.42;

/// What the renderer has to do with the tiles of one pass.
#[derive(Debug, Default)]
pub struct Grid {
    /// Where each drawn live's frame goes, in egui points.
    pub placements: Vec<(TileKey, egui::Rect)>,
    /// Tiles on their way out: where they stand and how shown they still are.
    pub leaving: Vec<(TileKey, egui::Rect, f32)>,
}

/// The shape of the picture this watch will show, known before its first frame arrives so the tile
/// does not change shape when it does. A frame of another shape still letterboxes inside it.
fn picture_aspect(snapshot: &RoomSnapshot, watch: &WatchView) -> f32 {
    let live = snapshot
        .members
        .iter()
        .find(|member| member.id == watch.publisher)
        .and_then(|member| member.lives.iter().find(|live| live.id == watch.live_id));
    let size = live.map(|live| {
        live.presets
            .iter()
            .find(|preset| preset.id == watch.preset_id)
            .map_or((live.source_width, live.source_height), |preset| {
                (preset.width, preset.height)
            })
    });
    match size {
        Some((width, height)) if width > 0 && height > 0 => width as f32 / height as f32,
        _ => USUAL_ASPECT,
    }
}

fn chip_text(snapshot: &RoomSnapshot, key: TileKey, title: &str) -> String {
    let owner = snapshot.members.iter().find(|m| m.id == key.0);
    owner.map_or_else(
        || title.to_string(),
        |member| format!("{} · {title}", member.nickname),
    )
}

/// Moves what the grid no longer shows from drawn to leaving. A popped-out tile went to its own
/// window rather than away, and one that came back mid-exit carries on from where it was.
fn note_departures(state: &mut UiState, watches: &[&WatchView], popped: &HashSet<TileKey>) {
    let shown: HashSet<TileKey> = watches.iter().map(|w| (w.publisher, w.live_id)).collect();
    for (key, tile) in std::mem::take(&mut state.drawn_tiles) {
        if !shown.contains(&key) && !popped.contains(&key) {
            state.leaving_tiles.insert(key, tile);
        }
    }
    state.leaving_tiles.retain(|key, _| !shown.contains(key));
}

/// Fades each leaving tile where it stands: the video darkens to the canvas in the renderer rather
/// than under a wash here, because a wash would also darken a neighbour moving into its space.
fn draw_leaving(ui: &mut egui::Ui, state: &mut UiState, grid: &mut Grid) {
    const WASH: f32 = 0.85;
    let id = ui.id();
    let mut finished = Vec::new();
    for (key, tile) in &state.leaving_tiles {
        let shown = motion::leave(ui.ctx(), id.with(("tile-leaving", *key)), motion::SURFACE);
        if shown <= 0.0 {
            finished.push(*key);
            continue;
        }
        // The renderer collapses the picture like a tube switching off, so the rect stays put.
        let rect = tile.rect;
        grid.leaving.push((*key, rect, shown));
        if tile.washed {
            ui.painter().rect_filled(
                rect,
                TILE_RADIUS,
                theme::surface().gamma_multiply(WASH * shown),
            );
        }
        let chrome = (shown * 3.0 - 2.0).max(0.0);
        if chrome > 0.0 {
            name_chip(ui, rect, &tile.chip, None, None, chrome);
            tile_edge(ui, rect, chrome);
        }
    }
    for key in &finished {
        state.leaving_tiles.remove(key);
    }
    state.gone_tiles.extend(finished);
}

/// The tube's edge. Video stays square and pixel exact right up to it.
fn tile_edge(ui: &egui::Ui, rect: egui::Rect, opacity: f32) {
    ui.painter().rect_stroke(
        rect,
        TILE_RADIUS,
        egui::Stroke::new(1.0, theme::rule().gamma_multiply(opacity)),
        egui::StrokeKind::Outside,
    );
}

/// Draws the tile grid without the popped-out lives, returning what the renderer has to place.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    ticket: &str,
    state: &mut UiState,
    popped: &HashSet<TileKey>,
    commands: &mut Vec<RoomCommand>,
    window_commands: &mut Vec<WindowCommand>,
) -> Grid {
    let mut grid = Grid::default();
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            let watches = visible_watches(snapshot, popped);
            let canvas_shown = motion::appear(
                ui.ctx(),
                ui.id().with("empty-canvas"),
                watches.is_empty(),
                motion::SURFACE,
            );
            if canvas_shown > 0.0 {
                let active = watches.is_empty();
                empty_canvas(
                    ui,
                    snapshot,
                    ticket,
                    state,
                    popped,
                    commands,
                    canvas_shown,
                    active,
                );
            }
            note_departures(state, &watches, popped);
            draw_leaving(ui, state, &mut grid);
            let area = ui.max_rect().shrink(TILE_GAP);
            let aspects: Vec<f32> = watches
                .iter()
                .map(|watch| picture_aspect(snapshot, watch))
                .collect();
            let current = state
                .grid_columns
                .filter(|(count, _)| *count == aspects.len())
                .map(|(_, columns)| columns);
            let columns = grid::columns(area.size(), &aspects, TILE_GAP, current);
            state.grid_columns = Some((aspects.len(), columns));
            // A tile changes place only when the grid itself changes, not when the window resizes.
            let layout_id = egui::Id::new((
                "tile-layout",
                columns,
                aspects.iter().map(|a| a.to_bits()).collect::<Vec<_>>(),
            ));
            let rects = grid::layout(area, &aspects, TILE_GAP, columns);
            for (index, (watch, target)) in watches.iter().zip(rects).enumerate() {
                let key = (watch.publisher, watch.live_id);
                // The video renderer reads this same rect, so the picture itself moves.
                let rect = motion::flowing_rect(
                    ui.ctx(),
                    ui.id().with(("tile-place", key)),
                    area,
                    target,
                    layout_id.with(index),
                );
                grid.placements.push((key, rect));
                tile_overlay(
                    ui,
                    snapshot,
                    state,
                    commands,
                    window_commands,
                    watch,
                    key,
                    rect,
                    Placement::Grid,
                );
                tile_edge(ui, rect, 1.0);
                let title =
                    live_title(snapshot, key).unwrap_or_else(|| SHARING_STOPPED.to_string());
                state.drawn_tiles.insert(
                    key,
                    DrawnTile {
                        rect,
                        chip: chip_text(snapshot, key, &title),
                        washed: watch.state != WatchState::Live,
                    },
                );
            }
        });
    grid
}

/// What the canvas holds while nothing is on it: whatever gets something onto it. Fades over the
/// tiles arriving or leaving, and takes no input once it is on its way out.
#[allow(clippy::too_many_arguments)]
fn empty_canvas(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    ticket: &str,
    state: &mut UiState,
    popped: &HashSet<TileKey>,
    commands: &mut Vec<RoomCommand>,
    shown: f32,
    active: bool,
) {
    const COLUMN_WIDTH: f32 = 380.0;
    if !snapshot.members.is_empty()
        && popped.is_empty()
        && super::room_map::fits(ui.max_rect(), snapshot.members.len())
    {
        super::room_map::draw(ui, snapshot, state, commands, shown, active);
        return;
    }
    super::room_map::light(ui, shown);
    let canvas = ui.max_rect();
    let column = egui::Rect::from_center_size(
        canvas.center() + motion::rise(ui.ctx(), shown, 6.0),
        egui::vec2(
            (canvas.width() - 32.0).clamp(0.0, COLUMN_WIDTH),
            canvas.height(),
        ),
    );
    ui.scope_builder(
        egui::UiBuilder::new()
            .id_salt("empty-canvas")
            .max_rect(column),
        |ui| {
            ui.set_opacity(shown);
            if !active {
                ui.disable();
            }
            egui::ScrollArea::vertical().show(ui, |ui| {
                theme::vertically_centred(ui, "empty-canvas", |ui| {
                    // The same backing as the map's labels, to keep this legible on the light.
                    egui::Frame::NONE
                        .fill(egui::Color32::BLACK)
                        .inner_margin(16)
                        .show(ui, |ui| {
                            canvas_contents(ui, snapshot, ticket, state, popped, commands);
                        });
                });
            });
        },
    );
}

fn canvas_contents(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    ticket: &str,
    state: &mut UiState,
    popped: &HashSet<TileKey>,
    commands: &mut Vec<RoomCommand>,
) {
    let heading = |ui: &mut egui::Ui, title: &str, hint: &str| {
        ui.vertical_centered(|ui| {
            ui.spacing_mut().item_spacing.y = EMPTY_GAP;
            ui.label(egui::RichText::new(title).font(theme::semibold(21.0)));
            ui.label(
                egui::RichText::new(hint)
                    .size(14.0)
                    .color(theme::secondary()),
            );
        });
        ui.add_space(16.0);
    };
    if !popped.is_empty() {
        heading(
            ui,
            "Watching in pop-outs",
            "Choose Back to grid in a pop-out to bring its stream here",
        );
        return;
    }
    let offers: Vec<_> = ordered_members(snapshot)
        .into_iter()
        .flat_map(|member| member.lives.iter().map(move |live| (member, live)))
        .filter_map(|(member, live)| {
            let key = (member.id, live.id);
            let wanted = state.preset_choice.get(&key).copied();
            offered_preset(live, wanted).map(|preset_id| (member, live, key, preset_id))
        })
        .collect();
    if !offers.is_empty() {
        heading(ui, "Ready to watch", "Pick a stream to bring it on screen");
        for (member, live, key, preset_id) in offers {
            if stream_offer(ui, &member.nickname, live).clicked() {
                commands.push(RoomCommand::Watch { key, preset_id });
            }
        }
        return;
    }
    if snapshot.members.is_empty() {
        heading(
            ui,
            "You’re the first here",
            "Send this link to bring someone in",
        );
        let link = link::share_link(ticket);
        let mut shown_link = link.as_str();
        theme::field(
            ui,
            egui::TextEdit::singleline(&mut shown_link),
            theme::FORM_HEIGHT,
        );
        copy_button(ui, icons::LINK, "Copy link", theme::FORM_HEIGHT, || link);
    } else {
        heading(
            ui,
            "Nothing on screen yet",
            "Streams appear here when someone shares",
        );
    }
    if nobody_is_sharing(snapshot) {
        let size = egui::vec2(ui.available_width(), theme::FORM_HEIGHT);
        if theme::wide_action(
            ui,
            icons::SHARE_MONITOR,
            "Share monitor",
            theme::Tone::Primary,
            can_share(snapshot, state),
            size,
        )
        .clicked()
        {
            commands.push(RoomCommand::Share {
                kind: SourceKind::Monitor,
                source: None,
            });
        }
    }
}

/// A stream on offer, as one target: who it is, what it is, and the word for what a click does.
fn stream_offer(ui: &mut egui::Ui, owner: &str, live: &LiveInfo) -> egui::Response {
    const HEIGHT: f32 = 56.0;
    const PADDING: f32 = 12.0;
    const LINE_GAP: f32 = 2.0;
    let title = format!("{owner} · {}", live.title);
    let response = theme::tactile(ui, theme::Tone::Neutral, false, true, |ui| {
        let size = egui::vec2(ui.available_width(), HEIGHT);
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
        let painter = ui.painter();
        let action =
            painter.layout_no_wrap("Watch".to_string(), theme::medium(14.0), theme::text());
        let room = (rect.width() - 3.0 * PADDING - action.size().x).max(0.0);
        let mut title_job = egui::text::LayoutJob::simple_singleline(
            title.clone(),
            theme::medium(14.0),
            theme::text(),
        );
        title_job.wrap = egui::text::TextWrapping::truncate_at_width(room);
        let title_galley = painter.layout_job(title_job);
        let mut detail_job = egui::text::LayoutJob::simple_singleline(
            format!(
                "{} × {} · {} fps",
                live.source_width, live.source_height, live.source_fps
            ),
            egui::FontId::proportional(12.0),
            theme::secondary(),
        );
        detail_job.wrap = egui::text::TextWrapping::truncate_at_width(room);
        let detail = painter.layout_job(detail_job);
        let left = rect.left() + PADDING;
        let top = rect.center().y - (title_galley.size().y + LINE_GAP + detail.size().y) / 2.0;
        let detail_top = top + title_galley.size().y + LINE_GAP;
        painter.galley(egui::pos2(left, top), title_galley, theme::text());
        painter.galley(egui::pos2(left, detail_top), detail, theme::secondary());
        painter.galley(
            egui::pos2(
                rect.right() - PADDING - action.size().x,
                rect.center().y - action.size().y / 2.0,
            ),
            action,
            theme::text(),
        );
        response
    });
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Watch {title}"))
    });
    response
}

/// The name that stays on the video, so a grid of screens can always be told apart, with how it
/// reaches you and how long that takes.
fn name_chip(
    ui: &egui::Ui,
    rect: egui::Rect,
    title: &str,
    path: Option<PathShape>,
    rtt_ms: Option<u32>,
    opacity: f32,
) {
    const PADDING: egui::Vec2 = egui::vec2(8.0, 4.0);
    const GLYPH: egui::Vec2 = egui::vec2(24.0, 12.0);
    let glyph_space = if path.is_some() { GLYPH.x + 8.0 } else { 0.0 };
    let figure = rtt_ms
        .map(|rtt| format!("  {}", delay_text(rtt)))
        .unwrap_or_default();
    let figure = ui
        .painter()
        .layout_no_wrap(figure, theme::medium(14.0), theme::secondary());
    let max_text =
        (rect.width() - 2.0 * TOOLBAR_MARGIN - 2.0 * PADDING.x - glyph_space - figure.size().x)
            .max(0.0);
    let mut job = egui::text::LayoutJob::simple_singleline(
        title.to_string(),
        theme::medium(14.0),
        theme::text(),
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_text);
    let galley = ui.painter().layout_job(job);
    let chip = egui::Rect::from_min_size(
        rect.left_top() + egui::Vec2::splat(TOOLBAR_MARGIN),
        egui::vec2(glyph_space + figure.size().x, 0.0) + galley.size() + 2.0 * PADDING,
    )
    .intersect(rect);
    let mut painter = ui.painter_at(rect);
    painter.multiply_opacity(opacity);
    painter.rect_filled(chip, 0, theme::ground());
    let mut left = chip.left() + PADDING.x;
    if let Some(path) = path {
        let y = chip.center().y;
        paint_path(
            &painter,
            egui::pos2(left + 2.0, y),
            egui::pos2(left + GLYPH.x - 2.0, y),
            path,
            2.0,
            theme::accent(),
        );
        left += glyph_space;
    }
    let text_top = chip.top() + PADDING.y;
    let text_width = galley.size().x;
    painter.galley(egui::pos2(left, text_top), galley, theme::text());
    painter.galley(
        egui::pos2(left + text_width, text_top),
        figure,
        theme::secondary(),
    );
}

/// The stream still on its way: the rail's own drawing of how it reaches you, with a pulse crossing
/// it from the publisher. Asks for 20 frames a second rather than every frame, because a stream can
/// sit in Reconnecting for as long as its publisher stays away.
fn crossing_path(
    ui: &egui::Ui,
    rect: egui::Rect,
    path: PathKind,
    pulse: egui::Color32,
    shown: f32,
) {
    const FRAME: std::time::Duration = std::time::Duration::from_millis(50);
    const CROSSING_SECONDS: f64 = 1.2;
    const DOT_RADIUS: f32 = 3.0;
    if !ui.is_rect_visible(rect) {
        return;
    }
    ui.ctx().request_repaint_after(FRAME);
    let (from, to) = (rect.left_center(), rect.right_center());
    let crossed = (ui.input(|input| input.time) / CROSSING_SECONDS).fract() as f32;
    let mut painter = ui.painter_at(rect.expand(DOT_RADIUS));
    painter.multiply_opacity(shown);
    paint_path(
        &painter,
        from,
        to,
        PathShape::of(path),
        DOT_RADIUS,
        theme::muted(),
    );
    // Dimmed at both ends, so the jump back to the start cannot be seen.
    let presence = (crossed * std::f32::consts::PI).sin();
    painter.circle_filled(
        from.lerp(to, crossed),
        DOT_RADIUS,
        pulse.gamma_multiply(presence),
    );
}

/// What is happening instead of video: the stream's name, why it is not showing, and its path with
/// a pulse crossing it while something is still in flight. A wash covers the whole tile: it dims the last frame a
/// reconnecting tile can still be holding, in any colour, and with no frame yet it shows where the
/// tile is, so the words read as centred in something rather than adrift on the black canvas. When
/// video arrives it fades off the picture still saying what it last said, rather than vanishing on
/// the frame the state changed.
fn tile_state(
    ui: &mut egui::Ui,
    id: egui::Id,
    rect: egui::Rect,
    radius: u8,
    title: &str,
    state: WatchState,
    path: PathKind,
) {
    const WASH: f32 = 0.85;
    const PATH_SIZE: egui::Vec2 = egui::vec2(64.0, 20.0);
    const PATH_GAP: f32 = 12.0;
    let shown = motion::appear(ui.ctx(), id, state != WatchState::Live, motion::TRAVEL);
    if shown <= 0.0 {
        return;
    }
    let last_id = id.with("last-state");
    if state != WatchState::Live {
        ui.ctx().data_mut(|data| data.insert_temp(last_id, state));
    }
    let said = ui
        .ctx()
        .data(|data| data.get_temp::<WatchState>(last_id))
        .unwrap_or(state);
    let (status, ink, in_flight) = match said {
        WatchState::Live => return,
        WatchState::Connecting => ("Connecting", theme::secondary(), true),
        WatchState::SignalLost => ("No signal", theme::error(), true),
        WatchState::Reconnecting => ("Reconnecting", theme::warning(), true),
        WatchState::Ended => ("Stream ended", theme::secondary(), false),
    };
    let mut painter = ui.painter_at(rect);
    painter.multiply_opacity(shown);
    painter.rect_filled(rect, radius, theme::surface().gamma_multiply(WASH));
    let mut title_job = egui::text::LayoutJob::simple_singleline(
        title.to_string(),
        theme::medium(16.0),
        theme::text(),
    );
    title_job.wrap =
        egui::text::TextWrapping::truncate_at_width((rect.width() - 2.0 * TOOLBAR_MARGIN).max(0.0));
    let title = painter.layout_job(title_job);
    let status = painter.layout_no_wrap(status.to_string(), egui::FontId::proportional(13.0), ink);
    let path_space = if in_flight {
        PATH_SIZE.y + PATH_GAP
    } else {
        0.0
    };
    let center = rect.center();
    let top = center.y - (path_space + title.size().y + status.size().y) / 2.0;
    if in_flight {
        let pulse = if said == WatchState::Reconnecting {
            theme::warning()
        } else {
            theme::accent()
        };
        crossing_path(
            ui,
            egui::Rect::from_min_size(egui::pos2(center.x - PATH_SIZE.x / 2.0, top), PATH_SIZE),
            path,
            pulse,
            shown,
        );
    }
    let title_top = top + path_space;
    let status_top = title_top + title.size().y;
    painter.galley(
        egui::pos2(center.x - title.size().x / 2.0, title_top),
        title,
        theme::text(),
    );
    painter.galley(
        egui::pos2(center.x - status.size().x / 2.0, status_top),
        status,
        ink,
    );
}

/// The words on a picture reducing to characters. Red is the one colour that does not change with
/// the phosphor, so an alarm reads the same on every tube.
fn no_signal(ui: &egui::Ui, rect: egui::Rect, seconds: u64, reconnecting: bool) {
    let painter = ui.painter_at(rect);
    let head = painter.layout_no_wrap(
        "No signal".to_string(),
        theme::semibold(14.0),
        egui::Color32::from_rgb(16, 0, 0),
    );
    let foot = painter.layout_no_wrap(
        if reconnecting {
            format!("Last heard {seconds} s ago · reconnecting")
        } else {
            format!("Last heard {seconds} s ago")
        },
        theme::medium(14.0),
        theme::secondary(),
    );
    let middle = rect.center();
    let head_rect = egui::Rect::from_center_size(
        middle - egui::vec2(0.0, 12.0),
        head.size() + egui::vec2(20.0, 4.0),
    );
    let foot_rect = egui::Rect::from_center_size(
        middle + egui::vec2(0.0, 12.0),
        foot.size() + egui::vec2(20.0, 4.0),
    );
    painter.rect_filled(head_rect, 0, theme::error());
    painter.rect_filled(foot_rect, 0, theme::ground());
    painter.galley(
        head_rect.min + egui::vec2(10.0, 2.0),
        head,
        egui::Color32::from_rgb(16, 0, 0),
    );
    painter.galley(
        foot_rect.min + egui::vec2(10.0, 2.0),
        foot,
        theme::secondary(),
    );
}

/// Status text, the stats readout, and the hover bar for one live drawn in `rect`.
#[allow(clippy::too_many_arguments)]
pub fn tile_overlay(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    state: &mut UiState,
    commands: &mut Vec<RoomCommand>,
    window_commands: &mut Vec<WindowCommand>,
    watch: &WatchView,
    key: TileKey,
    rect: egui::Rect,
    placement: Placement,
) {
    let response = ui.allocate_rect(rect, egui::Sense::hover());
    let publisher = snapshot.members.iter().find(|m| m.id == key.0);
    let live = publisher.and_then(|m| m.lives.iter().find(|l| l.id == key.1));
    let title = live_title(snapshot, key).unwrap_or_else(|| SHARING_STOPPED.to_string());
    let chip = chip_text(snapshot, key, &title);
    let lost = signal_lost(watch.state);
    let now = ui.input(|input| input.time);
    let since_id = egui::Id::new(("tile-lost-since", key));
    let since = ui.ctx().data(|data| data.get_temp::<f64>(since_id));
    let lost_for = if lost {
        let since = since.unwrap_or(now);
        ui.ctx().data_mut(|data| data.insert_temp(since_id, since));
        // Steps at the rate network motion does, and only while the picture is failing. Someone can
        // be watching from another window, so an unfocused one still has to count the seconds.
        let moving = ui.input(|input| input.focused) && motion::ambient_allowed(ui.ctx());
        let wait = if moving { 83 } else { 1000 };
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(wait));
        // The signal only reads as lost after this much silence, so it has been that long already.
        Some(SIGNAL_LOST_AFTER.as_secs() + (now - since) as u64)
    } else {
        ui.ctx().data_mut(|data| data.remove::<f64>(since_id));
        None
    };
    let fx = TileFx {
        ascii: motion::follow(
            ui.ctx(),
            egui::Id::new(("tile-ascii", key)),
            lost,
            REDUCE_SECONDS,
        ),
        warm: motion::follow(
            ui.ctx(),
            egui::Id::new(("tile-warm", key)),
            watch.state != WatchState::Connecting,
            WARM_UP_SECONDS,
        ),
    };
    ui.ctx()
        .data_mut(|data| data.insert_temp(egui::Id::new(("tile-fx", key)), fx));
    if let Some(seconds) = lost_for {
        no_signal(ui, rect, seconds, watch.state == WatchState::Reconnecting);
    } else {
        tile_state(
            ui,
            ui.id().with(("tile-state", key)),
            rect,
            match placement {
                Placement::Grid => TILE_RADIUS,
                Placement::PopOut { .. } => 0,
            },
            &title,
            watch.state,
            publisher.map_or(PathKind::Unknown, |member| member.path),
        );
    }
    if state.stats_visible.contains(&key) {
        let preset = live
            .and_then(|l| l.presets.iter().find(|p| p.id == watch.preset_id))
            .map(|p| p.name.as_str())
            .unwrap_or("?");
        // A frame of video can be any colour, so the readout carries its own backing.
        let galley = ui.painter().layout_no_wrap(
            format!(
                "decoded {}  keyframe requests {}  preset {preset}",
                watch.frames_decoded, watch.keyframe_requests
            ),
            theme::medium(14.0),
            theme::text(),
        );
        let at = rect.left_top() + egui::vec2(TOOLBAR_MARGIN + 8.0, TOOLBAR_MARGIN + 36.0);
        let painter = ui.painter_at(rect);
        painter.rect_filled(
            egui::Rect::from_min_size(at, galley.size()).expand(4.0),
            theme::RADIUS_CHIP,
            theme::surface(),
        );
        painter.galley(at, galley, theme::text());
    }
    // A preset dropdown extends past the tile; keeping overlays up while any popup is open stops
    // the dropdown from vanishing under the pointer.
    let show_controls = response.contains_pointer()
        || ui.input(|input| input.key_pressed(egui::Key::Tab))
        || egui::Popup::is_any_open(ui.ctx())
        || ui.ctx().memory(|memory| memory.focused().is_some());
    let opacity = motion::appear(
        ui.ctx(),
        ui.id().with(("stream-controls", key)),
        show_controls,
        motion::TRAVEL,
    );
    // A pop-out shows one stream, so its name has nothing to tell apart until the pointer arrives.
    let chip_opacity = match placement {
        Placement::Grid => 1.0,
        Placement::PopOut { .. } => opacity,
    };
    if chip_opacity > 0.0 {
        let path = publisher.map(|member| {
            PathShape::eased(ui.ctx(), ui.id().with(("chip-path", key)), member.path)
        });
        let rtt = publisher.and_then(|member| member.rtt_ms).filter(|_| !lost);
        name_chip(ui, rect, &chip, path, rtt, chip_opacity);
    }
    if opacity <= 0.0 {
        return;
    }
    // Sized to last pass's content, so the bar hugs its controls and covers as little video as it
    // can. The first pass has no measure yet and is invisible anyway.
    let measured_id = ui.id().with(("stream-controls-width", key));
    let available = (rect.width() - 2.0 * TOOLBAR_MARGIN).max(0.0);
    let width = ui
        .ctx()
        .data(|data| data.get_temp::<f32>(measured_id))
        .unwrap_or(available)
        .min(available);
    // The bar rises out of the bottom edge and sinks back the same way, retargeting mid-flight.
    let rise = (1.0 - opacity) * 6.0;
    let bar = egui::Rect::from_center_size(
        egui::pos2(
            rect.center().x,
            rect.bottom() - TOOLBAR_MARGIN - TOOLBAR_HEIGHT / 2.0 + rise,
        ),
        egui::vec2(width, TOOLBAR_HEIGHT.min((rect.height() - 24.0).max(0.0))),
    );
    // Concentric with the controls inside it, which sit `BAR_PADDING` in from its edge.
    let bar_painter = ui.painter_at(rect);
    bar_painter.rect_filled(bar, 0, theme::ground().gamma_multiply(opacity));
    let labelled = rect.width() >= LABELLED_TOOLBAR_WIDTH;
    ui.scope_builder(
        egui::UiBuilder::new()
            .id_salt(("stream-controls", key))
            .max_rect(bar.shrink(BAR_PADDING))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.set_clip_rect(ui.clip_rect().intersect(rect));
            ui.set_opacity(opacity);
            // Fading controls stop taking input as soon as the pointer leaves.
            if !show_controls {
                ui.disable();
            }
            // Before the row exists, which takes its height from this and centres its controls on it.
            theme::uniform_controls(ui);
            let output = egui::ScrollArea::horizontal()
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
                .id_salt(("tile-toolbar", key))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        if let Some(live) = live {
                            let Some(mut preset_id) = offered_preset(live, Some(watch.preset_id))
                            else {
                                return;
                            };
                            preset_selector(
                                ui,
                                ("tile-preset", key),
                                &live.presets,
                                &mut preset_id,
                            );
                            if preset_id != watch.preset_id {
                                state.preset_choice.insert(key, preset_id);
                                commands.push(RoomCommand::Watch { key, preset_id });
                            }
                        }
                        if watch.audio {
                            let gain = snapshot
                                .members
                                .iter()
                                .find(|m| m.id == key.0)
                                .map(|m| m.gain)
                                .unwrap_or(1.0);
                            if let Some(gain) =
                                super::volume_control(ui, ("tile-volume", key), gain)
                            {
                                commands.push(RoomCommand::SetVolume {
                                    publisher: key.0,
                                    gain,
                                });
                            }
                        }
                        let mut stats = state.stats_visible.contains(&key);
                        let stats_response = if labelled {
                            theme::quiet_toggle(ui, &mut stats, "Stats")
                        } else {
                            theme::icon_toggle(ui, &mut stats, icons::STATS, "Stats")
                        };
                        if stats_response.changed() {
                            if stats {
                                state.stats_visible.insert(key);
                            } else {
                                state.stats_visible.remove(&key);
                            }
                        }
                        let tool = |ui: &mut egui::Ui, glyph: &str, name: &str| {
                            if labelled {
                                theme::quiet_action(ui, name).clicked()
                            } else {
                                theme::icon_button(ui, glyph, name, true).clicked()
                            }
                        };
                        match placement {
                            Placement::Grid => {
                                if tool(ui, icons::POP_OUT, "Pop out") {
                                    window_commands.push(WindowCommand::PopOut(key));
                                }
                                if tool(ui, icons::FULLSCREEN, "Fullscreen") {
                                    window_commands.push(WindowCommand::PopOutFullscreen(key));
                                }
                            }
                            Placement::PopOut { fullscreen } => {
                                let (glyph, label) = if fullscreen {
                                    (icons::WINDOWED, "Exit fullscreen")
                                } else {
                                    (icons::FULLSCREEN, "Fullscreen")
                                };
                                if tool(ui, glyph, label) {
                                    window_commands.push(WindowCommand::ToggleFullscreen(key));
                                }
                                if tool(ui, icons::BACK_TO_GRID, "Back to grid") {
                                    window_commands.push(WindowCommand::ReturnToGrid(key));
                                }
                            }
                        }
                        if tool(ui, icons::CLOSE, "Close") {
                            commands.push(RoomCommand::Unwatch(key));
                        }
                    });
                });
            // Nothing else wakes this window when the measure changes without input, such as after
            // a preset rename, and the bar would sit at the old width until something did.
            let content_width = output.content_size.x + 2.0 * BAR_PADDING;
            // Against the clamped value: a toolbar wider than its tile would otherwise never match,
            // and keep the window drawing for as long as the toolbar shows.
            if content_width.min(available) != width {
                ui.ctx().request_repaint();
            }
            ui.ctx()
                .data_mut(|data| data.insert_temp(measured_id, content_width));
        },
    );
}

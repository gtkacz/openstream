//! Central panel: reserves one rect per watched live for the video renderer and draws the hover
//! overlay and status text on top. The panel frame is transparent so the tiles show through. The
//! overlay is shared with pop-out windows, which place one live over their whole panel.

use std::collections::HashSet;

use brp_room::{RoomSnapshot, WatchState, WatchView};

use super::members::{offered_preset, preset_selector};
use super::motion;
use super::state::{UiState, live_title, visible_watches};
use super::theme::{self, icons};
use crate::commands::{RoomCommand, WindowCommand};
use crate::render::grid;
use crate::render::tiles::TileKey;

/// Where an overlay is drawn, which decides its window buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// A cell of the main window's grid: offers pop out and fullscreen.
    Grid,
    /// A pop-out window: offers the fullscreen toggle and a return to the grid.
    PopOut { fullscreen: bool },
}

/// The gap around each tile, as the inset of the area and of each cell from its grid neighbours.
/// Equal, so the gap to the window's edge is the same as the gap between tiles.
const AREA_INSET: f32 = 4.0;
const CELL_INSET: f32 = 4.0;
const TILE_RADIUS: u8 = theme::RADIUS_CARD;
/// Wide enough to cover a square viewport's corners under `TILE_RADIUS`, which reach 0.42 of the
/// radius past the curve, and narrow enough not to touch the next tile.
const TILE_RING: f32 = 6.0;
/// Below this tile width the toolbar drops its labels, keeping its names as tooltips.
const LABELLED_TOOLBAR_WIDTH: f32 = 640.0;
const TOOLBAR_HEIGHT: f32 = 48.0;
const BAR_PADDING: f32 = 6.0;
const TOOLBAR_MARGIN: f32 = 12.0;
const EMPTY_GAP: f32 = 4.0;

/// Draws the tile grid without the popped-out lives, returning where the video renderer should
/// place each drawn live's frame, in egui points.
pub fn draw(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    state: &mut UiState,
    popped: &HashSet<TileKey>,
    commands: &mut Vec<RoomCommand>,
    window_commands: &mut Vec<WindowCommand>,
) -> Vec<(TileKey, egui::Rect)> {
    let mut placements = Vec::new();
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            let watches = visible_watches(snapshot, popped);
            if watches.is_empty() {
                empty_canvas(ui, snapshot, popped);
                return;
            }
            let area = ui.max_rect().shrink(AREA_INSET);
            let rects = grid::layout(area, watches.len());
            for (watch, cell) in watches.iter().zip(rects) {
                let key = (watch.publisher, watch.live_id);
                // The video renderer reads this same rect, so the picture itself moves.
                let rect = motion::flowing_rect(
                    ui.ctx(),
                    ui.id().with(("tile-place", key)),
                    area,
                    cell.shrink(CELL_INSET),
                );
                placements.push((key, rect));
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
                // The renderer draws square viewports under this layer. A canvas-coloured ring
                // just outside the tile covers the corners, which rounds the video without a mask
                // in the video shader.
                ui.painter().rect_stroke(
                    rect,
                    TILE_RADIUS,
                    egui::Stroke::new(TILE_RING, theme::CANVAS),
                    egui::StrokeKind::Outside,
                );
            }
        });
    placements
}

fn empty_canvas(ui: &mut egui::Ui, snapshot: &RoomSnapshot, popped: &HashSet<TileKey>) {
    let someone_is_sharing = snapshot.members.iter().any(|m| !m.lives.is_empty());
    let (title, hint) = if !popped.is_empty() {
        (
            "Watching in pop-outs",
            "Your streams are open in their own windows",
        )
    } else if someone_is_sharing {
        (
            "Ready to watch",
            "Switch on a stream in the room list to start watching",
        )
    } else {
        (
            "Nothing on screen yet",
            "Streams appear in the room list when someone shares",
        )
    };
    let shown = motion::arrive(ui.ctx(), ui.id().with("empty-canvas"), motion::SURFACE);
    let painter = ui.painter();
    let title = painter.layout_no_wrap(
        title.to_string(),
        theme::semibold(18.0),
        theme::TEXT.gamma_multiply(shown),
    );
    let mut hint_job = egui::text::LayoutJob::single_section(
        hint.to_string(),
        egui::TextFormat::simple(
            egui::FontId::proportional(14.0),
            theme::SECONDARY.gamma_multiply(shown),
        ),
    );
    hint_job.wrap.max_width = (ui.available_width() - 32.0).clamp(32.0, 360.0);
    hint_job.halign = egui::Align::Center;
    let hint = painter.layout_job(hint_job);
    // The pair is centred as one block, whichever way the hint wrapped.
    let center = ui.max_rect().center() + motion::rise(shown, 6.0);
    let top = center.y - (title.size().y + EMPTY_GAP + hint.size().y) / 2.0;
    let hint_top = top + title.size().y + EMPTY_GAP;
    painter.galley(
        egui::pos2(center.x - title.size().x / 2.0, top),
        title,
        theme::TEXT,
    );
    painter.galley(egui::pos2(center.x, hint_top), hint, theme::SECONDARY);
}

/// The name that stays on the video, so a grid of screens can always be told apart.
fn name_chip(ui: &egui::Ui, rect: egui::Rect, title: &str, live: f32, opacity: f32) {
    const PADDING: egui::Vec2 = egui::vec2(8.0, 4.0);
    const DOT_SPACE: f32 = 12.0;
    let max_text = (rect.width() - 2.0 * TOOLBAR_MARGIN - 2.0 * PADDING.x - DOT_SPACE).max(0.0);
    let mut job = egui::text::LayoutJob::simple_singleline(
        title.to_string(),
        theme::medium(12.0),
        theme::TEXT,
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_text);
    let galley = ui.painter().layout_job(job);
    let chip = egui::Rect::from_min_size(
        rect.left_top() + egui::Vec2::splat(TOOLBAR_MARGIN),
        egui::vec2(DOT_SPACE, 0.0) + galley.size() + 2.0 * PADDING,
    )
    .intersect(rect);
    let mut painter = ui.painter_at(rect);
    painter.multiply_opacity(opacity);
    painter.rect_filled(chip, theme::RADIUS_CHIP, theme::SURFACE);
    painter.circle_filled(
        chip.left_center() + egui::vec2(PADDING.x + 3.0, 0.0),
        3.0,
        theme::MUTED.lerp_to_gamma(theme::LIVE, live),
    );
    painter.galley(
        chip.min + PADDING + egui::vec2(DOT_SPACE, 0.0),
        galley,
        theme::TEXT,
    );
}

/// What is happening instead of video: the stream's name, why it is not showing, and a spinner
/// while something is still in flight. A wash covers the whole tile: it dims the last frame a
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
) {
    const WASH: f32 = 0.85;
    const SPINNER: f32 = 20.0;
    const SPINNER_GAP: f32 = 12.0;
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
        WatchState::Connecting => ("Connecting", theme::SECONDARY, true),
        WatchState::Reconnecting => ("Reconnecting", theme::WARNING, true),
        WatchState::Ended => ("Stream ended", theme::SECONDARY, false),
    };
    let mut painter = ui.painter_at(rect);
    painter.multiply_opacity(shown);
    painter.rect_filled(rect, radius, theme::SURFACE.gamma_multiply(WASH));
    let mut title_job = egui::text::LayoutJob::simple_singleline(
        title.to_string(),
        theme::medium(16.0),
        theme::TEXT,
    );
    title_job.wrap =
        egui::text::TextWrapping::truncate_at_width((rect.width() - 2.0 * TOOLBAR_MARGIN).max(0.0));
    let title = painter.layout_job(title_job);
    let status = painter.layout_no_wrap(status.to_string(), egui::FontId::proportional(13.0), ink);
    let spinner_space = if in_flight {
        SPINNER + SPINNER_GAP
    } else {
        0.0
    };
    let center = rect.center();
    let top = center.y - (spinner_space + title.size().y + status.size().y) / 2.0;
    if in_flight {
        theme::spinner_at(
            ui,
            egui::Rect::from_min_size(
                egui::pos2(center.x - SPINNER / 2.0, top),
                egui::Vec2::splat(SPINNER),
            ),
            shown,
        );
    }
    let title_top = top + spinner_space;
    let status_top = title_top + title.size().y;
    painter.galley(
        egui::pos2(center.x - title.size().x / 2.0, title_top),
        title,
        theme::TEXT,
    );
    painter.galley(
        egui::pos2(center.x - status.size().x / 2.0, status_top),
        status,
        ink,
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
    let live = snapshot
        .members
        .iter()
        .find(|m| m.id == key.0)
        .and_then(|m| m.lives.iter().find(|l| l.id == key.1));
    let title = live_title(snapshot, key).unwrap_or_else(|| "Publisher left".to_string());
    let owner = snapshot
        .members
        .iter()
        .find(|m| m.id == key.0)
        .map(|m| m.nickname.as_str());
    let chip_text = owner.map_or_else(|| title.clone(), |name| format!("{name} · {title}"));
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
    );
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
            egui::FontId::proportional(12.0),
            theme::TEXT,
        );
        let at = rect.left_top() + egui::vec2(TOOLBAR_MARGIN, TOOLBAR_MARGIN + 32.0);
        let painter = ui.painter_at(rect);
        painter.rect_filled(
            egui::Rect::from_min_size(at, galley.size()).expand(4.0),
            theme::RADIUS_CHIP,
            theme::SURFACE,
        );
        painter.galley(at, galley, theme::TEXT);
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
        let live = motion::follow(
            ui.ctx(),
            ui.id().with(("tile-live", key)),
            watch.state == WatchState::Live,
            motion::TRAVEL,
        );
        name_chip(ui, rect, &chip_text, live, chip_opacity);
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
    let bar_radius = theme::RADIUS_CONTROL + BAR_PADDING as u8;
    let mut shadow = ui.visuals().popup_shadow;
    shadow.color = shadow.color.gamma_multiply(opacity);
    let bar_painter = ui.painter_at(rect);
    bar_painter.add(shadow.as_shape(bar, bar_radius));
    bar_painter.rect_filled(bar, bar_radius, theme::SURFACE.gamma_multiply(opacity));
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
            let output = egui::ScrollArea::horizontal()
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
                .id_salt(("tile-toolbar", key))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        theme::uniform_controls(ui);
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
                            theme::toggle(ui, &mut stats, icons::STATS, "Stats")
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
                                theme::action(ui, glyph, name, true).clicked()
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
                                    (icons::WINDOWED, "Windowed")
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

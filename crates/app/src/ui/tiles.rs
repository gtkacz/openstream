//! Central panel: reserves one rect per watched live for the video renderer and draws the hover
//! overlay and status text on top. The panel frame is transparent so the tiles show through. The
//! overlay is shared with pop-out windows, which place one live over their whole panel.

use std::collections::HashSet;

use brp_room::{RoomSnapshot, WatchState, WatchView};

use super::members::{offered_preset, preset_selector};
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
const AREA_INSET: f32 = 5.0;
const CELL_INSET: f32 = 3.0;
const TILE_RADIUS: u8 = 8;
/// Below this tile width the toolbar drops its labels, keeping its names as tooltips.
const LABELLED_TOOLBAR_WIDTH: f32 = 640.0;
const TOOLBAR_HEIGHT: f32 = 48.0;
const TOOLBAR_MARGIN: f32 = 12.0;

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
            let rects = grid::layout(ui.max_rect().shrink(AREA_INSET), watches.len());
            for (watch, cell) in watches.iter().zip(rects) {
                let key = (watch.publisher, watch.live_id);
                let rect = cell.shrink(CELL_INSET);
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
                    egui::Stroke::new(CELL_INSET + 1.0, theme::CANVAS),
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
    let center = ui.max_rect().center();
    ui.painter().text(
        center - egui::vec2(0.0, 14.0),
        egui::Align2::CENTER_CENTER,
        title,
        theme::semibold(20.0),
        theme::TEXT,
    );
    ui.scope_builder(
        egui::UiBuilder::new().max_rect(egui::Rect::from_center_size(
            center + egui::vec2(0.0, 28.0),
            egui::vec2((ui.available_width() - 32.0).clamp(32.0, 360.0), 56.0),
        )),
        |ui| {
            ui.vertical_centered(|ui| {
                ui.weak(hint);
            });
        },
    );
}

/// The name that stays on the video, so a grid of screens can always be told apart.
fn name_chip(ui: &egui::Ui, rect: egui::Rect, title: &str, live: bool, opacity: f32) {
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
    painter.rect_filled(chip, 6.0, theme::SURFACE);
    painter.circle_filled(
        chip.left_center() + egui::vec2(PADDING.x + 3.0, 0.0),
        3.0,
        if live { theme::LIVE } else { theme::MUTED },
    );
    painter.galley(
        chip.min + PADDING + egui::vec2(DOT_SPACE, 0.0),
        galley,
        theme::TEXT,
    );
}

/// What is happening instead of video: the stream's name, why it is not showing, and a spinner
/// while something is still in flight. On its own surface, because a reconnecting tile can still
/// be holding its last frame underneath, in any colour.
fn tile_state(ui: &mut egui::Ui, rect: egui::Rect, title: &str, state: &WatchState) {
    let (status, ink, in_flight) = match state {
        WatchState::Live => return,
        WatchState::Connecting => ("Connecting", theme::SECONDARY, true),
        WatchState::Reconnecting => ("Reconnecting", theme::WARNING, true),
        WatchState::Ended => ("Stream ended", theme::SECONDARY, false),
    };
    let center = rect.center();
    let height = if in_flight { 108.0 } else { 72.0 };
    let card = egui::Rect::from_center_size(center, egui::vec2(rect.width().min(240.0), height))
        .intersect(rect);
    let painter = ui.painter_at(rect);
    painter.rect_filled(card, 10.0, theme::SURFACE);
    if in_flight {
        theme::spinner_at(
            ui,
            egui::Rect::from_center_size(center - egui::vec2(0.0, 28.0), egui::Vec2::splat(20.0)),
        );
    }
    let text_top = if in_flight {
        center.y + 6.0
    } else {
        center.y - 12.0
    };
    painter.text(
        egui::pos2(center.x, text_top),
        egui::Align2::CENTER_CENTER,
        title,
        theme::medium(16.0),
        theme::TEXT,
    );
    painter.text(
        egui::pos2(center.x, text_top + 22.0),
        egui::Align2::CENTER_CENTER,
        status,
        egui::FontId::proportional(13.0),
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
    tile_state(ui, rect, &title, &watch.state);
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
            egui::FontId::monospace(12.0),
            theme::TEXT,
        );
        let at = rect.left_top() + egui::vec2(TOOLBAR_MARGIN, TOOLBAR_MARGIN + 32.0);
        let painter = ui.painter_at(rect);
        painter.rect_filled(
            egui::Rect::from_min_size(at, galley.size()).expand(4.0),
            4.0,
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
    let opacity = ui.ctx().animate_bool_with_time_and_easing(
        ui.id().with(("stream-controls", key)),
        show_controls,
        0.16,
        egui::emath::easing::cubic_out,
    );
    // A pop-out shows one stream, so its name has nothing to tell apart until the pointer arrives.
    let chip_opacity = match placement {
        Placement::Grid => 1.0,
        Placement::PopOut { .. } => opacity,
    };
    if chip_opacity > 0.0 {
        name_chip(
            ui,
            rect,
            &chip_text,
            watch.state == WatchState::Live,
            chip_opacity,
        );
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
    ui.painter_at(rect)
        .rect_filled(bar, 10.0, theme::SURFACE.gamma_multiply(opacity));
    let labelled = rect.width() >= LABELLED_TOOLBAR_WIDTH;
    ui.scope_builder(
        egui::UiBuilder::new()
            .id_salt(("stream-controls", key))
            .max_rect(bar.shrink(6.0))
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
            let content_width = output.content_size.x + 12.0;
            if content_width != width {
                ui.ctx().request_repaint();
            }
            ui.ctx()
                .data_mut(|data| data.insert_temp(measured_id, content_width));
        },
    );
}

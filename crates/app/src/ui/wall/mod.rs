//! The start screen's wall: stills as 1-bit dots, drifting past a horizon of light. The pointer burns
//! through one to full colour, and it knits back cell by cell once the pointer has gone.

pub mod geometry;
mod reveal;
pub mod stills;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use egui::{Color32, Context, Id, Mesh, Pos2, Rect, Sense, Ui, pos2, vec2};
use geometry::{COLUMN_WIDTH, GAP, ROW_HEIGHT, TILE, column_speed, start_offset, still_index};
use reveal::Reveal;
use stills::{Grid, Loader, STILLS};

use super::motion;
use super::theme;

const STATE: &str = "wall-state";
const HOVER: &str = "wall-hover";
const COLUMN_PACE: &str = "wall-column-pace";
const STILL_ARRIVAL: &str = "wall-still-arrival";

const LOADING_POLL: f32 = 0.05;
const DRIFT_FRAME: f32 = 1.0 / 60.0;

const CELL_POINTS: f32 = 3.0;
/// How strong the dots are at rest, so the form's button and a revealed picture stay the brightest.
const DOTS: f32 = 0.62;
/// One still in this many takes the signal's colour, so the wall is not all one tone.
const SIGNAL_EVERY: usize = 6;
/// Wide enough that from near the middle of a tile the reveal covers nearly all of it, leaving only
/// the corners dotted.
const REVEAL_REACH: f32 = 0.75 * TILE.x;
/// How far from the wall's edge the tiles take to come up, leaving the form room.
const EDGE_FADE: f32 = 140.0;

struct WallState {
    loader: Loader,
    grid: Grid,
    offsets: HashMap<i32, f64>,
    last_time: Option<f64>,
    reveals: HashMap<(i32, i64), Reveal>,
    hovered: Option<(i32, i64)>,
    /// The stills the last frame had on screen.
    visible: HashSet<usize>,
}

impl WallState {
    fn new(loader: Loader, grid: Grid) -> Self {
        Self {
            loader,
            grid,
            offsets: HashMap::new(),
            last_time: None,
            reveals: HashMap::new(),
            hovered: None,
            visible: HashSet::new(),
        }
    }
}

fn state(ctx: &Context, grid: Grid) -> Arc<Mutex<WallState>> {
    ctx.data_mut(|data| {
        let slot = data.get_temp_mut_or_insert_with(Id::new(STATE), || {
            Arc::new(Mutex::new(WallState::new(Loader::start(grid), grid)))
        });
        // A new display scale means new dots; the old set is only the wrong size, so swap it whole.
        if slot.lock().is_ok_and(|wall| wall.grid != grid) {
            *slot = Arc::new(Mutex::new(WallState::new(Loader::start(grid), grid)));
        }
        slot.clone()
    })
}

/// Frees the stills, for when the start screen will not be drawn again.
pub fn release(ctx: &Context) {
    ctx.data_mut(|data| data.remove::<Arc<Mutex<WallState>>>(Id::new(STATE)));
}

/// A tile as a small grid of quads, so the wall can dissolve across it towards the form and the
/// horizon. A plain quad would fade too coarsely across a tile this size.
fn tile_mesh(
    rect: Rect,
    texture: egui::TextureId,
    tint: Color32,
    fade: &impl Fn(Pos2) -> f32,
) -> Mesh {
    const ACROSS: usize = 6;
    const DOWN: usize = 4;
    let mut mesh = Mesh::with_texture(texture);
    for row in 0..=DOWN {
        for column in 0..=ACROSS {
            let uv = pos2(column as f32 / ACROSS as f32, row as f32 / DOWN as f32);
            let at = rect.min + rect.size() * uv.to_vec2();
            mesh.vertices.push(egui::epaint::Vertex {
                pos: at,
                // The default texture is the font atlas, so a plain fill has to sample its white pixel.
                uv: if texture == egui::TextureId::default() {
                    egui::epaint::WHITE_UV
                } else {
                    uv
                },
                color: tint.gamma_multiply(fade(at)),
            });
        }
    }
    let stride = (ACROSS + 1) as u32;
    for row in 0..DOWN as u32 {
        for column in 0..ACROSS as u32 {
            let at = row * stride + column;
            mesh.add_triangle(at, at + 1, at + stride);
            mesh.add_triangle(at + 1, at + stride + 1, at + stride);
        }
    }
    mesh
}

/// Draws the wall into `area`. Call it before anything that should sit on top of it.
pub fn draw(ui: &mut Ui, area: Rect) {
    let ctx = ui.ctx().clone();
    let scale = ctx.pixels_per_point();
    let grid = Grid::for_tile(TILE, CELL_POINTS, scale);
    let state = state(&ctx, grid);
    let Ok(mut wall) = state.lock() else {
        return;
    };
    let wall = &mut *wall;
    let now = ctx.input(|input| input.time);
    let gently = !motion::ambient_allowed(&ctx);
    let settling = wall.loader.poll(&ctx, now, gently, &wall.visible);
    wall.visible.clear();
    let drifting = !gently && ctx.input(|input| input.focused);
    let response = ui.interact(area, Id::new(HOVER), Sense::hover());
    let pointer = response.hover_pos();
    let pressed = ctx.input(|input| input.pointer.primary_pressed());
    let window = ctx.content_rect();
    let fade = |at: Pos2| {
        let across = ((at.x - area.left()) / EDGE_FADE).clamp(0.0, 1.0);
        let down = at.y / window.height();
        let sink = 1.0 - ((down - 0.56) / 0.3).clamp(0.0, 1.0);
        across * sink
    };
    let (tile_w, tile_h) = (grid.width as f32 / scale, grid.height as f32 / scale);
    let columns = ((area.width() / COLUMN_WIDTH).ceil() as i32).max(1);
    let painter = ui.painter().with_clip_rect(area);
    // Halfway to the wire, because dimming this blue towards black alone turns it teal.
    let blue = theme::accent().lerp_to_gamma(theme::wire(), 0.5);
    // Partway to its lit wire, for the same reason, and so that it sits as deep as the blue.
    let green = theme::signal().lerp_to_gamma(theme::signal_line(), 0.5);
    let tint_of = |still: usize| {
        if still % SIGNAL_EVERY == 1 {
            green
        } else {
            blue
        }
    };
    let rim = theme::signal();
    let step = (now - wall.last_time.unwrap_or(now)).clamp(0.0, f64::from(motion::LONGEST_STEP));
    wall.last_time = Some(now);
    let mut hovered = None;
    // Every loaded still, in view or not: started on first sight, this would keep asking for uncapped
    // frames for as long as new stills scroll in, and fade them in at the window's edge.
    let arrivals: Vec<f32> = (0..STILLS.len())
        .map(|still| {
            if wall.loader.texture(still).is_some() {
                motion::appear(&ctx, Id::new((STILL_ARRIVAL, still)), true, motion::SURFACE)
            } else {
                0.0
            }
        })
        .collect();
    for column in 0..columns {
        let offset = *wall
            .offsets
            .entry(column)
            .or_insert_with(|| start_offset(column, STILLS.len()));
        let x_px = ((area.left() + GAP + column as f32 * COLUMN_WIDTH) * scale).round();
        let shift = offset as f32;
        let first = ((area.top() + shift) / ROW_HEIGHT).floor() as i64 - 1;
        let last = ((area.bottom() + shift) / ROW_HEIGHT).ceil() as i64;
        for row in first..=last {
            let y_px = ((row as f32 * ROW_HEIGHT - shift) * scale).round();
            let rect = Rect::from_min_size(pos2(x_px / scale, y_px / scale), vec2(tile_w, tile_h));
            if !rect.intersects(area) {
                continue;
            }
            let still = still_index(column, row, STILLS.len());
            wall.visible.insert(still);
            if let Some(at) = pointer
                && rect.contains(at)
            {
                hovered = Some(((column, row), still, rect, at));
            }
            let Some(texture) = wall.loader.texture(still) else {
                continue;
            };
            let shown = arrivals[still];
            let backing = tile_mesh(rect, egui::TextureId::default(), Color32::BLACK, &|at| {
                fade(at) * shown
            });
            painter.add(egui::Shape::mesh(backing));
            let mesh = match wall.reveals.get(&(column, row)) {
                Some(reveal) => tile_mesh(rect, reveal.texture().id(), Color32::WHITE, &|at| {
                    fade(at) * shown
                }),
                None => tile_mesh(
                    rect,
                    texture.id(),
                    tint_of(still).gamma_multiply(DOTS),
                    &|at| fade(at) * shown,
                ),
            };
            painter.add(egui::Shape::mesh(mesh));
            painter.rect_stroke(
                rect,
                0,
                egui::Stroke::new(
                    1.0,
                    theme::rule().gamma_multiply(fade(rect.center()) * shown),
                ),
                egui::StrokeKind::Outside,
            );
        }
    }
    wall.hovered = hovered.map(|(key, ..)| key);
    for reveal in wall.reveals.values_mut() {
        reveal.pointer = None;
    }
    if let Some((key, still, rect, at)) = hovered
        && wall.loader.prepared(still).is_some()
    {
        let reveal = wall
            .reveals
            .entry(key)
            .or_insert_with(|| Reveal::new(&ctx, still, grid));
        let cell_points = grid.cell as f32 / scale;
        reveal.pointer = Some((
            (at.x - rect.left()) / cell_points,
            (at.y - rect.top()) / cell_points,
        ));
        if pressed && !gently {
            reveal.pulse();
        }
    }
    let reach = REVEAL_REACH * scale / grid.cell as f32;
    let loader = &wall.loader;
    wall.reveals.retain(|_, reveal| {
        loader.prepared(reveal.still).is_some_and(|prepared| {
            let dot = tint_of(reveal.still).lerp_to_gamma(Color32::BLACK, 1.0 - DOTS);
            reveal.step(step as f32, reach, gently, prepared, dot, rim)
        })
    });
    let hovered_column = wall.hovered.map(|(column, _)| column);
    for column in 0..columns {
        let moving = drifting && hovered_column != Some(column);
        let pace = motion::appear(&ctx, Id::new((COLUMN_PACE, column)), moving, motion::REFLOW);
        let offset = wall
            .offsets
            .entry(column)
            .or_insert_with(|| start_offset(column, STILLS.len()));
        *offset += f64::from(column_speed(column)) * f64::from(pace) * step;
    }
    // egui takes one predicted frame off any requested delay, and a delay of zero makes the window
    // loop redraw at once, so the frame it should take goes on top of that prediction. The surface
    // takes the first present mode the driver lists, which is not always vsynced, so an uncapped
    // request here could spin the GPU.
    let predicted = ctx.input(|input| input.predicted_dt);
    if settling {
        ctx.request_repaint_after(Duration::from_secs_f32(LOADING_POLL + predicted));
    }
    if drifting || !wall.reveals.is_empty() {
        ctx.request_repaint_after(Duration::from_secs_f32(DRIFT_FRAME + predicted));
    }
}

#[cfg(test)]
pub(crate) fn hovered(ctx: &Context) -> Option<(i32, i64)> {
    let grid = Grid::for_tile(TILE, CELL_POINTS, ctx.pixels_per_point());
    state(ctx, grid).lock().ok()?.hovered
}

#[cfg(test)]
pub(crate) fn has_state(ctx: &Context) -> bool {
    ctx.data(|data| {
        data.get_temp::<Arc<Mutex<WallState>>>(Id::new(STATE))
            .is_some()
    })
}

/// Solid stills ready at once, so a test neither waits on decoding nor races it.
#[cfg(test)]
pub(crate) fn install_loaded(ctx: &Context) {
    let grid = Grid::for_tile(TILE, CELL_POINTS, ctx.pixels_per_point());
    let textures = STILLS
        .iter()
        .map(|still| {
            ctx.load_texture(
                still.name,
                egui::ColorImage::filled([2, 2], Color32::WHITE),
                egui::TextureOptions::NEAREST,
            )
        })
        .collect();
    let state = Arc::new(Mutex::new(WallState::new(
        Loader::loaded(grid, textures),
        grid,
    )));
    ctx.data_mut(|data| data.insert_temp(Id::new(STATE), state));
}

#[cfg(test)]
mod tests {
    use egui::{Event, RawInput, ViewportId};

    use super::*;

    const WINDOW: egui::Vec2 = vec2(1300.0, 780.0);
    const SETTLED: u32 = 120;

    fn area() -> Rect {
        Rect::from_min_max(pos2(468.0, 0.0), WINDOW.to_pos2())
    }

    struct Run {
        ctx: Context,
        frame: u32,
    }

    impl Run {
        fn new(less_motion: Option<bool>) -> Self {
            let ctx = Context::default();
            install_loaded(&ctx);
            if let Some(less) = less_motion {
                motion::set_less_motion(&ctx, less);
            }
            Self { ctx, frame: 0 }
        }

        fn step(&mut self, events: Vec<Event>, focused: bool) -> egui::FullOutput {
            self.frame += 1;
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, WINDOW)),
                time: Some(f64::from(self.frame) / 60.0),
                focused,
                events,
                ..Default::default()
            };
            let mut output = self.ctx.run_ui(raw, |ui| draw(ui, area()));
            output.textures_delta.clear();
            output
        }

        fn settle(&mut self, events: &[Event], focused: bool) -> egui::FullOutput {
            let mut output = self.step(events.to_vec(), focused);
            for _ in 0..SETTLED {
                output = self.step(events.to_vec(), focused);
            }
            output
        }

        fn offsets(&self) -> HashMap<i32, f64> {
            let grid = Grid::for_tile(TILE, CELL_POINTS, self.ctx.pixels_per_point());
            state(&self.ctx, grid).lock().unwrap().offsets.clone()
        }

        fn reveals(&self) -> usize {
            let grid = Grid::for_tile(TILE, CELL_POINTS, self.ctx.pixels_per_point());
            state(&self.ctx, grid).lock().unwrap().reveals.len()
        }
    }

    fn delay(output: &egui::FullOutput) -> Duration {
        output.viewport_output[&ViewportId::ROOT].repaint_delay
    }

    fn on_a_tile() -> Pos2 {
        area().center()
    }

    #[test]
    fn motion_stays_off_until_the_preference_is_known() {
        let mut run = Run::new(None);
        run.settle(&[], true);
        let before = run.offsets();
        run.settle(&[], true);
        assert_eq!(run.offsets(), before, "drifted before anyone said it could");
    }

    #[test]
    fn steady_drift_asks_for_about_sixty_frames_a_second() {
        let mut run = Run::new(Some(false));
        let output = run.settle(&[], true);
        let wait = delay(&output);
        assert!(wait <= Duration::from_millis(40), "{wait:?}");
        let before = run.offsets();
        run.settle(&[], true);
        assert_ne!(run.offsets(), before);
    }

    #[test]
    fn losing_focus_settles_the_wall_and_asks_for_no_frames() {
        let mut run = Run::new(Some(false));
        run.settle(&[], true);
        let output = run.settle(&[], false);
        assert_eq!(delay(&output), Duration::MAX);
    }

    #[test]
    fn hovering_a_tile_pauses_its_column_and_reveals_it() {
        let mut run = Run::new(Some(false));
        run.settle(&[], true);
        let moved = [Event::PointerMoved(on_a_tile())];
        run.settle(&moved, true);
        let hovered = hovered(&run.ctx).expect("a tile sits under the pointer");
        assert_eq!(run.reveals(), 1);
        let before = run.offsets();
        run.settle(&moved, true);
        let after = run.offsets();
        assert_eq!(before[&hovered.0], after[&hovered.0], "its column paused");
        let other = (0..3).find(|column| *column != hovered.0).unwrap();
        assert_ne!(before[&other], after[&other], "the others kept drifting");
    }

    #[test]
    fn a_reveal_ends_a_while_after_the_pointer_leaves() {
        let mut run = Run::new(Some(false));
        run.settle(&[Event::PointerMoved(on_a_tile())], true);
        run.step(vec![Event::PointerGone], true);
        for _ in 0..10 {
            run.step(vec![], true);
        }
        assert_eq!(run.reveals(), 1, "it lingers");
        run.settle(&[], true);
        assert_eq!(run.reveals(), 0, "and then it has knitted back");
    }

    #[test]
    fn asked_for_less_motion_the_wall_holds_still_and_a_tile_just_shows_on_hover() {
        let mut run = Run::new(Some(true));
        run.settle(&[], true);
        let before = run.offsets();
        run.settle(&[Event::PointerMoved(on_a_tile())], true);
        assert_eq!(run.offsets(), before);
        assert_eq!(run.reveals(), 1);
        run.step(vec![Event::PointerGone], true);
        run.step(vec![], true);
        assert_eq!(run.reveals(), 0, "it goes as soon as the pointer does");
    }

    #[test]
    fn a_long_gap_between_frames_moves_the_wall_by_one_short_step() {
        let mut run = Run::new(Some(false));
        run.settle(&[], true);
        let before = run.offsets();
        run.frame += 600;
        run.step(vec![], true);
        let after = run.offsets();
        let moved = (after[&0] - before[&0]).abs();
        assert!(
            moved <= f64::from(column_speed(0).abs() * motion::LONGEST_STEP) + 0.001,
            "{moved}"
        );
    }

    #[test]
    fn opening_a_room_mid_load_releases_cleanly_and_a_later_draw_starts_fresh() {
        let mut run = Run::new(Some(false));
        run.step(vec![], true);
        assert!(has_state(&run.ctx));
        release(&run.ctx);
        assert!(!has_state(&run.ctx));
        run.step(vec![], true);
        assert!(has_state(&run.ctx));
    }

    #[test]
    fn tile_meshes_are_clipped_to_the_area() {
        let mut run = Run::new(Some(false));
        let output = run.settle(&[], true);
        let clips: Vec<Rect> = output
            .shapes
            .iter()
            .filter(|clipped| matches!(clipped.shape, egui::Shape::Mesh(_)))
            .map(|clipped| clipped.clip_rect)
            .collect();
        assert!(!clips.is_empty());
        assert!(clips.iter().all(|clip| *clip == area()));
    }
}

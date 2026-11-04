//! The start screen's wall: still frames of shared screens drifting past in perspective.

pub mod geometry;
pub mod stills;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use egui::{Context, Id, Rect, Sense, Shape, Ui};
use geometry::{
    Cell, LIFT, Pose, REST_BRIGHTNESS, TILT_PER_OFFSET_DEG, cell_at, column_speed, start_offset,
    still_index, tile_mesh, visible_cells,
};
use stills::{Loader, STILLS};

use super::motion;

const STATE: &str = "wall-state";
const MOTION_ALLOWED: &str = "wall-motion-allowed";
const HOVER: &str = "wall-hover";
const TILT_X: &str = "wall-tilt-x";
const TILT_Y: &str = "wall-tilt-y";
const COLUMN_PACE: &str = "wall-column-pace";
const TILE_LIFT: &str = "wall-tile-lift";
const STILL_ARRIVAL: &str = "wall-still-arrival";

const LOADING_POLL: f32 = 0.05;
const DRIFT_FRAME: f32 = 1.0 / 60.0;

struct WallState {
    loader: Loader,
    offsets: HashMap<i32, f64>,
    last_time: Option<f64>,
    hovered: Option<(i32, i64)>,
    lifting: Vec<(i32, i64)>,
}

impl WallState {
    fn new(loader: Loader) -> Self {
        Self {
            loader,
            offsets: HashMap::new(),
            last_time: None,
            hovered: None,
            lifting: Vec::new(),
        }
    }
}

fn state(ctx: &Context) -> Arc<Mutex<WallState>> {
    ctx.data_mut(|data| {
        data.get_temp_mut_or_insert_with(Id::new(STATE), || {
            Arc::new(Mutex::new(WallState::new(Loader::start())))
        })
        .clone()
    })
}

/// Says whether the wall may move. Until this is called it stays still, so nobody who asked their
/// system for less motion sees a burst of it while the answer is on its way.
pub fn allow_motion(ctx: &Context, allowed: bool) {
    ctx.data_mut(|data| data.insert_temp(Id::new(MOTION_ALLOWED), allowed));
}

/// Frees the stills, for when the start screen will not be drawn again.
pub fn release(ctx: &Context) {
    ctx.data_mut(|data| data.remove::<Arc<Mutex<WallState>>>(Id::new(STATE)));
}

/// The degrees the wall leans toward the pointer, added to its resting yaw and pitch.
fn tilt_target(area: Rect, pointer: Option<egui::Pos2>, allowed: bool) -> (f32, f32) {
    pointer.filter(|_| allowed).map_or((0.0, 0.0), |pointer| {
        let across = (pointer.x - area.left()) / area.width() - 0.5;
        let down = (pointer.y - area.top()) / area.height() - 0.5;
        (across * TILT_PER_OFFSET_DEG, -down * TILT_PER_OFFSET_DEG)
    })
}

/// Draws the wall into `area`. Call it before anything that should sit on top of it.
pub fn draw(ui: &mut Ui, area: Rect) {
    let ctx = ui.ctx().clone();
    let state = state(&ctx);
    let Ok(mut wall) = state.lock() else {
        return;
    };
    let wall = &mut *wall;
    let loading = wall.loader.poll(&ctx);
    let allowed = ctx
        .data(|data| data.get_temp::<bool>(Id::new(MOTION_ALLOWED)))
        .unwrap_or(false);
    let drifting = allowed && ctx.input(|input| input.focused);
    let pointer = ui
        .interact(area, Id::new(HOVER), Sense::hover())
        .hover_pos();

    let (tilt_x, tilt_y) = tilt_target(area, pointer, allowed);
    let pose = Pose {
        center: area.center(),
        tilt_x_deg: motion::glide(&ctx, Id::new(TILT_X), tilt_x, motion::REFLOW),
        tilt_y_deg: motion::glide(&ctx, Id::new(TILT_Y), tilt_y, motion::REFLOW),
    };

    let offsets = &wall.offsets;
    let offset_of = |column: i32| {
        offsets
            .get(&column)
            .copied()
            .unwrap_or_else(|| start_offset(column, STILLS.len()))
    };
    let hovered = pointer.map(|pointer| cell_at(&pose, pointer, &offset_of));
    let mut cells = visible_cells(&pose, area, &offset_of);
    let columns: Vec<i32> =
        cells
            .iter()
            .map(|cell| cell.column)
            .fold(Vec::new(), |mut columns, column| {
                if columns.last() != Some(&column) {
                    columns.push(column);
                }
                columns
            });

    if let Some(cell) = hovered
        && !wall.lifting.contains(&cell)
    {
        wall.lifting.push(cell);
    }
    // Asked only for tiles in motion: rows keep changing as columns drift, and a spring per tile
    // would grow egui's temporary memory for as long as the wall runs.
    let lifts: Vec<((i32, i64), f32)> = wall
        .lifting
        .iter()
        .map(|&cell| {
            let lift = motion::appear(
                &ctx,
                Id::new((TILE_LIFT, cell)),
                hovered == Some(cell),
                motion::REFLOW,
            );
            (cell, lift)
        })
        .collect();
    wall.lifting = lifts
        .iter()
        .filter(|(cell, lift)| *lift > 0.0 || hovered == Some(*cell))
        .map(|(cell, _)| *cell)
        .collect();
    let lift_of = |cell: &Cell| {
        lifts
            .iter()
            .find(|(key, _)| *key == (cell.column, cell.row))
            .map(|(_, lift)| *lift)
    };

    cells.sort_by_key(|cell| {
        if hovered == Some((cell.column, cell.row)) {
            2
        } else {
            u8::from(lift_of(cell).is_some())
        }
    });
    let painter = ui.painter().with_clip_rect(area);
    for cell in &cells {
        let still = still_index(cell.column, cell.row, STILLS.len());
        let Some(texture) = wall.loader.texture(still) else {
            continue;
        };
        let lift = lift_of(cell).unwrap_or(0.0);
        let arrival = motion::appear(&ctx, Id::new((STILL_ARRIVAL, still)), true, motion::SURFACE);
        let brightness = egui::lerp(REST_BRIGHTNESS..=1.0, lift) * arrival;
        let lift_z = if allowed { LIFT * lift } else { 0.0 };
        painter.add(Shape::mesh(tile_mesh(
            &pose,
            cell,
            lift_z,
            brightness,
            area,
            texture.id(),
        )));
    }

    let now = ctx.input(|input| input.time);
    let step = (now - wall.last_time.unwrap_or(now)).clamp(0.0, f64::from(motion::LONGEST_STEP));
    wall.last_time = Some(now);
    for column in columns {
        let moving = drifting && hovered.map(|(hovered, _)| hovered) != Some(column);
        let pace = motion::appear(&ctx, Id::new((COLUMN_PACE, column)), moving, motion::REFLOW);
        let offset = wall
            .offsets
            .entry(column)
            .or_insert_with(|| start_offset(column, STILLS.len()));
        *offset += f64::from(column_speed(column) * pace) * step;
    }
    wall.hovered = hovered;

    // egui takes one predicted frame off any requested delay, and a delay of zero makes the window
    // loop redraw at once, so the frame it should take goes on top of that prediction. The surface
    // takes the first present mode the driver lists, which is not always vsynced, so an uncapped
    // request here could spin the GPU.
    let predicted = ctx.input(|input| input.predicted_dt);
    if loading {
        ctx.request_repaint_after(Duration::from_secs_f32(LOADING_POLL + predicted));
    }
    if drifting {
        ctx.request_repaint_after(Duration::from_secs_f32(DRIFT_FRAME + predicted));
    }
}

#[cfg(test)]
pub(crate) fn hovered(ctx: &Context) -> Option<(i32, i64)> {
    state(ctx).lock().ok()?.hovered
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
    let textures = STILLS
        .iter()
        .map(|still| {
            ctx.load_texture(
                still.name,
                egui::ColorImage::filled([2, 2], egui::Color32::WHITE),
                egui::TextureOptions::LINEAR,
            )
        })
        .collect();
    let state = Arc::new(Mutex::new(WallState::new(Loader::loaded(textures))));
    ctx.data_mut(|data| data.insert_temp(Id::new(STATE), state));
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use egui::{Event, Pos2, RawInput, ViewportId, pos2, vec2};

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
        fn new() -> Self {
            let ctx = Context::default();
            install_loaded(&ctx);
            Self { ctx, frame: 0 }
        }

        fn raw(&mut self, events: Vec<Event>, focused: bool) -> RawInput {
            self.frame += 1;
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, WINDOW)),
                time: Some(f64::from(self.frame) / 60.0),
                focused,
                events,
                ..Default::default()
            }
        }

        fn pass(&mut self, raw: RawInput) -> egui::FullOutput {
            let mut output = self.ctx.run_ui(raw, |ui| draw(ui, area()));
            output.textures_delta.clear();
            output
        }

        fn step(&mut self, events: Vec<Event>, focused: bool) -> egui::FullOutput {
            let raw = self.raw(events, focused);
            self.pass(raw)
        }

        fn settle(&mut self, events: &[Event], focused: bool) -> egui::FullOutput {
            let mut output = self.step(events.to_vec(), focused);
            for _ in 0..SETTLED {
                output = self.step(events.to_vec(), focused);
            }
            output
        }

        fn offsets(&self) -> HashMap<i32, f64> {
            state(&self.ctx).lock().unwrap().offsets.clone()
        }
    }

    fn meshes(output: &egui::FullOutput) -> Vec<std::sync::Arc<egui::Mesh>> {
        output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                Shape::Mesh(mesh) => Some(mesh.clone()),
                _ => None,
            })
            .collect()
    }

    fn delay(output: &egui::FullOutput) -> Duration {
        output.viewport_output[&ViewportId::ROOT].repaint_delay
    }

    fn top_edge(mesh: &egui::Mesh) -> f32 {
        let stride = geometry::GRID as usize;
        (mesh.vertices[stride].pos - mesh.vertices[0].pos).length()
    }

    /// The grid's middle vertex, well inside the fade at the centre of the wall.
    fn middle_alpha(mesh: &egui::Mesh) -> u8 {
        let stride = geometry::GRID as usize + 1;
        let middle = geometry::GRID as usize / 2;
        mesh.vertices[middle * stride + middle].color.a()
    }

    #[test]
    fn motion_stays_off_until_the_preference_is_known() {
        let mut run = Run::new();
        run.settle(&[], true);
        let before = run.offsets();
        assert!(!before.is_empty());
        run.settle(&[], true);
        assert_eq!(before, run.offsets());
    }

    #[test]
    fn steady_drift_asks_for_about_sixty_frames_a_second() {
        let mut run = Run::new();
        allow_motion(&run.ctx, true);
        let before = {
            run.settle(&[], true);
            run.offsets()
        };
        let output = run.settle(&[], true);
        assert_ne!(before, run.offsets());
        let delay = delay(&output);
        assert!(
            delay >= Duration::from_millis(15) && delay <= Duration::from_millis(18),
            "repaint delay is {delay:?}"
        );
    }

    #[test]
    fn losing_focus_settles_the_wall_and_asks_for_no_frames() {
        let mut run = Run::new();
        allow_motion(&run.ctx, true);
        run.settle(&[], true);
        run.settle(&[], false);
        let before = run.offsets();
        let output = run.step(Vec::new(), false);
        assert_eq!(before, run.offsets());
        assert_eq!(delay(&output), Duration::MAX);
    }

    #[test]
    fn reduced_motion_holds_the_wall_still_and_only_brightens_on_hover() {
        let mut run = Run::new();
        allow_motion(&run.ctx, false);
        let resting = run.settle(&[], true);
        let before = run.offsets();
        let hovered = run.settle(&[Event::PointerMoved(area().center())], true);
        assert_eq!(before, run.offsets());
        let lit = meshes(&hovered).last().unwrap().clone();
        let at_rest = meshes(&resting);
        let same_place = at_rest
            .iter()
            .find(|mesh| mesh.vertices[0].pos == lit.vertices[0].pos)
            .expect("neither the tilt nor a lift moved the hovered tile");
        assert_eq!(
            same_place.vertices.last().unwrap().pos,
            lit.vertices.last().unwrap().pos
        );
        assert_eq!(middle_alpha(&lit), 255);
        assert!(
            middle_alpha(same_place).abs_diff(140) <= 1,
            "55% of 255 is 140"
        );
    }

    #[test]
    fn hovering_a_tile_pauses_its_column_and_lifts_it_above_the_others() {
        let mut run = Run::new();
        allow_motion(&run.ctx, true);
        run.settle(&[], true);
        let pointer = [Event::PointerMoved(area().center())];
        let arriving = run.step(pointer.to_vec(), true);
        let before_lift = top_edge(meshes(&arriving).last().unwrap());
        let settled = run.settle(&pointer, true);
        let (column, _) = hovered(&run.ctx).expect("the pointer is over the wall");
        let before = run.offsets();
        run.step(pointer.to_vec(), true);
        let after = run.offsets();
        assert_eq!(
            before[&column], after[&column],
            "its column should be paused"
        );
        assert_ne!(
            before[&(column + 1)],
            after[&(column + 1)],
            "its neighbour should not be"
        );
        let lifted = meshes(&settled).last().unwrap().clone();
        assert_eq!(middle_alpha(&lifted), 255);
        // 64 points toward a viewer 1320 away is about 5% larger.
        assert!(
            top_edge(&lifted) > before_lift * 1.03,
            "{} did not lift from {before_lift}",
            top_edge(&lifted)
        );
    }

    #[test]
    fn a_long_gap_between_frames_moves_the_wall_by_one_short_step() {
        let mut run = Run::new();
        allow_motion(&run.ctx, true);
        run.settle(&[], true);
        let before = run.offsets();
        let mut raw = run.raw(Vec::new(), true);
        raw.time = raw.time.map(|time| time + 600.0);
        run.pass(raw);
        for (column, offset) in run.offsets() {
            let moved = (offset - before[&column]).abs();
            let longest = f64::from(motion::LONGEST_STEP * column_speed(column).abs());
            assert!(moved > 0.0, "column {column} never moved");
            assert!(moved <= longest + 0.01, "column {column} jumped {moved}");
        }
    }

    #[test]
    fn the_pointer_leaving_the_window_lets_lift_and_tilt_settle() {
        let mut run = Run::new();
        allow_motion(&run.ctx, true);
        run.settle(
            &[Event::PointerMoved(area().right_bottom() - vec2(5.0, 5.0))],
            true,
        );
        assert!(hovered(&run.ctx).is_some());
        run.settle(&[Event::PointerGone], true);
        assert_eq!(hovered(&run.ctx), None);
        assert!(state(&run.ctx).lock().unwrap().lifting.is_empty());
        let tilt = (
            motion::glide(&run.ctx, Id::new(TILT_X), 0.0, motion::REFLOW),
            motion::glide(&run.ctx, Id::new(TILT_Y), 0.0, motion::REFLOW),
        );
        assert_eq!(tilt, (0.0, 0.0));
        run.settle(&[], false);
        assert_eq!(delay(&run.step(Vec::new(), false)), Duration::MAX);
    }

    #[test]
    fn the_tilt_peaks_at_the_mockups_angle_at_a_corner() {
        let corner = Some(area().right_bottom());
        // A corner is 0.5 from the centre on each axis, and right and low lean opposite ways.
        assert_eq!(tilt_target(area(), corner, true), (2.4, -2.4));
        assert_eq!(
            tilt_target(area(), Some(area().center()), true),
            (0.0, -0.0)
        );
        assert_eq!(tilt_target(area(), None, true), (0.0, 0.0));
        assert_eq!(tilt_target(area(), corner, false), (0.0, 0.0));
    }

    #[test]
    fn the_wall_leans_toward_the_pointer_when_motion_is_allowed() {
        let mut run = Run::new();
        allow_motion(&run.ctx, true);
        // Unfocused, so the drift is not what moves anything here.
        let resting = run.settle(&[], false);
        let corner = area().right_bottom() - vec2(5.0, 5.0);
        let leaning = run.settle(&[Event::PointerMoved(corner)], false);
        let first = meshes(&resting)[0].vertices[0].pos;
        assert!(
            !meshes(&leaning)
                .iter()
                .any(|mesh| mesh.vertices[0].pos == first),
            "nothing moved from {first:?}"
        );
    }

    #[test]
    fn opening_a_room_mid_load_releases_cleanly_and_a_later_draw_starts_fresh() {
        let ctx = Context::default();
        let pass = |ctx: &Context| {
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, WINDOW)),
                ..Default::default()
            };
            let mut output = ctx.run_ui(raw, |ui| draw(ui, area()));
            output.textures_delta.clear();
        };
        pass(&ctx);
        state(&ctx).lock().unwrap().offsets.insert(999, 1.0);
        release(&ctx);
        assert!(!has_state(&ctx));
        pass(&ctx);
        assert!(has_state(&ctx));
        assert!(!state(&ctx).lock().unwrap().offsets.contains_key(&999));
    }

    #[test]
    fn stills_keep_arriving_while_the_wall_stands_still() {
        let ctx = Context::default();
        allow_motion(&ctx, false);
        let mut output = egui::FullOutput::default();
        // egui asks for a frame of its own after the first, so this reads the wall's request. Decoding
        // every still takes far longer than these passes.
        for _ in 0..3 {
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, WINDOW)),
                ..Default::default()
            };
            output = ctx.run_ui(raw, |ui| draw(ui, area()));
            output.textures_delta.clear();
        }
        let delay = delay(&output);
        assert!(
            delay >= Duration::from_millis(40) && delay <= Duration::from_millis(60),
            "repaint delay is {delay:?}"
        );
    }

    #[test]
    fn tile_meshes_are_clipped_to_the_area() {
        let mut run = Run::new();
        let output = run.settle(&[], true);
        let clips: Vec<Rect> = output
            .shapes
            .iter()
            .filter(|clipped| matches!(clipped.shape, Shape::Mesh(_)))
            .map(|clipped| clipped.clip_rect)
            .collect();
        assert!(!clips.is_empty());
        assert!(clips.iter().all(|clip| *clip == area()), "{clips:?}");
    }
}

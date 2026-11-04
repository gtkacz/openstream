//! The wall's perspective and layout, free of egui's frame state so it tests without a window.

use egui::epaint::{Mesh, Vertex};
use egui::{Color32, Pos2, Rect, TextureId, pos2, vec2};

use super::stills::STRANDS;

pub const TILE_WIDTH: f32 = 200.0;
/// Shared screens are 16:9.
pub const TILE_HEIGHT: f32 = 112.5;
pub const GAP: f32 = 18.0;
pub const COLUMN_WIDTH: f32 = TILE_WIDTH + GAP;
pub const ROW_HEIGHT: f32 = TILE_HEIGHT + GAP;
pub const SCALE: f32 = 1.18;
pub const PITCH_DEG: f32 = 16.0;
pub const YAW_DEG: f32 = -14.0;
pub const PERSPECTIVE: f32 = 1200.0;
pub const DEPTH: f32 = 120.0;
pub const BASE_SPEED: f32 = 42.0;
pub const SPEED_SPREAD: f32 = 0.45;
pub const LIFT: f32 = 64.0;
pub const REST_BRIGHTNESS: f32 = 0.55;
pub const FADE_SPAN: f32 = 0.26;
/// The pointer's offset across the wall runs from -0.5 to 0.5, so the tilt peaks at half of this.
pub const TILT_PER_OFFSET_DEG: f32 = 4.8;
/// Enough that egui's affine texture mapping stays under a tenth of a point from true perspective.
pub const GRID: u32 = 6;

/// How the wall is held: the rectangle's centre, and the pointer's tilt on top of the resting angle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub center: Pos2,
    pub tilt_x_deg: f32,
    pub tilt_y_deg: f32,
}

impl Pose {
    fn angles(&self) -> (f32, f32) {
        (
            (PITCH_DEG + self.tilt_y_deg).to_radians(),
            (YAW_DEG + self.tilt_x_deg).to_radians(),
        )
    }

    /// Where a point on the plane lands on screen, in the order CSS applies
    /// `scale() rotateX() rotateY() translateZ()` to it. `z` is toward the viewer.
    pub fn project(&self, x: f32, y: f32, z: f32) -> Pos2 {
        let (pitch, yaw) = self.angles();
        let (sin_a, cos_a) = pitch.sin_cos();
        let (sin_b, cos_b) = yaw.sin_cos();
        let z0 = z - DEPTH;
        let x1 = x * cos_b + z0 * sin_b;
        let z1 = -x * sin_b + z0 * cos_b;
        let y2 = y * cos_a - z1 * sin_a;
        let z2 = y * sin_a + z1 * cos_a;
        let k = PERSPECTIVE / (PERSPECTIVE - z2);
        self.center + vec2(SCALE * x1 * k, SCALE * y2 * k)
    }

    /// The point on the plane at `z = 0` that lands on `screen`. Both of [`Pose::project`]'s screen
    /// equations are linear in `x` and `y` once multiplied through by the perspective divisor.
    pub fn unproject(&self, screen: Pos2) -> (f32, f32) {
        let (pitch, yaw) = self.angles();
        let (sin_a, cos_a) = pitch.sin_cos();
        let (sin_b, cos_b) = yaw.sin_cos();
        let (sx, sy) = (screen.x - self.center.x, screen.y - self.center.y);
        let reach = SCALE * PERSPECTIVE;
        let z0 = -DEPTH;
        let a11 = sx * sin_b * cos_a - reach * cos_b;
        let a12 = -sx * sin_a;
        let b1 = reach * z0 * sin_b - sx * PERSPECTIVE + sx * z0 * cos_b * cos_a;
        let a21 = sy * sin_b * cos_a - reach * sin_b * sin_a;
        let a22 = -sy * sin_a - reach * cos_a;
        let b2 = -reach * z0 * cos_b * sin_a - sy * PERSPECTIVE + sy * z0 * cos_b * cos_a;
        let determinant = a11 * a22 - a12 * a21;
        (
            (b1 * a22 - a12 * b2) / determinant,
            (a11 * b2 - b1 * a21) / determinant,
        )
    }
}

/// One slot of the wall, placed on the plane by its top-left corner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cell {
    pub column: i32,
    pub row: i64,
    pub x: f32,
    pub y: f32,
}

/// Signed points per second, positive moving upward. Neighbouring columns run opposite ways.
pub fn column_speed(column: i32) -> f32 {
    let spread = (f64::from(column) * 0.618_033_988_7 + 0.35).rem_euclid(1.0) as f32 * 2.0 - 1.0;
    let direction = if column.rem_euclid(2) == 0 { 1.0 } else { -1.0 };
    BASE_SPEED * (1.0 + SPEED_SPREAD * spread) * direction
}

fn strand_of(column: i32) -> usize {
    column.rem_euclid(STRANDS as i32) as usize
}

fn strand_len(strand: usize, stills: usize) -> usize {
    ((stills + STRANDS - 1 - strand) / STRANDS).max(1)
}

/// Where a column starts, so the wall opens staggered rather than in rows.
pub fn start_offset(column: i32, stills: usize) -> f64 {
    let repeat = strand_len(strand_of(column), stills) as f64 * f64::from(ROW_HEIGHT);
    (f64::from(column) * 0.37).rem_euclid(1.0) * repeat
}

/// Which still a slot shows. It repeats through `rem_euclid` rather than by wrapping the offset, so
/// a tile's row stays the same however far its column drifts, and a lifted tile never loses it.
pub fn still_index(column: i32, row: i64, stills: usize) -> usize {
    let strand = strand_of(column);
    strand + STRANDS * row.rem_euclid(strand_len(strand, stills) as i64) as usize
}

fn column_of(x: f32) -> i32 {
    (x / COLUMN_WIDTH + 0.5).floor() as i32
}

fn row_of(y: f32, offset: f64) -> i64 {
    ((f64::from(y) + offset) / f64::from(ROW_HEIGHT)).floor() as i64
}

/// The cell under a screen point at rest. Cells touch, so every point has one.
pub fn cell_at(pose: &Pose, screen: Pos2, offset: &dyn Fn(i32) -> f64) -> (i32, i64) {
    let (x, y) = pose.unproject(screen);
    let column = column_of(x);
    (column, row_of(y, offset(column)))
}

/// Every cell reaching `area`. The plane's unbounded, so only these exist as far as drawing goes,
/// and any window size is covered. A rectangle stays convex through the projection, so its corners
/// bound it.
pub fn visible_cells(pose: &Pose, area: Rect, offset: &dyn Fn(i32) -> f64) -> Vec<Cell> {
    let corners = [
        area.left_top(),
        area.right_top(),
        area.left_bottom(),
        area.right_bottom(),
    ]
    .map(|corner| pose.unproject(corner));
    let bound = |pick: fn(&(f32, f32)) -> f32, choose: fn(f32, f32) -> f32| {
        corners.iter().map(pick).reduce(choose).unwrap_or_default()
    };
    let (left, right) = (bound(|c| c.0, f32::min), bound(|c| c.0, f32::max));
    let (top, bottom) = (bound(|c| c.1, f32::min), bound(|c| c.1, f32::max));
    let mut cells = Vec::new();
    for column in column_of(left)..=column_of(right) {
        let offset = offset(column);
        for row in row_of(top, offset)..=row_of(bottom, offset) {
            cells.push(Cell {
                column,
                row,
                x: (column as f32 - 0.5) * COLUMN_WIDTH,
                y: (row as f64 * f64::from(ROW_HEIGHT) - offset) as f32,
            });
        }
    }
    cells
}

/// How much of the wall shows at `x`: nothing at its left edge, all of it past `FADE_SPAN`.
pub fn fade_at(area: Rect, x: f32) -> f32 {
    ((x - area.left()) / (area.width() * FADE_SPAN)).clamp(0.0, 1.0)
}

/// A tile as a grid of small quads: egui maps a texture linearly across a triangle, so a tile drawn
/// as two triangles would show a seam along its diagonal.
pub fn tile_mesh(
    pose: &Pose,
    cell: &Cell,
    lift_z: f32,
    brightness: f32,
    area: Rect,
    texture: TextureId,
) -> Mesh {
    let (left, top) = (cell.x + GAP / 2.0, cell.y + GAP / 2.0);
    let stride = GRID + 1;
    let mut mesh = Mesh::with_texture(texture);
    mesh.reserve_vertices((stride * stride) as usize);
    mesh.reserve_triangles((2 * GRID * GRID) as usize);
    for j in 0..=GRID {
        for i in 0..=GRID {
            let (u, v) = (i as f32 / GRID as f32, j as f32 / GRID as f32);
            let pos = pose.project(left + u * TILE_WIDTH, top + v * TILE_HEIGHT, lift_z);
            mesh.vertices.push(Vertex {
                pos,
                uv: pos2(u, v),
                color: Color32::WHITE.gamma_multiply(brightness * fade_at(area, pos.x)),
            });
        }
    }
    for j in 0..GRID {
        for i in 0..GRID {
            let top_left = j * stride + i;
            let top_right = top_left + 1;
            let bottom_left = top_left + stride;
            mesh.add_triangle(top_left, top_right, bottom_left);
            mesh.add_triangle(top_right, bottom_left + 1, bottom_left);
        }
    }
    mesh
}

#[cfg(test)]
mod tests {
    use egui::{Pos2, Rect, pos2, vec2};

    use super::*;

    const STILLS: usize = 42;
    /// Six stills per strand at 130.5 points each.
    const REPEAT: f64 = 783.0;

    fn rest() -> Pose {
        Pose {
            center: Pos2::ZERO,
            tilt_x_deg: 0.0,
            tilt_y_deg: 0.0,
        }
    }

    fn tilted(center: Pos2, tilt_x_deg: f32, tilt_y_deg: f32) -> Pose {
        Pose {
            center,
            tilt_x_deg,
            tilt_y_deg,
        }
    }

    fn tilt_extremes(center: Pos2) -> [Pose; 4] {
        [
            tilted(center, 2.4, 2.4),
            tilted(center, 2.4, -2.4),
            tilted(center, -2.4, 2.4),
            tilted(center, -2.4, -2.4),
        ]
    }

    fn assert_near(actual: Pos2, x: f32, y: f32, tolerance: f32) {
        assert!(
            (actual.x - x).abs() <= tolerance && (actual.y - y).abs() <= tolerance,
            "{actual:?} is not within {tolerance} of ({x}, {y})"
        );
    }

    #[test]
    fn projection_matches_the_mockups_transform() {
        // Worked by hand from the CSS transform order in the design.
        assert_near(rest().project(0.0, 0.0, 0.0), 31.33, 34.64, 0.01);
        assert_near(rest().project(100.0, 50.0, 0.0), 140.01, 81.62, 0.01);
        assert_near(rest().project(0.0, 0.0, 64.0), 15.32, 16.94, 0.01);
        let flat = tilted(Pos2::ZERO, 14.0, -16.0);
        assert_near(flat.project(100.0, 0.0, 0.0), 107.27, 0.0, 0.01);
        let moved = tilted(pos2(500.0, 400.0), 0.0, 0.0);
        assert_near(moved.project(0.0, 0.0, 0.0), 531.33, 434.64, 0.01);
    }

    #[test]
    fn unprojecting_a_projected_point_returns_it() {
        for pose in tilt_extremes(pos2(300.0, 200.0)) {
            for x in (-1500..=1500).step_by(300) {
                for y in (-1500..=1500).step_by(300) {
                    let (x, y) = (x as f32, y as f32);
                    let (back_x, back_y) = pose.unproject(pose.project(x, y, 0.0));
                    assert!(
                        (back_x - x).abs() <= 0.05 && (back_y - y).abs() <= 0.05,
                        "({x}, {y}) came back as ({back_x}, {back_y}) under {pose:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn columns_alternate_direction_at_their_own_speeds() {
        for (column, speed) in [(0, 36.33), (1, -59.69), (-1, -50.77)] {
            let actual = column_speed(column);
            assert!((actual - speed).abs() <= 0.01, "column {column}: {actual}");
        }
    }

    #[test]
    fn columns_start_staggered_by_a_fraction_of_their_repeat() {
        for (column, offset) in [(0, 0.0), (1, 289.71), (-1, 493.29)] {
            let actual = start_offset(column, STILLS);
            assert!((actual - offset).abs() <= 0.01, "column {column}: {actual}");
        }
    }

    #[test]
    fn each_column_walks_its_own_strand() {
        for (column, row, stills, index) in [
            (0, 0, 42, 0),
            (0, 1, 42, 7),
            (0, 5, 42, 35),
            (0, 6, 42, 0),
            (0, -1, 42, 35),
            (3, 2, 42, 17),
            (-1, 0, 42, 6),
            (7, 0, 42, 0),
            (0, 6, 44, 42),
            (2, 6, 44, 2),
        ] {
            assert_eq!(
                still_index(column, row, stills),
                index,
                "column {column}, row {row}, {stills} stills"
            );
        }
    }

    #[test]
    fn the_cell_under_the_pointer_includes_its_share_of_the_gaps() {
        let pose = rest();
        let at_rest = |_: i32| 0.0;
        let shifted = |_: i32| 100.0;
        let cell = |x: f32, y: f32, offset: &dyn Fn(i32) -> f64| {
            cell_at(&pose, pose.project(x, y, 0.0), offset)
        };
        // Column 0 is centred, and its tile ends 9 points before the column does.
        assert_eq!(cell(0.0, 65.0, &at_rest), (0, 0));
        assert_eq!(cell(108.0, 65.0, &at_rest), (0, 0));
        assert_eq!(cell(110.0, 65.0, &at_rest), (1, 0));
        assert_eq!(cell(-110.0, 65.0, &at_rest), (-1, 0));
        assert_eq!(cell(0.0, 130.0, &at_rest), (0, 0));
        assert_eq!(cell(0.0, 131.0, &at_rest), (0, 1));
        assert_eq!(cell(0.0, -1.0, &at_rest), (0, -1));
        assert_eq!(cell(0.0, 0.0, &shifted), (0, 0));
        assert_eq!(cell(0.0, 31.0, &shifted), (0, 1));
    }

    #[test]
    fn every_visible_point_lands_on_a_listed_cell() {
        for size in [
            vec2(492.0, 540.0),
            vec2(832.0, 780.0),
            vec2(1116.0, 1080.0),
            vec2(2076.0, 2160.0),
        ] {
            let area = Rect::from_min_size(pos2(468.0, 0.0), size);
            let offset = |column: i32| start_offset(column, STILLS);
            for pose in tilt_extremes(area.center()) {
                let cells = visible_cells(&pose, area, &offset);
                assert!(cells.len() < 1000, "{} cells at {size:?}", cells.len());
                for step_y in 0..20 {
                    for step_x in 0..20 {
                        let point = area.min
                            + vec2(size.x * step_x as f32 / 19.0, size.y * step_y as f32 / 19.0);
                        let (column, row) = cell_at(&pose, point, &offset);
                        assert!(
                            cells.iter().any(|c| c.column == column && c.row == row),
                            "no cell for {point:?} at {size:?} under {pose:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn crossing_a_repeat_boundary_keeps_the_same_still_in_place() {
        let area = Rect::from_min_size(pos2(468.0, 0.0), vec2(832.0, 780.0));
        let pose = tilted(area.center(), 0.0, 0.0);
        let drawn = |offset: f64| -> Vec<(f32, f32, usize)> {
            visible_cells(&pose, area, &|_| offset)
                .iter()
                .map(|c| (c.x, c.y, still_index(c.column, c.row, STILLS)))
                .collect()
        };
        for offset in [40.0, -40.0] {
            let before = drawn(offset);
            for after in [drawn(offset + REPEAT), drawn(offset - REPEAT)] {
                assert_eq!(before.len(), after.len());
                for (a, b) in before.iter().zip(&after) {
                    assert!((a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01);
                    assert_eq!(a.2, b.2);
                }
            }
        }
    }

    #[test]
    fn a_column_and_a_row_place_a_cell_with_column_zero_centred() {
        let area = Rect::from_min_size(pos2(-50.0, -50.0), vec2(100.0, 100.0));
        let cells = visible_cells(&tilted(Pos2::ZERO, 14.0, -16.0), area, &|_| 10.0);
        let cell = cells
            .iter()
            .find(|c| c.column == 0 && c.row == 0)
            .expect("the plane's centre is on screen");
        assert_eq!(cell.x, -109.0);
        assert_eq!(cell.y, -10.0);
    }

    #[test]
    fn the_fade_reaches_full_at_26_percent_of_the_width() {
        let area = Rect::from_min_size(pos2(100.0, 0.0), vec2(1000.0, 500.0));
        for (x, fade) in [
            (50.0, 0.0),
            (100.0, 0.0),
            (230.0, 0.5),
            (360.0, 1.0),
            (1000.0, 1.0),
        ] {
            assert!((fade_at(area, x) - fade).abs() < 0.001, "at {x}");
        }
    }

    #[test]
    fn a_tile_mesh_maps_its_texture_corners_to_the_inset_tile() {
        let pose = rest();
        let cell = Cell {
            column: 0,
            row: 0,
            x: -109.0,
            y: 0.0,
        };
        let area = Rect::from_min_size(pos2(-5000.0, -5000.0), vec2(10000.0, 10000.0));
        let mesh = tile_mesh(&pose, &cell, 64.0, 1.0, area, egui::TextureId::Managed(7));
        assert_eq!(mesh.texture_id, egui::TextureId::Managed(7));
        assert!(mesh.is_valid());
        assert_eq!(mesh.indices.len(), 216, "two triangles per grid cell");
        let at = |u: f32, v: f32| {
            mesh.vertices
                .iter()
                .find(|vertex| vertex.uv == pos2(u, v))
                .expect("a corner vertex")
                .pos
        };
        for (u, v, x, y) in [
            (0.0, 0.0, -100.0, 9.0),
            (1.0, 0.0, 100.0, 9.0),
            (0.0, 1.0, -100.0, 121.5),
            (1.0, 1.0, 100.0, 121.5),
        ] {
            let expected = pose.project(x, y, 64.0);
            assert_near(at(u, v), expected.x, expected.y, 0.001);
        }
    }

    #[test]
    fn brightness_and_the_fade_set_how_solid_a_vertex_is() {
        let pose = rest();
        let cell = Cell {
            column: 0,
            row: 0,
            x: -109.0,
            y: 0.0,
        };
        let texture = egui::TextureId::Managed(1);
        let everywhere = Rect::from_min_size(pos2(-5000.0, -5000.0), vec2(10000.0, 10000.0));
        let dimmed = tile_mesh(&pose, &cell, 0.0, 0.55, everywhere, texture);
        assert!(
            dimmed
                .vertices
                .iter()
                .all(|vertex| vertex.color.a().abs_diff(140) <= 1),
            "55% of 255 is 140"
        );

        let left = dimmed
            .vertices
            .iter()
            .map(|vertex| vertex.pos.x)
            .fold(f32::INFINITY, f32::min);
        let starting_at_the_tile = Rect::from_min_size(pos2(left, -5000.0), vec2(100.0, 10000.0));
        let faded = tile_mesh(&pose, &cell, 0.0, 1.0, starting_at_the_tile, texture);
        let at_left_edge = faded
            .vertices
            .iter()
            .min_by(|a, b| a.pos.x.total_cmp(&b.pos.x))
            .unwrap();
        let far_right = faded
            .vertices
            .iter()
            .max_by(|a, b| a.pos.x.total_cmp(&b.pos.x))
            .unwrap();
        assert_eq!(at_left_edge.color.a(), 0);
        assert_eq!(far_right.color.a(), 255);
    }

    #[test]
    fn the_mesh_stays_within_a_quarter_point_of_true_perspective() {
        let area = Rect::from_min_size(pos2(1764.0, 0.0), vec2(2076.0, 2160.0));
        let stride = GRID as usize + 1;
        let cells_in_grid = GRID as f32;
        let texture = egui::TextureId::Managed(1);
        for pose in tilt_extremes(area.center()) {
            for cell in visible_cells(&pose, area, &|_| 0.0) {
                for lift in [0.0, LIFT] {
                    let mesh = tile_mesh(&pose, &cell, lift, 1.0, area, texture);
                    for j in 0..GRID as usize {
                        for i in 0..GRID as usize {
                            let top_right = mesh.vertices[j * stride + i + 1].pos;
                            let bottom_left = mesh.vertices[(j + 1) * stride + i].pos;
                            let drawn = top_right + (bottom_left - top_right) / 2.0;
                            let true_place = pose.project(
                                cell.x + GAP / 2.0 + TILE_WIDTH * (i as f32 + 0.5) / cells_in_grid,
                                cell.y + GAP / 2.0 + TILE_HEIGHT * (j as f32 + 0.5) / cells_in_grid,
                                lift,
                            );
                            let error = (drawn - true_place).length();
                            assert!(error < 0.25, "{error} points off in {cell:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn extreme_windows_give_finite_geometry_and_a_bounded_tile_count() {
        let texture = egui::TextureId::Managed(1);
        for size in [
            vec2(492.0, 1.0),
            vec2(2656.0, 1.0),
            vec2(1.0, 1.0),
            vec2(1.0, 2160.0),
        ] {
            let area = Rect::from_min_size(pos2(468.0, 0.0), size);
            for pose in tilt_extremes(area.center()) {
                let cells = visible_cells(&pose, area, &|column| start_offset(column, STILLS));
                assert!(!cells.is_empty(), "nothing at {size:?}");
                assert!(cells.len() < 1000, "{} cells at {size:?}", cells.len());
                for cell in &cells {
                    let mesh = tile_mesh(&pose, cell, LIFT, 1.0, area, texture);
                    assert!(
                        mesh.vertices
                            .iter()
                            .all(|vertex| vertex.pos.x.is_finite() && vertex.pos.y.is_finite()),
                        "a vertex is not finite at {size:?}"
                    );
                }
            }
        }
    }
}

//! Tile grid geometry. Columns are the ceiling of the square root of the tile count, which yields
//! the master spec's one to nine layouts.

use egui::{Rect, Vec2};

/// Columns and rows for `count` tiles: 1x1, 2x1, 2x2, 2x2, 3x2, 3x2, 3x3, 3x3, 3x3 for one to nine.
pub fn dimensions(count: usize) -> (usize, usize) {
    if count == 0 {
        return (0, 0);
    }
    let cols = (count as f64).sqrt().ceil() as usize;
    (cols, count.div_ceil(cols))
}

/// One rect per tile in row-major order, each sized to one grid cell of `area`. A last row that is
/// not full sits centred, so the grid keeps one axis instead of leaving a hole at its corner.
pub fn layout(area: Rect, count: usize) -> Vec<Rect> {
    let (cols, rows) = dimensions(count);
    if cols == 0 {
        return Vec::new();
    }
    let cell = Vec2::new(area.width() / cols as f32, area.height() / rows as f32);
    let empty_cells = cols * rows - count;
    (0..count)
        .map(|index| {
            let (col, row) = (index % cols, index / cols);
            let centring = if row + 1 == rows {
                empty_cells as f32 * cell.x / 2.0
            } else {
                0.0
            };
            let offset = Vec2::new(col as f32 * cell.x + centring, row as f32 * cell.y);
            Rect::from_min_size(area.min + offset, cell)
        })
        .collect()
}

/// A viewport in physical pixels, clamped to the surface so wgpu never sees an out-of-range viewport.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PixelRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Converts `rect`, in egui points, to a viewport in physical pixels by scaling with
/// `pixels_per_point`, clamped to `surface` (width, height in pixels) so the result never leaves
/// the surface and never has negative size.
pub fn to_pixels(rect: Rect, pixels_per_point: f32, surface: (u32, u32)) -> PixelRect {
    let (max_x, max_y) = (surface.0 as f32, surface.1 as f32);
    let x = (rect.min.x * pixels_per_point).clamp(0.0, max_x);
    let y = (rect.min.y * pixels_per_point).clamp(0.0, max_y);
    let right = (rect.max.x * pixels_per_point).clamp(x, max_x);
    let bottom = (rect.max.y * pixels_per_point).clamp(y, max_y);
    PixelRect {
        x,
        y,
        width: right - x,
        height: bottom - y,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{pos2, vec2};

    #[test]
    fn dimensions_follow_the_ceiling_square_root_rule() {
        let dims: Vec<(usize, usize)> = (0..=9).map(dimensions).collect();
        assert_eq!(
            dims,
            [
                (0, 0),
                (1, 1),
                (2, 1),
                (2, 2),
                (2, 2),
                (3, 2),
                (3, 2),
                (3, 3),
                (3, 3),
                (3, 3)
            ]
        );
    }

    #[test]
    fn three_tiles_fill_two_cells_and_centre_the_third_beneath_them() {
        let area = Rect::from_min_size(pos2(100.0, 50.0), vec2(200.0, 100.0));
        let rects = layout(area, 3);
        assert_eq!(rects.len(), 3);
        assert_eq!(
            rects[0],
            Rect::from_min_size(pos2(100.0, 50.0), vec2(100.0, 50.0))
        );
        assert_eq!(
            rects[1],
            Rect::from_min_size(pos2(200.0, 50.0), vec2(100.0, 50.0))
        );
        assert_eq!(
            rects[2],
            Rect::from_min_size(pos2(150.0, 100.0), vec2(100.0, 50.0))
        );
    }

    #[test]
    fn a_partly_filled_last_row_stays_on_the_axis_of_the_area_at_every_count() {
        let area = Rect::from_min_size(pos2(10.0, 20.0), vec2(900.0, 600.0));
        for count in 1..=9 {
            let rects = layout(area, count);
            let (columns, _) = dimensions(count);
            let last_row = &rects[(count - 1) / columns * columns..];
            let (left, right) = (last_row[0].left(), last_row[last_row.len() - 1].right());
            assert!(
                ((left + right) / 2.0 - area.center().x).abs() < 0.01,
                "{count} tiles: last row spans {left} to {right}"
            );
        }
    }

    #[test]
    fn zero_tiles_yield_no_rects() {
        assert!(layout(Rect::from_min_size(pos2(0.0, 0.0), vec2(10.0, 10.0)), 0).is_empty());
    }

    #[test]
    fn pixels_scale_by_the_point_ratio_and_clamp_to_the_surface() {
        let rect = Rect::from_min_size(pos2(10.0, 20.0), vec2(300.0, 100.0));
        let px = to_pixels(rect, 2.0, (400, 400));
        assert_eq!(
            px,
            PixelRect {
                x: 20.0,
                y: 40.0,
                width: 380.0,
                height: 200.0
            }
        );
    }
}

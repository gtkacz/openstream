//! Tile grid geometry. Every tile takes the shape of its picture and all of them share one height,
//! so a tile's corners and controls sit on the video rather than around a letterbox. The column
//! count is whichever shows the most picture in the area.

use egui::{Rect, Vec2, pos2, vec2};

/// How much of the best layout's picture the current one must still show to be kept, so dragging a
/// window across the point where two layouts tie does not flip the grid back and forth.
const KEEP_COLUMNS_ABOVE: f32 = 0.9;

/// The height every tile takes when `aspects` fill rows of `columns`: as tall as the rows allow,
/// unless the widest row runs out of width first.
fn tile_height(area: Vec2, aspects: &[f32], columns: usize, gap: f32) -> f32 {
    let rows = aspects.len().div_ceil(columns);
    let by_height = (area.y - gap * (rows - 1) as f32) / rows as f32;
    aspects
        .chunks(columns)
        .map(|row| (area.x - gap * (row.len() - 1) as f32) / row.iter().sum::<f32>())
        .fold(by_height, f32::min)
        .max(0.0)
}

fn picture_shown(area: Vec2, aspects: &[f32], columns: usize, gap: f32) -> f32 {
    let height = tile_height(area, aspects, columns, gap);
    aspects.iter().map(|aspect| aspect * height * height).sum()
}

/// The column count that shows the most picture, keeping `current` while it is nearly as good. On a
/// tie the fewest columns win, which balances the rows.
pub fn columns(area: Vec2, aspects: &[f32], gap: f32, current: Option<usize>) -> usize {
    let shown = |columns| picture_shown(area, aspects, columns, gap);
    let best = (2..=aspects.len()).fold(1, |best, candidate| {
        if shown(candidate) > shown(best) {
            candidate
        } else {
            best
        }
    });
    match current {
        Some(kept)
            if (1..=aspects.len()).contains(&kept)
                && shown(kept) >= shown(best) * KEEP_COLUMNS_ABOVE =>
        {
            kept
        }
        _ => best,
    }
}

/// One rect per picture in row-major order, each of its picture's shape and `gap` from the next. The
/// block sits centred in `area`, and so does each row, so a last row that is not full keeps the grid
/// on one axis instead of leaving a hole at its corner.
pub fn layout(area: Rect, aspects: &[f32], gap: f32, columns: usize) -> Vec<Rect> {
    if aspects.is_empty() {
        return Vec::new();
    }
    let height = tile_height(area.size(), aspects, columns, gap);
    let rows = aspects.len().div_ceil(columns);
    let block_height = rows as f32 * height + gap * (rows - 1) as f32;
    let mut top = area.center().y - block_height / 2.0;
    let mut rects = Vec::with_capacity(aspects.len());
    for row in aspects.chunks(columns) {
        let row_width = row.iter().sum::<f32>() * height + gap * row.len().saturating_sub(1) as f32;
        let mut left = area.center().x - row_width / 2.0;
        for aspect in row {
            let width = aspect * height;
            rects.push(Rect::from_min_size(pos2(left, top), vec2(width, height)));
            left += width + gap;
        }
        top += height + gap;
    }
    rects
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

    const WIDE: f32 = 16.0 / 9.0;
    const GAP: f32 = 8.0;

    fn area(width: f32, height: f32) -> Rect {
        Rect::from_min_size(pos2(100.0, 50.0), vec2(width, height))
    }

    fn placed(width: f32, height: f32, aspects: &[f32]) -> Vec<Rect> {
        let area = area(width, height);
        layout(area, aspects, GAP, columns(area.size(), aspects, GAP, None))
    }

    #[test]
    fn a_tile_takes_the_shape_of_its_picture_and_reaches_the_limiting_edge() {
        let [tile] = placed(1136.0, 827.0, &[WIDE])[..] else {
            panic!("one picture, one tile");
        };
        assert!((tile.width() / tile.height() - WIDE).abs() < 0.001);
        assert_eq!(tile.width(), 1136.0);
        assert_eq!(tile.center(), area(1136.0, 827.0).center());
    }

    #[test]
    fn a_portrait_picture_is_held_by_its_height_rather_than_stretched_across_the_area() {
        let [tile] = placed(1136.0, 827.0, &[9.0 / 16.0])[..] else {
            panic!("one picture, one tile");
        };
        assert_eq!(tile.height(), 827.0);
        assert!(tile.width() < 480.0);
    }

    #[test]
    fn two_wide_pictures_stack_in_a_normal_window_because_that_shows_more_picture() {
        let rects = placed(1136.0, 827.0, &[WIDE, WIDE]);
        assert_eq!(rects[0].left(), rects[1].left(), "one column");
        assert!(rects[0].width() > 700.0, "was {}", rects[0].width());
    }

    #[test]
    fn two_wide_pictures_sit_side_by_side_in_a_short_wide_window() {
        let rects = placed(1800.0, 500.0, &[WIDE, WIDE]);
        assert_eq!(rects[0].top(), rects[1].top(), "one row");
    }

    #[test]
    fn three_wide_pictures_in_a_narrow_window_take_a_column_for_nearly_twice_the_picture() {
        let rects = placed(704.0, 827.0, &[WIDE; 3]);
        assert_eq!(rects[0].left(), rects[2].left());
        assert!(rects[0].height() > 260.0, "was {}", rects[0].height());
    }

    #[test]
    fn tiles_are_one_gap_apart_and_stay_inside_the_area_at_any_size_and_count() {
        for count in 1..=9 {
            for (width, height) in [(1136.0, 827.0), (374.0, 780.0), (2500.0, 400.0)] {
                let bounds = area(width, height);
                let rects = placed(width, height, &vec![WIDE; count]);
                assert_eq!(rects.len(), count);
                for rect in &rects {
                    assert!(
                        bounds.expand(0.01).contains_rect(*rect),
                        "{count} in {width} by {height} left the area: {rect:?}"
                    );
                }
                for pair in rects.windows(2) {
                    if pair[0].top() == pair[1].top() {
                        assert!((pair[1].left() - pair[0].right() - GAP).abs() < 0.01);
                    }
                }
            }
        }
    }

    #[test]
    fn a_partly_filled_last_row_stays_on_the_axis_of_the_area_at_every_count() {
        let bounds = area(900.0, 600.0);
        for count in 1..=9 {
            let rects = placed(900.0, 600.0, &vec![WIDE; count]);
            let last = rects[count - 1];
            let row: Vec<_> = rects.iter().filter(|r| r.top() == last.top()).collect();
            let middle = (row[0].left() + row[row.len() - 1].right()) / 2.0;
            assert!(
                (middle - bounds.center().x).abs() < 0.01,
                "{count} tiles: last row centred on {middle}"
            );
        }
    }

    #[test]
    fn mixed_shapes_share_one_height_and_keep_their_own_widths() {
        let rects = placed(1136.0, 827.0, &[WIDE, 4.0 / 3.0]);
        assert_eq!(rects[0].height(), rects[1].height());
        assert!((rects[1].width() / rects[1].height() - 4.0 / 3.0).abs() < 0.001);
    }

    #[test]
    fn the_current_layout_is_kept_near_a_tie_and_given_up_once_clearly_worse() {
        let aspects = [WIDE, WIDE];
        let tied = vec2(1100.0, 640.0);
        let best = columns(tied, &aspects, GAP, None);
        let other = 3 - best;
        assert_eq!(columns(tied, &aspects, GAP, Some(other)), other, "kept");
        assert_eq!(columns(vec2(600.0, 2000.0), &aspects, GAP, Some(2)), 1);
    }

    #[test]
    fn a_current_count_the_tiles_no_longer_allow_is_ignored() {
        assert_eq!(columns(vec2(800.0, 600.0), &[WIDE], GAP, Some(3)), 1);
    }

    #[test]
    fn zero_tiles_yield_no_rects() {
        assert!(layout(area(10.0, 10.0), &[], GAP, 1).is_empty());
        assert_eq!(columns(vec2(10.0, 10.0), &[], GAP, None), 1);
    }

    #[test]
    fn an_area_too_small_for_its_gaps_collapses_instead_of_inverting() {
        for rect in placed(4.0, 4.0, &[WIDE; 6]) {
            assert!(rect.width() >= 0.0 && rect.height() >= 0.0, "{rect:?}");
        }
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

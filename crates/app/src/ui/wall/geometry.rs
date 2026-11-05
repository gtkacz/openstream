//! Where the wall's tiles sit and which still each shows, free of egui's frame state so it tests
//! without a window.

use super::stills::STRANDS;

pub const TILE: egui::Vec2 = egui::vec2(336.0, 189.0);
pub const GAP: f32 = 12.0;
pub const COLUMN_WIDTH: f32 = TILE.x + GAP;
pub const ROW_HEIGHT: f32 = TILE.y + GAP;
const BASE_SPEED: f32 = 9.0;
const SPEED_SPREAD: f32 = 0.3;

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
/// a tile's row stays the same however far its column drifts.
pub fn still_index(column: i32, row: i64, stills: usize) -> usize {
    let strand = strand_of(column);
    strand + STRANDS * row.rem_euclid(strand_len(strand, stills) as i64) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_alternate_direction_at_their_own_speeds() {
        let speeds: Vec<f32> = (0..6).map(column_speed).collect();
        for pair in speeds.windows(2) {
            assert!(pair[0] * pair[1] < 0.0, "{speeds:?}");
            assert_ne!(pair[0].abs(), pair[1].abs(), "{speeds:?}");
        }
        assert!(
            speeds
                .iter()
                .all(|speed| speed.abs() >= BASE_SPEED * (1.0 - SPEED_SPREAD)
                    && speed.abs() <= BASE_SPEED * (1.0 + SPEED_SPREAD))
        );
    }

    #[test]
    fn columns_start_staggered_within_their_repeat() {
        let first = start_offset(1, 120);
        let second = start_offset(2, 120);
        assert_ne!(first, second);
        let repeat = 15.0 * f64::from(ROW_HEIGHT);
        for column in -3..6 {
            let offset = start_offset(column, 120);
            assert!((0.0..repeat).contains(&offset), "column {column}: {offset}");
        }
    }

    #[test]
    fn each_column_walks_its_own_strand() {
        for (column, row, stills, index) in [
            (0, 0, 120, 0),
            (0, 1, 120, 8),
            (0, 14, 120, 112),
            (0, 15, 120, 0),
            (0, -1, 120, 112),
            (3, 2, 120, 19),
            (-1, 0, 120, 7),
            (8, 0, 120, 0),
            (0, 15, 122, 120),
            (2, 15, 122, 2),
        ] {
            assert_eq!(
                still_index(column, row, stills),
                index,
                "column {column}, row {row}, {stills} stills"
            );
        }
    }
}

//! A still seen through its dots. The pointer heats the cells near it, a hot cell shows the picture
//! beneath, and once the pointer has gone each cell cools back to a dot in its own time. It is plain
//! CPU work on a small grid, redone only for the tile being touched.

use egui::{Color32, ColorImage, Context, TextureHandle, TextureOptions};

use super::stills::{Grid, Prepared, stipple};

/// How long a cell takes to catch the pointer's heat.
const HEAT_SECONDS: f32 = 0.12;
/// How long a cell takes to cool from fully hot, which is what the reveal lingers for.
const COOL_SECONDS: f32 = 1.4;
/// The soft outer share of the pointer's reach, where cells are only partly heated.
const SOFT_EDGE: f32 = 0.35;
/// How wide the band of bright cells is at the edge of what is showing, as a share of the heat.
const RIM: f32 = 0.1;
const PULSE_SECONDS: f32 = 1.2;
const PULSE_WIDTH_CELLS: f32 = 20.0;

/// What one cell shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shown {
    Dot,
    Rim,
    Picture,
}

/// A click: heat running outward from where it landed.
#[derive(Debug, Clone, Copy)]
struct Pulse {
    x: f32,
    y: f32,
    age: f32,
}

/// The cells' heat, apart from anything that draws it.
#[derive(Debug)]
pub struct Heat {
    columns: usize,
    rows: usize,
    cells: Vec<f32>,
    pulses: Vec<Pulse>,
}

impl Heat {
    pub fn new(columns: usize, rows: usize) -> Self {
        Self {
            columns,
            rows,
            cells: vec![0.0; columns * rows],
            pulses: Vec::new(),
        }
    }

    pub fn pulse(&mut self, x: f32, y: f32) {
        self.pulses.insert(0, Pulse { x, y, age: 0.0 });
        self.pulses.truncate(4);
    }

    /// Moves the heat on by `dt` with the pointer at `pointer`, in cells. `reach` is how far the
    /// pointer's heat goes, in cells. False once every cell has gone cold. With `gently`, the whole
    /// tile simply follows whether the pointer is on it.
    pub fn step(&mut self, dt: f32, pointer: Option<(f32, f32)>, reach: f32, gently: bool) -> bool {
        if gently {
            self.pulses.clear();
            self.cells.fill(f32::from(u8::from(pointer.is_some())));
            return pointer.is_some();
        }
        for pulse in &mut self.pulses {
            pulse.age += dt;
        }
        self.pulses.retain(|pulse| pulse.age < PULSE_SECONDS);
        let catch_up = 1.0 - (-dt / HEAT_SECONDS).exp();
        let cool = dt / COOL_SECONDS;
        let far = (self.columns as f32).hypot(self.rows as f32) + PULSE_WIDTH_CELLS;
        let solid = reach * (1.0 - SOFT_EDGE);
        let mut warm = false;
        for row in 0..self.rows {
            for column in 0..self.columns {
                let (x, y) = (column as f32, row as f32);
                let mut wanted = pointer.map_or(0.0, |(px, py)| {
                    let distance = (x - px).hypot(y - py);
                    (1.0 - (distance - solid) / (reach - solid)).clamp(0.0, 1.0)
                });
                for pulse in &self.pulses {
                    let progress = pulse.age / PULSE_SECONDS;
                    let front = far * (1.0 - (1.0 - progress) * (1.0 - progress));
                    let off = ((x - pulse.x).hypot(y - pulse.y) - front).abs();
                    let strength = 1.0 - progress * progress;
                    wanted = wanted.max((1.0 - off / PULSE_WIDTH_CELLS).clamp(0.0, 1.0) * strength);
                }
                let cell = &mut self.cells[row * self.columns + column];
                *cell = if wanted > *cell {
                    *cell + (wanted - *cell) * catch_up
                } else {
                    (*cell - cool).max(wanted)
                };
                warm |= *cell > 0.0;
            }
        }
        warm || !self.pulses.is_empty()
    }

    pub fn shown(&self, column: usize, row: usize) -> Shown {
        let heat = self.cells[row * self.columns + column];
        let needs = stipple(column, row) * (1.0 - RIM);
        if heat > needs + RIM {
            Shown::Picture
        } else if heat > needs && heat > 0.0 {
            Shown::Rim
        } else {
            Shown::Dot
        }
    }
}

/// One tile part way through being revealed, with the texture it is drawn from meanwhile.
pub struct Reveal {
    pub still: usize,
    pub pointer: Option<(f32, f32)>,
    heat: Heat,
    grid: Grid,
    texture: TextureHandle,
}

impl Reveal {
    pub fn new(ctx: &Context, still: usize, grid: Grid) -> Self {
        Self {
            still,
            pointer: None,
            heat: Heat::new(grid.columns, grid.rows),
            grid,
            texture: ctx.load_texture(
                "wall-reveal",
                ColorImage::filled([grid.width, grid.height], Color32::BLACK),
                TextureOptions::NEAREST,
            ),
        }
    }

    pub fn texture(&self) -> &TextureHandle {
        &self.texture
    }

    pub fn pulse(&mut self) {
        if let Some((x, y)) = self.pointer {
            self.heat.pulse(x, y);
        }
    }

    /// Moves the reveal on and redraws the tile. False once there is nothing left to show.
    pub fn step(
        &mut self,
        dt: f32,
        reach: f32,
        gently: bool,
        still: &Prepared,
        dot: Color32,
        rim: Color32,
    ) -> bool {
        let alive = self.heat.step(dt, self.pointer, reach, gently);
        let grid = self.grid;
        let mut pixels = vec![Color32::BLACK; grid.width * grid.height];
        for y in 0..grid.height {
            let row = y / grid.cell;
            let in_dot_row = y % grid.cell < grid.dot;
            for x in 0..grid.width {
                let column = x / grid.cell;
                pixels[y * grid.width + x] = match self.heat.shown(column, row) {
                    Shown::Picture => still.photo[y * grid.width + x],
                    Shown::Rim => rim,
                    Shown::Dot
                        if in_dot_row
                            && x % grid.cell < grid.dot
                            && still.bits[row * grid.columns + column] == 1 =>
                    {
                        dot
                    }
                    Shown::Dot => Color32::BLACK,
                };
            }
        }
        self.texture.set(
            ColorImage::new([grid.width, grid.height], pixels),
            TextureOptions::NEAREST,
        );
        alive
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COLUMNS: usize = 112;
    const ROWS: usize = 63;
    const REACH: f32 = 84.0;
    const FRAME: f32 = 1.0 / 60.0;

    fn hold(heat: &mut Heat, pointer: Option<(f32, f32)>, seconds: f32) -> bool {
        let mut alive = false;
        for _ in 0..(seconds / FRAME) as u32 {
            alive = heat.step(FRAME, pointer, REACH, false);
        }
        alive
    }

    fn picture_share(heat: &Heat) -> f32 {
        let mut picture = 0;
        for row in 0..ROWS {
            for column in 0..COLUMNS {
                picture += u32::from(heat.shown(column, row) == Shown::Picture);
            }
        }
        picture as f32 / (COLUMNS * ROWS) as f32
    }

    #[test]
    fn from_the_middle_of_a_tile_the_reveal_covers_most_of_it() {
        let mut heat = Heat::new(COLUMNS, ROWS);
        hold(&mut heat, Some((56.0, 31.0)), 0.5);
        assert!(picture_share(&heat) > 0.9, "{}", picture_share(&heat));
    }

    #[test]
    fn cold_cells_show_their_dots() {
        let heat = Heat::new(COLUMNS, ROWS);
        assert_eq!(picture_share(&heat), 0.0);
        assert_eq!(heat.shown(0, 0), Shown::Dot);
    }

    #[test]
    fn it_lingers_after_the_pointer_leaves_and_then_goes_cold() {
        let mut heat = Heat::new(COLUMNS, ROWS);
        hold(&mut heat, Some((56.0, 31.0)), 0.5);
        hold(&mut heat, None, 0.4);
        assert!(picture_share(&heat) > 0.3, "still lingering");
        assert!(!hold(&mut heat, None, COOL_SECONDS), "gone cold");
        assert_eq!(picture_share(&heat), 0.0);
    }

    #[test]
    fn a_click_sends_heat_outward_and_ends() {
        let mut heat = Heat::new(COLUMNS, ROWS);
        heat.pulse(56.0, 31.0);
        assert!(hold(&mut heat, None, 0.3));
        assert!(picture_share(&heat) > 0.0);
        assert!(!hold(&mut heat, None, PULSE_SECONDS + COOL_SECONDS));
    }

    #[test]
    fn gently_the_whole_tile_just_follows_the_pointer() {
        let mut heat = Heat::new(COLUMNS, ROWS);
        assert!(heat.step(FRAME, Some((1.0, 1.0)), REACH, true));
        assert_eq!(picture_share(&heat), 1.0);
        assert!(!heat.step(FRAME, None, REACH, true));
        assert_eq!(picture_share(&heat), 0.0);
    }
}

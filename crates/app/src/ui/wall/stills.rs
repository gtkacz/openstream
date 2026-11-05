//! The wall's still frames: embedded at build time, decoded off the UI thread.

use std::collections::HashSet;
use std::sync::mpsc::{self, Receiver, TryRecvError};

use egui::{Color32, ColorImage, Context, TextureHandle, TextureOptions};

/// One embedded frame.
pub struct Still {
    pub name: &'static str,
    pub bytes: &'static [u8],
}

macro_rules! still {
    ($name:literal) => {
        Still {
            name: $name,
            bytes: include_bytes!(concat!("../../../assets/stills/", $name, ".jpg")),
        }
    };
}

/// A wall column draws from one strand: the stills at every `STRANDS`th index.
pub const STRANDS: usize = 8;

/// Read as rows of `STRANDS`, each column of this list is a strand, so each mixes kinds of content.
pub static STILLS: &[Still] = &[
    still!("game-amongus"),
    still!("anime-akira"),
    still!("show-andor"),
    still!("game-apex"),
    still!("web-archive"),
    still!("film-arrival"),
    still!("game-balatro"),
    still!("anime-aot"),
    still!("show-arcane"),
    still!("game-beatsaber"),
    still!("web-blender"),
    still!("game-bg3"),
    still!("anime-bebop"),
    still!("show-blackmirror"),
    still!("game-celeste"),
    still!("game-civ6"),
    still!("anime-codegeass"),
    still!("film-dune"),
    still!("show-bojackhorseman"),
    still!("game-cs2"),
    still!("web-code"),
    still!("game-cuphead"),
    still!("anime-demonslayer"),
    still!("show-breakingbad"),
    still!("game-cyberpunk"),
    still!("web-crates"),
    still!("anime-dragonballz"),
    still!("game-davediver"),
    still!("film-fellowship"),
    still!("game-disco"),
    still!("show-chernobyl"),
    still!("anime-drstone"),
    still!("game-dota2"),
    still!("web-gutenberg"),
    still!("show-doctorwho"),
    still!("game-drg"),
    still!("anime-fmab"),
    still!("film-interstellar"),
    still!("game-elden"),
    still!("web-hn"),
    still!("show-fallout"),
    still!("game-ets2"),
    still!("anime-frieren"),
    still!("game-factorio"),
    still!("show-friends"),
    still!("anime-haikyu"),
    still!("game-fallguys"),
    still!("film-madmax"),
    still!("web-kubernetes"),
    still!("game-ffxiv"),
    still!("anime-howl"),
    still!("show-planetearth"),
    still!("game-forza"),
    still!("web-lobsters"),
    still!("film-matrix"),
    still!("anime-hunterxhunter"),
    still!("game-hades"),
    still!("show-rickandmorty"),
    still!("game-hades2"),
    still!("web-mdn"),
    still!("anime-jujutsukaisen"),
    still!("show-sherlock"),
    still!("game-helldivers"),
    still!("film-nemo"),
    still!("game-hollow"),
    still!("anime-mobpsycho100"),
    still!("show-startrekstrangenewworlds"),
    still!("web-nasa"),
    still!("game-ittakestwo"),
    still!("anime-myheroacademia"),
    still!("film-spiderverse"),
    still!("game-ksp"),
    still!("show-strangerthings"),
    still!("web-python"),
    still!("game-mhw"),
    still!("anime-narutoshippuden"),
    still!("show-succession"),
    still!("game-nms"),
    still!("film-walle"),
    still!("web-repo"),
    still!("anime-neongenesisevangelion"),
    still!("game-ori"),
    still!("show-thebear"),
    still!("game-outerwilds"),
    still!("anime-onepiece"),
    still!("web-rust"),
    still!("show-theexpanse"),
    still!("game-overwatch"),
    still!("anime-pokemon"),
    still!("web-rustbook"),
    still!("game-palworld"),
    still!("show-thelastofus"),
    still!("anime-spirited"),
    still!("game-persona5"),
    still!("web-wikipedia"),
    still!("show-themandalorian"),
    still!("game-portal2"),
    still!("anime-spyxfamily"),
    still!("game-pubg"),
    still!("show-theoffice"),
    still!("anime-steinsgate"),
    still!("game-rimworld"),
    still!("show-thesimpsons"),
    still!("anime-totoro"),
    still!("game-rivals"),
    still!("show-wednesday"),
    still!("game-rocket"),
    still!("anime-vinlandsaga"),
    still!("game-satisfactory"),
    still!("anime-yourname"),
    still!("game-seaofthieves"),
    still!("game-sf6"),
    still!("game-skylines"),
    still!("game-slay"),
    still!("game-stardew"),
    still!("game-stray"),
    still!("game-subnautica"),
    still!("game-terraria"),
    still!("game-undertale"),
    still!("game-valheim"),
];

/// The sizes a still is prepared at, in physical pixels, so its dots land on whole pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grid {
    pub width: usize,
    pub height: usize,
    /// How far apart dots are.
    pub cell: usize,
    /// How wide a dot is, which leaves a dark gap around each.
    pub dot: usize,
    pub columns: usize,
    pub rows: usize,
    pub pixels_per_point: f32,
}

impl Grid {
    pub fn for_tile(tile: egui::Vec2, cell_points: f32, pixels_per_point: f32) -> Self {
        let width = (tile.x * pixels_per_point).round() as usize;
        let height = (tile.y * pixels_per_point).round() as usize;
        let cell = (cell_points * pixels_per_point).round().max(2.0) as usize;
        let gap = (0.75 * pixels_per_point).round().max(1.0) as usize;
        Self {
            width,
            height,
            cell,
            dot: cell.saturating_sub(gap).max(1),
            columns: width.div_ceil(cell),
            rows: height.div_ceil(cell),
            pixels_per_point,
        }
    }

    fn dots_image(&self, bits: &[u8], threshold: impl Fn(usize, usize) -> bool) -> ColorImage {
        let mut pixels = vec![Color32::TRANSPARENT; self.width * self.height];
        for y in 0..self.height {
            let row = y / self.cell;
            if y % self.cell >= self.dot {
                continue;
            }
            for x in 0..self.width {
                let column = x / self.cell;
                if x % self.cell < self.dot
                    && bits[row * self.columns + column] == 1
                    && threshold(column, row)
                {
                    pixels[y * self.width + x] = Color32::WHITE;
                }
            }
        }
        ColorImage::new([self.width, self.height], pixels)
    }
}

/// A still ready for the wall: its picture at tile size, and which of its dots are lit.
pub struct Prepared {
    pub photo: Vec<Color32>,
    pub bits: Vec<u8>,
}

/// Bright pages become bright text on dark, so a white web page does not turn into a solid block of
/// light, and each still gets its own contrast range. Atkinson's diffusion spreads only part of each
/// cell's error, which leaves clean blacks and whites and suits so few dots.
fn dither(rgba: &image::RgbaImage, columns: usize, rows: usize) -> Vec<u8> {
    let small = image::imageops::resize(
        rgba,
        columns as u32,
        rows as u32,
        image::imageops::FilterType::Triangle,
    );
    let mut values: Vec<f32> = small
        .pixels()
        .map(|p| {
            (0.2126 * f32::from(p.0[0]) + 0.7152 * f32::from(p.0[1]) + 0.0722 * f32::from(p.0[2]))
                / 255.0
        })
        .collect();
    let mean = values.iter().sum::<f32>() / values.len().max(1) as f32;
    if mean > 0.5 {
        values.iter_mut().for_each(|value| *value = 1.0 - *value);
    }
    let mut sorted = values.clone();
    sorted.sort_by(f32::total_cmp);
    let low = sorted[sorted.len() * 4 / 100];
    let high = sorted[sorted.len() * 985 / 1000];
    let range = (high - low).max(0.05);
    for value in &mut values {
        let stretched = ((*value - low) / range).clamp(0.0, 1.0);
        *value = stretched.powf(1.35);
    }
    let mut bits = vec![0u8; columns * rows];
    for y in 0..rows {
        for x in 0..columns {
            let at = y * columns + x;
            let on = values[at] > 0.5;
            bits[at] = u8::from(on);
            let share = (values[at] - f32::from(u8::from(on))) / 8.0;
            for (dx, dy) in [(1isize, 0usize), (2, 0), (-1, 1), (0, 1), (1, 1), (0, 2)] {
                let nx = x as isize + dx;
                let ny = y + dy;
                if nx >= 0 && (nx as usize) < columns && ny < rows {
                    values[ny * columns + nx as usize] += share;
                }
            }
        }
    }
    bits
}

fn prepare(bytes: &[u8], grid: Grid) -> Result<Prepared, image::ImageError> {
    let rgba = image::load_from_memory_with_format(bytes, image::ImageFormat::Jpeg)?.into_rgba8();
    let bits = dither(&rgba, grid.columns, grid.rows);
    let sized = image::imageops::resize(
        &rgba,
        grid.width as u32,
        grid.height as u32,
        image::imageops::FilterType::Triangle,
    );
    let photo = sized
        .pixels()
        .map(|p| Color32::from_rgb(p.0[0], p.0[1], p.0[2]))
        .collect();
    Ok(Prepared { photo, bits })
}

type Arrival = (usize, Result<Prepared, image::ImageError>);

/// How many steps a still's dots come on in, and how long each lasts.
const ARRIVAL_STEPS: u8 = 6;
const ARRIVAL_STEP_SECONDS: f64 = 1.0 / 12.0;

/// One loaded still: its dots as a texture, and how far they have come on.
struct Loaded {
    texture: TextureHandle,
    prepared: Option<Prepared>,
    arrived_at: f64,
    step: u8,
}

/// Decodes stills on its own thread and uploads them as they arrive, so the first frame never
/// waits. Dropping it stops the decoding.
pub struct Loader {
    stills: &'static [Still],
    grid: Grid,
    arrivals: Receiver<Arrival>,
    loaded: Vec<Option<Loaded>>,
    outstanding: usize,
}

impl Loader {
    pub fn start(grid: Grid) -> Self {
        Self::start_from(STILLS, grid)
    }

    pub fn start_from(stills: &'static [Still], grid: Grid) -> Self {
        let (sender, arrivals) = mpsc::channel();
        // Holding only the sender: an egui context here would keep itself alive through the
        // temporary memory that owns this loader, and the thread could never stop early.
        std::thread::spawn(move || {
            for (index, still) in stills.iter().enumerate() {
                if sender.send((index, prepare(still.bytes, grid))).is_err() {
                    return;
                }
            }
        });
        Self {
            stills,
            grid,
            arrivals,
            loaded: stills.iter().map(|_| None).collect(),
            outstanding: stills.len(),
        }
    }

    /// Uploads whatever has arrived and brings dots on. True while there is still more of that to do.
    /// Only stills in `visible` come on in steps, because redrawing every still that loads at once
    /// would hold up the window. With `all_at_once` none do, for less motion.
    pub fn poll(
        &mut self,
        ctx: &Context,
        now: f64,
        all_at_once: bool,
        visible: &HashSet<usize>,
    ) -> bool {
        loop {
            match self.arrivals.try_recv() {
                Ok(arrival) => {
                    let whole = all_at_once || !visible.contains(&arrival.0);
                    self.receive(ctx, arrival, now, whole);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.outstanding = 0;
                    break;
                }
            }
        }
        let grid = self.grid;
        let mut arriving = false;
        for loaded in self.loaded.iter_mut().flatten() {
            if loaded.step >= ARRIVAL_STEPS {
                continue;
            }
            let due =
                (((now - loaded.arrived_at) / ARRIVAL_STEP_SECONDS) as u8 + 1).min(ARRIVAL_STEPS);
            if due != loaded.step {
                loaded.step = due;
                if let Some(prepared) = &loaded.prepared {
                    let image = grid.dots_image(&prepared.bits, |column, row| {
                        stipple(column, row) * f32::from(ARRIVAL_STEPS) < f32::from(due)
                    });
                    loaded.texture.set(image, TextureOptions::NEAREST);
                }
            }
            arriving = true;
        }
        self.outstanding > 0 || arriving
    }

    pub fn texture(&self, index: usize) -> Option<&TextureHandle> {
        Some(&self.loaded.get(index)?.as_ref()?.texture)
    }

    pub fn prepared(&self, index: usize) -> Option<&Prepared> {
        self.loaded.get(index)?.as_ref()?.prepared.as_ref()
    }

    fn receive(&mut self, ctx: &Context, (index, decoded): Arrival, now: f64, whole: bool) {
        self.outstanding -= 1;
        let name = self.stills[index].name;
        match decoded {
            Ok(prepared) => {
                let step = if whole { ARRIVAL_STEPS } else { 0 };
                let image = self
                    .grid
                    .dots_image(&prepared.bits, |_, _| step == ARRIVAL_STEPS);
                self.loaded[index] = Some(Loaded {
                    texture: ctx.load_texture(name, image, TextureOptions::NEAREST),
                    prepared: Some(prepared),
                    arrived_at: now,
                    step,
                });
            }
            Err(error) => tracing::warn!(still = name, %error, "could not decode a still"),
        }
    }

    #[cfg(test)]
    pub fn loaded(grid: Grid, textures: Vec<TextureHandle>) -> Self {
        let (_, arrivals) = mpsc::channel();
        Self {
            stills: &[],
            grid,
            arrivals,
            loaded: textures
                .into_iter()
                .map(|texture| {
                    Some(Loaded {
                        texture,
                        prepared: Some(Prepared {
                            photo: vec![Color32::WHITE; grid.width * grid.height],
                            bits: vec![1; grid.columns * grid.rows],
                        }),
                        arrived_at: 0.0,
                        step: ARRIVAL_STEPS,
                    })
                })
                .collect(),
            outstanding: 0,
        }
    }

    #[cfg(test)]
    pub fn finish(&mut self, ctx: &Context) {
        while self.outstanding > 0 {
            match self.arrivals.recv() {
                Ok(arrival) => self.receive(ctx, arrival, 0.0, true),
                Err(_) => self.outstanding = 0,
            }
        }
    }
}

/// A number from 0 to 1 for each cell, spread evenly enough that the cells below any threshold look
/// like a dither. Interleaved gradient noise, from Jimenez's 2014 talk on next-generation
/// post-processing, in place of a repeating matrix whose grid would show on something this coarse.
pub fn stipple(column: usize, row: usize) -> f32 {
    let wave = 0.067_110_56 * column as f32 + 0.005_837_15 * row as f32;
    (52.982_918 * wave.fract()).fract()
}

#[cfg(test)]
mod tests {
    use super::*;

    static ONE_GOOD_ONE_CORRUPT: &[Still] = &[
        Still {
            name: "good",
            bytes: include_bytes!("../../../assets/stills/game-stardew.jpg"),
        },
        Still {
            name: "corrupt",
            bytes: b"not a jpeg",
        },
    ];

    fn grid() -> Grid {
        Grid::for_tile(egui::vec2(48.0, 27.0), 3.0, 1.0)
    }

    #[test]
    fn every_embedded_still_prepares_into_a_full_set_of_dots_and_pixels() {
        let grid = grid();
        for still in STILLS {
            let prepared =
                prepare(still.bytes, grid).unwrap_or_else(|e| panic!("{}: {e}", still.name));
            assert_eq!(
                prepared.bits.len(),
                grid.columns * grid.rows,
                "{}",
                still.name
            );
            assert_eq!(
                prepared.photo.len(),
                grid.width * grid.height,
                "{}",
                still.name
            );
        }
    }

    #[test]
    fn every_still_file_is_embedded_and_named_in_the_notice() {
        let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/stills");
        let notice = std::fs::read_to_string(folder.join("README.md")).unwrap();
        let embedded: HashSet<&str> = STILLS.iter().map(|still| still.name).collect();
        assert_eq!(embedded.len(), STILLS.len(), "a still is listed twice");
        assert!(
            (110..=130).contains(&STILLS.len()),
            "{} stills",
            STILLS.len()
        );
        let mut files = 0;
        for entry in std::fs::read_dir(folder).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|extension| extension == "jpg") {
                files += 1;
                let stem = path.file_stem().unwrap().to_str().unwrap();
                assert!(embedded.contains(stem), "{stem} is not embedded");
                assert!(
                    notice.contains(&format!("`{stem}.jpg`")),
                    "{stem} has no row"
                );
            }
        }
        assert_eq!(files, STILLS.len(), "a listed still has no file");
    }

    #[test]
    fn a_white_page_comes_out_mostly_dark_rather_than_a_block_of_light() {
        let page = image::RgbaImage::from_pixel(64, 36, image::Rgba([250, 250, 250, 255]));
        let bits = dither(&page, 32, 18);
        let lit = bits.iter().filter(|bit| **bit == 1).count();
        assert!(lit * 4 < bits.len(), "{lit} of {} lit", bits.len());
    }

    #[test]
    fn the_stipple_covers_the_range_evenly() {
        let values: Vec<f32> = (0..64)
            .flat_map(|row| (0..64).map(move |column| stipple(column, row)))
            .collect();
        assert!(values.iter().all(|value| (0.0..1.0).contains(value)));
        let below_half = values.iter().filter(|value| **value < 0.5).count();
        let share = below_half as f32 / values.len() as f32;
        assert!((0.4..0.6).contains(&share), "{share}");
    }

    #[test]
    fn a_corrupt_still_is_an_error_rather_than_a_panic() {
        assert!(prepare(b"not a jpeg", grid()).is_err());
    }

    #[test]
    fn a_failed_still_stays_empty_while_the_others_load() {
        let ctx = egui::Context::default();
        let mut loader = Loader::start_from(ONE_GOOD_ONE_CORRUPT, grid());
        loader.finish(&ctx);
        assert!(loader.texture(0).is_some());
        assert!(loader.texture(1).is_none());
        assert!(!loader.poll(&ctx, 0.0, true, &HashSet::new()));
    }

    fn still_loading(loader: &mut Loader, ctx: &Context, visible: &HashSet<usize>) {
        while loader.outstanding > 0 {
            loader.poll(ctx, 0.0, false, visible);
            std::thread::yield_now();
        }
    }

    #[test]
    fn a_still_in_view_comes_on_in_steps_and_then_asks_for_nothing() {
        let ctx = egui::Context::default();
        let visible = HashSet::from([0]);
        let mut loader = Loader::start_from(&ONE_GOOD_ONE_CORRUPT[..1], grid());
        still_loading(&mut loader, &ctx, &visible);
        let mut now = ARRIVAL_STEP_SECONDS;
        assert!(loader.poll(&ctx, now, false, &visible), "still coming on");
        now += ARRIVAL_STEP_SECONDS * f64::from(ARRIVAL_STEPS);
        loader.poll(&ctx, now, false, &visible);
        assert!(
            !loader.poll(&ctx, now, false, &visible),
            "whole, so nothing to redo"
        );
    }

    #[test]
    fn a_still_out_of_view_arrives_whole() {
        let ctx = egui::Context::default();
        let mut loader = Loader::start_from(&ONE_GOOD_ONE_CORRUPT[..1], grid());
        still_loading(&mut loader, &ctx, &HashSet::new());
        assert!(!loader.poll(&ctx, 0.0, false, &HashSet::new()));
    }
}

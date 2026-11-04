//! The wall's still frames: embedded at build time, decoded off the UI thread.

use std::sync::mpsc::{self, Receiver, TryRecvError};

use egui::{ColorImage, Context, TextureHandle, TextureOptions};

use super::geometry::TILE_WIDTH;
use crate::ui::theme;

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
pub const STRANDS: usize = 7;

/// Read as rows of `STRANDS`, each column of this list is a strand, so each mixes kinds of content.
pub static STILLS: &[Still] = &[
    still!("game-elden"),
    still!("anime-spirited"),
    still!("web-code"),
    still!("film-madmax"),
    still!("game-stardew"),
    still!("anime-akira"),
    still!("film-matrix"),
    still!("web-hn"),
    still!("game-cyberpunk"),
    still!("anime-yourname"),
    still!("film-interstellar"),
    still!("game-hades"),
    still!("web-wikipedia"),
    still!("anime-bebop"),
    still!("game-rocket"),
    still!("film-dune"),
    still!("game-disco"),
    still!("anime-aot"),
    still!("web-repo"),
    still!("game-hollow"),
    still!("film-spiderverse"),
    still!("anime-totoro"),
    still!("game-bg3"),
    still!("film-fellowship"),
    still!("web-mdn"),
    still!("game-factorio"),
    still!("film-nemo"),
    still!("game-cs2"),
    still!("film-walle"),
    still!("web-rust"),
    still!("game-celeste"),
    still!("anime-demonslayer"),
    still!("film-arrival"),
    still!("game-balatro"),
    still!("anime-frieren"),
    still!("game-dota2"),
    still!("anime-howl"),
    still!("game-civ6"),
    still!("game-terraria"),
    still!("anime-fmab"),
    still!("web-rustbook"),
    still!("game-portal2"),
];

/// Decodes a JPEG and bakes the tile's rounded corners into its alpha, so they tilt with the tile.
pub fn decode(bytes: &[u8]) -> Result<ColorImage, image::ImageError> {
    let mut rgba =
        image::load_from_memory_with_format(bytes, image::ImageFormat::Jpeg)?.into_rgba8();
    round_corners(&mut rgba);
    let size = [rgba.width() as usize, rgba.height() as usize];
    Ok(ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()))
}

fn round_corners(rgba: &mut image::RgbaImage) {
    let (width, height) = rgba.dimensions();
    let radius = f32::from(theme::RADIUS_CARD) * width as f32 / TILE_WIDTH;
    let reach = radius.ceil() as u32;
    for y in 0..reach.min(height) {
        for x in 0..reach.min(width) {
            let distance = (radius - (x as f32 + 0.5)).hypot(radius - (y as f32 + 0.5));
            let coverage = (radius + 0.5 - distance).clamp(0.0, 1.0);
            let alpha = (coverage * 255.0).round() as u8;
            for (corner_x, corner_y) in [
                (x, y),
                (width - 1 - x, y),
                (x, height - 1 - y),
                (width - 1 - x, height - 1 - y),
            ] {
                rgba.get_pixel_mut(corner_x, corner_y).0[3] = alpha;
            }
        }
    }
}

type Arrival = (usize, Result<ColorImage, image::ImageError>);

/// Decodes stills on its own thread and uploads them as they arrive, so the first frame never
/// waits. Dropping it stops the decoding.
pub struct Loader {
    stills: &'static [Still],
    arrivals: Receiver<Arrival>,
    textures: Vec<Option<TextureHandle>>,
    outstanding: usize,
}

impl Loader {
    pub fn start() -> Self {
        Self::start_from(STILLS)
    }

    pub fn start_from(stills: &'static [Still]) -> Self {
        let (sender, arrivals) = mpsc::channel();
        // Holding only the sender: an egui context here would keep itself alive through the
        // temporary memory that owns this loader, and the thread could never stop early.
        std::thread::spawn(move || {
            for (index, still) in stills.iter().enumerate() {
                if sender.send((index, decode(still.bytes))).is_err() {
                    return;
                }
            }
        });
        Self {
            stills,
            arrivals,
            textures: stills.iter().map(|_| None).collect(),
            outstanding: stills.len(),
        }
    }

    /// Uploads whatever has arrived. True while stills are still on their way.
    pub fn poll(&mut self, ctx: &Context) -> bool {
        loop {
            match self.arrivals.try_recv() {
                Ok(arrival) => self.receive(ctx, arrival),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.outstanding = 0;
                    break;
                }
            }
        }
        self.outstanding > 0
    }

    pub fn texture(&self, index: usize) -> Option<&TextureHandle> {
        self.textures.get(index)?.as_ref()
    }

    fn receive(&mut self, ctx: &Context, (index, decoded): Arrival) {
        self.outstanding -= 1;
        let name = self.stills[index].name;
        match decoded {
            Ok(image) => {
                self.textures[index] = Some(ctx.load_texture(name, image, TextureOptions::LINEAR));
            }
            Err(error) => tracing::warn!(still = name, %error, "could not decode a still"),
        }
    }

    #[cfg(test)]
    pub fn loaded(textures: Vec<TextureHandle>) -> Self {
        let (_, arrivals) = mpsc::channel();
        Self {
            stills: &[],
            arrivals,
            textures: textures.into_iter().map(Some).collect(),
            outstanding: 0,
        }
    }

    #[cfg(test)]
    pub fn finish(&mut self, ctx: &Context) {
        while self.outstanding > 0 {
            match self.arrivals.recv() {
                Ok(arrival) => self.receive(ctx, arrival),
                Err(_) => self.outstanding = 0,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

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

    #[test]
    fn every_embedded_still_decodes_at_480_by_270() {
        for still in STILLS {
            let image = decode(still.bytes).unwrap_or_else(|e| panic!("{}: {e}", still.name));
            assert_eq!(image.size, [480, 270], "{}", still.name);
        }
    }

    #[test]
    fn every_still_file_is_embedded_and_named_in_the_notice() {
        let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/stills");
        let notice = std::fs::read_to_string(folder.join("README.md")).unwrap();
        let embedded: HashSet<&str> = STILLS.iter().map(|still| still.name).collect();
        assert_eq!(embedded.len(), STILLS.len(), "a still is listed twice");
        assert!((38..=46).contains(&STILLS.len()), "{} stills", STILLS.len());
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
    fn a_decoded_still_has_rounded_transparent_corners_and_an_opaque_body() {
        let image = decode(STILLS[0].bytes).unwrap();
        for corner in [(0, 0), (479, 0), (0, 269), (479, 269)] {
            assert_eq!(image[corner].a(), 0, "corner {corner:?}");
        }
        for body in [(240, 135), (240, 0), (0, 135), (479, 135), (240, 269)] {
            assert_eq!(image[body].a(), 255, "body {body:?}");
        }
        // A 12-point radius on a 200-point tile is 28.8 pixels here, so this pixel straddles the arc.
        let on_the_arc = image[(8, 8)].a();
        assert!(on_the_arc > 0 && on_the_arc < 255, "alpha {on_the_arc}");
    }

    #[test]
    fn a_corrupt_still_is_an_error_rather_than_a_panic() {
        assert!(decode(b"not a jpeg").is_err());
    }

    #[test]
    fn a_failed_still_stays_empty_while_the_others_load() {
        let ctx = egui::Context::default();
        let mut loader = Loader::start_from(ONE_GOOD_ONE_CORRUPT);
        loader.finish(&ctx);
        assert!(loader.texture(0).is_some());
        assert!(loader.texture(1).is_none());
        assert!(!loader.poll(&ctx));
    }

    #[test]
    fn a_loader_ends_with_a_texture_for_every_still() {
        let ctx = egui::Context::default();
        let mut loader = Loader::start();
        loader.finish(&ctx);
        assert!((0..STILLS.len()).all(|index| loader.texture(index).is_some()));
        assert!(!loader.poll(&ctx));
    }
}

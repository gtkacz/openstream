//! The icon every window carries. `assets/icon/openstream.svg` is the source; the README beside it
//! says how the raw pixels are made.

#[cfg(target_os = "linux")]
use winit::platform::wayland::WindowAttributesExtWayland;
#[cfg(windows)]
use winit::platform::windows::WindowAttributesExtWindows;
use winit::window::{Icon, WindowAttributes};

#[cfg(target_os = "linux")]
use crate::handler::APP_ID;

/// Raw because winit takes RGBA, and decoding a PNG would add a dependency for this one call.
const WINDOW_ICON_RGBA: &[u8] = include_bytes!("../assets/icon/openstream-64.rgba");
const WINDOW_ICON_SIZE: u32 = 64;

fn window_icon() -> Option<Icon> {
    Icon::from_rgba(
        WINDOW_ICON_RGBA.to_vec(),
        WINDOW_ICON_SIZE,
        WINDOW_ICON_SIZE,
    )
    .inspect_err(|error| tracing::warn!(%error, "could not load the window icon"))
    .ok()
}

/// Gives a window the app's icon on every platform that lets a window choose one.
pub fn with_app_icon(attributes: WindowAttributes) -> WindowAttributes {
    // Wayland has no per-window icon: the compositor reads it from the desktop entry this id names.
    #[cfg(target_os = "linux")]
    let attributes = attributes.with_name(APP_ID, APP_ID);
    // On Windows `with_window_icon` only covers the title bar; the taskbar takes its own.
    #[cfg(windows)]
    let attributes = attributes.with_taskbar_icon(window_icon());
    attributes.with_window_icon(window_icon())
}

/// The scanlines of `openstream.svg` as `(row, start, end)`, with `start` and `end` as fractions of
/// the disc's width. Painted rather than loaded so the mark stays sharp at any display scale.
const MARK_BARS: [(u8, f32, f32); 22] = [
    (0, 0.308, 0.693),
    (1, 0.181, 0.820),
    (2, 0.106, 0.416),
    (2, 0.797, 0.894),
    (3, 0.056, 0.242),
    (3, 0.771, 0.944),
    (4, 0.024, 0.202),
    (4, 0.421, 0.976),
    (5, 0.006, 0.224),
    (5, 0.511, 0.994),
    (6, 0.000, 0.323),
    (6, 0.678, 1.000),
    (7, 0.006, 0.490),
    (7, 0.776, 0.994),
    (8, 0.024, 0.580),
    (8, 0.799, 0.976),
    (9, 0.056, 0.230),
    (9, 0.759, 0.944),
    (10, 0.106, 0.203),
    (10, 0.584, 0.894),
    (11, 0.181, 0.820),
    (12, 0.308, 0.693),
];
const MARK_ROWS: u8 = 13;
const MARK_BAR_TO_ROW: f32 = 0.64;
/// The icon's own blue rather than a theme colour, so the logo in the app always matches the icon
/// the desktop shows.
const MARK_COLOR: egui::Color32 = egui::Color32::from_rgb(0x5C, 0xC8, 0xFF);

/// Paints the icon's mark, without its tile, into a square of `size` points.
pub fn paint_mark(ui: &mut egui::Ui, size: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(size), egui::Sense::hover());
    paint_mark_at(ui.painter(), rect);
}

/// Paints the mark into `rect`, which should be square.
pub fn paint_mark_at(painter: &egui::Painter, rect: egui::Rect) {
    let size = rect.width();
    let row_height = size / f32::from(MARK_ROWS);
    for &(row, start, end) in &MARK_BARS {
        // Bars hug the top of their row: centred, one straddles two pixels and the gaps vanish.
        let top = rect.top() + row_height * f32::from(row);
        let bar = egui::Rect::from_min_max(
            egui::pos2(rect.left() + start * size, top),
            egui::pos2(rect.left() + end * size, top + row_height * MARK_BAR_TO_ROW),
        );
        painter.rect_filled(bar, bar.height() / 2.0, MARK_COLOR);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_pixels_make_a_valid_icon() {
        assert!(window_icon().is_some());
    }

    #[test]
    fn the_painted_mark_matches_the_svg() {
        let svg = include_str!("../assets/icon/openstream.svg");
        assert_eq!(svg.matches('Z').count(), MARK_BARS.len());
        let [red, green, blue, _] = MARK_COLOR.to_array();
        assert!(svg.contains(&format!("#{red:02X}{green:02X}{blue:02X}")));
    }
}

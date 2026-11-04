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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_pixels_make_a_valid_icon() {
        assert!(window_icon().is_some());
    }
}

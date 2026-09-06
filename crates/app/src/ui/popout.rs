//! A pop-out window's egui pass: one live filling the window, the tile overlay on top, and the
//! keyboard rule for fullscreen. The panels of the main window are not drawn here.

use brp_room::RoomSnapshot;

use super::UiOutput;
use super::state::UiState;
use super::tiles::{Placement, tile_overlay};
use crate::commands::WindowCommand;
use crate::render::tiles::TileKey;

/// Draws the pop-out for `key`. The output's `tile_rects` holds the one placement, or nothing
/// when the watch is gone from the snapshot, in which case the window is about to be closed.
pub fn draw(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    state: &mut UiState,
    key: TileKey,
    fullscreen: bool,
) -> UiOutput {
    let mut output = UiOutput::default();
    let Some(watch) = snapshot
        .watches
        .iter()
        .find(|w| (w.publisher, w.live_id) == key)
    else {
        return output;
    };
    // Sampled before the pass: a popup reacts to Esc synchronously while it is drawn and closes
    // itself, so reading this after `show` would always see it as already closed.
    let popup_open = egui::Popup::is_any_open(ui.ctx());
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            let rect = ui.max_rect();
            output.tile_rects.push((key, rect));
            tile_overlay(
                ui,
                snapshot,
                state,
                &mut output.commands,
                &mut output.window_commands,
                watch,
                key,
                rect,
                Placement::PopOut { fullscreen },
            );
        });
    let (f11, escape) = ui.input(|i| {
        // `key_pressed` fires on key-repeat too, which would retoggle fullscreen every frame
        // while F11 is held; only the initial press should count.
        let f11 = i.events.iter().any(|event| {
            matches!(
                event,
                egui::Event::Key {
                    key: egui::Key::F11,
                    pressed: true,
                    repeat: false,
                    ..
                }
            )
        });
        (f11, i.key_pressed(egui::Key::Escape))
    });
    if fullscreen_toggle_requested(f11, escape, fullscreen, popup_open) {
        output
            .window_commands
            .push(WindowCommand::ToggleFullscreen(key));
    }
    output
}

/// F11 always toggles; Esc only leaves fullscreen, so it cannot enter it by accident. Esc is
/// ignored while a popup is open because the same press is dismissing the popup.
pub fn fullscreen_toggle_requested(
    f11: bool,
    escape: bool,
    fullscreen: bool,
    popup_open: bool,
) -> bool {
    f11 || (escape && fullscreen && !popup_open)
}

#[cfg(test)]
mod tests {
    use super::fullscreen_toggle_requested;

    #[test]
    fn f11_toggles_both_ways_and_escape_only_leaves_fullscreen() {
        assert!(fullscreen_toggle_requested(true, false, false, false));
        assert!(fullscreen_toggle_requested(true, false, true, false));
        assert!(fullscreen_toggle_requested(false, true, true, false));
        assert!(!fullscreen_toggle_requested(false, true, false, false));
        assert!(!fullscreen_toggle_requested(false, false, true, false));
    }

    #[test]
    fn escape_is_ignored_while_a_popup_is_open_but_f11_still_toggles() {
        assert!(!fullscreen_toggle_requested(false, true, true, true));
        assert!(fullscreen_toggle_requested(true, false, true, true));
    }
}

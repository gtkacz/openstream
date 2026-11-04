//! The decisions the macOS backend makes about what ScreenCaptureKit reports, kept apart from the
//! Objective-C calls so every runner tests them.

// Compiled into tests everywhere and used for real only on macOS.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use brp_proto::PixelFormat;

use crate::error::CaptureError;
use crate::frame::CaptureFrame;

/// `SCStreamErrorUserDeclined`: Screen Recording is not allowed for this app.
pub(crate) const USER_DECLINED: isize = -3801;

/// The error for a ScreenCaptureKit failure with this `NSError` code and description.
pub(crate) fn error_for(code: isize, message: String) -> CaptureError {
    if code == USER_DECLINED {
        CaptureError::PermissionDenied(message)
    } else {
        CaptureError::MacOs(message)
    }
}

/// `SCFrameStatus` values, as ScreenCaptureKit attaches them to each video sample.
pub(crate) const STATUS_COMPLETE: isize = 0;
pub(crate) const STATUS_STARTED: isize = 4;
// The statuses that carry no new picture, named for the tests that pin them.
#[cfg(test)]
const STATUS_IDLE: isize = 1;
#[cfg(test)]
const STATUS_BLANK: isize = 2;
#[cfg(test)]
const STATUS_SUSPENDED: isize = 3;
#[cfg(test)]
const STATUS_STOPPED: isize = 5;

/// Windows smaller than this in either dimension, in points, are not offered: they are helper
/// windows and slivers no one means to share.
const MIN_WINDOW_POINTS: f64 = 32.0;
/// The window layer ordinary application windows sit on.
const NORMAL_WINDOW_LAYER: isize = 0;
/// Used when the requested rate is zero; the room caps it with `--fps` anyway.
const DEFAULT_FPS: u32 = 60;

/// Whether a video sample carries a new picture worth encoding. An idle sample repeats the last
/// one, and blank, suspended, and stopped samples carry nothing; a status that cannot be read is
/// given the benefit of the doubt, since dropping every frame is the worse failure.
pub(crate) fn deliverable(status: Option<isize>, has_image: bool) -> bool {
    has_image
        && match status {
            None => true,
            Some(status) => status == STATUS_COMPLETE || status == STATUS_STARTED,
        }
}

/// A BGRA frame of `height` rows of `stride` bytes copied out of `data`, which may run longer than
/// the rows (CoreVideo pads the allocation). `None` when `data` is too short for the rows or a
/// row is narrower than its pixels.
pub(crate) fn bgra_frame_checked(
    width: u32,
    height: u32,
    stride: usize,
    data: &[u8],
    capture_ts_us: u64,
) -> Option<CaptureFrame> {
    let row_bytes = width as usize * PixelFormat::Bgra.bytes_per_pixel();
    let len = stride.checked_mul(height as usize)?;
    if stride < row_bytes || data.len() < len {
        return None;
    }
    Some(CaptureFrame {
        width,
        height,
        stride,
        format: PixelFormat::Bgra,
        data: data[..len].to_vec(),
        capture_ts_us,
    })
}

/// A length in points as whole pixels at `scale`, never zero. ScreenCaptureKit sizes displays and
/// windows in points and the stream in pixels.
pub(crate) fn pixels(points: f64, scale: f64) -> u32 {
    let scale = if scale > 0.0 { scale } else { 1.0 };
    let value = (points * scale).round();
    if value.is_finite() && value >= 1.0 {
        value.min(f64::from(u32::MAX)) as u32
    } else {
        1
    }
}

/// The stream's minimum frame interval as a `CMTime` value and timescale.
pub(crate) fn frame_interval(fps: u32) -> (i64, i32) {
    let fps = if fps == 0 { DEFAULT_FPS } else { fps };
    (1, i32::try_from(fps).unwrap_or(i32::MAX))
}

/// What the picker calls display `index`: its own name when the system has one.
pub(crate) fn display_name(index: usize, localized: Option<&str>) -> String {
    match localized.map(str::trim) {
        Some(name) if !name.is_empty() => name.to_string(),
        _ => format!("Display {}", index + 1),
    }
}

/// One window as ScreenCaptureKit lists it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WindowInfo {
    pub id: u32,
    pub pid: i32,
    pub layer: isize,
    pub on_screen: bool,
    pub title: Option<String>,
    pub application: Option<String>,
    /// In points.
    pub width: f64,
    pub height: f64,
}

/// Whether the picker offers this window: a titled, visible, normal-layer window big enough to
/// mean something, owned by any process but brp, since sharing brp's own window would only ever
/// show the picker to the room.
pub(crate) fn listable(window: &WindowInfo, own_pid: i32) -> bool {
    window.pid != own_pid
        && window.layer == NORMAL_WINDOW_LAYER
        && window.on_screen
        && window
            .title
            .as_deref()
            .is_some_and(|t| !t.trim().is_empty())
        && window.width >= MIN_WINDOW_POINTS
        && window.height >= MIN_WINDOW_POINTS
}

/// What the picker calls a window: "Application — Title", or the title alone when the
/// application is unknown or the title already is its name.
pub(crate) fn window_name(window: &WindowInfo) -> String {
    let title = window.title.as_deref().unwrap_or_default().trim();
    match window.application.as_deref().map(str::trim) {
        Some(app) if !app.is_empty() && app != title => format!("{app} — {title}"),
        _ => title.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refused_permission_is_told_apart_from_other_failures() {
        assert!(matches!(
            error_for(USER_DECLINED, "denied".into()),
            CaptureError::PermissionDenied(message) if message == "denied"
        ));
        assert!(matches!(
            error_for(-3802, "no content".into()),
            CaptureError::MacOs(message) if message == "no content"
        ));
    }

    #[test]
    fn only_frames_with_new_pixels_are_delivered() {
        assert!(deliverable(Some(STATUS_COMPLETE), true));
        assert!(deliverable(Some(STATUS_STARTED), true));
        assert!(
            !deliverable(Some(STATUS_IDLE), true),
            "idle repeats the last frame"
        );
        assert!(!deliverable(Some(STATUS_BLANK), true));
        assert!(!deliverable(Some(STATUS_SUSPENDED), true));
        assert!(!deliverable(Some(STATUS_STOPPED), true));
        assert!(
            !deliverable(Some(STATUS_COMPLETE), false),
            "no pixels, nothing to send"
        );
        assert!(
            deliverable(None, true),
            "a frame whose status cannot be read is sent rather than dropped"
        );
    }

    #[test]
    fn a_frame_keeps_its_stride_and_drops_the_buffer_tail() {
        // 2x2 BGRA with 12-byte rows (4 bytes of row padding) and 8 bytes of trailing slack.
        let data: Vec<u8> = (0..32).collect();
        let frame = bgra_frame_checked(2, 2, 12, &data, 77).unwrap();
        assert_eq!(frame.width, 2);
        assert_eq!(frame.height, 2);
        assert_eq!(frame.stride, 12);
        assert_eq!(frame.format, PixelFormat::Bgra);
        assert_eq!(frame.data, (0..24).collect::<Vec<u8>>());
        assert_eq!(frame.capture_ts_us, 77);
    }

    #[test]
    fn a_frame_shorter_than_its_rows_says_so() {
        assert!(bgra_frame_checked(2, 2, 12, &[0; 20], 0).is_none());
        assert!(
            bgra_frame_checked(2, 2, 4, &[0; 16], 0).is_none(),
            "a stride narrower than a row is malformed"
        );
        assert!(bgra_frame_checked(2, 2, 8, &[0; 16], 0).is_some());
    }

    #[test]
    fn point_sizes_become_pixels_at_the_display_scale() {
        assert_eq!(pixels(1512.0, 2.0), 3024);
        assert_eq!(pixels(1920.0, 1.0), 1920);
        assert_eq!(pixels(100.4, 1.5), 151, "rounded to the nearest pixel");
        assert_eq!(pixels(0.0, 2.0), 1, "never zero");
        assert_eq!(pixels(-5.0, 2.0), 1);
        assert_eq!(pixels(10.0, 0.0), 10, "an unknown scale counts as 1");
    }

    #[test]
    fn the_frame_interval_is_one_over_the_target_rate() {
        assert_eq!(frame_interval(60), (1, 60));
        assert_eq!(frame_interval(30), (1, 30));
        assert_eq!(frame_interval(0), (1, 60), "zero falls back to 60");
    }

    #[test]
    fn displays_without_a_name_are_numbered_from_one() {
        assert_eq!(
            display_name(0, Some("Built-in Retina Display")),
            "Built-in Retina Display"
        );
        assert_eq!(display_name(0, None), "Display 1");
        assert_eq!(display_name(2, Some("  ")), "Display 3");
    }

    fn window(pid: i32, layer: isize, title: &str) -> WindowInfo {
        WindowInfo {
            id: 42,
            pid,
            layer,
            on_screen: true,
            title: Some(title.to_string()),
            application: Some("Safari".to_string()),
            width: 800.0,
            height: 600.0,
        }
    }

    #[test]
    fn listed_windows_are_titled_normal_windows_of_other_processes() {
        let own = 100;
        assert!(listable(&window(7, 0, "Docs"), own));
        assert!(
            !listable(&window(own, 0, "OpenStream"), own),
            "never brp's own window"
        );
        assert!(
            !listable(&window(7, 25, "Menu"), own),
            "only the normal window layer"
        );
        assert!(
            !listable(&window(7, 0, ""), own),
            "untitled windows are not offered"
        );
        let mut hidden = window(7, 0, "Docs");
        hidden.on_screen = false;
        assert!(!listable(&hidden, own));
        let mut tiny = window(7, 0, "Docs");
        tiny.width = 4.0;
        assert!(
            !listable(&tiny, own),
            "slivers and helper windows are not offered"
        );
    }

    #[test]
    fn a_window_is_named_after_its_application_and_title() {
        assert_eq!(window_name(&window(7, 0, "Docs")), "Safari — Docs");
        let mut bare = window(7, 0, "Docs");
        bare.application = None;
        assert_eq!(window_name(&bare), "Docs");
        let mut same = window(7, 0, "Safari");
        same.application = Some("Safari".to_string());
        assert_eq!(window_name(&same), "Safari", "no repetition");
    }
}

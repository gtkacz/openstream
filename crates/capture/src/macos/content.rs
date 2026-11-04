//! Displays and windows as ScreenCaptureKit reports them, as source descriptors, and the way back
//! from a descriptor's id to the object a stream is opened on.

use std::sync::mpsc;
use std::time::Duration;

use block2::RcBlock;
use brp_proto::SourceKind;
use objc2::rc::Retained;
use objc2_app_kit::NSScreen;
use objc2_core_graphics::CGMainDisplayID;
use objc2_foundation::{MainThreadMarker, NSError, NSNumber, NSString};
use objc2_screen_capture_kit::{SCDisplay, SCShareableContent, SCWindow};

use crate::error::CaptureError;
use crate::frame::{SourceDescriptor, SourceId};
use crate::macos_model::{WindowInfo, display_name, error_for, listable, pixels, window_name};

/// How long the system may take to answer a content query. It answers in milliseconds once the
/// permission is settled; this only guards against a wedged daemon.
const CONTENT_TIMEOUT: Duration = Duration::from_secs(10);

/// ScreenCaptureKit's objects are documented as safe to use from any thread; objc2 cannot know
/// that, so they cross thread boundaries in this wrapper.
pub(super) struct Shared<T>(pub Retained<T>);
// SAFETY: see the type's documentation; nothing here relies on thread affinity.
unsafe impl<T> Send for Shared<T> {}

/// What a stream is opened on.
pub(super) enum Target {
    Display(Shared<SCDisplay>),
    Window(Shared<SCWindow>),
}

/// The displays and windows this process may capture, on-screen windows only.
pub(super) fn shareable() -> Result<Retained<SCShareableContent>, CaptureError> {
    let (tx, rx) = mpsc::sync_channel::<Result<Shared<SCShareableContent>, CaptureError>>(1);
    let handler = RcBlock::new(
        move |content: *mut SCShareableContent, error: *mut NSError| {
            // SAFETY: the handler receives either a live content object or a live error, each valid
            // for the duration of the call; retaining keeps them past it.
            let result = match unsafe { Retained::retain(content) } {
                Some(content) => Ok(Shared(content)),
                None => Err(unsafe { Retained::retain(error) }.map_or_else(
                    || CaptureError::MacOs("no shareable content".into()),
                    |e| ns_error(&e),
                )),
            };
            let _ = tx.try_send(result);
        },
    );
    // SAFETY: the block lives until the call returns and is copied by the framework.
    unsafe {
        SCShareableContent::getShareableContentExcludingDesktopWindows_onScreenWindowsOnly_completionHandler(
            true, true, &handler,
        );
    }
    match rx.recv_timeout(CONTENT_TIMEOUT) {
        Ok(result) => result.map(|shared| shared.0),
        Err(_) => Err(CaptureError::MacOs(
            "the system did not list shareable content in time".into(),
        )),
    }
}

pub(super) fn list(kind: SourceKind) -> Result<Vec<SourceDescriptor>, CaptureError> {
    let content = shareable()?;
    match kind {
        SourceKind::Monitor => {
            let names = screen_names();
            // SAFETY: plain property reads on objects the content keeps alive.
            let displays = unsafe { content.displays() };
            Ok(displays
                .iter()
                .enumerate()
                .map(|(index, display)| {
                    let id = unsafe { display.displayID() };
                    let scale = display_scale(id);
                    let name = names
                        .iter()
                        .find(|(screen, _)| *screen == id)
                        .map(|(_, name)| name.as_str());
                    SourceDescriptor {
                        id: SourceId(u64::from(id)),
                        kind,
                        name: display_name(index, name),
                        width: pixels(unsafe { display.width() } as f64, scale),
                        height: pixels(unsafe { display.height() } as f64, scale),
                    }
                })
                .collect())
        }
        SourceKind::Window => {
            let own_pid = std::process::id() as i32;
            // Listing sizes are a hint; the stream measures the window on the display it is on.
            let scale = display_scale(CGMainDisplayID());
            let windows = unsafe { content.windows() };
            Ok(windows
                .iter()
                .filter_map(|window| {
                    let info = window_info(&window);
                    listable(&info, own_pid).then(|| SourceDescriptor {
                        id: SourceId(u64::from(info.id)),
                        kind,
                        name: window_name(&info),
                        width: pixels(info.width, scale),
                        height: pixels(info.height, scale),
                    })
                })
                .collect())
        }
    }
}

/// Turns a request into the object to open. No id means the main display for displays, so the
/// headless publisher works without a picker, and an error for windows, which have no sensible
/// default.
pub(super) fn resolve(kind: SourceKind, source: Option<SourceId>) -> Result<Target, CaptureError> {
    if kind == SourceKind::Window && source.is_none() {
        return Err(CaptureError::SourceLost(
            "window sharing needs a window picked from the list".into(),
        ));
    }
    let content = shareable()?;
    match kind {
        SourceKind::Monitor => {
            let wanted = source.map_or_else(|| u64::from(CGMainDisplayID()), |id| id.0);
            unsafe { content.displays() }
                .iter()
                .find(|display| u64::from(unsafe { display.displayID() }) == wanted)
                .map(|display| Target::Display(Shared(display)))
                .ok_or_else(|| {
                    CaptureError::SourceLost(format!("display {wanted} is no longer attached"))
                })
        }
        SourceKind::Window => {
            let wanted = source.map_or(0, |id| id.0);
            unsafe { content.windows() }
                .iter()
                .find(|window| u64::from(unsafe { window.windowID() }) == wanted)
                .map(|window| Target::Window(Shared(window)))
                .ok_or_else(|| {
                    CaptureError::SourceLost(format!("window {wanted} is no longer open"))
                })
        }
    }
}

fn window_info(window: &SCWindow) -> WindowInfo {
    // SAFETY: plain property reads on a live window object.
    unsafe {
        let frame = window.frame();
        let owner = window.owningApplication();
        WindowInfo {
            id: window.windowID(),
            pid: owner.as_ref().map_or(-1, |app| app.processID()),
            layer: window.windowLayer(),
            on_screen: window.isOnScreen(),
            title: window.title().map(|title| title.to_string()),
            application: owner.map(|app| app.applicationName().to_string()),
            width: frame.size.width,
            height: frame.size.height,
        }
    }
}

/// Backing pixels per point for a display, from its current mode; 1 when it cannot be read.
pub(super) fn display_scale(id: u32) -> f64 {
    use objc2_core_graphics::{CGDisplayCopyDisplayMode, CGDisplayMode};
    let Some(mode) = CGDisplayCopyDisplayMode(id) else {
        return 1.0;
    };
    let points = CGDisplayMode::width(Some(&mode));
    let pixels = CGDisplayMode::pixel_width(Some(&mode));
    if points == 0 {
        1.0
    } else {
        pixels as f64 / points as f64
    }
}

/// Each screen's display id and its localized name. AppKit answers only on the main thread, which
/// is where the picker asks; elsewhere the picker falls back to numbering.
fn screen_names() -> Vec<(u32, String)> {
    let Some(mtm) = MainThreadMarker::new() else {
        return Vec::new();
    };
    let key = NSString::from_str("NSScreenNumber");
    NSScreen::screens(mtm)
        .iter()
        .filter_map(|screen| {
            let number = screen.deviceDescription().objectForKey(&key)?;
            let number = number.downcast::<NSNumber>().ok()?;
            Some((
                number.unsignedIntValue(),
                screen.localizedName().to_string(),
            ))
        })
        .collect()
}

pub(super) fn ns_error(error: &NSError) -> CaptureError {
    error_for(error.code(), error.localizedDescription().to_string())
}

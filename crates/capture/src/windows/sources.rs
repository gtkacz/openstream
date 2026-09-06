//! Monitors and windows as source descriptors, and the way back from an id to a handle.

use std::ffi::c_void;

use brp_proto::SourceKind;
use windows_capture::monitor::Monitor;
use windows_capture::window::Window;

use crate::error::CaptureError;
use crate::frame::{SourceDescriptor, SourceId};

/// Used when Windows cannot report a refresh rate; the room caps it with `--fps` anyway.
const DEFAULT_REFRESH_RATE: u32 = 60;

/// A resolved source: the handle capture is opened on.
#[derive(Clone, Copy)]
pub(super) enum Target {
    Monitor(Monitor),
    Window(Window),
}

impl Target {
    /// The rate frames can arrive at: the monitor's refresh rate, or that of the monitor the
    /// window is on.
    pub(super) fn refresh_rate(&self) -> u32 {
        let rate = match self {
            Target::Monitor(monitor) => monitor.refresh_rate().ok(),
            Target::Window(window) => window.monitor().and_then(|m| m.refresh_rate().ok()),
        };
        rate.filter(|rate| *rate > 0)
            .unwrap_or(DEFAULT_REFRESH_RATE)
    }
}

pub(super) fn list(kind: SourceKind) -> Result<Vec<SourceDescriptor>, CaptureError> {
    match kind {
        SourceKind::Monitor => Monitor::enumerate()
            .map_err(|error| windows_error("monitor enumeration", error))?
            .into_iter()
            .enumerate()
            .map(|(index, monitor)| {
                Ok(SourceDescriptor {
                    id: monitor_id(&monitor),
                    kind,
                    name: monitor
                        .name()
                        .unwrap_or_else(|_| format!("Monitor {}", index + 1)),
                    width: monitor
                        .width()
                        .map_err(|error| windows_error("monitor width", error))?,
                    height: monitor
                        .height()
                        .map_err(|error| windows_error("monitor height", error))?,
                })
            })
            .collect(),
        SourceKind::Window => {
            // Sharing brp's own window would only ever show the picker to the room.
            let own_process = std::process::id();
            Ok(Window::enumerate()
                .map_err(|error| windows_error("window enumeration", error))?
                .into_iter()
                .filter(|window| window.process_id().is_ok_and(|pid| pid != own_process))
                .filter_map(|window| {
                    let title = window.title().ok().filter(|title| !title.is_empty())?;
                    Some(SourceDescriptor {
                        id: window_id(&window),
                        kind,
                        name: title,
                        width: u32::try_from(window.width().ok()?).ok()?,
                        height: u32::try_from(window.height().ok()?).ok()?,
                    })
                })
                .collect())
        }
    }
}

/// Turns a request into a handle. No id means the primary monitor for monitors, so the headless
/// publisher works without a picker, and an error for windows, which have no sensible default.
pub(super) fn resolve(kind: SourceKind, source: Option<SourceId>) -> Result<Target, CaptureError> {
    match (kind, source) {
        (SourceKind::Monitor, None) => Monitor::primary()
            .map(Target::Monitor)
            .map_err(|error| windows_error("primary monitor", error)),
        (SourceKind::Monitor, Some(id)) => {
            let monitor = Monitor::from_raw_hmonitor(id.0 as usize as *mut c_void);
            let attached = Monitor::enumerate()
                .map_err(|error| windows_error("monitor enumeration", error))?
                .contains(&monitor);
            if attached {
                Ok(Target::Monitor(monitor))
            } else {
                Err(CaptureError::SourceLost(format!(
                    "monitor {} is no longer attached",
                    id.0
                )))
            }
        }
        (SourceKind::Window, None) => Err(CaptureError::SourceLost(
            "window sharing needs a window picked from the list".into(),
        )),
        (SourceKind::Window, Some(id)) => {
            let window = Window::from_raw_hwnd(id.0 as usize as *mut c_void);
            if window.is_valid() {
                Ok(Target::Window(window))
            } else {
                Err(CaptureError::SourceLost(format!(
                    "window {} is no longer open",
                    id.0
                )))
            }
        }
    }
}

fn monitor_id(monitor: &Monitor) -> SourceId {
    SourceId(monitor.as_raw_hmonitor() as usize as u64)
}

fn window_id(window: &Window) -> SourceId {
    SourceId(window.as_raw_hwnd() as usize as u64)
}

fn windows_error(call: &str, error: impl std::fmt::Display) -> CaptureError {
    CaptureError::Windows(format!("{call} failed: {error}"))
}

//! A Windows Graphics Capture session on the windows-capture crate's own capture thread.

use std::sync::PoisonError;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use brp_proto::{PixelFormat, monotonic_us};
use windows_capture::capture::{CaptureControl, Context, GraphicsCaptureApiHandler};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::{GraphicsCaptureApi, InternalCaptureControl};
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
    GraphicsCaptureItemType, MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
};

use super::SharedSink;
use super::sources::Target;
use crate::error::CaptureError;
use crate::fallback::Started;
use crate::frame::{CaptureFrame, CaptureSession, SourceInfo};

type Control = CaptureControl<Handler, CaptureError>;

/// Starts capturing `target`. `refresh_rate` is what the session reports as its frame rate;
/// `target_fps` caps how often Graphics Capture wakes us where the OS supports that.
pub(super) fn start(
    target: Target,
    target_fps: u32,
    refresh_rate: u32,
    sink: SharedSink,
) -> Result<Box<dyn Started>, CaptureError> {
    let (first_tx, first_rx) = mpsc::channel();
    let flags = HandlerFlags {
        sink,
        first: first_tx,
        fps: refresh_rate,
    };
    let control = match target {
        Target::Monitor(monitor) => run(monitor, target_fps, flags)?,
        Target::Window(window) => run(window, target_fps, flags)?,
    };
    Ok(Box::new(GcStarted {
        control: Some(control),
        first: first_rx,
    }))
}

fn run<T>(item: T, target_fps: u32, flags: HandlerFlags) -> Result<Control, CaptureError>
where
    T: TryInto<GraphicsCaptureItemType> + Send + 'static,
{
    let settings = Settings::new(
        item,
        cursor(),
        border(),
        SecondaryWindowSettings::Default,
        update_interval(target_fps),
        DirtyRegionSettings::Default,
        ColorFormat::Bgra8,
        flags,
    );
    Handler::start_free_threaded(settings).map_err(|error| {
        CaptureError::Windows(format!("graphics capture failed to start: {error}"))
    })
}

// Each setting is only requested where the OS build supports it; asking on an older build fails
// the whole session instead of being ignored.
fn cursor() -> CursorCaptureSettings {
    match GraphicsCaptureApi::is_cursor_settings_supported() {
        Ok(true) => CursorCaptureSettings::WithCursor,
        _ => CursorCaptureSettings::Default,
    }
}

fn border() -> DrawBorderSettings {
    match GraphicsCaptureApi::is_border_settings_supported() {
        Ok(true) => DrawBorderSettings::WithoutBorder,
        _ => DrawBorderSettings::Default,
    }
}

fn update_interval(target_fps: u32) -> MinimumUpdateIntervalSettings {
    match GraphicsCaptureApi::is_minimum_update_interval_supported() {
        Ok(true) => {
            MinimumUpdateIntervalSettings::Custom(Duration::from_secs(1) / target_fps.max(1))
        }
        _ => MinimumUpdateIntervalSettings::Default,
    }
}

struct HandlerFlags {
    sink: SharedSink,
    first: Sender<SourceInfo>,
    fps: u32,
}

/// Runs on the capture thread: copies each frame out of its D3D texture and hands it to the sink.
struct Handler {
    sink: SharedSink,
    first: Option<Sender<SourceInfo>>,
    fps: u32,
}

impl GraphicsCaptureApiHandler for Handler {
    type Flags = HandlerFlags;
    type Error = CaptureError;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        Ok(Self {
            sink: ctx.flags.sink,
            first: Some(ctx.flags.first),
            fps: ctx.flags.fps,
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        _: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        let mut buffer = frame
            .buffer()
            .map_err(|error| CaptureError::Windows(format!("frame readback failed: {error}")))?;
        let captured = CaptureFrame {
            width: buffer.width(),
            height: buffer.height(),
            stride: buffer.row_pitch() as usize,
            format: PixelFormat::Bgra,
            data: buffer.as_raw_buffer().to_vec(),
            capture_ts_us: monotonic_us(),
        };
        if let Some(first) = self.first.take() {
            // The receiver is gone once the fallback driver stopped waiting; a late first frame
            // is not an error.
            let _ = first.send(SourceInfo {
                width: captured.width,
                height: captured.height,
                fps: self.fps,
            });
        }
        let mut sink = self.sink.lock().unwrap_or_else(PoisonError::into_inner);
        (*sink)(captured);
        Ok(())
    }

    fn on_closed(&mut self) -> Result<(), Self::Error> {
        tracing::warn!("captured window closed; the live keeps its last frame until stopped");
        Ok(())
    }
}

struct GcStarted {
    control: Option<Control>,
    first: Receiver<SourceInfo>,
}

impl Started for GcStarted {
    fn wait_first_frame(&mut self, timeout: Duration) -> Result<Option<SourceInfo>, CaptureError> {
        match self.first.recv_timeout(timeout) {
            Ok(info) => Ok(Some(info)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(CaptureError::SourceLost(
                "graphics capture ended before its first frame".into(),
            )),
        }
    }

    fn into_session(mut self: Box<Self>, info: SourceInfo) -> Box<dyn CaptureSession> {
        Box::new(GcSession {
            info,
            control: self.control.take(),
        })
    }
}

impl Drop for GcStarted {
    fn drop(&mut self) {
        stop(self.control.take());
    }
}

struct GcSession {
    info: SourceInfo,
    control: Option<Control>,
}

impl CaptureSession for GcSession {
    fn info(&self) -> SourceInfo {
        self.info
    }

    fn stop(mut self: Box<Self>) {
        stop(self.control.take());
    }
}

impl Drop for GcSession {
    fn drop(&mut self) {
        stop(self.control.take());
    }
}

fn stop(control: Option<Control>) {
    if let Some(control) = control
        && let Err(error) = control.stop()
    {
        tracing::warn!(%error, "graphics capture did not stop cleanly");
    }
}

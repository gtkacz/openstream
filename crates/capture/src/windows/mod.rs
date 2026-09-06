//! Windows capture: Graphics Capture for monitors and windows, desktop duplication as the monitor
//! fallback, and a source list for the in-app picker.

mod duplication;
mod graphics_capture;
mod sources;

use std::sync::{Arc, Mutex};

use brp_proto::SourceKind;
use brp_proto::constants::CAPTURE_FALLBACK_TIMEOUT;

use crate::error::CaptureError;
use crate::fallback::{Attempt, start_with_fallback};
use crate::frame::{
    CaptureBackend, CaptureSession, FrameSink, SourceListing, SourceRequest, StartFuture,
};

use self::sources::Target;

/// The sink is shared between the primary and the fallback attempt. Only one capture thread runs
/// at a time, so the lock is never contended.
pub(crate) type SharedSink = Arc<Mutex<FrameSink>>;

/// Captures one monitor or window chosen from [`CaptureBackend::sources`].
pub struct WindowsCapture;

impl CaptureBackend for WindowsCapture {
    fn sources(&self, kind: SourceKind) -> Result<SourceListing, CaptureError> {
        sources::list(kind).map(SourceListing::Choices)
    }

    fn start(&self, request: SourceRequest, sink: FrameSink) -> StartFuture<'_> {
        Box::pin(async move {
            // Both attempts block on their first frame for up to the fallback timeout each.
            tokio::task::spawn_blocking(move || start_blocking(request, sink))
                .await
                .map_err(|error| {
                    CaptureError::Windows(format!("capture start task failed: {error}"))
                })?
        })
    }
}

fn start_blocking(
    request: SourceRequest,
    sink: FrameSink,
) -> Result<Box<dyn CaptureSession>, CaptureError> {
    let target = sources::resolve(request.kind, request.source)?;
    let refresh_rate = target.refresh_rate();
    let sink: SharedSink = Arc::new(Mutex::new(sink));
    let primary = Attempt {
        name: "graphics capture",
        start: Box::new({
            let sink = sink.clone();
            move || graphics_capture::start(target, request.target_fps, refresh_rate, sink)
        }),
    };
    let fallback = match target {
        Target::Monitor(monitor) => Some(Attempt {
            name: "desktop duplication",
            start: Box::new(move || duplication::start(monitor, refresh_rate, sink)),
        }),
        Target::Window(_) => None,
    };
    let session = start_with_fallback(CAPTURE_FALLBACK_TIMEOUT, primary, fallback)?;
    tracing::info!(kind = ?request.kind, info = ?session.info(), "windows capture started");
    Ok(session)
}

//! macOS capture: ScreenCaptureKit for monitors and windows, listed for the in-app picker.
//!
//! Every call needs the Screen & System Audio Recording permission. The first listing asks the
//! system for it; until the user allows it, listing and starting fail with
//! [`CaptureError::PermissionDenied`].

mod content;
mod stream;

use brp_proto::SourceKind;
use brp_proto::constants::CAPTURE_FALLBACK_TIMEOUT;

use crate::error::CaptureError;
use crate::fallback::{Attempt, start_with_fallback};
use crate::frame::{
    CaptureBackend, CaptureSession, FrameSink, SourceListing, SourceRequest, StartFuture,
};

/// Captures one display or window chosen from [`CaptureBackend::sources`].
pub struct MacCapture;

impl CaptureBackend for MacCapture {
    fn sources(&self, kind: SourceKind) -> Result<SourceListing, CaptureError> {
        content::list(kind).map(SourceListing::Choices)
    }

    fn start(&self, request: SourceRequest, sink: FrameSink) -> StartFuture<'_> {
        Box::pin(async move {
            // Fetching the shareable content and waiting for the first frame both block.
            tokio::task::spawn_blocking(move || start_blocking(request, sink))
                .await
                .map_err(|error| {
                    CaptureError::MacOs(format!("capture start task failed: {error}"))
                })?
        })
    }
}

fn start_blocking(
    request: SourceRequest,
    sink: FrameSink,
) -> Result<Box<dyn CaptureSession>, CaptureError> {
    let target = content::resolve(request.kind, request.source)?;
    // ScreenCaptureKit is the only capture path on macOS, so there is no fallback; the driver
    // still bounds the wait for the first frame and stops a silent stream.
    let attempt = Attempt {
        name: "screencapturekit",
        start: Box::new(move || stream::start(target, request.target_fps, sink)),
    };
    let session = start_with_fallback(CAPTURE_FALLBACK_TIMEOUT, attempt, None)?;
    tracing::info!(kind = ?request.kind, info = ?session.info(), "macos capture started");
    Ok(session)
}

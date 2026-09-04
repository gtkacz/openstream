//! Linux capture: the desktop portal picks the source and PipeWire delivers frames.

mod pipewire;
mod portal;

use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use ::pipewire as pw;
use brp_proto::constants::PORTAL_FORMAT_TIMEOUT;

use crate::error::CaptureError;
use crate::frame::{
    CaptureBackend, CaptureSession, FrameSink, SourceInfo, SourceRequest, StartFuture,
};

use self::pipewire::PwEvent;
use self::portal::PortalHandle;

/// Captures one monitor or window selected by the xdg-desktop-portal picker.
pub struct PortalCapture;

struct PortalSession {
    info: SourceInfo,
    quit: pw::channel::Sender<()>,
    thread: Option<JoinHandle<Result<(), CaptureError>>>,
    _portal: PortalHandle,
}

impl CaptureBackend for PortalCapture {
    fn start(&self, request: SourceRequest, sink: FrameSink) -> StartFuture<'_> {
        Box::pin(async move {
            let stream = portal::open_screencast(request.kind).await?;
            let (events_tx, events_rx) = mpsc::channel();
            let (quit_tx, quit_rx) = pw::channel::channel();
            let thread = thread::Builder::new()
                .name("brp-pipewire".into())
                .spawn(move || {
                    pipewire::run_stream(
                        stream.fd,
                        stream.node_id,
                        request.target_fps,
                        events_tx,
                        sink,
                        quit_rx,
                    )
                })
                .map_err(|error| {
                    CaptureError::PipeWire(format!("failed to spawn PipeWire thread: {error}"))
                })?;
            let first =
                tokio::task::spawn_blocking(move || events_rx.recv_timeout(PORTAL_FORMAT_TIMEOUT))
                    .await
                    .map_err(|error| {
                        CaptureError::PipeWire(format!("format wait task failed: {error}"))
                    })?;
            let info = match first {
                Ok(PwEvent::Format(info)) => info,
                Ok(PwEvent::Error(error)) => {
                    let _ = quit_tx.send(());
                    let _ = thread.join();
                    return Err(error);
                }
                Err(error) => {
                    let _ = quit_tx.send(());
                    let _ = thread.join();
                    return Err(CaptureError::SourceLost(format!(
                        "no format negotiated before the timeout: {error}"
                    )));
                }
            };
            tracing::info!(
                width = info.width,
                height = info.height,
                fps = info.fps,
                "portal capture started"
            );
            Ok(Box::new(PortalSession {
                info,
                quit: quit_tx,
                thread: Some(thread),
                _portal: stream.handle,
            }) as Box<dyn CaptureSession>)
        })
    }
}

impl CaptureSession for PortalSession {
    fn info(&self) -> SourceInfo {
        self.info
    }

    fn stop(mut self: Box<Self>) {
        self.shutdown();
    }
}

impl PortalSession {
    fn shutdown(&mut self) {
        let _ = self.quit.send(());
        if let Some(thread) = self.thread.take() {
            match thread.join() {
                Ok(Err(error)) => {
                    tracing::warn!(error = %error, "PipeWire thread ended with an error")
                }
                Ok(Ok(())) => {}
                Err(_) => tracing::warn!("PipeWire thread panicked"),
            }
        }
    }
}

impl Drop for PortalSession {
    fn drop(&mut self) {
        self.shutdown();
    }
}

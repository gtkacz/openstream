//! DXGI desktop duplication on a thread of our own: the fallback for monitors that Graphics
//! Capture cannot serve, typically exclusive-fullscreen games.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use brp_proto::{PixelFormat, monotonic_us};
use windows_capture::dxgi_duplication_api::{DxgiDuplicationApi, DxgiDuplicationFormat, Error};
use windows_capture::monitor::Monitor;

use super::SharedSink;
use crate::error::CaptureError;
use crate::fallback::Started;
use crate::frame::{CaptureFrame, CaptureSession, SourceInfo};

/// The converter expects BGRA; asking for it up front avoids a per-frame format check.
const FORMATS: [DxgiDuplicationFormat; 1] = [DxgiDuplicationFormat::Bgra8];

type FirstFrame = Result<SourceInfo, CaptureError>;

pub(super) fn start(
    monitor: Monitor,
    fps: u32,
    sink: SharedSink,
) -> Result<Box<dyn Started>, CaptureError> {
    let stop = Arc::new(AtomicBool::new(false));
    let (first_tx, first_rx) = mpsc::channel();
    let thread = thread::Builder::new()
        .name("brp-duplication".into())
        .spawn({
            let stop = stop.clone();
            move || run(monitor, fps, sink, first_tx, stop)
        })
        .map_err(|error| {
            CaptureError::Windows(format!("failed to spawn the duplication thread: {error}"))
        })?;
    Ok(Box::new(DupStarted {
        stop,
        thread: Some(thread),
        first: first_rx,
    }))
}

/// The duplication is created on this thread so every D3D call for it happens in one place.
fn run(
    monitor: Monitor,
    fps: u32,
    sink: SharedSink,
    first: Sender<FirstFrame>,
    stop: Arc<AtomicBool>,
) {
    let mut duplication = match DxgiDuplicationApi::new_options(monitor, &FORMATS) {
        Ok(duplication) => duplication,
        Err(error) => {
            let _ = first.send(Err(CaptureError::Windows(format!(
                "desktop duplication failed to start: {error}"
            ))));
            return;
        }
    };
    let timeout_ms = (1_000 / fps.max(1)).max(1);
    let mut first = Some(first);
    while !stop.load(Ordering::Relaxed) {
        match next_frame(&mut duplication, timeout_ms) {
            Ok(Some(frame)) => {
                if let Some(first) = first.take() {
                    let _ = first.send(Ok(SourceInfo {
                        width: frame.width,
                        height: frame.height,
                        fps,
                    }));
                }
                let mut sink = sink.lock().unwrap_or_else(PoisonError::into_inner);
                (*sink)(frame);
            }
            Ok(None) => {}
            // Windows drops the duplication on mode changes and desktop switches; a new one
            // carries on from the current desktop.
            Err(Error::AccessLost) => match duplication.recreate_options(&FORMATS) {
                Ok(recreated) => duplication = recreated,
                Err(error) => {
                    tracing::warn!(%error, "desktop duplication could not be recreated; the live keeps its last frame");
                    return;
                }
            },
            Err(error) => {
                tracing::warn!(%error, "desktop duplication ended; the live keeps its last frame");
                return;
            }
        }
    }
}

/// `Ok(None)` when nothing changed within the timeout. Returns an owned frame so the caller can
/// recreate the duplication with no borrow outstanding.
fn next_frame(
    duplication: &mut DxgiDuplicationApi,
    timeout_ms: u32,
) -> Result<Option<CaptureFrame>, Error> {
    let mut frame = match duplication.acquire_next_frame(timeout_ms) {
        Ok(frame) => frame,
        Err(Error::Timeout) => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut buffer = frame.buffer()?;
    Ok(Some(CaptureFrame {
        width: buffer.width(),
        height: buffer.height(),
        stride: buffer.row_pitch() as usize,
        format: PixelFormat::Bgra,
        data: buffer.as_raw_buffer().to_vec(),
        capture_ts_us: monotonic_us(),
    }))
}

struct DupStarted {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    first: Receiver<FirstFrame>,
}

impl Started for DupStarted {
    fn wait_first_frame(&mut self, timeout: Duration) -> Result<Option<SourceInfo>, CaptureError> {
        match self.first.recv_timeout(timeout) {
            Ok(result) => result.map(Some),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(CaptureError::SourceLost(
                "desktop duplication ended before its first frame".into(),
            )),
        }
    }

    fn into_session(mut self: Box<Self>, info: SourceInfo) -> Box<dyn CaptureSession> {
        Box::new(DupSession {
            info,
            stop: self.stop.clone(),
            thread: self.thread.take(),
        })
    }
}

impl Drop for DupStarted {
    fn drop(&mut self) {
        shutdown(&self.stop, self.thread.take());
    }
}

struct DupSession {
    info: SourceInfo,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl CaptureSession for DupSession {
    fn info(&self) -> SourceInfo {
        self.info
    }

    fn stop(mut self: Box<Self>) {
        shutdown(&self.stop, self.thread.take());
    }
}

impl Drop for DupSession {
    fn drop(&mut self) {
        shutdown(&self.stop, self.thread.take());
    }
}

fn shutdown(stop: &AtomicBool, thread: Option<JoinHandle<()>>) {
    // A missing handle means the thread now belongs to the session; its flag is not ours.
    let Some(thread) = thread else {
        return;
    };
    stop.store(true, Ordering::Relaxed);
    if thread.join().is_err() {
        tracing::warn!("duplication thread panicked");
    }
}

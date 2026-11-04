//! One ScreenCaptureKit stream of BGRA frames from a display or a window.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use block2::RcBlock;
use brp_proto::monotonic_us;
use dispatch2::{DispatchQueue, DispatchRetained};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{AnyThread, DefinedClass, define_class, msg_send};
use objc2_core_foundation::CFArray;
use objc2_core_media::{CMSampleBuffer, CMTime};
use objc2_core_video::{
    CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow, CVPixelBufferGetHeight,
    CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags,
    CVPixelBufferUnlockBaseAddress, kCVPixelFormatType_32BGRA,
};
use objc2_foundation::{
    NSArray, NSDictionary, NSError, NSNumber, NSObject, NSObjectProtocol, NSString,
};
use objc2_screen_capture_kit::{
    SCContentFilter, SCShareableContent, SCStream, SCStreamConfiguration, SCStreamDelegate,
    SCStreamFrameInfoStatus, SCStreamOutput, SCStreamOutputType,
};

use super::content::{Shared, Target, ns_error};
use crate::error::CaptureError;
use crate::fallback::Started;
use crate::frame::{CaptureSession, FrameSink, SourceInfo};
use crate::macos_model::{bgra_frame_checked, deliverable, frame_interval, pixels};

/// Frames ScreenCaptureKit may hold for us before it drops; small, since every frame we keep is
/// latency.
const QUEUE_DEPTH: isize = 3;
/// How long a stop may take before the session gives up waiting for its confirmation.
const STOP_TIMEOUT: Duration = Duration::from_secs(2);

/// What the stream's callbacks tell the thread that started it.
enum Event {
    /// The first frame arrived, at this size in pixels.
    FirstFrame(u32, u32),
    /// The stream ended or failed to start.
    Ended(CaptureError),
}

struct OutputIvars {
    sink: Mutex<FrameSink>,
    events: SyncSender<Event>,
    first_seen: AtomicBool,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and this class does not implement Drop.
    #[unsafe(super(NSObject))]
    #[name = "BrpScreenCaptureOutput"]
    #[ivars = OutputIvars]
    struct Output;

    unsafe impl NSObjectProtocol for Output {}

    unsafe impl SCStreamOutput for Output {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        fn stream_did_output(
            &self,
            _stream: &SCStream,
            sample: &CMSampleBuffer,
            kind: SCStreamOutputType,
        ) {
            if kind == SCStreamOutputType::Screen {
                self.video(sample);
            }
        }
    }

    unsafe impl SCStreamDelegate for Output {
        #[unsafe(method(stream:didStopWithError:))]
        fn stream_did_stop(&self, _stream: &SCStream, error: &NSError) {
            let _ = self.ivars().events.try_send(Event::Ended(ns_error(error)));
        }
    }
);

impl Output {
    fn new(sink: FrameSink, events: SyncSender<Event>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(OutputIvars {
            sink: Mutex::new(sink),
            events,
            first_seen: AtomicBool::new(false),
        });
        // SAFETY: NSObject's designated initialiser.
        unsafe { msg_send![super(this), init] }
    }

    fn video(&self, sample: &CMSampleBuffer) {
        // SAFETY: the sample is valid for the duration of the callback.
        let Some(image) = (unsafe { sample.image_buffer() }) else {
            return;
        };
        if !deliverable(frame_status(sample), true) {
            return;
        }
        let pixels = &*image;
        // SAFETY: the base address is read only between a successful lock and the unlock, and
        // the slice covers exactly the rows CoreVideo reports.
        let frame = unsafe {
            if CVPixelBufferLockBaseAddress(pixels, CVPixelBufferLockFlags::ReadOnly) != 0 {
                return;
            }
            let base = CVPixelBufferGetBaseAddress(pixels).cast::<u8>();
            let stride = CVPixelBufferGetBytesPerRow(pixels);
            let width = CVPixelBufferGetWidth(pixels) as u32;
            let height = CVPixelBufferGetHeight(pixels) as u32;
            let frame = if base.is_null() {
                None
            } else {
                let data = std::slice::from_raw_parts(base, stride * height as usize);
                bgra_frame_checked(width, height, stride, data, monotonic_us())
            };
            CVPixelBufferUnlockBaseAddress(pixels, CVPixelBufferLockFlags::ReadOnly);
            frame
        };
        let Some(frame) = frame else {
            return;
        };
        let ivars = self.ivars();
        if !ivars.first_seen.swap(true, Ordering::AcqRel) {
            let _ = ivars
                .events
                .try_send(Event::FirstFrame(frame.width, frame.height));
        }
        (lock(&ivars.sink))(frame);
    }
}

/// The `SCFrameStatus` ScreenCaptureKit attached to a video sample, if it can be read.
fn frame_status(sample: &CMSampleBuffer) -> Option<isize> {
    // SAFETY: the attachments array is a CFArray of CFDictionary, toll-free bridged to the
    // Foundation types it is read as here, and lives as long as `attachments`.
    unsafe {
        let attachments = sample.sample_attachments_array(false)?;
        let array: *const CFArray = &*attachments;
        let array = &*array.cast::<NSArray<NSDictionary<NSString, AnyObject>>>();
        let info = array.firstObject()?;
        let status = info.objectForKey(SCStreamFrameInfoStatus)?;
        Some(status.downcast::<NSNumber>().ok()?.integerValue())
    }
}

/// A started stream that has not yet delivered a frame. Dropping it stops the stream.
struct Running {
    stream: Retained<SCStream>,
    /// The stream holds its output and delegate weakly; this keeps them alive.
    _output: Retained<Output>,
    _queue: DispatchRetained<DispatchQueue>,
    events: Receiver<Event>,
    fps: u32,
    stopped: bool,
}

// SAFETY: ScreenCaptureKit streams may be stopped from any thread; the output object is only
// touched by the framework on its own queue.
unsafe impl Send for Running {}

impl Running {
    fn stop(&mut self) {
        if std::mem::replace(&mut self.stopped, true) {
            return;
        }
        let (tx, rx) = mpsc::sync_channel::<()>(1);
        let done = RcBlock::new(move |_error: *mut NSError| {
            let _ = tx.try_send(());
        });
        // SAFETY: the block is copied by the framework and outlives the call.
        unsafe { self.stream.stopCaptureWithCompletionHandler(Some(&done)) };
        let _ = rx.recv_timeout(STOP_TIMEOUT);
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Started for Running {
    fn wait_first_frame(&mut self, timeout: Duration) -> Result<Option<SourceInfo>, CaptureError> {
        match self.events.recv_timeout(timeout) {
            Ok(Event::FirstFrame(width, height)) => Ok(Some(SourceInfo {
                width,
                height,
                fps: self.fps,
            })),
            Ok(Event::Ended(error)) => Err(error),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(CaptureError::SourceLost(
                "the capture stream went away".into(),
            )),
        }
    }

    fn into_session(self: Box<Self>, info: SourceInfo) -> Box<dyn CaptureSession> {
        Box::new(Session {
            running: Mutex::new(*self),
            info,
        })
    }
}

struct Session {
    running: Mutex<Running>,
    info: SourceInfo,
}

impl CaptureSession for Session {
    fn info(&self) -> SourceInfo {
        self.info
    }

    fn stop(self: Box<Self>) {
        lock(&self.running).stop();
    }
}

/// Opens a stream on `target` at up to `fps` frames a second, delivering BGRA frames to `sink`.
pub(super) fn start(
    target: Target,
    fps: u32,
    sink: FrameSink,
) -> Result<Box<dyn Started>, CaptureError> {
    // SAFETY: every call below is an ordinary ScreenCaptureKit call on live objects; the blocks
    // and the output object outlive the calls that take them.
    unsafe {
        let filter = match &target {
            Target::Display(Shared(display)) => SCContentFilter::initWithDisplay_excludingWindows(
                SCContentFilter::alloc(),
                display,
                &NSArray::new(),
            ),
            Target::Window(Shared(window)) => {
                SCContentFilter::initWithDesktopIndependentWindow(SCContentFilter::alloc(), window)
            }
        };
        // The filter knows the content's size in points and its pixels per point.
        let info = SCShareableContent::infoForFilter(&filter);
        let rect = info.contentRect();
        let scale = f64::from(info.pointPixelScale());

        let config = SCStreamConfiguration::new();
        config.setWidth(pixels(rect.size.width, scale) as usize);
        config.setHeight(pixels(rect.size.height, scale) as usize);
        let (value, timescale) = frame_interval(fps);
        config.setMinimumFrameInterval(CMTime::new(value, timescale));
        config.setPixelFormat(kCVPixelFormatType_32BGRA);
        config.setShowsCursor(true);
        config.setQueueDepth(QUEUE_DEPTH);
        config.setCapturesAudio(false);

        let (events_tx, events) = mpsc::sync_channel(4);
        let output = Output::new(sink, events_tx.clone());
        let stream = SCStream::initWithFilter_configuration_delegate(
            SCStream::alloc(),
            &filter,
            &config,
            Some(ProtocolObject::from_ref(&*output)),
        );
        let queue = DispatchQueue::new("brp.capture.screen", None);
        stream
            .addStreamOutput_type_sampleHandlerQueue_error(
                ProtocolObject::from_ref(&*output),
                SCStreamOutputType::Screen,
                Some(&queue),
            )
            .map_err(|error| ns_error(&error))?;
        let started = RcBlock::new(move |error: *mut NSError| {
            if let Some(error) = Retained::retain(error) {
                let _ = events_tx.try_send(Event::Ended(ns_error(&error)));
            }
        });
        stream.startCaptureWithCompletionHandler(Some(&started));

        Ok(Box::new(Running {
            stream,
            _output: output,
            _queue: queue,
            events,
            fps: frame_interval(fps).1 as u32,
            stopped: false,
        }))
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

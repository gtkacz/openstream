//! An audio-only ScreenCaptureKit stream: system audio filtered by application, as 48 kHz stereo
//! float.

use std::mem::{align_of, size_of};
use std::ptr::{self, NonNull};
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use block2::RcBlock;
use brp_proto::constants::{AUDIO_CHANNELS, AUDIO_SAMPLE_RATE};
use brp_proto::monotonic_us;
use dispatch2::{DispatchQueue, DispatchRetained};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{AnyThread, DefinedClass, define_class, msg_send};
use objc2_core_audio_types::{
    AudioBuffer, AudioBufferList, kAudioFormatFlagIsFloat, kAudioFormatFlagIsNonInterleaved,
};
use objc2_core_foundation::CFRetained;
use objc2_core_media::{
    CMAudioFormatDescriptionGetStreamBasicDescription, CMBlockBuffer, CMSampleBuffer, CMTime,
};
use objc2_foundation::{NSArray, NSError, NSObject, NSObjectProtocol};
use objc2_screen_capture_kit::{
    SCContentFilter, SCDisplay, SCRunningApplication, SCShareableContent, SCStream,
    SCStreamConfiguration, SCStreamDelegate, SCStreamOutput, SCStreamOutputType,
};

use crate::chunk::{AudioChunk, AudioSink};
use crate::error::AudioError;
use crate::macos_model::{interleaved_to_stereo, planar_to_stereo};

/// `kCMSampleBufferFlag_AudioBufferList_Assure16ByteAlignment`.
const ASSURE_16_BYTE_ALIGNMENT: u32 = 1;
/// The smallest video the stream can be asked for: audio-only streams still carry a video track.
const VIDEO_SIZE: usize = 2;
/// One video frame a second is the least the stream can be asked for.
const VIDEO_FPS: i32 = 1;
/// How long a stop may take before the session gives up waiting for its confirmation.
const STOP_TIMEOUT: Duration = Duration::from_secs(2);
/// How long a filter update may take to be confirmed before it counts as failed and is retried.
const UPDATE_TIMEOUT: Duration = Duration::from_secs(2);

/// ScreenCaptureKit's objects are documented as safe to use from any thread; objc2 cannot know
/// that, so they cross thread boundaries in this wrapper.
pub(super) struct Shared<T>(pub Retained<T>);
// SAFETY: see the type's documentation.
unsafe impl<T> Send for Shared<T> {}

/// The applications a filter is built from.
pub(super) enum Audience {
    /// Everything except these applications (brp's own).
    AllExcept(Vec<Shared<SCRunningApplication>>),
    /// Only these applications; empty is silence.
    Only(Vec<Shared<SCRunningApplication>>),
}

struct OutputIvars {
    sink: Mutex<AudioSink>,
    /// Set once by the delegate when the stream dies; the session reports it.
    error: Arc<Mutex<Option<String>>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and this class does not implement Drop.
    #[unsafe(super(NSObject))]
    #[name = "BrpAudioCaptureOutput"]
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
            if kind == SCStreamOutputType::Audio
                && let Some(samples) = stereo_samples(sample)
                && !samples.is_empty()
            {
                (lock(&self.ivars().sink))(AudioChunk {
                    samples,
                    capture_ts_us: monotonic_us(),
                });
            }
        }
    }

    unsafe impl SCStreamDelegate for Output {
        #[unsafe(method(stream:didStopWithError:))]
        fn stream_did_stop(&self, _stream: &SCStream, error: &NSError) {
            *lock(&self.ivars().error) = Some(error.localizedDescription().to_string());
        }
    }
);

impl Output {
    fn new(sink: AudioSink, error: Arc<Mutex<Option<String>>>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(OutputIvars {
            sink: Mutex::new(sink),
            error,
        });
        // SAFETY: NSObject's designated initialiser.
        unsafe { msg_send![super(this), init] }
    }
}

/// The sample's audio as interleaved stereo float, or `None` when it is not float PCM.
fn stereo_samples(sample: &CMSampleBuffer) -> Option<Vec<f32>> {
    // SAFETY: every pointer read below comes from CoreMedia for this live sample, and the buffer
    // list storage is sized and aligned as CoreMedia asked; the block buffer that owns the audio
    // bytes is held until the copy is made.
    unsafe {
        let format = sample.format_description()?;
        let description = CMAudioFormatDescriptionGetStreamBasicDescription(&format);
        let description = description.as_ref()?;
        if description.mFormatFlags & kAudioFormatFlagIsFloat == 0
            || description.mBitsPerChannel != 32
        {
            return None;
        }
        let non_interleaved = description.mFormatFlags & kAudioFormatFlagIsNonInterleaved != 0;

        let mut needed = 0usize;
        sample.audio_buffer_list_with_retained_block_buffer(
            &mut needed,
            ptr::null_mut(),
            0,
            None,
            None,
            ASSURE_16_BYTE_ALIGNMENT,
            ptr::null_mut(),
        );
        if needed < size_of::<AudioBufferList>() {
            return None;
        }
        // u64 words keep the list aligned for its pointer fields.
        let mut storage = vec![0u64; needed.div_ceil(size_of::<u64>())];
        debug_assert!(align_of::<AudioBufferList>() <= align_of::<u64>());
        let list = storage.as_mut_ptr().cast::<AudioBufferList>();
        let mut block: *mut CMBlockBuffer = ptr::null_mut();
        let status = sample.audio_buffer_list_with_retained_block_buffer(
            ptr::null_mut(),
            list,
            needed,
            None,
            None,
            ASSURE_16_BYTE_ALIGNMENT,
            &mut block,
        );
        if status != 0 {
            return None;
        }
        // Released when this function returns, after the samples are copied out.
        let _block = NonNull::new(block).map(|block| CFRetained::from_raw(block));

        let count = (*list).mNumberBuffers as usize;
        let first: *const AudioBuffer = ptr::addr_of!((*list).mBuffers).cast();
        let buffers: Vec<&[f32]> = (0..count)
            .filter_map(|index| {
                let buffer = &*first.add(index);
                let data = buffer.mData.cast::<f32>().cast_const();
                (!data.is_null()).then(|| {
                    std::slice::from_raw_parts(
                        data,
                        buffer.mDataByteSize as usize / size_of::<f32>(),
                    )
                })
            })
            .collect();
        Some(if non_interleaved {
            planar_to_stereo(&buffers)
        } else {
            buffers.first().map_or_else(Vec::new, |samples| {
                interleaved_to_stereo(samples, description.mChannelsPerFrame as usize)
            })
        })
    }
}

/// A running stream; stopping it (or dropping it) ends capture.
pub(super) struct Running {
    stream: Retained<SCStream>,
    display: Retained<SCDisplay>,
    _output: Retained<Output>,
    _queue: DispatchRetained<DispatchQueue>,
    stopped: bool,
}

// SAFETY: ScreenCaptureKit streams may be updated and stopped from any thread; the output object
// is only touched by the framework on its own queue.
unsafe impl Send for Running {}

impl Running {
    /// Points the stream at a new set of applications without restarting it.
    /// Points the stream at a new set of applications without restarting it, and waits for
    /// ScreenCaptureKit to accept or refuse the change so the caller knows which filter is live.
    pub(super) fn retarget(&self, audience: &Audience) -> Result<(), AudioError> {
        let filter = filter_for(&self.display, audience);
        let (tx, rx) = mpsc::sync_channel::<Result<(), AudioError>>(1);
        let done = completion(tx);
        // SAFETY: an ordinary update on a live stream with a live filter; the block is copied by
        // the framework and outlives the call.
        unsafe {
            self.stream
                .updateContentFilter_completionHandler(&filter, Some(&done))
        };
        rx.recv_timeout(UPDATE_TIMEOUT).unwrap_or_else(|_| {
            Err(AudioError::MacOs(format!(
                "the audio filter update was not confirmed within {UPDATE_TIMEOUT:?}"
            )))
        })
    }

    pub(super) fn stop(&mut self) {
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

fn filter_for(display: &SCDisplay, audience: &Audience) -> Retained<SCContentFilter> {
    let apps = |list: &[Shared<SCRunningApplication>]| {
        let refs: Vec<&SCRunningApplication> = list.iter().map(|app| &*app.0).collect();
        NSArray::from_slice(&refs)
    };
    // SAFETY: plain initialisers on live objects.
    unsafe {
        match audience {
            Audience::AllExcept(excluded) => {
                SCContentFilter::initWithDisplay_excludingApplications_exceptingWindows(
                    SCContentFilter::alloc(),
                    display,
                    &apps(excluded),
                    &NSArray::new(),
                )
            }
            Audience::Only(included) => {
                SCContentFilter::initWithDisplay_includingApplications_exceptingWindows(
                    SCContentFilter::alloc(),
                    display,
                    &apps(included),
                    &NSArray::new(),
                )
            }
        }
    }
}

/// Starts capturing what `audience` names, playing on any device, delivering to `sink`. Waits
/// until the system confirms the start or refuses it.
pub(super) fn start(
    content: &SCShareableContent,
    audience: &Audience,
    sink: AudioSink,
    error: Arc<Mutex<Option<String>>>,
    timeout: Duration,
) -> Result<Running, AudioError> {
    // SAFETY: every call below is an ordinary ScreenCaptureKit call on live objects; the blocks
    // and the output object outlive the calls that take them.
    unsafe {
        // A stream is always anchored to a display, though its audio is the whole system's.
        let display = content
            .displays()
            .firstObject()
            .ok_or_else(|| AudioError::MacOs("no display to anchor the audio stream to".into()))?;
        let filter = filter_for(&display, audience);

        let config = SCStreamConfiguration::new();
        config.setCapturesAudio(true);
        config.setExcludesCurrentProcessAudio(true);
        config.setSampleRate(AUDIO_SAMPLE_RATE as isize);
        config.setChannelCount(isize::from(AUDIO_CHANNELS));
        config.setWidth(VIDEO_SIZE);
        config.setHeight(VIDEO_SIZE);
        config.setMinimumFrameInterval(CMTime::new(1, VIDEO_FPS));
        config.setShowsCursor(false);

        let output = Output::new(sink, error);
        let stream = SCStream::initWithFilter_configuration_delegate(
            SCStream::alloc(),
            &filter,
            &config,
            Some(ProtocolObject::from_ref(&*output)),
        );
        let queue = DispatchQueue::new("brp.capture.audio", None);
        for kind in [SCStreamOutputType::Audio, SCStreamOutputType::Screen] {
            // The screen output only exists so the framework has somewhere to put the video
            // frames it produces anyway; the handler ignores them.
            stream
                .addStreamOutput_type_sampleHandlerQueue_error(
                    ProtocolObject::from_ref(&*output),
                    kind,
                    Some(&queue),
                )
                .map_err(|error| ns_error(&error))?;
        }
        let (tx, rx) = mpsc::sync_channel::<Result<(), AudioError>>(1);
        let started = completion(tx);
        stream.startCaptureWithCompletionHandler(Some(&started));
        let mut running = Running {
            stream,
            display,
            _output: output,
            _queue: queue,
            stopped: false,
        };
        match rx.recv_timeout(timeout) {
            Ok(Ok(())) => Ok(running),
            Ok(Err(error)) => {
                running.stopped = true;
                Err(error)
            }
            Err(_) => {
                running.stop();
                Err(AudioError::MacOs(format!(
                    "the audio stream did not start within {timeout:?}"
                )))
            }
        }
    }
}

fn completion(tx: SyncSender<Result<(), AudioError>>) -> RcBlock<dyn Fn(*mut NSError)> {
    RcBlock::new(move |error: *mut NSError| {
        // SAFETY: a non-null error is live for the duration of the call.
        let result = match unsafe { Retained::retain(error) } {
            Some(error) => Err(ns_error(&error)),
            None => Ok(()),
        };
        let _ = tx.try_send(result);
    })
}

pub(super) fn ns_error(error: &NSError) -> AudioError {
    AudioError::MacOs(error.localizedDescription().to_string())
}

/// The displays, windows and applications this process may capture.
pub(super) fn shareable(timeout: Duration) -> Result<Shared<SCShareableContent>, AudioError> {
    let (tx, rx) = mpsc::sync_channel::<Result<Shared<SCShareableContent>, AudioError>>(1);
    let handler = RcBlock::new(
        move |content: *mut SCShareableContent, error: *mut NSError| {
            // SAFETY: the handler receives either a live content object or a live error.
            let result = match unsafe { Retained::retain(content) } {
                Some(content) => Ok(Shared(content)),
                None => Err(unsafe { Retained::retain(error) }.map_or_else(
                    || AudioError::MacOs("no shareable content".into()),
                    |error| ns_error(&error),
                )),
            };
            let _ = tx.try_send(result);
        },
    );
    // SAFETY: the block is copied by the framework and outlives the call.
    unsafe { SCShareableContent::getShareableContentWithCompletionHandler(&handler) };
    rx.recv_timeout(timeout).unwrap_or_else(|_| {
        Err(AudioError::MacOs(
            "the system did not list shareable content in time".into(),
        ))
    })
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

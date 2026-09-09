//! Windows audio capture through WASAPI process loopback.

mod sessions;

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use brp_proto::constants::{
    AUDIO_CAPTURE_START_TIMEOUT, AUDIO_CHANNELS, AUDIO_PACKET_DURATION, AUDIO_SAMPLE_RATE,
    AUDIO_SESSION_POLL_INTERVAL,
};
use brp_proto::monotonic_us;
use wasapi::{
    AudioCaptureClient, AudioClient, Direction, Handle, SampleType, StreamMode, WaveFormat,
    initialize_mta,
};

use crate::chunk::{AudioCapture, AudioCaptureSession, AudioChunk, AudioSink};
use crate::error::AudioError;
use crate::mix::{CaptureMixer, CaptureSource};
use crate::selection::{AppKey, AudioSelection, AudioSource};

const EVENT_TIMEOUT_MS: u32 = 1000;

pub struct ProcessLoopbackCapture {
    process_id: u32,
}

impl ProcessLoopbackCapture {
    pub fn new(process_id: u32) -> Self {
        Self { process_id }
    }

    fn start_all(&self, sink: AudioSink) -> Result<Box<dyn AudioCaptureSession>, AudioError> {
        let stop = Arc::new(AtomicBool::new(false));
        let error = Arc::new(Mutex::new(None));
        let (ready_tx, ready_rx) = mpsc::channel();
        let (flag, error_slot, process_id) = (stop.clone(), error.clone(), self.process_id);
        let thread = thread::Builder::new()
            .name("brp-audio-wasapi".into())
            .spawn(move || {
                if let Err(capture_error) = run_client(ClientRequest {
                    process_id,
                    include_tree: false,
                    sink,
                    stop: flag,
                    ready: ready_tx.clone(),
                }) {
                    let message = capture_error.to_string();
                    let _ = ready_tx.send(Err(capture_error));
                    *lock(&error_slot) = Some(message);
                }
            })
            .map_err(|error| {
                AudioError::Windows(format!("failed to spawn the WASAPI thread: {error}"))
            })?;
        finish_start(stop, error, vec![thread], ready_rx)
    }

    fn start_selected(
        &self,
        selected: BTreeSet<AppKey>,
        sink: AudioSink,
    ) -> Result<Box<dyn AudioCaptureSession>, AudioError> {
        let stop = Arc::new(AtomicBool::new(false));
        let error = Arc::new(Mutex::new(None));
        let mixer = CaptureMixer::default();
        let (ready_tx, ready_rx) = mpsc::channel();

        let watch_stop = stop.clone();
        let watch_error = error.clone();
        let watch_mixer = mixer.clone();
        let process_id = self.process_id;
        let watcher = thread::Builder::new()
            .name("brp-audio-wasapi-watch".into())
            .spawn(move || {
                if let Err(capture_error) = watch_selected(
                    process_id,
                    selected,
                    watch_mixer,
                    watch_stop,
                    ready_tx.clone(),
                ) {
                    let message = capture_error.to_string();
                    let _ = ready_tx.send(Err(capture_error));
                    *lock(&watch_error) = Some(message);
                }
            })
            .map_err(|spawn_error| {
                AudioError::Windows(format!("failed to spawn the WASAPI watcher: {spawn_error}"))
            })?;

        let mix_stop = stop.clone();
        let output = match thread::Builder::new()
            .name("brp-audio-wasapi-mix".into())
            .spawn(move || mix_output(mixer, sink, mix_stop))
        {
            Ok(thread) => thread,
            Err(spawn_error) => {
                stop.store(true, Ordering::Relaxed);
                let _ = watcher.join();
                return Err(AudioError::Windows(format!(
                    "failed to spawn the capture mixer: {spawn_error}"
                )));
            }
        };

        finish_start(stop, error, vec![watcher, output], ready_rx)
    }
}

struct Session {
    stop: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
    threads: Vec<JoinHandle<()>>,
}

impl AudioCapture for ProcessLoopbackCapture {
    fn sources(&self) -> Result<Vec<AudioSource>, AudioError> {
        sessions::list_sources(self.process_id)
    }

    fn start(
        &self,
        selection: AudioSelection,
        sink: AudioSink,
    ) -> Result<Box<dyn AudioCaptureSession>, AudioError> {
        match selection {
            AudioSelection::All => self.start_all(sink),
            AudioSelection::Only(selected) => self.start_selected(selected, sink),
        }
    }
}

fn finish_start(
    stop: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
    threads: Vec<JoinHandle<()>>,
    ready: mpsc::Receiver<Result<(), AudioError>>,
) -> Result<Box<dyn AudioCaptureSession>, AudioError> {
    let failure = match ready.recv_timeout(AUDIO_CAPTURE_START_TIMEOUT) {
        Ok(Ok(())) => {
            return Ok(Box::new(Session {
                stop,
                error,
                threads,
            }));
        }
        Ok(Err(error)) => error,
        Err(RecvTimeoutError::Timeout) => AudioError::Windows(format!(
            "the capture client did not become ready within {AUDIO_CAPTURE_START_TIMEOUT:?}"
        )),
        Err(RecvTimeoutError::Disconnected) => {
            AudioError::Windows("WASAPI thread exited before reporting".into())
        }
    };
    stop.store(true, Ordering::Relaxed);
    for thread in threads {
        let _ = thread.join();
    }
    Err(failure)
}

fn watch_selected(
    process_id: u32,
    selected: BTreeSet<AppKey>,
    mixer: CaptureMixer,
    stop: Arc<AtomicBool>,
    ready: mpsc::Sender<Result<(), AudioError>>,
) -> Result<(), AudioError> {
    if selected.is_empty() {
        let _ = ready.send(Ok(()));
        while !stop.load(Ordering::Relaxed) {
            thread::sleep(AUDIO_SESSION_POLL_INTERVAL);
        }
        return Ok(());
    }
    initialize_mta()
        .ok()
        .map_err(|error| AudioError::Windows(format!("CoInitializeEx: {error}")))?;
    let mut clients = SelectedClients::default();
    clients.reconcile(process_id, &selected, &mixer, &stop)?;
    let _ = ready.send(Ok(()));

    while !stop.load(Ordering::Relaxed) {
        thread::sleep(AUDIO_SESSION_POLL_INTERVAL);
        if stop.load(Ordering::Relaxed) {
            continue;
        }
        if let Err(error) = clients.reconcile(process_id, &selected, &mixer, &stop) {
            tracing::warn!(%error, "could not reconcile selected Windows audio sessions");
        }
    }
    clients.stop();
    Ok(())
}

#[derive(Default)]
struct SelectedClients {
    running: HashMap<u32, ClientThread>,
    failed_roots: BTreeSet<u32>,
}

impl SelectedClients {
    fn reconcile(
        &mut self,
        process_id: u32,
        selected: &BTreeSet<AppKey>,
        mixer: &CaptureMixer,
        stop: &AtomicBool,
    ) -> Result<(), AudioError> {
        let desired = sessions::selected_roots(process_id, selected)?;
        let stopped: Vec<u32> = self
            .running
            .iter()
            .filter_map(|(root, client)| {
                (!desired.contains(root) || client.is_finished()).then_some(*root)
            })
            .collect();
        let stopped = stopped
            .into_iter()
            .filter_map(|root| self.running.remove(&root));
        stop_clients(stopped);
        self.failed_roots.retain(|root| desired.contains(root));

        let missing: Vec<u32> = desired
            .into_iter()
            .filter(|root| !self.running.contains_key(root))
            .collect();
        for root in missing {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            self.start(root, mixer);
        }
        Ok(())
    }

    fn start(&mut self, root: u32, mixer: &CaptureMixer) {
        match ClientThread::start(root, mixer.add_source(root)) {
            Ok(client) => {
                self.failed_roots.remove(&root);
                self.running.insert(root, client);
            }
            Err(error) => {
                if self.failed_roots.insert(root) {
                    tracing::warn!(root, %error, "could not capture selected process tree");
                }
            }
        }
    }

    fn stop(self) {
        stop_clients(self.running.into_values());
    }
}

struct ClientThread {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

impl ClientThread {
    fn start(process_id: u32, source: CaptureSource) -> Result<Self, AudioError> {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let (ready_tx, ready_rx) = mpsc::channel();
        let sink: AudioSink = Box::new(move |chunk| source.push(chunk));
        let thread = thread::Builder::new()
            .name(format!("brp-audio-wasapi-{process_id}"))
            .spawn(move || {
                if let Err(error) = run_client(ClientRequest {
                    process_id,
                    include_tree: true,
                    sink,
                    stop: flag,
                    ready: ready_tx.clone(),
                }) {
                    let message = error.to_string();
                    let _ = ready_tx.send(Err(error));
                    tracing::warn!(process_id, error = %message, "process-loopback client stopped");
                }
            })
            .map_err(|error| {
                AudioError::Windows(format!("failed to spawn process-loopback client: {error}"))
            })?;

        let failure = match ready_rx.recv_timeout(AUDIO_CAPTURE_START_TIMEOUT) {
            Ok(Ok(())) => return Ok(Self { stop, thread }),
            Ok(Err(error)) => error,
            Err(RecvTimeoutError::Timeout) => AudioError::Windows(format!(
                "process {process_id} did not become ready within {AUDIO_CAPTURE_START_TIMEOUT:?}"
            )),
            Err(RecvTimeoutError::Disconnected) => {
                AudioError::Windows(format!("process {process_id} exited before reporting"))
            }
        };
        stop.store(true, Ordering::Relaxed);
        let _ = thread.join();
        Err(failure)
    }

    fn is_finished(&self) -> bool {
        self.thread.is_finished()
    }
}

fn stop_clients(clients: impl IntoIterator<Item = ClientThread>) {
    let clients: Vec<ClientThread> = clients.into_iter().collect();
    for client in &clients {
        client.stop.store(true, Ordering::Relaxed);
    }
    for client in clients {
        let _ = client.thread.join();
    }
}

fn mix_output(mixer: CaptureMixer, mut sink: AudioSink, stop: Arc<AtomicBool>) {
    let mut next = Instant::now() + AUDIO_PACKET_DURATION;
    while !stop.load(Ordering::Relaxed) {
        let now = Instant::now();
        if next > now {
            thread::sleep(next - now);
        } else {
            next = now;
        }
        next += AUDIO_PACKET_DURATION;
        if stop.load(Ordering::Relaxed) {
            break;
        }
        if let Some(chunk) = mixer.mix() {
            sink(chunk);
        }
    }
}

struct ClientRequest {
    process_id: u32,
    include_tree: bool,
    sink: AudioSink,
    stop: Arc<AtomicBool>,
    ready: mpsc::Sender<Result<(), AudioError>>,
}

struct ConfiguredClient {
    client: AudioClient,
    capture: AudioCaptureClient,
    event: Handle,
}

fn configure_client(process_id: u32, include_tree: bool) -> Result<ConfiguredClient, AudioError> {
    let mut client = AudioClient::new_application_loopback_client(process_id, include_tree)
        .map_err(|error| {
            AudioError::Unsupported(format!("process loopback activation: {error}"))
        })?;
    let format = WaveFormat::new(
        32,
        32,
        &SampleType::Float,
        AUDIO_SAMPLE_RATE as usize,
        AUDIO_CHANNELS as usize,
        None,
    );
    let mode = StreamMode::EventsShared {
        autoconvert: true,
        buffer_duration_hns: 0,
    };
    client
        .initialize_client(&format, &Direction::Capture, &mode)
        .map_err(|error| AudioError::Format(format!("IAudioClient::Initialize: {error}")))?;
    let event = client
        .set_get_eventhandle()
        .map_err(|error| AudioError::Windows(format!("SetEventHandle: {error}")))?;
    let capture = client.get_audiocaptureclient().map_err(|error| {
        AudioError::Windows(format!("GetService(IAudioCaptureClient): {error}"))
    })?;
    Ok(ConfiguredClient {
        client,
        capture,
        event,
    })
}

fn run_client(mut request: ClientRequest) -> Result<(), AudioError> {
    initialize_mta()
        .ok()
        .map_err(|error| AudioError::Windows(format!("CoInitializeEx: {error}")))?;
    let configured = configure_client(request.process_id, request.include_tree)?;
    configured
        .client
        .start_stream()
        .map_err(|error| AudioError::Windows(format!("Start: {error}")))?;
    let _ = request.ready.send(Ok(()));

    let mut bytes = VecDeque::new();
    while !request.stop.load(Ordering::Relaxed) {
        if let Some(chunk) = read_chunk(&configured.capture, &mut bytes)? {
            (request.sink)(chunk);
        }
        let _ = configured.event.wait_for_event(EVENT_TIMEOUT_MS);
    }
    configured
        .client
        .stop_stream()
        .map_err(|error| AudioError::Windows(format!("Stop: {error}")))?;
    Ok(())
}

fn read_chunk(
    capture: &AudioCaptureClient,
    bytes: &mut VecDeque<u8>,
) -> Result<Option<AudioChunk>, AudioError> {
    let info = capture
        .read_from_device_to_deque(bytes)
        .map_err(|error| AudioError::Windows(format!("GetBuffer: {error}")))?;
    let frame_bytes = AUDIO_CHANNELS as usize * 4;
    let whole = bytes.len() - bytes.len() % frame_bytes;
    if whole == 0 {
        return Ok(None);
    }
    let drained = bytes.drain(..whole).collect::<Vec<u8>>();
    let (frames, _) = drained.as_chunks::<4>();
    let mut samples: Vec<f32> = frames
        .iter()
        .map(|bytes| f32::from_le_bytes(*bytes))
        .collect();
    if info.flags.silent {
        samples.fill(0.0);
    }
    Ok(Some(AudioChunk {
        samples,
        capture_ts_us: monotonic_us(),
    }))
}

impl AudioCaptureSession for Session {
    fn error(&self) -> Option<String> {
        lock(&self.error).clone()
    }

    fn stop(mut self: Box<Self>) {
        self.stop.store(true, Ordering::Relaxed);
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

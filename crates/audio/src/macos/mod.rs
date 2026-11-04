//! macOS audio capture through ScreenCaptureKit: everything the machine plays except brp, or only
//! the applications a selection names, as 48 kHz stereo float.
//!
//! It shares the Screen & System Audio Recording permission with screen capture.

mod processes;
mod stream;

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};

use brp_proto::constants::{
    AUDIO_CAPTURE_START_TIMEOUT, AUDIO_SESSION_POLL_INTERVAL, AUDIO_SOURCE_LIST_TIMEOUT,
};
use objc2_screen_capture_kit::{SCRunningApplication, SCShareableContent};

use self::stream::{Audience, Running, Shared, shareable};
use crate::chunk::{AudioCapture, AudioCaptureSession, AudioSink};
use crate::error::AudioError;
use crate::macos_model::{
    AppIds, AppliedTarget, AudioProcess, audible_sources, excluded_pids, selected_pids,
};
use crate::selection::{AppKey, AudioSelection, AudioSource};

pub struct ScreenCaptureAudio {
    process_id: u32,
}

impl ScreenCaptureAudio {
    pub fn new(process_id: u32) -> Self {
        Self { process_id }
    }

    fn own_pid(&self) -> i32 {
        self.process_id as i32
    }
}

impl AudioCapture for ScreenCaptureAudio {
    fn sources(&self) -> Result<Vec<AudioSource>, AudioError> {
        let processes = match processes::audio_processes() {
            Ok(processes) => processes,
            // Before macOS 14.2 there are no process objects; the running applications with a
            // window stand in for the audible ones.
            Err(error) => {
                tracing::debug!(%error, "no CoreAudio process list; listing running applications");
                running_applications(AUDIO_SOURCE_LIST_TIMEOUT)?
            }
        };
        Ok(audible_sources(processes, self.own_pid()))
    }

    fn start(
        &self,
        selection: AudioSelection,
        sink: AudioSink,
    ) -> Result<Box<dyn AudioCaptureSession>, AudioError> {
        let own_pid = self.own_pid();
        let content = shareable(AUDIO_CAPTURE_START_TIMEOUT)?;
        let error = Arc::new(Mutex::new(None));
        let audience = audience(&content.0, &selection, own_pid);
        let running = stream::start(
            &content.0,
            &audience,
            sink,
            error.clone(),
            AUDIO_CAPTURE_START_TIMEOUT,
        )?;
        let running = Arc::new(Mutex::new(running));
        let stop = Arc::new(AtomicBool::new(false));
        // Under `Only`, an application launched after the start must still be heard: the
        // watcher re-reads the running applications and retargets the stream when they change.
        let watcher = match selection {
            AudioSelection::All => None,
            AudioSelection::Only(selected) => {
                // The stream started with the selection as it stood then, so the watcher begins
                // from that set rather than re-applying it.
                let ids: Vec<AppIds> = applications(&content.0)
                    .into_iter()
                    .map(|(ids, _)| ids)
                    .collect();
                let initial = selected_pids(&ids, &selected, own_pid);
                Some(spawn_watcher(
                    selected,
                    initial,
                    own_pid,
                    running.clone(),
                    stop.clone(),
                )?)
            }
        };
        tracing::info!("macos audio capture started");
        Ok(Box::new(Session {
            running,
            stop,
            error,
            watcher,
        }))
    }
}

/// The applications `selection` names among those running now, or everything but OpenStream.
fn audience(content: &SCShareableContent, selection: &AudioSelection, own_pid: i32) -> Audience {
    let apps = applications(content);
    match selection {
        AudioSelection::All => {
            let ids: Vec<AppIds> = apps.iter().map(|(ids, _)| ids.clone()).collect();
            Audience::AllExcept(pick(apps, &excluded_pids(&ids, own_pid)))
        }
        AudioSelection::Only(selected) => {
            let ids: Vec<AppIds> = apps.iter().map(|(ids, _)| ids.clone()).collect();
            Audience::Only(pick(apps, &selected_pids(&ids, selected, own_pid)))
        }
    }
}

fn pick(
    apps: Vec<(AppIds, Shared<SCRunningApplication>)>,
    pids: &BTreeSet<i32>,
) -> Vec<Shared<SCRunningApplication>> {
    apps.into_iter()
        .filter(|(ids, _)| pids.contains(&ids.pid))
        .map(|(_, app)| app)
        .collect()
}

/// Every running application with its identity.
fn applications(content: &SCShareableContent) -> Vec<(AppIds, Shared<SCRunningApplication>)> {
    // SAFETY: plain property reads on objects the content keeps alive.
    unsafe {
        content
            .applications()
            .iter()
            .map(|app| {
                let ids = AppIds {
                    pid: app.processID(),
                    bundle_id: app.bundleIdentifier().to_string(),
                    name: app.applicationName().to_string(),
                };
                (ids, Shared(app))
            })
            .collect()
    }
}

/// The fallback source list: applications that own a window, all reported as playing.
fn running_applications(timeout: std::time::Duration) -> Result<Vec<AudioProcess>, AudioError> {
    let content = shareable(timeout)?;
    // SAFETY: plain property reads on objects the content keeps alive.
    let with_windows: BTreeSet<i32> = unsafe {
        content
            .0
            .windows()
            .iter()
            .filter_map(|window| window.owningApplication().map(|app| app.processID()))
            .collect()
    };
    Ok(applications(&content.0)
        .into_iter()
        .filter(|(ids, _)| with_windows.contains(&ids.pid))
        .map(|(ids, _)| AudioProcess {
            pid: ids.pid,
            label: Some(ids.name),
            bundle_id: Some(ids.bundle_id),
            running_output: true,
        })
        .collect())
}

fn spawn_watcher(
    selected: BTreeSet<AppKey>,
    initial: BTreeSet<i32>,
    own_pid: i32,
    running: Arc<Mutex<Running>>,
    stop: Arc<AtomicBool>,
) -> Result<JoinHandle<()>, AudioError> {
    thread::Builder::new()
        .name("brp-audio-sck-watch".into())
        .spawn(move || {
            let mut target = AppliedTarget::default();
            target.record(initial, true);
            while !stop.load(Ordering::Relaxed) {
                thread::sleep(AUDIO_SESSION_POLL_INTERVAL);
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                let content = match shareable(AUDIO_SESSION_POLL_INTERVAL) {
                    Ok(content) => content,
                    Err(failure) => {
                        tracing::warn!(error = %failure, "could not re-list running applications");
                        continue;
                    }
                };
                let apps = applications(&content.0);
                let ids: Vec<AppIds> = apps.iter().map(|(ids, _)| ids.clone()).collect();
                let pids = selected_pids(&ids, &selected, own_pid);
                if !target.wants(&pids) {
                    continue;
                }
                // One update at a time: the watcher is the only caller, and it waits for the
                // answer before recording which filter is live.
                let audience = Audience::Only(pick(apps, &pids));
                let outcome = lock(&running).retarget(&audience);
                match &outcome {
                    Ok(()) => tracing::debug!(?pids, "retargeted the audio stream"),
                    Err(failure) => {
                        tracing::warn!(error = %failure, ?pids, "audio filter update failed; retrying")
                    }
                }
                target.record(pids, outcome.is_ok());
            }
        })
        .map_err(|spawn_error| {
            AudioError::MacOs(format!("failed to spawn the audio watcher: {spawn_error}"))
        })
}

struct Session {
    running: Arc<Mutex<Running>>,
    stop: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
    watcher: Option<JoinHandle<()>>,
}

impl AudioCaptureSession for Session {
    fn error(&self) -> Option<String> {
        lock(&self.error).clone()
    }

    fn stop(mut self: Box<Self>) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(watcher) = self.watcher.take() {
            let _ = watcher.join();
        }
        lock(&self.running).stop();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

//! A sine tone for tests, chunked like a real backend would deliver it.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use brp_proto::constants::{AUDIO_CHANNELS, AUDIO_SAMPLE_RATE};
use brp_proto::monotonic_us;

use crate::chunk::{AudioCapture, AudioCaptureSession, AudioChunk, AudioSink};
use crate::error::AudioError;
use crate::selection::{AppKey, AudioSelection, AudioSource};

const CHUNK: Duration = Duration::from_millis(10);

#[derive(Debug, Clone, Copy)]
pub struct SyntheticTone {
    pub frequency_hz: f32,
    pub amplitude: f32,
}

struct Session {
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl SyntheticTone {
    /// The identity the tone answers to: a selection naming it is audible.
    pub fn tone_key() -> AppKey {
        AppKey::new("tone")
    }

    /// A second listed identity that never produces samples, so a test can select something the
    /// platform reports and still hear nothing.
    pub fn silent_key() -> AppKey {
        AppKey::new("silent")
    }
}

impl AudioCapture for SyntheticTone {
    fn sources(&self) -> Result<Vec<AudioSource>, AudioError> {
        Ok(vec![
            AudioSource {
                key: Self::tone_key(),
                label: "Tone".into(),
            },
            AudioSource {
                key: Self::silent_key(),
                label: "Silent".into(),
            },
        ])
    }

    fn start(
        &self,
        selection: AudioSelection,
        mut sink: AudioSink,
    ) -> Result<Box<dyn AudioCaptureSession>, AudioError> {
        let audible = selection.admits(Some(&Self::tone_key()));
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let tone = *self;
        let per_chunk = (AUDIO_SAMPLE_RATE as u64 * CHUNK.as_millis() as u64 / 1000) as usize;
        let thread = thread::spawn(move || {
            let started = Instant::now();
            let mut index = 0u32;
            let mut phase = 0.0f32;
            let step = std::f32::consts::TAU * tone.frequency_hz / AUDIO_SAMPLE_RATE as f32;
            while !flag.load(Ordering::Relaxed) {
                if let Some(wait) = (started + CHUNK * index).checked_duration_since(Instant::now())
                {
                    thread::sleep(wait);
                }
                let mut samples = Vec::with_capacity(per_chunk * AUDIO_CHANNELS as usize);
                for _ in 0..per_chunk {
                    let value = tone.amplitude * phase.sin();
                    phase = (phase + step) % std::f32::consts::TAU;
                    samples.extend_from_slice(&[value, value]);
                }
                // A selection that does not name the tone leaves the stream idle with no chunks
                // and no error, exactly as a silent desktop does.
                if audible {
                    sink(AudioChunk {
                        samples,
                        capture_ts_us: monotonic_us(),
                    });
                }
                index = index.wrapping_add(1);
            }
        });
        Ok(Box::new(Session {
            stop,
            thread: Some(thread),
        }))
    }
}

impl AudioCaptureSession for Session {
    fn error(&self) -> Option<String> {
        None
    }
    fn stop(mut self: Box<Self>) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[test]
    fn the_tone_arrives_in_ten_millisecond_stereo_chunks_and_stops_on_stop() {
        let chunks = Arc::new(Mutex::new(Vec::new()));
        let sink_chunks = chunks.clone();
        let session = SyntheticTone {
            frequency_hz: 440.0,
            amplitude: 0.5,
        }
        .start(
            AudioSelection::All,
            Box::new(move |c| sink_chunks.lock().unwrap().push(c)),
        )
        .unwrap();
        thread::sleep(Duration::from_millis(120));
        assert!(session.error().is_none());
        session.stop();
        let count = chunks.lock().unwrap().len();
        thread::sleep(Duration::from_millis(30));
        assert_eq!(chunks.lock().unwrap().len(), count, "chunks after stop");
        let chunks = chunks.lock().unwrap();
        assert!(chunks.len() >= 8, "only {} chunks in 120 ms", chunks.len());
        for chunk in chunks.iter() {
            assert_eq!(chunk.samples.len(), 960);
            assert!(chunk.samples.iter().any(|s| s.abs() > 0.1));
        }
        assert!(
            chunks
                .windows(2)
                .all(|w| w[1].capture_ts_us > w[0].capture_ts_us)
        );
    }

    #[test]
    fn the_tone_lists_itself_and_a_silent_neighbour() {
        let tone = SyntheticTone {
            frequency_hz: 440.0,
            amplitude: 0.5,
        };
        let listed: Vec<(String, String)> = tone
            .sources()
            .unwrap()
            .into_iter()
            .map(|source| (source.key.as_str().to_string(), source.label))
            .collect();
        assert_eq!(
            listed,
            [
                ("tone".to_string(), "Tone".to_string()),
                ("silent".to_string(), "Silent".to_string()),
            ]
        );
    }

    #[test]
    fn a_selection_without_the_tone_delivers_no_chunks() {
        let chunks = Arc::new(Mutex::new(Vec::new()));
        let sink_chunks = chunks.clone();
        let session = SyntheticTone {
            frequency_hz: 440.0,
            amplitude: 0.5,
        }
        .start(
            AudioSelection::Only(std::collections::BTreeSet::from([
                SyntheticTone::silent_key(),
            ])),
            Box::new(move |c| sink_chunks.lock().unwrap().push(c)),
        )
        .unwrap();
        thread::sleep(Duration::from_millis(120));
        assert!(
            session.error().is_none(),
            "an idle selection is not a failure"
        );
        session.stop();
        assert!(chunks.lock().unwrap().is_empty());
    }

    #[test]
    fn a_selection_naming_the_tone_delivers_chunks() {
        let chunks = Arc::new(Mutex::new(Vec::new()));
        let sink_chunks = chunks.clone();
        let session = SyntheticTone {
            frequency_hz: 440.0,
            amplitude: 0.5,
        }
        .start(
            AudioSelection::Only(std::collections::BTreeSet::from(
                [SyntheticTone::tone_key()],
            )),
            Box::new(move |c| sink_chunks.lock().unwrap().push(c)),
        )
        .unwrap();
        thread::sleep(Duration::from_millis(120));
        session.stop();
        assert!(!chunks.lock().unwrap().is_empty());
    }
}

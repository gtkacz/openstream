//! Capture-side mixer for Windows include-tree process-loopback clients.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use brp_proto::constants::{
    AUDIO_CHANNELS, AUDIO_PACKET_DURATION, AUDIO_SAMPLE_RATE, CAPTURE_MIX_MAX_LAG,
};
#[cfg(windows)]
use brp_proto::monotonic_us;

use crate::AudioChunk;

#[derive(Default)]
struct SourceState {
    samples: VecDeque<f32>,
    front_ts_us: Option<u64>,
    underruns: u64,
}

impl SourceState {
    fn push(&mut self, chunk: AudioChunk) {
        let channels = AUDIO_CHANNELS as usize;
        let whole = chunk.samples.len() - chunk.samples.len() % channels;
        if whole == 0 {
            return;
        }
        if self.samples.is_empty() {
            self.front_ts_us = Some(chunk.capture_ts_us);
        }
        self.samples.extend(chunk.samples.into_iter().take(whole));

        let excess = self.samples.len().saturating_sub(max_lag_samples());
        if excess > 0 {
            self.samples.drain(..excess);
            self.advance_timestamp(excess);
        }
    }

    fn advance_timestamp(&mut self, samples: usize) {
        let frames = samples / AUDIO_CHANNELS as usize;
        if let Some(timestamp) = self.front_ts_us.as_mut() {
            *timestamp = timestamp.saturating_add(
                (frames as u64).saturating_mul(1_000_000) / AUDIO_SAMPLE_RATE as u64,
            );
        }
        if self.samples.is_empty() {
            self.front_ts_us = None;
        }
    }
}

#[derive(Default)]
struct MixerState {
    sources: BTreeMap<u32, SourceState>,
}

#[derive(Clone, Default)]
pub(crate) struct CaptureMixer {
    inner: Arc<Mutex<MixerState>>,
}

impl CaptureMixer {
    pub(crate) fn add_source(&self, id: u32) -> CaptureSource {
        lock(&self.inner).sources.insert(id, SourceState::default());
        CaptureSource {
            id,
            mixer: self.clone(),
        }
    }

    #[cfg(windows)]
    pub(crate) fn mix(&self) -> Option<AudioChunk> {
        self.mix_at(monotonic_us())
    }

    fn mix_at(&self, fallback_ts_us: u64) -> Option<AudioChunk> {
        let mut state = lock(&self.inner);
        if state.sources.is_empty() {
            return None;
        }

        let quantum = quantum_samples();
        let mut samples = vec![0.0; quantum];
        let mut capture_ts_us = None;
        for source in state.sources.values_mut() {
            if source.samples.len() < quantum {
                source.underruns = source.underruns.saturating_add(1);
            }
            if let Some(timestamp) = source.front_ts_us {
                capture_ts_us =
                    Some(capture_ts_us.map_or(timestamp, |current: u64| current.min(timestamp)));
            }
            let available = quantum.min(source.samples.len());
            for output in samples.iter_mut().take(available) {
                *output += source.samples.pop_front().expect("length checked");
            }
            source.advance_timestamp(available);
        }
        for sample in &mut samples {
            *sample = sample.clamp(-1.0, 1.0);
        }

        Some(AudioChunk {
            samples,
            capture_ts_us: capture_ts_us.unwrap_or(fallback_ts_us),
        })
    }

    #[cfg(test)]
    fn underruns(&self, id: u32) -> u64 {
        lock(&self.inner)
            .sources
            .get(&id)
            .map(|source| source.underruns)
            .unwrap_or(0)
    }
}

pub(crate) struct CaptureSource {
    id: u32,
    mixer: CaptureMixer,
}

impl CaptureSource {
    pub(crate) fn push(&self, chunk: AudioChunk) {
        if let Some(source) = lock(&self.mixer.inner).sources.get_mut(&self.id) {
            source.push(chunk);
        }
    }
}

impl Drop for CaptureSource {
    fn drop(&mut self) {
        lock(&self.mixer.inner).sources.remove(&self.id);
    }
}

fn quantum_samples() -> usize {
    (AUDIO_SAMPLE_RATE as u128 * AUDIO_PACKET_DURATION.as_millis() / 1000) as usize
        * AUDIO_CHANNELS as usize
}

fn max_lag_samples() -> usize {
    (AUDIO_SAMPLE_RATE as u128 * CAPTURE_MIX_MAX_LAG.as_millis() / 1000) as usize
        * AUDIO_CHANNELS as usize
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(value: f32, frames: usize, timestamp: u64) -> AudioChunk {
        AudioChunk {
            samples: vec![value; frames * AUDIO_CHANNELS as usize],
            capture_ts_us: timestamp,
        }
    }

    #[test]
    fn aligned_sources_sum_and_use_the_earliest_timestamp() {
        let mixer = CaptureMixer::default();
        let a = mixer.add_source(1);
        let b = mixer.add_source(2);
        let frames = quantum_samples() / AUDIO_CHANNELS as usize;
        a.push(chunk(0.25, frames, 2_000));
        b.push(chunk(0.5, frames, 1_000));

        let mixed = mixer.mix_at(9_000).unwrap();

        assert!(mixed.samples.iter().all(|sample| *sample == 0.75));
        assert_eq!(mixed.capture_ts_us, 1_000);
    }

    #[test]
    fn a_starved_source_contributes_silence_and_counts_an_underrun() {
        let mixer = CaptureMixer::default();
        let a = mixer.add_source(1);
        let _starved = mixer.add_source(2);
        let frames = quantum_samples() / AUDIO_CHANNELS as usize;
        a.push(chunk(0.5, frames, 1_000));

        let mixed = mixer.mix_at(9_000).unwrap();

        assert!(mixed.samples.iter().all(|sample| *sample == 0.5));
        assert_eq!(mixer.underruns(1), 0);
        assert_eq!(mixer.underruns(2), 1);
    }

    #[test]
    fn lagging_input_is_trimmed_to_the_maximum_lag() {
        let mixer = CaptureMixer::default();
        let source = mixer.add_source(1);
        let capacity_frames = max_lag_samples() / AUDIO_CHANNELS as usize;
        source.push(chunk(0.25, capacity_frames, 1_000));
        source.push(chunk(0.75, capacity_frames, 41_000));

        let first = mixer.mix_at(99_000).unwrap();
        let second = mixer.mix_at(99_000).unwrap();

        assert!(first.samples.iter().all(|sample| *sample == 0.75));
        assert!(second.samples.iter().all(|sample| *sample == 0.75));
        assert_eq!(first.capture_ts_us, 41_000);
    }

    #[test]
    fn dropping_the_last_handle_removes_the_source() {
        let mixer = CaptureMixer::default();
        let source = mixer.add_source(1);
        drop(source);

        assert!(mixer.mix_at(1_000).is_none());
    }
}

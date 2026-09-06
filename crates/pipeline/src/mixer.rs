//! Sums one track per publisher into the output buffer with a gain each and a master mute.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, TryLockError};

use brp_audio::RenderFn;
use brp_proto::constants::{AUDIO_CHANNELS, AUDIO_SAMPLE_RATE, MIXER_TRACK_CAPACITY};

pub type TrackKey = [u8; 32];

struct TrackInner {
    buffer: Mutex<VecDeque<f32>>,
    gain_bits: AtomicU32,
    underruns: AtomicU64,
}

/// The decode thread's handle to one publisher's ring buffer.
#[derive(Clone)]
pub struct Track {
    inner: Arc<TrackInner>,
}

impl Track {
    /// Interleaved samples held per track: the jitter maximum plus scheduling slack.
    pub fn capacity() -> usize {
        (AUDIO_SAMPLE_RATE as u128 * MIXER_TRACK_CAPACITY.as_millis() / 1000) as usize
            * AUDIO_CHANNELS as usize
    }

    fn new(gain: f32) -> Self {
        Self {
            inner: Arc::new(TrackInner {
                buffer: Mutex::new(VecDeque::with_capacity(Self::capacity())),
                gain_bits: AtomicU32::new(gain.to_bits()),
                underruns: AtomicU64::new(0),
            }),
        }
    }

    /// Appends samples, dropping the oldest beyond the capacity so a drifting clock costs a
    /// glitch rather than unbounded latency.
    pub fn push(&self, samples: &[f32]) {
        let mut buffer = lock(&self.inner.buffer);
        buffer.extend(samples.iter().copied());
        let excess = buffer.len().saturating_sub(Self::capacity());
        if excess > 0 {
            buffer.drain(..excess);
        }
    }

    pub fn queued(&self) -> usize {
        lock(&self.inner.buffer).len()
    }

    pub fn underruns(&self) -> u64 {
        self.inner.underruns.load(Ordering::Relaxed)
    }

    fn gain(&self) -> f32 {
        f32::from_bits(self.inner.gain_bits.load(Ordering::Relaxed))
    }

    fn set_gain(&self, gain: f32) {
        self.inner
            .gain_bits
            .store(gain.to_bits(), Ordering::Relaxed);
    }
}

#[derive(Default)]
struct MixerState {
    tracks: HashMap<TrackKey, Track>,
    /// Gains outlive tracks so a slider set before a watch goes live, or across a reconnect, holds.
    gains: HashMap<TrackKey, f32>,
}

#[derive(Default)]
struct MixerInner {
    // One lock for both maps: add_track reads the remembered gain and inserts the track as a
    // single critical section, so a concurrent set_gain can never land between the read and the
    // insert and go unseen by the new track.
    state: Mutex<MixerState>,
    muted: AtomicBool,
}

#[derive(Clone, Default)]
pub struct Mixer {
    inner: Arc<MixerInner>,
}

impl Mixer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_track(&self, key: TrackKey) -> Track {
        let mut state = lock(&self.inner.state);
        let gain = state.gains.get(&key).copied().unwrap_or(1.0);
        state
            .tracks
            .entry(key)
            .or_insert_with(|| Track::new(gain))
            .clone()
    }

    pub fn remove_track(&self, key: &TrackKey) {
        lock(&self.inner.state).tracks.remove(key);
    }

    pub fn set_gain(&self, key: TrackKey, gain: f32) {
        let gain = gain.clamp(0.0, 1.0);
        let mut state = lock(&self.inner.state);
        state.gains.insert(key, gain);
        if let Some(track) = state.tracks.get(&key) {
            track.set_gain(gain);
        }
    }

    pub fn gain(&self, key: &TrackKey) -> f32 {
        lock(&self.inner.state)
            .gains
            .get(key)
            .copied()
            .unwrap_or(1.0)
    }

    pub fn set_muted(&self, muted: bool) {
        self.inner.muted.store(muted, Ordering::Relaxed);
    }

    pub fn muted(&self) -> bool {
        self.inner.muted.load(Ordering::Relaxed)
    }

    pub fn underruns(&self, key: &TrackKey) -> u64 {
        lock(&self.inner.state)
            .tracks
            .get(key)
            .map(Track::underruns)
            .unwrap_or(0)
    }

    /// Fills `out` with the mix. Runs on the device thread, so nothing here ever waits on a lock:
    /// the mixer state and every track buffer are only tried, and a contended one contributes
    /// silence for that callback and counts an underrun.
    pub fn render(&self, out: &mut [f32]) {
        out.fill(0.0);
        let Some(state) = try_lock(&self.inner.state) else {
            return;
        };
        let muted = self.muted();
        for track in state.tracks.values() {
            let gain = track.gain();
            let Some(mut buffer) = try_lock(&track.inner.buffer) else {
                track.inner.underruns.fetch_add(1, Ordering::Relaxed);
                continue;
            };
            if buffer.len() < out.len() {
                // Short of a full callback: play what there is and let the rest stay silent, so a
                // track that is merely behind keeps its audio instead of losing it.
                track.inner.underruns.fetch_add(1, Ordering::Relaxed);
            }
            for sample in out.iter_mut() {
                let Some(value) = buffer.pop_front() else {
                    break;
                };
                if !muted {
                    *sample += value * gain;
                }
            }
        }
        for sample in out.iter_mut() {
            *sample = sample.clamp(-1.0, 1.0);
        }
    }

    pub fn render_fn(&self) -> RenderFn {
        let mixer = self.clone();
        Box::new(move |out| mixer.render(out))
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// `lock`'s device-thread sibling: recovers a poisoned mutex like it does, but never waits.
fn try_lock<T>(mutex: &Mutex<T>) -> Option<std::sync::MutexGuard<'_, T>> {
    match mutex.try_lock() {
        Ok(guard) => Some(guard),
        Err(TryLockError::Poisoned(poisoned)) => Some(poisoned.into_inner()),
        Err(TryLockError::WouldBlock) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: TrackKey = [1; 32];
    const B: TrackKey = [2; 32];

    #[test]
    fn tracks_sum_with_their_gains_and_the_output_is_clamped() {
        let mixer = Mixer::new();
        let a = mixer.add_track(A);
        let b = mixer.add_track(B);
        mixer.set_gain(B, 0.5);
        a.push(&[0.4; 4]);
        b.push(&[0.4; 4]);
        let mut out = [0.0; 4];
        mixer.render(&mut out);
        assert!(
            out.iter().all(|s| (*s - 0.6).abs() < 1e-6),
            "0.4 + 0.2 sums to 0.6"
        );
        assert_eq!(mixer.gain(&B), 0.5);
        assert_eq!(mixer.gain(&A), 1.0);

        a.push(&[0.8; 4]);
        b.push(&[0.8; 4]);
        mixer.render(&mut out);
        assert!(
            out.iter().all(|s| (*s - 1.0).abs() < 1e-6),
            "0.8 + 0.4 clamps to 1.0"
        );
    }

    #[test]
    fn set_gain_on_a_live_track_reaches_the_output() {
        let mixer = Mixer::new();
        let a = mixer.add_track(A);
        a.push(&[1.0; 4]);
        mixer.set_gain(A, 0.25);
        let mut out = [0.0; 4];
        mixer.render(&mut out);
        assert!(out.iter().all(|s| (*s - 0.25).abs() < 1e-6));
    }

    #[test]
    fn a_short_track_contributes_what_it_has_and_counts_one_underrun() {
        let mixer = Mixer::new();
        let a = mixer.add_track(A);
        a.push(&[0.5; 2]);
        let mut out = [0.0; 4];
        mixer.render(&mut out);
        assert_eq!(
            out,
            [0.5, 0.5, 0.0, 0.0],
            "the tail is zero-filled, not dropped"
        );
        assert_eq!(mixer.underruns(&A), 1);
        assert_eq!(a.queued(), 0);
    }

    #[test]
    fn a_contended_track_renders_silence_and_counts_an_underrun() {
        let mixer = Mixer::new();
        let a = mixer.add_track(A);
        a.push(&[0.5; 4]);
        let held = a.inner.buffer.lock().unwrap();
        let mut out = [0.0; 4];
        mixer.render(&mut out);
        assert_eq!(out, [0.0; 4]);
        assert_eq!(mixer.underruns(&A), 1);
        drop(held);
        mixer.render(&mut out);
        assert!(
            out.iter().all(|s| (*s - 0.5).abs() < 1e-6),
            "the samples waited for the next callback"
        );
    }

    #[test]
    fn the_master_mute_silences_everything_but_keeps_consuming() {
        let mixer = Mixer::new();
        let a = mixer.add_track(A);
        a.push(&[0.5; 8]);
        mixer.set_muted(true);
        let mut out = [1.0; 4];
        mixer.render(&mut out);
        assert_eq!(out, [0.0; 4]);
        assert_eq!(a.queued(), 4);
        assert!(mixer.muted());
    }

    #[test]
    fn gains_survive_track_removal_and_are_clamped() {
        let mixer = Mixer::new();
        mixer.set_gain(A, 1.7);
        assert_eq!(mixer.gain(&A), 1.0);
        mixer.set_gain(A, 0.3);
        let a = mixer.add_track(A);
        a.push(&[1.0; 2]);
        let mut out = [0.0; 2];
        mixer.render(&mut out);
        assert!((out[0] - 0.3).abs() < 1e-6);
        mixer.remove_track(&A);
        assert_eq!(mixer.gain(&A), 0.3);
        assert!(mixer.add_track(A).queued() == 0);
    }

    #[test]
    fn a_track_keeps_only_the_newest_half_second() {
        let mixer = Mixer::new();
        let a = mixer.add_track(A);
        let capacity = Track::capacity();
        a.push(&vec![0.1; capacity]);
        a.push(&[0.9; 2]);
        assert_eq!(a.queued(), capacity);
        let mut out = vec![0.0; capacity];
        mixer.render(&mut out);
        assert!((out[capacity - 1] - 0.9).abs() < 1e-6);
        assert!((out[0] - 0.1).abs() < 1e-6);
    }
}

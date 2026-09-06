//! Sums one track per publisher into the output buffer with a gain each and a master mute.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

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
struct MixerInner {
    tracks: Mutex<HashMap<TrackKey, Track>>,
    /// Gains outlive tracks so a slider set before a watch goes live, or across a reconnect, holds.
    gains: Mutex<HashMap<TrackKey, f32>>,
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
        let gain = self.gain(&key);
        lock(&self.inner.tracks)
            .entry(key)
            .or_insert_with(|| Track::new(gain))
            .clone()
    }

    pub fn remove_track(&self, key: &TrackKey) {
        lock(&self.inner.tracks).remove(key);
    }

    pub fn set_gain(&self, key: TrackKey, gain: f32) {
        let gain = gain.clamp(0.0, 1.0);
        lock(&self.inner.gains).insert(key, gain);
        if let Some(track) = lock(&self.inner.tracks).get(&key) {
            track.set_gain(gain);
        }
    }

    pub fn gain(&self, key: &TrackKey) -> f32 {
        lock(&self.inner.gains).get(key).copied().unwrap_or(1.0)
    }

    pub fn set_muted(&self, muted: bool) {
        self.inner.muted.store(muted, Ordering::Relaxed);
    }

    pub fn muted(&self) -> bool {
        self.inner.muted.load(Ordering::Relaxed)
    }

    pub fn underruns(&self, key: &TrackKey) -> u64 {
        lock(&self.inner.tracks)
            .get(key)
            .map(Track::underruns)
            .unwrap_or(0)
    }

    /// Fills `out` with the mix. Runs on the device thread: the track map is only tried, and a
    /// contended callback renders silence rather than blocking; each track's buffer lock is held
    /// by the decode thread for a memcpy at most.
    pub fn render(&self, out: &mut [f32]) {
        out.fill(0.0);
        let Ok(tracks) = self.inner.tracks.try_lock() else {
            return;
        };
        let muted = self.muted();
        for track in tracks.values() {
            let gain = track.gain();
            let mut buffer = lock(&track.inner.buffer);
            if buffer.len() < out.len() {
                // A partial burst followed by a gap clicks; one callback of silence does not.
                track.inner.underruns.fetch_add(1, Ordering::Relaxed);
                buffer.clear();
                continue;
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
        a.push(&[0.8; 4]);
        b.push(&[0.8; 4]);
        let mut out = [0.0; 4];
        mixer.render(&mut out);
        assert!(
            out.iter().all(|s| (*s - 1.0).abs() < 1e-6),
            "0.8 + 0.4 clamps to 1.0"
        );
        assert_eq!(mixer.gain(&B), 0.5);
        assert_eq!(mixer.gain(&A), 1.0);
    }

    #[test]
    fn a_short_track_contributes_silence_and_counts_one_underrun() {
        let mixer = Mixer::new();
        let a = mixer.add_track(A);
        a.push(&[0.5; 2]);
        let mut out = [0.0; 4];
        mixer.render(&mut out);
        assert_eq!(out, [0.0; 4]);
        assert_eq!(mixer.underruns(&A), 1);
        assert_eq!(a.queued(), 0, "the short remainder is consumed, not kept");
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

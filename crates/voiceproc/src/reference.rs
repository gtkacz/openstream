use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use brp_proto::constants::{AUDIO_SAMPLE_RATE, ECHO_REFERENCE_MAX};

pub(crate) const HOP: usize = 480;
const CHANNELS: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReferenceChannel {
    Voice,
    Streams,
}

impl ReferenceChannel {
    fn index(self) -> usize {
        match self {
            Self::Voice => 0,
            Self::Streams => 1,
        }
    }
}

#[derive(Default)]
struct Buffers {
    samples: [VecDeque<f32>; CHANNELS],
    playing: [bool; CHANNELS],
}

#[derive(Default)]
struct Shared {
    // A count, because a finished session's processor can be dropped after its successor started.
    active: AtomicUsize,
    buffers: Mutex<Buffers>,
}

/// What the room plays, as mono, until the encode thread asks for whole hops.
#[derive(Clone, Default)]
pub struct EchoReference(Arc<Shared>);

impl EchoReference {
    pub fn new() -> Self {
        Self::default()
    }

    fn cap() -> usize {
        (ECHO_REFERENCE_MAX.as_millis() as usize) * AUDIO_SAMPLE_RATE as usize / 1000
    }

    /// Never waits: a device callback that cannot take the lock loses its audio. `tap` keeps
    /// such audio and retries instead, so a playing channel is never shifted in time.
    pub fn push(&self, channel: ReferenceChannel, stereo: &[f32]) {
        self.try_push(channel, stereo);
    }

    /// Returns false only when the lock was contended; inactive counts as handled.
    fn try_push(&self, channel: ReferenceChannel, stereo: &[f32]) -> bool {
        if self.0.active.load(Ordering::Acquire) == 0 {
            return true;
        }
        let Ok(mut buffers) = self.0.buffers.try_lock() else {
            return false;
        };
        let this = channel.index();
        let other = 1 - this;
        buffers.playing[this] = true;
        buffers.samples[this].extend(
            stereo
                .as_chunks::<2>()
                .0
                .iter()
                .map(|frame| (frame[0] + frame[1]) / 2.0),
        );
        let excess = buffers.samples[this].len().saturating_sub(Self::cap());
        if excess > 0 {
            buffers.samples[this].drain(..excess);
            // A device that stopped would otherwise hold this one back forever.
            if buffers.playing[other] && buffers.samples[other].len() < HOP {
                buffers.samples[other].clear();
                buffers.playing[other] = false;
            }
        }
        true
    }

    pub fn tap(
        &self,
        channel: ReferenceChannel,
        mut render: impl FnMut(&mut [f32]) + Send + 'static,
    ) -> impl FnMut(&mut [f32]) + Send + 'static {
        let reference = self.clone();
        // Stereo samples; reserved up front so the device thread never allocates.
        let backlog_cap = Self::cap() * 2;
        let mut backlog: Vec<f32> = Vec::with_capacity(backlog_cap);
        move |out| {
            render(out);
            if backlog.is_empty() {
                if !reference.try_push(channel, out) {
                    backlog.extend_from_slice(out);
                }
                return;
            }
            backlog.extend_from_slice(out);
            let excess = backlog.len().saturating_sub(backlog_cap);
            backlog.drain(..excess);
            if reference.try_push(channel, &backlog) {
                backlog.clear();
            }
        }
    }

    pub(crate) fn activate(&self) {
        self.0.active.fetch_add(1, Ordering::AcqRel);
    }

    pub(crate) fn deactivate(&self) {
        let before = self.0.active.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(before > 0, "deactivated more often than activated");
        if before == 1 {
            *self.0.buffers.lock().unwrap() = Buffers::default();
        }
    }

    pub(crate) fn drain_hops(&self) -> Vec<[[f32; HOP]; CHANNELS]> {
        let mut buffers = self.0.buffers.lock().unwrap();
        let Some(hops) = (0..CHANNELS)
            .filter(|&c| buffers.playing[c])
            .map(|c| buffers.samples[c].len() / HOP)
            .min()
        else {
            return Vec::new();
        };
        let mut out = vec![[[0.0; HOP]; CHANNELS]; hops];
        let playing = buffers.playing;
        for c in (0..CHANNELS).filter(|&c| playing[c]) {
            for hop in &mut out {
                for (slot, sample) in hop[c].iter_mut().zip(buffers.samples[c].drain(..HOP)) {
                    *slot = sample;
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VOICE: ReferenceChannel = ReferenceChannel::Voice;
    const STREAMS: ReferenceChannel = ReferenceChannel::Streams;

    fn active() -> EchoReference {
        let reference = EchoReference::new();
        reference.activate();
        reference
    }

    fn stereo(frames: usize, value: f32) -> Vec<f32> {
        vec![value; frames * 2]
    }

    fn ramp(start: usize, frames: usize) -> Vec<f32> {
        (start..start + frames)
            .flat_map(|i| [i as f32; 2])
            .collect()
    }

    #[test]
    fn nothing_is_kept_while_inactive() {
        let reference = EchoReference::new();
        reference.push(VOICE, &stereo(2400, 1.0));
        assert!(reference.drain_hops().is_empty());
    }

    #[test]
    fn odd_callback_sizes_come_out_as_whole_hops_in_order() {
        let reference = active();
        let mut next = 0;
        for size in [441, 1024, 441, 1024, 441, 629] {
            reference.push(VOICE, &ramp(next, size));
            next += size;
        }
        assert_eq!(next, 4000);

        let hops = reference.drain_hops();
        assert_eq!(hops.len(), 8);
        let voice: Vec<f32> = hops.iter().flat_map(|h| h[0]).collect();
        assert_eq!(voice, (0..3840).map(|i| i as f32).collect::<Vec<_>>());
        assert!(hops.iter().all(|h| h[1].iter().all(|&s| s == 0.0)));

        reference.push(VOICE, &ramp(next, 320));
        let hops = reference.drain_hops();
        assert_eq!(hops.len(), 1);
        assert_eq!(hops[0][0][0], 3840.0);
    }

    #[test]
    fn a_channel_that_never_played_is_zero_filled() {
        let reference = active();
        reference.push(VOICE, &stereo(960, 1.0));
        let hops = reference.drain_hops();
        assert_eq!(hops.len(), 2);
        assert!(hops.iter().all(|h| h[1].iter().all(|&s| s == 0.0)));
    }

    #[test]
    fn both_channels_wait_for_each_other_once_both_play() {
        let reference = active();
        reference.push(VOICE, &stereo(960, 1.0));
        reference.push(STREAMS, &stereo(480, 0.5));
        assert_eq!(reference.drain_hops().len(), 1);

        reference.push(STREAMS, &stereo(480, 0.5));
        let hops = reference.drain_hops();
        assert_eq!(hops.len(), 1);
        assert!(hops[0][0].iter().all(|&s| s == 1.0));
        assert!(hops[0][1].iter().all(|&s| s == 0.5));
    }

    #[test]
    fn a_stopped_channel_stops_holding_back_the_other() {
        let reference = active();
        reference.push(STREAMS, &stereo(100, 0.5));
        reference.push(VOICE, &stereo(9600 + 960, 1.0));
        let hops = reference.drain_hops();
        assert!(!hops.is_empty());
        assert!(hops.iter().all(|h| h[1].iter().all(|&s| s == 0.0)));
        assert!(hops.iter().all(|h| h[0].iter().all(|&s| s == 1.0)));
    }

    #[test]
    fn the_buffer_never_holds_more_than_the_cap() {
        let reference = active();
        reference.push(VOICE, &stereo(96_000, 1.0));
        reference.push(STREAMS, &stereo(96_000, 1.0));
        assert!(reference.drain_hops().len() * HOP <= 9600);
    }

    #[test]
    fn deactivating_empties_both_channels() {
        let reference = active();
        reference.push(VOICE, &stereo(960, 1.0));
        reference.push(STREAMS, &stereo(960, 1.0));
        reference.deactivate();
        reference.activate();
        assert!(reference.drain_hops().is_empty());
    }

    #[test]
    fn an_earlier_session_ending_late_does_not_stop_a_newer_one() {
        let reference = active();
        reference.activate();
        reference.deactivate();
        reference.push(VOICE, &stereo(960, 1.0));
        assert_eq!(reference.drain_hops().len(), 2);
    }

    #[test]
    fn contention_delays_audio_instead_of_losing_it() {
        let reference = active();
        let mut next = 0;
        let mut tapped = reference.tap(VOICE, move |out| {
            for frame in out.as_chunks_mut::<2>().0 {
                frame.fill(next as f32);
                next += 1;
            }
        });
        let mut buffer = vec![0.0; 960];

        let held = reference.0.buffers.lock().unwrap();
        tapped(&mut buffer);
        drop(held);
        assert!(reference.drain_hops().is_empty());

        tapped(&mut buffer);
        let hops = reference.drain_hops();
        assert_eq!(hops.len(), 2);
        let voice: Vec<f32> = hops.iter().flat_map(|h| h[0]).collect();
        assert_eq!(voice, (0..960).map(|i| i as f32).collect::<Vec<_>>());
    }

    #[test]
    fn the_tap_renders_then_records() {
        let reference = active();
        let mut tapped = reference.tap(VOICE, |out| out.fill(0.25));
        let mut buffer = vec![0.0; 960];
        tapped(&mut buffer);
        assert!(buffer.iter().all(|&s| s == 0.25));
        let hops = reference.drain_hops();
        assert_eq!(hops.len(), 1);
        assert!(hops[0][0].iter().all(|&s| s == 0.25));
    }
}

use std::f32::consts::TAU;

use brp_codec::{AudioFrame, open_audio_decoder, open_audio_encoder};
use brp_proto::AudioParams;
use brp_proto::constants::{AUDIO_CHANNELS, AUDIO_FRAME_SAMPLES, AUDIO_SAMPLE_RATE};

const LEFT_HZ: f32 = 440.0;
const RIGHT_HZ: f32 = 1320.0;
const LEFT_AMPLITUDE: f32 = 0.5;
/// A quarter of the left channel's, so the transpose FLTP planes need is observable per channel.
const RIGHT_AMPLITUDE: f32 = 0.125;

fn sine_frame(index: u64) -> AudioFrame {
    let mut frame = AudioFrame::silence(index * 20_000);
    for n in 0..AUDIO_FRAME_SAMPLES {
        let t = (index as usize * AUDIO_FRAME_SAMPLES + n) as f32 / AUDIO_SAMPLE_RATE as f32;
        frame.samples[n * 2] = LEFT_AMPLITUDE * (TAU * LEFT_HZ * t).sin();
        frame.samples[n * 2 + 1] = RIGHT_AMPLITUDE * (TAU * RIGHT_HZ * t).sin();
    }
    frame
}

fn channel_rms(samples: &[f32], channel: usize) -> f32 {
    let values: Vec<f32> = samples
        .iter()
        .skip(channel)
        .step_by(AUDIO_CHANNELS as usize)
        .copied()
        .collect();
    (values.iter().map(|s| s * s).sum::<f32>() / values.len().max(1) as f32).sqrt()
}

#[test]
fn a_stereo_sine_survives_the_opus_round_trip_channel_by_channel() {
    let mut encoder = open_audio_encoder().expect("libopus is linked into FFmpeg");
    assert_eq!(encoder.params(), AudioParams::STANDARD);
    let mut decoder = open_audio_decoder(&AudioParams::STANDARD).unwrap();

    let mut decoded_total = 0usize;
    let mut tail = Vec::new();
    for index in 0..50u64 {
        let packets = encoder.encode(&sine_frame(index)).unwrap();
        for packet in &packets {
            assert!(packet.keyframe);
            assert_eq!(packet.capture_ts_us, index * 20_000);
            for frame in decoder.decode(packet).unwrap() {
                decoded_total += frame.samples.len();
                if index >= 40 {
                    tail.extend_from_slice(&frame.samples);
                }
            }
        }
    }
    // libopus drops its pre-skip once, so slightly less than fifty frames come back.
    assert!(
        decoded_total >= 45 * AudioFrame::FRAME_LEN,
        "decoded {decoded_total} samples"
    );
    let reference = sine_frame(45);
    for channel in 0..AUDIO_CHANNELS as usize {
        let expected = channel_rms(&reference.samples, channel);
        let got = channel_rms(&tail, channel);
        assert!(
            (got - expected).abs() < expected * 0.2,
            "channel {channel} rms {got} vs {expected}"
        );
    }
}

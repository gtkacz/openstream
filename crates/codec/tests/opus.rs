use std::f32::consts::TAU;

use brp_codec::{AudioFrame, open_audio_decoder, open_audio_encoder};
use brp_proto::AudioParams;
use brp_proto::constants::{AUDIO_FRAME_SAMPLES, AUDIO_SAMPLE_RATE};

fn sine_frame(index: u64) -> AudioFrame {
    let mut frame = AudioFrame::silence(index * 20_000);
    for n in 0..AUDIO_FRAME_SAMPLES {
        let t = (index as usize * AUDIO_FRAME_SAMPLES + n) as f32 / AUDIO_SAMPLE_RATE as f32;
        let value = 0.5 * (TAU * 440.0 * t).sin();
        frame.samples[n * 2] = value;
        frame.samples[n * 2 + 1] = value;
    }
    frame
}

fn rms(samples: &[f32]) -> f32 {
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len().max(1) as f32).sqrt()
}

#[test]
fn a_sine_survives_the_opus_round_trip() {
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
    let expected = rms(&sine_frame(45).samples);
    let got = rms(&tail);
    assert!(
        (got - expected).abs() < expected * 0.2,
        "rms {got} vs {expected}"
    );
}

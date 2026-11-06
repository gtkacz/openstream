use crate::processor::VoiceProcessor;

const SAMPLE_RATE: usize = 48_000;
const FRAME: usize = 960;

pub fn speech() -> Vec<f32> {
    include_bytes!("../tests/fixtures/speech-48k-mono-s16le.raw")
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
        .collect()
}

pub fn noise(len: usize, seed: u64) -> Vec<f32> {
    let mut state = seed;
    let mut y = 0.0f32;
    (0..len)
        .map(|_| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let x = ((state >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0;
            y = 0.97 * y + 0.03 * x;
            y
        })
        .collect()
}

pub fn at_peak(x: &[f32], dbfs: f32) -> Vec<f32> {
    let peak = x.iter().fold(0.0f32, |p, s| p.max(s.abs()));
    scaled(x, 10f32.powf(dbfs / 20.0) / peak)
}

pub fn at_rms(x: &[f32], dbfs: f32) -> Vec<f32> {
    scaled(x, 10f32.powf((dbfs - rms_db(x)) / 20.0))
}

fn scaled(x: &[f32], gain: f32) -> Vec<f32> {
    x.iter().map(|s| s * gain).collect()
}

pub fn rms_db(x: &[f32]) -> f32 {
    let mean_square = x.iter().map(|s| s * s).sum::<f32>() / x.len().max(1) as f32;
    10.0 * mean_square.max(1e-24).log10()
}

pub fn looped(x: &[f32], seconds: f32) -> Vec<f32> {
    x.iter()
        .cycle()
        .take((seconds * SAMPLE_RATE as f32) as usize)
        .copied()
        .collect()
}

pub fn run(processor: &mut VoiceProcessor, mono: &[f32]) -> Vec<f32> {
    mono.as_chunks::<FRAME>()
        .0
        .iter()
        .flat_map(|chunk| {
            let mut stereo: Vec<f32> = chunk.iter().flat_map(|&s| [s, s]).collect();
            processor.process(&mut stereo);
            stereo
                .as_chunks::<2>()
                .0
                .iter()
                .map(|f| f[0])
                .collect::<Vec<_>>()
        })
        .collect()
}

fn lcg(state: &mut u64) -> f32 {
    *state = state
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    ((*state >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0
}

/// A speaker-to-microphone path of one direct arrival and 20 sparse reflections, plus a noise floor.
pub fn sparse_echo(far: &[f32], delay_ms: usize, seed: u64) -> Vec<f32> {
    let ms = SAMPLE_RATE / 1000;
    let mut state = seed;
    let direct = delay_ms * ms;
    let mut taps = vec![(direct, 0.5f32)];
    for k in 0..20 {
        let jitter = (lcg(&mut state).abs() * 40.0) as usize;
        let at = direct + (2 + 3 * k) * ms + jitter;
        taps.push((at, 0.25 * lcg(&mut state) * (-(k as f32) / 6.0).exp()));
    }
    let mut noise = seed.wrapping_add(1000);
    (0..far.len())
        .map(|i| {
            let echo: f32 = taps
                .iter()
                .filter(|(t, _)| i >= *t)
                .map(|(t, g)| g * far[i - t])
                .sum();
            echo + lcg(&mut noise) * 3e-4
        })
        .collect()
}

/// Plays `x` back at `rate` times its speed, interpolating linearly between samples.
pub fn stretched(x: &[f32], rate: f32) -> Vec<f32> {
    (0..(x.len() as f32 / rate) as usize - 1)
        .map(|i| {
            let at = i as f32 * rate;
            let (whole, frac) = (at as usize, at.fract());
            x[whole] * (1.0 - frac) + x[(whole + 1).min(x.len() - 1)] * frac
        })
        .collect()
}

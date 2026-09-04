use crate::constants::{MAX_BITRATE_KBPS, MIN_BITRATE_KBPS};
const A: [(u64, u64); 4] = [
    (1280 * 720, 10000),
    (1920 * 1080, 20000),
    (2560 * 1440, 40000),
    (3840 * 2160, 80000),
];
pub fn default_bitrate_kbps(w: u32, h: u32, f: u32) -> u32 {
    (interpolate(u64::from(w) * u64::from(h)) * u64::from(f) / 60)
        .clamp(u64::from(MIN_BITRATE_KBPS), u64::from(MAX_BITRATE_KBPS)) as u32
}
fn interpolate(p: u64) -> u64 {
    if p <= A[0].0 {
        return A[0].1 * p / A[0].0;
    }
    for x in A.windows(2) {
        if p <= x[1].0 {
            return x[0].1 + (x[1].1 - x[0].1) * (p - x[0].0) / (x[1].0 - x[0].0);
        }
    }
    A[3].1 * p / A[3].0
}

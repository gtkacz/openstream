use std::{sync::OnceLock, time::Instant};
static EPOCH: OnceLock<Instant> = OnceLock::new();
pub fn monotonic_us() -> u64 {
    EPOCH.get_or_init(Instant::now).elapsed().as_micros() as u64
}

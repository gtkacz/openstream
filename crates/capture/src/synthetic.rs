use crate::frame::*;
use brp_proto::{PixelFormat, monotonic_us};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
#[derive(Debug, Clone, Copy)]
pub struct SyntheticSource {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}
struct Session {
    info: SourceInfo,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl CaptureBackend for SyntheticSource {
    fn start(&self, _: SourceRequest, mut sink: FrameSink) -> StartFuture<'_> {
        let info = SourceInfo {
            width: self.width,
            height: self.height,
            fps: self.fps.max(1),
        };
        Box::pin(async move {
            let stop = Arc::new(AtomicBool::new(false));
            let flag = stop.clone();
            let thread = thread::spawn(move || {
                let interval = Duration::from_secs_f64(1.0 / f64::from(info.fps));
                let started = Instant::now();
                let mut index = 0u32;
                while !flag.load(Ordering::Relaxed) {
                    if let Some(wait) =
                        (started + interval * index).checked_duration_since(Instant::now())
                    {
                        thread::sleep(wait);
                    }
                    let stride = info.width as usize * 4;
                    let mut data = vec![0; stride * info.height as usize];
                    data[..4].copy_from_slice(&index.to_le_bytes());
                    sink(CaptureFrame {
                        width: info.width,
                        height: info.height,
                        stride,
                        format: PixelFormat::Bgra,
                        data,
                        capture_ts_us: monotonic_us(),
                    });
                    index = index.wrapping_add(1);
                }
            });
            Ok(Box::new(Session {
                info,
                stop,
                thread: Some(thread),
            }) as Box<dyn CaptureSession>)
        })
    }
}
impl CaptureSession for Session {
    fn info(&self) -> SourceInfo {
        self.info
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

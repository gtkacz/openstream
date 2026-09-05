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

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use brp_proto::SourceKind;

    use super::*;

    #[tokio::test]
    async fn synthetic_source_paces_frames_and_numbers_them() {
        let frames = Arc::new(Mutex::new(Vec::new()));
        let sink_frames = frames.clone();
        let session = SyntheticSource {
            width: 64,
            height: 32,
            fps: 100,
        }
        .start(
            SourceRequest {
                kind: SourceKind::Monitor,
                target_fps: 100,
            },
            Box::new(move |f| sink_frames.lock().unwrap().push(f)),
        )
        .await
        .unwrap();
        assert_eq!(
            session.info(),
            SourceInfo {
                width: 64,
                height: 32,
                fps: 100
            }
        );
        tokio::time::sleep(Duration::from_millis(120)).await;
        session.stop();
        let count_at_stop = frames.lock().unwrap().len();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(
            frames.lock().unwrap().len(),
            count_at_stop,
            "frames arrived after stop"
        );
        let frames = frames.lock().unwrap();
        assert!(
            frames.len() >= 6,
            "only {} frames in 120 ms at 100 fps",
            frames.len()
        );
        for (i, f) in frames.iter().enumerate() {
            assert_eq!(
                (f.width, f.height, f.stride, f.format),
                (64, 32, 256, PixelFormat::Bgra)
            );
            assert_eq!(f.data.len(), 256 * 32);
            assert_eq!(
                u32::from_le_bytes([f.data[0], f.data[1], f.data[2], f.data[3]]),
                i as u32
            );
        }
        assert!(
            frames
                .windows(2)
                .all(|w| w[1].capture_ts_us > w[0].capture_ts_us)
        );
    }
}

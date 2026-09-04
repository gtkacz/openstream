//! Manually verify portal selection and the first captured frame.

use std::fs::File;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use brp_capture::{CaptureBackend, CaptureFrame, PortalCapture, SourceRequest};
use brp_proto::{PixelFormat, SourceKind};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let first: Arc<Mutex<Option<CaptureFrame>>> = Arc::default();
    let count = Arc::new(Mutex::new(0u64));
    let (first_sink, count_sink) = (first.clone(), count.clone());
    let session = PortalCapture
        .start(
            SourceRequest {
                kind: SourceKind::Monitor,
                target_fps: 60,
            },
            Box::new(move |frame| {
                *count_sink.lock().unwrap() += 1;
                first_sink.lock().unwrap().get_or_insert(frame);
            }),
        )
        .await?;
    println!("negotiated {:?}", session.info());
    let started = Instant::now();
    tokio::time::sleep(Duration::from_secs(5)).await;
    let frames = *count.lock().unwrap();
    println!(
        "{frames} frames in {:.1?} = {:.1} fps (move a window; static screens produce no frames)",
        started.elapsed(),
        frames as f64 / started.elapsed().as_secs_f64()
    );
    if let Some(frame) = first.lock().unwrap().take() {
        let mut output = File::create("/tmp/brp-first-frame.ppm")?;
        writeln!(output, "P6\n{} {}\n255", frame.width, frame.height)?;
        for row in frame
            .data
            .chunks_exact(frame.stride)
            .take(frame.height as usize)
        {
            for pixel in row[..frame.width as usize * 4].chunks_exact(4) {
                let rgb = match frame.format {
                    PixelFormat::Bgra | PixelFormat::Bgrx => [pixel[2], pixel[1], pixel[0]],
                    PixelFormat::Rgba | PixelFormat::Rgbx => [pixel[0], pixel[1], pixel[2]],
                };
                output.write_all(&rgb)?;
            }
        }
        println!(
            "wrote /tmp/brp-first-frame.ppm ({:?}, stride {})",
            frame.format, frame.stride
        );
    }
    session.stop();
    Ok(())
}

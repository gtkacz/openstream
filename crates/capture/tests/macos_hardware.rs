//! Gated checks of ScreenCaptureKit on a real Mac: `BRP_HARDWARE_TESTS=1 cargo test -p
//! brp-capture --test macos_hardware -- --nocapture`. They need the Screen & System Audio
//! Recording permission for the app that runs them (Terminal, when run from it), so CI runners,
//! which cannot grant it, skip them.
#![cfg(target_os = "macos")]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use brp_capture::{
    CaptureBackend, CaptureError, CaptureFrame, PlatformCapture, SourceId, SourceListing,
    SourceRequest,
};
use brp_proto::SourceKind;

fn gated() -> bool {
    std::env::var_os("BRP_HARDWARE_TESTS").is_some()
}

fn choices(kind: SourceKind) -> Vec<brp_capture::SourceDescriptor> {
    match PlatformCapture.sources(kind).expect("listing") {
        SourceListing::Choices(choices) => choices,
        SourceListing::PlatformPicker => panic!("macOS lists its sources for the in-app picker"),
    }
}

/// Starts `request`, keeps it for a second, and returns how many frames arrived and the last one.
async fn capture(request: SourceRequest) -> Result<(usize, Option<CaptureFrame>), CaptureError> {
    let count = Arc::new(AtomicUsize::new(0));
    let last = Arc::new(Mutex::new(None));
    let sink = {
        let (count, last) = (count.clone(), last.clone());
        Box::new(move |frame: CaptureFrame| {
            count.fetch_add(1, Ordering::Relaxed);
            *last.lock().unwrap() = Some(frame);
        })
    };
    let session = PlatformCapture.start(request, sink).await?;
    let info = session.info();
    eprintln!("{:?} capture: {info:?}", request.kind);
    tokio::time::sleep(Duration::from_secs(1)).await;
    session.stop();
    let frame = last.lock().unwrap().take();
    Ok((count.load(Ordering::Relaxed), frame))
}

fn assert_well_formed(frame: &CaptureFrame) {
    assert!(frame.width > 0 && frame.height > 0);
    assert!(frame.stride >= frame.width as usize * 4);
    assert_eq!(frame.data.len(), frame.stride * frame.height as usize);
}

#[test]
fn displays_and_windows_are_listed_without_brp_itself() {
    if !gated() {
        return;
    }
    let displays = choices(SourceKind::Monitor);
    eprintln!("displays: {displays:#?}");
    assert!(!displays.is_empty());
    assert!(displays.iter().all(|d| d.width > 0 && d.height > 0));
    let windows = choices(SourceKind::Window);
    eprintln!("{} windows, e.g. {:?}", windows.len(), windows.first());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_main_display_delivers_bgra_frames_at_its_pixel_size() {
    if !gated() {
        return;
    }
    let (frames, last) = capture(SourceRequest {
        kind: SourceKind::Monitor,
        source: None,
        target_fps: 30,
    })
    .await
    .expect("display capture");
    assert!(frames >= 1, "no frame from the main display");
    assert_well_formed(&last.expect("a frame"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_listed_window_delivers_frames() {
    if !gated() {
        return;
    }
    let Some(window) = choices(SourceKind::Window).into_iter().next() else {
        eprintln!("skipping: no window on screen");
        return;
    };
    eprintln!("capturing window {:?}", window.name);
    let (frames, last) = capture(SourceRequest {
        kind: SourceKind::Window,
        source: Some(window.id),
        target_fps: 30,
    })
    .await
    .expect("window capture");
    // A window that does not change may send only its first frame.
    assert!(frames >= 1, "no frame from {}", window.name);
    assert_well_formed(&last.expect("a frame"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_window_that_is_gone_is_source_lost() {
    if !gated() {
        return;
    }
    let error = capture(SourceRequest {
        kind: SourceKind::Window,
        source: Some(SourceId(u64::from(u32::MAX))),
        target_fps: 30,
    })
    .await
    .expect_err("no such window");
    assert!(matches!(error, CaptureError::SourceLost(_)), "{error}");
}

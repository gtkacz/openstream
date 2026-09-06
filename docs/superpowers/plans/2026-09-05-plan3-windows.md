# Plan 3: Windows Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** brp builds, tests, and packages on Windows: monitors and windows are captured through Windows Graphics Capture with desktop duplication as the monitor fallback, the in-app picker lists what can be shared, decoding tries D3D11VA first, Media Foundation joins the encoder probe, and every push produces a runnable Windows zip from CI.

**Architecture:** The capture trait gains a `sources` listing and an optional source id on the request; a new Windows module of the capture crate implements both on the `windows-capture` crate, and a platform-neutral fallback driver decides between Graphics Capture and duplication so its logic is unit-tested on Linux. The room passes the source id through and exposes the listing; the window opens an egui picker when the platform has no picker of its own. The codec crate changes two constant tables. A second CI job on `windows-latest` consumes a pinned BtbN FFmpeg LGPL shared build through `FFMPEG_DIR` and uploads the executable with its four DLLs.

**Tech Stack:** Rust 2024, windows-capture 2.0.1, ffmpeg-sys-next 9.0 against FFmpeg 8.1 (BtbN LGPL shared), egui 0.36, tokio, GitHub Actions `windows-latest`.

**Spec:** `docs/superpowers/specs/2026-09-05-phase3-windows-design.md`, refining `docs/superpowers/specs/2026-09-04-p2p-screen-sharing-design.md` (sections 5.3, 5.4, 8, 10, 11). Read both.

## Global Constraints

- No Windows hardware exists for this phase. The Windows CI job is the compile oracle; the hardware-free suite must pass on both runners. Runtime checks listed in spec section 10 are deferred, and the plan never claims they ran.
- Everything Windows-only in the capture crate lives under `crates/capture/src/windows/` behind `#[cfg(windows)]`. The Linux backend, `SyntheticSource`, and the test fakes are not modified beyond the new `SourceRequest` field.
- The protocol does not change. No file under `crates/proto/src` changes except `constants.rs`, which gains `CAPTURE_FALLBACK_TIMEOUT: Duration = Duration::from_secs(2)`.
- FFmpeg on Windows is exactly BtbN release `autobuild-2026-09-05-13-10`, asset `ffmpeg-n8.1.2-50-g1a748fe2cd-win64-lgpl-shared-8.1.zip`, consumed through `FFMPEG_DIR`. The artifact bundles `brp.exe`, `avcodec-62.dll`, `avutil-60.dll`, `swscale-9.dll`, `swresample-6.dll`, FFmpeg's licence as `FFMPEG-LICENSE.txt`, and the project `LICENSE`.
- The Windows binary stays a console application. No `windows_subsystem` attribute.
- Comments explain why. Doc comments state contracts on new public items. No task ids in code.
- One Conventional Commit per task, imperative subject, no co-author lines. The `Claude-Session:` trailer the harness requires on every commit of this session is expected; no other trailers. `cargo fmt --all` and `cargo clippy --workspace --all-targets -- -D warnings` pass on Linux before each commit.
- Local Windows cross-check (an accelerator, not the oracle). The Windows FFmpeg build is extracted at `/tmp/ffwin/ffmpeg-n8.1-latest-win64-lgpl-shared-8.1`, the Windows CRT and SDK headers from `xwin` at `/home/gtkacz/.xwin`. If either directory is missing, skip the cross-check steps; CI decides. The command, run from the repository root:

  ```sh
  FFMPEG_DIR=/tmp/ffwin/ffmpeg-n8.1-latest-win64-lgpl-shared-8.1 \
  BINDGEN_EXTRA_CLANG_ARGS_x86_64_pc_windows_msvc="-I/home/gtkacz/.xwin/crt/include -I/home/gtkacz/.xwin/sdk/include/ucrt -I/home/gtkacz/.xwin/sdk/include/um -I/home/gtkacz/.xwin/sdk/include/shared" \
  cargo check --target x86_64-pc-windows-msvc -p brp-codec -p brp-capture --all-targets
  ```

  Cargo does not track that bindgen variable, so after changing it run `cargo clean -p ffmpeg-sys-next --target x86_64-pc-windows-msvc` once or stale Linux bindings are reused. Only the codec and capture crates cross-check: the app crate pulls `iroh`, whose `blake3` dependency needs the MSVC assembler, which this machine lacks. The app changes are platform-neutral and verified by the Linux suite.
- Verified library facts this plan relies on (windows-capture 2.0.1): `Monitor::{enumerate, primary, name, width, height, refresh_rate, as_raw_hmonitor, from_raw_hmonitor}` with `Monitor: Copy + PartialEq + Send`; `Window::{enumerate, title, process_id, width, height, monitor, is_valid, as_raw_hwnd, from_raw_hwnd}` with `Window: Copy + Send`, `width`/`height` returning `i32`; both implement `TryInto<GraphicsCaptureItemType>`; `GraphicsCaptureApiHandler { type Flags; type Error: Send + Sync; fn new(Context<Flags>); fn on_frame_arrived(&mut self, &mut Frame, InternalCaptureControl); fn on_closed(&mut self) }` with `start_free_threaded(Settings) -> Result<CaptureControl<Self, Self::Error>, GraphicsCaptureApiError<Self::Error>>`; `CaptureControl::stop(self) -> Result<(), CaptureControlError<E>>`; `Settings::new(item, CursorCaptureSettings, DrawBorderSettings, SecondaryWindowSettings, MinimumUpdateIntervalSettings, DirtyRegionSettings, ColorFormat, flags)`; `CursorCaptureSettings::{Default, WithCursor, WithoutCursor}`, `DrawBorderSettings::{Default, WithBorder, WithoutBorder}`, `MinimumUpdateIntervalSettings::{Default, Custom(Duration)}`, `ColorFormat::Bgra8`; `GraphicsCaptureApi::{is_cursor_settings_supported, is_border_settings_supported, is_minimum_update_interval_supported} -> Result<bool, _>`; `Frame::buffer(&mut self) -> Result<FrameBuffer, _>` with `FrameBuffer::{width, height, row_pitch, as_raw_buffer(&mut self) -> &mut [u8]}`; `DxgiDuplicationApi::new_options(Monitor, &[DxgiDuplicationFormat]) -> Result<Self, Error>`, `acquire_next_frame(&mut self, timeout_ms: u32) -> Result<DxgiDuplicationFrame, Error>`, `recreate_options(self, &[DxgiDuplicationFormat]) -> Result<Self, Error>`, `Error::{Timeout, AccessLost, ..}`, `DxgiDuplicationFormat::Bgra8`, `DxgiDuplicationFrame::buffer(&mut self) -> Result<DxgiDuplicationFrameBuffer, Error>` with `{width, height, row_pitch, as_raw_buffer}`. Module paths: `windows_capture::{capture, frame, graphics_capture_api, dxgi_duplication_api, monitor, window, settings}`. egui 0.36.1: `Window::new(title).collapsible(false).resizable(false).anchor(Align2, [f32; 2]).show(&Context, |ui| ..)`, `ScrollArea::vertical().max_height(f32)`, `Ui::{selectable_label, button, weak, separator, ctx}`.

## File Structure

```
crates/proto/src/constants.rs               + CAPTURE_FALLBACK_TIMEOUT

crates/capture/Cargo.toml                   + [target.'cfg(windows)'.dependencies] windows-capture, tokio
crates/capture/src/frame.rs                 + SourceId, SourceDescriptor, SourceListing; SourceRequest.source; CaptureBackend::sources
crates/capture/src/error.rs                 + CaptureError::Windows
crates/capture/src/lib.rs                   + pub mod fallback; PlatformCapture alias per OS; windows module
crates/capture/src/fallback.rs              Started trait, Attempt, start_with_fallback + tests
crates/capture/src/windows/mod.rs           WindowsCapture backend, SharedSink, start_blocking
crates/capture/src/windows/sources.rs       list(kind), resolve(kind, id), Target, ids from raw handles
crates/capture/src/windows/graphics_capture.rs  Handler, GcStarted, GcSession
crates/capture/src/windows/duplication.rs   duplication thread, DupStarted, DupSession
crates/capture/src/synthetic.rs             test literal gains source: None; sources() default test
crates/capture/examples/portal_dump.rs      Linux gate; literal gains source: None

crates/codec/src/select.rs                  + hevc_mf, h264_mf in PROBE_ORDER; order test
crates/codec/src/ffmpeg/encoder.rs          + Media Foundation low-latency options
crates/codec/src/ffmpeg/decoder.rs          HW_DEVICE_ORDER per OS

crates/room/src/room.rs                     start_live(kind, source, title); sources(kind)
crates/room/tests/two_rooms.rs              call sites; RecordingCapture tests
crates/room/tests/registry.rs               literal gains source: None
crates/pipeline/tests/publisher.rs          literal gains source: None

crates/app/src/commands.rs                  Share { kind, source }
crates/app/src/ui/state.rs                  SourcePicker, picker field, open_picker/pick_source/cancel_picker + tests
crates/app/src/ui/picker.rs                 egui window listing the choices
crates/app/src/ui/mod.rs                    + pub mod picker; draw call
crates/app/src/ui/own_lives.rs              Share command shape; waiting text
crates/app/src/window.rs                    Share handling: listing, picker, share(kind, source)
crates/app/src/participant.rs               PlatformCapture
crates/app/src/publish.rs                   PlatformCapture; start_live(kind, None, title)

.github/workflows/ci.yml                    + windows job with FFmpeg download, tests, artifact
README.md                                   Windows build instructions and the artifact
Cargo.toml                                  + windows-capture workspace dependency
```

---

### Task 1: Source listing in the capture trait

**Files:**
- Modify: `crates/capture/src/frame.rs`
- Modify: `crates/capture/src/error.rs`
- Modify: `crates/capture/src/lib.rs`
- Modify: `crates/proto/src/constants.rs`
- Modify: `crates/capture/src/synthetic.rs:96-106` (test literal) and its tests module
- Modify: `crates/capture/examples/portal_dump.rs`
- Modify: `crates/room/src/room.rs:269-272`
- Modify: `crates/room/tests/registry.rs:36-39`
- Modify: `crates/pipeline/tests/publisher.rs:32-35`

**Interfaces:**
- Produces: `brp_capture::{SourceId(pub u64), SourceDescriptor { id, kind, name, width, height }, SourceListing::{PlatformPicker, Choices(Vec<SourceDescriptor>)}}`; `SourceRequest { kind, source: Option<SourceId>, target_fps }`; `CaptureBackend::sources(&self, kind: SourceKind) -> Result<SourceListing, CaptureError>` with a default body returning `PlatformPicker`; `CaptureError::Windows(String)`; `brp_capture::PlatformCapture` (Linux: `PortalCapture`); `brp_proto::constants::CAPTURE_FALLBACK_TIMEOUT`.

- [ ] **Step 1: Write the failing test**

Append to the `tests` module at the bottom of `crates/capture/src/synthetic.rs`:

```rust
    #[test]
    fn synthetic_source_leaves_picking_to_the_platform() {
        let source = SyntheticSource {
            width: 64,
            height: 32,
            fps: 30,
        };
        assert_eq!(
            source.sources(SourceKind::Monitor).unwrap(),
            SourceListing::PlatformPicker
        );
    }
```

Add `use brp_proto::SourceKind;` to that tests module's imports if it is not already there (the `SourceRequest` literal on line 100 already names `SourceKind::Monitor`, so it is).

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p brp-capture synthetic_source_leaves_picking_to_the_platform`
Expected: compile error, `no method named sources` and `SourceListing` not found.

- [ ] **Step 3: Extend the trait and the request**

In `crates/capture/src/frame.rs`, replace the `SourceRequest` struct and the `CaptureBackend` trait with:

```rust
/// One capturable source on platforms that list them. The value is the platform's raw handle (an
/// `HMONITOR` or `HWND` on Windows) and means nothing across processes or reboots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceId(pub u64);
/// One entry of a source list: what the picker shows and what `start` is asked to open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceDescriptor {
    pub id: SourceId,
    pub kind: SourceKind,
    pub name: String,
    pub width: u32,
    pub height: u32,
}
/// How a platform lets the user choose what to share.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceListing {
    /// The platform shows its own picker when `start` runs (the Linux portal); nothing to draw.
    PlatformPicker,
    /// The app draws a picker from these and passes the chosen id in `SourceRequest::source`.
    Choices(Vec<SourceDescriptor>),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceRequest {
    pub kind: SourceKind,
    /// Which listed source to open. Platforms that pick for themselves ignore it.
    pub source: Option<SourceId>,
    pub target_fps: u32,
}
pub type FrameSink = Box<dyn FnMut(CaptureFrame) + Send + 'static>;
pub type StartFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Box<dyn CaptureSession>, CaptureError>> + Send + 'a>>;
pub trait CaptureBackend: Send + Sync {
    /// What `kind` can share. Backends whose platform owns the picker keep the default.
    fn sources(&self, _kind: SourceKind) -> Result<SourceListing, CaptureError> {
        Ok(SourceListing::PlatformPicker)
    }
    fn start(&self, request: SourceRequest, sink: FrameSink) -> StartFuture<'_>;
}
```

In `crates/capture/src/error.rs` add a variant after `UnsupportedFormat`:

```rust
    #[error("windows: {0}")]
    Windows(String),
```

In `crates/proto/src/constants.rs` append:

```rust
/// Graphics Capture normally delivers its first frame within milliseconds; a monitor still silent
/// after this is served by desktop duplication instead.
pub const CAPTURE_FALLBACK_TIMEOUT: Duration = Duration::from_secs(2);
```

In `crates/capture/src/lib.rs` add the alias after the existing Linux re-export:

```rust
#[cfg(target_os = "linux")]
pub use linux::PortalCapture as PlatformCapture;
```

- [ ] **Step 4: Update every `SourceRequest` literal**

Each of these literals gains `source: None,` between `kind` and `target_fps`:

`crates/room/src/room.rs` (inside `start_live`):

```rust
                SourceRequest {
                    kind,
                    source: None,
                    target_fps: self.target_fps,
                },
```

`crates/room/tests/registry.rs` (inside `synthetic_live`):

```rust
        SourceRequest {
            kind: SourceKind::Monitor,
            source: None,
            target_fps: 60,
        },
```

`crates/pipeline/tests/publisher.rs`:

```rust
        SourceRequest {
            kind: SourceKind::Monitor,
            source: None,
            target_fps: 60,
        },
```

`crates/capture/src/synthetic.rs` tests module:

```rust
            SourceRequest {
                kind: SourceKind::Monitor,
                source: None,
                target_fps: 100,
            },
```

- [ ] **Step 5: Gate the portal example to Linux**

Replace the whole of `crates/capture/examples/portal_dump.rs` with:

```rust
//! Manually verify portal selection and the first captured frame. Linux only: it drives the
//! desktop portal directly.

#[cfg(target_os = "linux")]
mod portal {
    use std::fs::File;
    use std::io::Write;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use brp_capture::{CaptureBackend, CaptureFrame, PortalCapture, SourceRequest};
    use brp_proto::{PixelFormat, SourceKind};

    pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
        let first: Arc<Mutex<Option<CaptureFrame>>> = Arc::default();
        let count = Arc::new(Mutex::new(0u64));
        let (first_sink, count_sink) = (first.clone(), count.clone());
        let session = PortalCapture
            .start(
                SourceRequest {
                    kind: SourceKind::Monitor,
                    source: None,
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
}

#[cfg(target_os = "linux")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    portal::run().await
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("the portal example only runs on Linux");
}
```

- [ ] **Step 6: Run the tests and lints**

Run: `cargo test --workspace && cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`
Expected: all green, including the new test.

- [ ] **Step 7: Commit**

```bash
git add crates/capture crates/proto/src/constants.rs crates/room/src/room.rs crates/room/tests/registry.rs crates/pipeline/tests/publisher.rs
git commit -m "feat: let capture backends list their sources and take a source id"
```

---

### Task 2: The fallback driver

**Files:**
- Create: `crates/capture/src/fallback.rs`
- Modify: `crates/capture/src/lib.rs`

**Interfaces:**
- Consumes: `CaptureSession`, `SourceInfo`, `CaptureError` from Task 1's crate state.
- Produces: `brp_capture::fallback::{Started, Attempt, start_with_fallback}` with exactly these signatures:

```rust
pub trait Started {
    fn wait_first_frame(&mut self, timeout: Duration) -> Result<Option<SourceInfo>, CaptureError>;
    fn into_session(self: Box<Self>, info: SourceInfo) -> Box<dyn CaptureSession>;
}
pub struct Attempt<'a> {
    pub name: &'static str,
    pub start: Box<dyn FnOnce() -> Result<Box<dyn Started>, CaptureError> + 'a>,
}
pub fn start_with_fallback(timeout: Duration, primary: Attempt<'_>, fallback: Option<Attempt<'_>>)
    -> Result<Box<dyn CaptureSession>, CaptureError>;
```

- [ ] **Step 1: Write the failing tests**

Create `crates/capture/src/fallback.rs` with the tests first (the implementation follows in step 3; the file must contain both for the crate to compile, so write the whole file now but leave `start_with_fallback`'s body as `todo!()`):

```rust
//! Tries a primary capture path and, when it fails to start, dies, or delivers no frame in time,
//! an optional fallback. Platform neutral so the decision logic is tested without a display.

use std::time::Duration;

use crate::error::CaptureError;
use crate::frame::{CaptureSession, SourceInfo};

/// A capture path that is running but has not yet delivered a frame. Dropping it must stop the
/// capture: the driver drops a silent primary before it starts the fallback.
pub trait Started {
    /// Blocks until the first frame is reported (`Ok(Some)`), `timeout` passes (`Ok(None)`), or
    /// the capture ends before producing one (`Err`).
    fn wait_first_frame(&mut self, timeout: Duration) -> Result<Option<SourceInfo>, CaptureError>;
    /// Turns the proven capture into the session the room owns.
    fn into_session(self: Box<Self>, info: SourceInfo) -> Box<dyn CaptureSession>;
}

/// One way to open a source: a name for error messages and the function that starts it.
pub struct Attempt<'a> {
    pub name: &'static str,
    pub start: Box<dyn FnOnce() -> Result<Box<dyn Started>, CaptureError> + 'a>,
}

/// Returns the first attempt that delivers a frame within `timeout`. A primary that fails to
/// start, ends early, or stays silent is dropped, and therefore stopped, before the fallback
/// starts. With nothing left to try, the error names every attempt and why it failed.
pub fn start_with_fallback(
    timeout: Duration,
    primary: Attempt<'_>,
    fallback: Option<Attempt<'_>>,
) -> Result<Box<dyn CaptureSession>, CaptureError> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    const INFO_A: SourceInfo = SourceInfo {
        width: 64,
        height: 32,
        fps: 30,
    };
    const INFO_B: SourceInfo = SourceInfo {
        width: 128,
        height: 64,
        fps: 60,
    };
    const TIMEOUT: Duration = Duration::from_millis(10);

    struct FakeSession(SourceInfo);

    impl CaptureSession for FakeSession {
        fn info(&self) -> SourceInfo {
            self.0
        }
        fn stop(self: Box<Self>) {}
    }

    /// Answers the first-frame wait at once with a fixed outcome; flips `stopped` when dropped.
    struct FakeStarted {
        outcome: Result<Option<SourceInfo>, &'static str>,
        stopped: Arc<AtomicBool>,
    }

    impl Started for FakeStarted {
        fn wait_first_frame(
            &mut self,
            _: Duration,
        ) -> Result<Option<SourceInfo>, CaptureError> {
            self.outcome
                .map_err(|message| CaptureError::SourceLost(message.into()))
        }
        fn into_session(self: Box<Self>, info: SourceInfo) -> Box<dyn CaptureSession> {
            Box::new(FakeSession(info))
        }
    }

    impl Drop for FakeStarted {
        fn drop(&mut self) {
            self.stopped.store(true, Ordering::SeqCst);
        }
    }

    struct Probe {
        called: Arc<AtomicBool>,
        stopped: Arc<AtomicBool>,
    }

    impl Probe {
        fn new() -> Self {
            Self {
                called: Arc::default(),
                stopped: Arc::default(),
            }
        }
        fn called(&self) -> bool {
            self.called.load(Ordering::SeqCst)
        }
        fn stopped(&self) -> bool {
            self.stopped.load(Ordering::SeqCst)
        }
        /// An attempt that starts and then answers the wait with `outcome`.
        fn starting(
            &self,
            name: &'static str,
            outcome: Result<Option<SourceInfo>, &'static str>,
        ) -> Attempt<'static> {
            let (called, stopped) = (self.called.clone(), self.stopped.clone());
            Attempt {
                name,
                start: Box::new(move || {
                    called.store(true, Ordering::SeqCst);
                    Ok(Box::new(FakeStarted { outcome, stopped }) as Box<dyn Started>)
                }),
            }
        }
        /// An attempt whose start itself fails.
        fn failing(&self, name: &'static str, message: &'static str) -> Attempt<'static> {
            let called = self.called.clone();
            Attempt {
                name,
                start: Box::new(move || {
                    called.store(true, Ordering::SeqCst);
                    Err(CaptureError::Windows(message.into()))
                }),
            }
        }
    }

    #[test]
    fn a_primary_that_delivers_is_used_and_the_fallback_never_starts() {
        let (primary, fallback) = (Probe::new(), Probe::new());
        let session = start_with_fallback(
            TIMEOUT,
            primary.starting("graphics capture", Ok(Some(INFO_A))),
            Some(fallback.starting("desktop duplication", Ok(Some(INFO_B)))),
        )
        .unwrap();
        assert_eq!(session.info(), INFO_A);
        assert!(!fallback.called());
    }

    #[test]
    fn a_silent_primary_is_stopped_before_the_fallback_starts() {
        let (primary, fallback) = (Probe::new(), Probe::new());
        let primary_stopped = primary.stopped.clone();
        let fallback_attempt = Attempt {
            name: "desktop duplication",
            start: Box::new({
                let (called, stopped) = (fallback.called.clone(), fallback.stopped.clone());
                move || {
                    called.store(true, Ordering::SeqCst);
                    assert!(
                        primary_stopped.load(Ordering::SeqCst),
                        "the fallback started while the primary still ran"
                    );
                    Ok(Box::new(FakeStarted {
                        outcome: Ok(Some(INFO_B)),
                        stopped,
                    }) as Box<dyn Started>)
                }
            }),
        };
        let session = start_with_fallback(
            TIMEOUT,
            primary.starting("graphics capture", Ok(None)),
            Some(fallback_attempt),
        )
        .unwrap();
        assert_eq!(session.info(), INFO_B);
        assert!(fallback.called());
    }

    #[test]
    fn a_primary_that_fails_to_start_hands_over_to_the_fallback() {
        let (primary, fallback) = (Probe::new(), Probe::new());
        let session = start_with_fallback(
            TIMEOUT,
            primary.failing("graphics capture", "unsupported"),
            Some(fallback.starting("desktop duplication", Ok(Some(INFO_B)))),
        )
        .unwrap();
        assert_eq!(session.info(), INFO_B);
    }

    #[test]
    fn a_primary_that_dies_before_its_first_frame_hands_over_to_the_fallback() {
        let (primary, fallback) = (Probe::new(), Probe::new());
        let session = start_with_fallback(
            TIMEOUT,
            primary.starting("graphics capture", Err("thread ended")),
            Some(fallback.starting("desktop duplication", Ok(Some(INFO_B)))),
        )
        .unwrap();
        assert_eq!(session.info(), INFO_B);
        assert!(primary.stopped());
    }

    #[test]
    fn a_silent_primary_without_a_fallback_reports_the_attempt() {
        let primary = Probe::new();
        let error = start_with_fallback(
            TIMEOUT,
            primary.starting("graphics capture", Ok(None)),
            None,
        )
        .err()
        .expect("no session without a frame");
        let text = error.to_string();
        assert!(text.contains("graphics capture"), "{text}");
        assert!(text.contains("no frame within"), "{text}");
        assert!(primary.stopped());
    }

    #[test]
    fn two_silent_attempts_are_both_named_in_the_error() {
        let (primary, fallback) = (Probe::new(), Probe::new());
        let error = start_with_fallback(
            TIMEOUT,
            primary.starting("graphics capture", Ok(None)),
            Some(fallback.failing("desktop duplication", "no output for monitor")),
        )
        .err()
        .expect("no session without a frame");
        let text = error.to_string();
        assert!(text.contains("graphics capture"), "{text}");
        assert!(text.contains("desktop duplication"), "{text}");
        assert!(text.contains("no output for monitor"), "{text}");
        assert!(primary.stopped());
    }
}
```

Register the module in `crates/capture/src/lib.rs` after `pub mod error;`:

```rust
pub mod fallback;
```

It is public so the Linux build does not report its items as dead code.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p brp-capture fallback`
Expected: six tests FAIL with `not yet implemented`.

- [ ] **Step 3: Implement the driver**

Replace the `todo!()` body:

```rust
    let mut failures = Vec::new();
    for attempt in std::iter::once(primary).chain(fallback) {
        match (attempt.start)() {
            Ok(mut started) => match started.wait_first_frame(timeout) {
                Ok(Some(info)) => return Ok(started.into_session(info)),
                // `started` drops at the end of this arm, so the capture has stopped by the time
                // the loop starts the next attempt.
                Ok(None) => failures.push(format!(
                    "{}: no frame within {:?}",
                    attempt.name, timeout
                )),
                Err(error) => failures.push(format!("{}: {error}", attempt.name)),
            },
            Err(error) => failures.push(format!("{}: {error}", attempt.name)),
        }
    }
    Err(CaptureError::SourceLost(failures.join("; ")))
```

- [ ] **Step 4: Run the tests and lints**

Run: `cargo test -p brp-capture fallback && cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`
Expected: six tests PASS, lints clean.

- [ ] **Step 5: Commit**

```bash
git add crates/capture/src/fallback.rs crates/capture/src/lib.rs
git commit -m "feat: add the capture fallback driver"
```

---

### Task 3: Source ids through the room

**Files:**
- Modify: `crates/room/src/room.rs` (`start_live`, new `sources`)
- Modify: `crates/room/tests/two_rooms.rs`
- Modify: `crates/app/src/window.rs:226` (call site only)
- Modify: `crates/app/src/publish.rs:45`

**Interfaces:**
- Consumes: Task 1's `SourceId`, `SourceListing`, `SourceDescriptor`, `SourceRequest::source`.
- Produces: `Room::start_live(&self, kind: SourceKind, source: Option<SourceId>, title: String) -> Result<u32, RoomError>`; `Room::sources(&self, kind: SourceKind) -> Result<SourceListing, RoomError>`.

- [ ] **Step 1: Write the failing tests**

Append to `crates/room/tests/two_rooms.rs`. Extend the import block at the top so it reads:

```rust
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use brp_capture::{
    CaptureBackend, CaptureError, FrameSink, SourceDescriptor, SourceId, SourceListing,
    SourceRequest, StartFuture, SyntheticSource,
};
```

(Keep any other existing `use` lines. `AtomicUsize` and `Ordering` are already imported somewhere in the file for `CountingCapture`; merge rather than duplicate.)

Then add at the end of the file:

```rust
/// Records the request it was started with and answers `sources` with one fixed choice, so the
/// tests can see what the room passes through without a real platform.
struct RecordingCapture {
    seen: Arc<Mutex<Option<SourceRequest>>>,
    inner: SyntheticSource,
}

impl CaptureBackend for RecordingCapture {
    fn sources(&self, kind: SourceKind) -> Result<SourceListing, CaptureError> {
        Ok(SourceListing::Choices(vec![SourceDescriptor {
            id: SourceId(42),
            kind,
            name: "Fake display".into(),
            width: 64,
            height: 32,
        }]))
    }

    fn start(&self, request: SourceRequest, sink: FrameSink) -> StartFuture<'_> {
        *self.seen.lock().unwrap() = Some(request);
        self.inner.start(request, sink)
    }
}

fn recording_config(seen: Arc<Mutex<Option<SourceRequest>>>) -> RoomConfig {
    let mut cfg = config("alice");
    cfg.capture = Arc::new(RecordingCapture {
        seen,
        inner: SyntheticSource {
            width: 64,
            height: 32,
            fps: 30,
        },
    });
    cfg
}

#[tokio::test]
async fn start_live_passes_the_source_id_into_the_capture_request() {
    let seen = Arc::new(Mutex::new(None));
    let room = Room::create(recording_config(seen.clone())).await.unwrap();

    room.start_live(SourceKind::Window, Some(SourceId(42)), "game".into())
        .await
        .unwrap();

    let request = (*seen.lock().unwrap()).expect("capture was started");
    assert_eq!(request.kind, SourceKind::Window);
    assert_eq!(request.source, Some(SourceId(42)));
    assert_eq!(request.target_fps, 30);
    room.leave().await;
}

#[tokio::test]
async fn sources_passes_the_backend_listing_through() {
    let room = Room::create(recording_config(Arc::default())).await.unwrap();

    let listing = room.sources(SourceKind::Monitor).unwrap();

    match listing {
        SourceListing::Choices(choices) => {
            assert_eq!(choices.len(), 1);
            assert_eq!(choices[0].id, SourceId(42));
            assert_eq!(choices[0].kind, SourceKind::Monitor);
        }
        SourceListing::PlatformPicker => panic!("the fake lists a choice"),
    }
    room.leave().await;
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p brp-room --test two_rooms source`
Expected: compile error, `start_live` takes 2 arguments and `sources` does not exist.

- [ ] **Step 3: Change the room**

In `crates/room/src/room.rs`, change the import line to bring in the new types:

```rust
use brp_capture::{CaptureBackend, SourceId, SourceListing, SourceRequest};
```

Replace the `start_live` signature and its request literal:

```rust
    /// Lists what the platform can share, or says that it picks for itself.
    pub fn sources(&self, kind: SourceKind) -> Result<SourceListing, RoomError> {
        Ok(self.capture.sources(kind)?)
    }

    pub async fn start_live(
        &self,
        kind: SourceKind,
        source: Option<SourceId>,
        title: String,
    ) -> Result<u32, RoomError> {
        // Cheap check before capture opens a session (a portal permission dialog for real users),
        // so a session isn't opened only to be rejected once `add_live` re-checks the same cap.
        if self.registry.live_count() >= MAX_LIVES_PER_PARTICIPANT {
            return Err(RoomError::TooManyLives);
        }
        let fan = Arc::new(CaptureFan::default());
        let sink = fan.clone();
        let session = self
            .capture
            .start(
                SourceRequest {
                    kind,
                    source,
                    target_fps: self.target_fps,
                },
                Box::new(move |frame| sink.push(frame)),
            )
            .await?;
```

The rest of `start_live` is unchanged.

- [ ] **Step 4: Update the callers**

`crates/room/tests/two_rooms.rs`: every existing `start_live(SourceKind::X, "title".into())` and `start_live(SourceKind::Monitor, format!("l{i}"))` gains `None` as the second argument, for example:

```rust
        .start_live(SourceKind::Monitor, None, "desk".into())
```

and

```rust
        room.start_live(SourceKind::Monitor, None, format!("l{i}"))
```

`crates/app/src/window.rs`, inside `share`:

```rust
            let outcome = room
                .start_live(kind, None, title)
```

`crates/app/src/publish.rs`:

```rust
    let live = room.start_live(kind, None, title.into()).await?;
```

- [ ] **Step 5: Run the tests and lints**

Run: `cargo test --workspace && cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`
Expected: green, including the two new room tests.

- [ ] **Step 6: Commit**

```bash
git add crates/room crates/app/src/window.rs crates/app/src/publish.rs
git commit -m "feat: pass a source id through the room and expose the source listing"
```

---

### Task 4: Codec tables for Windows

**Files:**
- Modify: `crates/codec/src/select.rs`
- Modify: `crates/codec/src/ffmpeg/encoder.rs:166-192`
- Modify: `crates/codec/src/ffmpeg/decoder.rs:26-29`

**Interfaces:**
- Produces: nothing new; `PROBE_ORDER` gains two entries, `HW_DEVICE_ORDER` differs per OS.

- [ ] **Step 1: Write the failing test**

Append to `crates/codec/src/select.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::PROBE_ORDER;

    fn position(name: &str) -> usize {
        PROBE_ORDER
            .iter()
            .position(|(candidate, _)| *candidate == name)
            .unwrap_or_else(|| panic!("{name} is not in the probe order"))
    }

    #[test]
    fn media_foundation_is_probed_after_qsv_and_before_the_software_fallback() {
        assert!(position("av1_qsv") < position("hevc_mf"));
        assert!(position("hevc_mf") < position("h264_mf"));
        assert!(position("h264_mf") < position("libsvtav1"));
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p brp-codec media_foundation`
Expected: FAIL with `hevc_mf is not in the probe order`.

- [ ] **Step 3: Extend the tables**

In `crates/codec/src/select.rs`, insert after `("av1_qsv", Codec::Av1),`:

```rust
    ("hevc_mf", Codec::Hevc),
    ("h264_mf", Codec::H264),
```

No platform gate: FFmpeg reports the names it lacks and `open_encoder` already logs and skips them, exactly as the VAAPI entries behave on Windows.

In `crates/codec/src/ffmpeg/encoder.rs`, add an arm to `apply_low_latency_options` after the QSV arm:

```rust
        "h264_mf" | "hevc_mf" => {
            // Hardware only: a software Media Foundation transform would defeat the probe order,
            // where software AV1 is the deliberate last resort.
            set_opt_int(ctx, "hw_encoding", 1)?;
            set_opt(ctx, "rate_control", "cbr")?;
            set_opt(ctx, "scenario", "display_remoting")?;
        }
```

In `crates/codec/src/ffmpeg/decoder.rs`, replace the `HW_DEVICE_ORDER` constant with:

```rust
/// Hardware decoders tried before software: the platform's own API first, then NVDEC.
#[cfg(windows)]
const HW_DEVICE_ORDER: [(ff::AVHWDeviceType, &str); 2] = [
    (ff::AVHWDeviceType::AV_HWDEVICE_TYPE_D3D11VA, "d3d11va"),
    (ff::AVHWDeviceType::AV_HWDEVICE_TYPE_CUDA, "cuda"),
];
#[cfg(not(windows))]
const HW_DEVICE_ORDER: [(ff::AVHWDeviceType, &str); 2] = [
    (ff::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI, "vaapi"),
    (ff::AVHWDeviceType::AV_HWDEVICE_TYPE_CUDA, "cuda"),
];
```

- [ ] **Step 4: Run the tests and lints, then the cross-check**

Run: `cargo test -p brp-codec && cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`
Expected: green.

Run the cross-check command from Global Constraints (skip if `/tmp/ffwin` or `/home/gtkacz/.xwin` is missing).
Expected: `brp-codec` checks; `brp-capture` checks as well since Task 1 gated the example.

- [ ] **Step 5: Commit**

```bash
git add crates/codec
git commit -m "feat: probe Media Foundation encoders and D3D11VA decode on Windows"
```

---

### Task 5: The Windows capture backend

**Files:**
- Modify: `Cargo.toml` (workspace dependencies)
- Modify: `crates/capture/Cargo.toml`
- Modify: `crates/capture/src/lib.rs`
- Create: `crates/capture/src/windows/mod.rs`
- Create: `crates/capture/src/windows/sources.rs`
- Create: `crates/capture/src/windows/graphics_capture.rs`
- Create: `crates/capture/src/windows/duplication.rs`

**Interfaces:**
- Consumes: Task 1's trait and types, Task 2's `Started`, `Attempt`, `start_with_fallback`, `CAPTURE_FALLBACK_TIMEOUT`, `brp_proto::monotonic_us`, `brp_proto::PixelFormat::Bgra`.
- Produces: `brp_capture::PlatformCapture` on Windows (`WindowsCapture`, a unit struct implementing `CaptureBackend` with `sources` returning `Choices`).

This task has no hardware-free test of its own: nothing here compiles on Linux. Its verification is the cross-check and the Windows CI job of Task 7. Write the code exactly as given; the signatures were checked against windows-capture 2.0.1 (Global Constraints).

- [ ] **Step 1: Dependencies**

In the root `Cargo.toml` under `[workspace.dependencies]` add:

```toml
windows-capture = "2.0"
```

In `crates/capture/Cargo.toml` add after the Linux target block:

```toml
[target.'cfg(windows)'.dependencies]
windows-capture.workspace = true
tokio = { workspace = true, features = ["rt", "sync", "time"] }
```

In `crates/capture/src/lib.rs` add after the Linux lines:

```rust
#[cfg(windows)]
pub mod windows;
#[cfg(windows)]
pub use windows::WindowsCapture as PlatformCapture;
```

- [ ] **Step 2: The source list**

Create `crates/capture/src/windows/sources.rs`:

```rust
//! Monitors and windows as source descriptors, and the way back from an id to a handle.

use std::ffi::c_void;

use brp_proto::SourceKind;
use windows_capture::monitor::Monitor;
use windows_capture::window::Window;

use crate::error::CaptureError;
use crate::frame::{SourceDescriptor, SourceId};

/// Used when Windows cannot report a refresh rate; the room caps it with `--fps` anyway.
const DEFAULT_REFRESH_RATE: u32 = 60;

/// A resolved source: the handle capture is opened on.
#[derive(Clone, Copy)]
pub(super) enum Target {
    Monitor(Monitor),
    Window(Window),
}

impl Target {
    /// The rate frames can arrive at: the monitor's refresh rate, or that of the monitor the
    /// window is on.
    pub(super) fn refresh_rate(&self) -> u32 {
        let rate = match self {
            Target::Monitor(monitor) => monitor.refresh_rate().ok(),
            Target::Window(window) => window.monitor().and_then(|m| m.refresh_rate().ok()),
        };
        rate.filter(|rate| *rate > 0).unwrap_or(DEFAULT_REFRESH_RATE)
    }
}

pub(super) fn list(kind: SourceKind) -> Result<Vec<SourceDescriptor>, CaptureError> {
    match kind {
        SourceKind::Monitor => Monitor::enumerate()
            .map_err(|error| windows_error("monitor enumeration", error))?
            .into_iter()
            .enumerate()
            .map(|(index, monitor)| {
                Ok(SourceDescriptor {
                    id: monitor_id(&monitor),
                    kind,
                    name: monitor
                        .name()
                        .unwrap_or_else(|_| format!("Monitor {}", index + 1)),
                    width: monitor
                        .width()
                        .map_err(|error| windows_error("monitor width", error))?,
                    height: monitor
                        .height()
                        .map_err(|error| windows_error("monitor height", error))?,
                })
            })
            .collect(),
        SourceKind::Window => {
            // Sharing brp's own window would only ever show the picker to the room.
            let own_process = std::process::id();
            Ok(Window::enumerate()
                .map_err(|error| windows_error("window enumeration", error))?
                .into_iter()
                .filter(|window| window.process_id().is_ok_and(|pid| pid != own_process))
                .filter_map(|window| {
                    let title = window.title().ok().filter(|title| !title.is_empty())?;
                    Some(SourceDescriptor {
                        id: window_id(&window),
                        kind,
                        name: title,
                        width: u32::try_from(window.width().ok()?).ok()?,
                        height: u32::try_from(window.height().ok()?).ok()?,
                    })
                })
                .collect())
        }
    }
}

/// Turns a request into a handle. No id means the primary monitor for monitors, so the headless
/// publisher works without a picker, and an error for windows, which have no sensible default.
pub(super) fn resolve(kind: SourceKind, source: Option<SourceId>) -> Result<Target, CaptureError> {
    match (kind, source) {
        (SourceKind::Monitor, None) => Monitor::primary()
            .map(Target::Monitor)
            .map_err(|error| windows_error("primary monitor", error)),
        (SourceKind::Monitor, Some(id)) => {
            let monitor = Monitor::from_raw_hmonitor(id.0 as usize as *mut c_void);
            let attached = Monitor::enumerate()
                .map_err(|error| windows_error("monitor enumeration", error))?
                .contains(&monitor);
            if attached {
                Ok(Target::Monitor(monitor))
            } else {
                Err(CaptureError::SourceLost(format!(
                    "monitor {} is no longer attached",
                    id.0
                )))
            }
        }
        (SourceKind::Window, None) => Err(CaptureError::SourceLost(
            "window sharing needs a window picked from the list".into(),
        )),
        (SourceKind::Window, Some(id)) => {
            let window = Window::from_raw_hwnd(id.0 as usize as *mut c_void);
            if window.is_valid() {
                Ok(Target::Window(window))
            } else {
                Err(CaptureError::SourceLost(format!(
                    "window {} is no longer open",
                    id.0
                )))
            }
        }
    }
}

fn monitor_id(monitor: &Monitor) -> SourceId {
    SourceId(monitor.as_raw_hmonitor() as usize as u64)
}

fn window_id(window: &Window) -> SourceId {
    SourceId(window.as_raw_hwnd() as usize as u64)
}

fn windows_error(call: &str, error: impl std::fmt::Display) -> CaptureError {
    CaptureError::Windows(format!("{call} failed: {error}"))
}
```

- [ ] **Step 3: The Graphics Capture session**

Create `crates/capture/src/windows/graphics_capture.rs`:

```rust
//! A Windows Graphics Capture session on the windows-capture crate's own capture thread.

use std::sync::PoisonError;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use brp_proto::{PixelFormat, monotonic_us};
use windows_capture::capture::{CaptureControl, Context, GraphicsCaptureApiHandler};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::{GraphicsCaptureApi, InternalCaptureControl};
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
    GraphicsCaptureItemType, MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
};

use super::SharedSink;
use super::sources::Target;
use crate::error::CaptureError;
use crate::fallback::Started;
use crate::frame::{CaptureFrame, CaptureSession, SourceInfo};

type Control = CaptureControl<Handler, CaptureError>;

/// Starts capturing `target`. `refresh_rate` is what the session reports as its frame rate;
/// `target_fps` caps how often Graphics Capture wakes us where the OS supports that.
pub(super) fn start(
    target: Target,
    target_fps: u32,
    refresh_rate: u32,
    sink: SharedSink,
) -> Result<Box<dyn Started>, CaptureError> {
    let (first_tx, first_rx) = mpsc::channel();
    let flags = HandlerFlags {
        sink,
        first: first_tx,
        fps: refresh_rate,
    };
    let control = match target {
        Target::Monitor(monitor) => run(monitor, target_fps, flags)?,
        Target::Window(window) => run(window, target_fps, flags)?,
    };
    Ok(Box::new(GcStarted {
        control: Some(control),
        first: first_rx,
    }))
}

fn run<T>(item: T, target_fps: u32, flags: HandlerFlags) -> Result<Control, CaptureError>
where
    T: TryInto<GraphicsCaptureItemType> + Send + 'static,
{
    let settings = Settings::new(
        item,
        cursor(),
        border(),
        SecondaryWindowSettings::Default,
        update_interval(target_fps),
        DirtyRegionSettings::Default,
        ColorFormat::Bgra8,
        flags,
    );
    Handler::start_free_threaded(settings)
        .map_err(|error| CaptureError::Windows(format!("graphics capture failed to start: {error}")))
}

// Each setting is only requested where the OS build supports it; asking on an older build fails
// the whole session instead of being ignored.
fn cursor() -> CursorCaptureSettings {
    match GraphicsCaptureApi::is_cursor_settings_supported() {
        Ok(true) => CursorCaptureSettings::WithCursor,
        _ => CursorCaptureSettings::Default,
    }
}

fn border() -> DrawBorderSettings {
    match GraphicsCaptureApi::is_border_settings_supported() {
        Ok(true) => DrawBorderSettings::WithoutBorder,
        _ => DrawBorderSettings::Default,
    }
}

fn update_interval(target_fps: u32) -> MinimumUpdateIntervalSettings {
    match GraphicsCaptureApi::is_minimum_update_interval_supported() {
        Ok(true) => {
            MinimumUpdateIntervalSettings::Custom(Duration::from_secs(1) / target_fps.max(1))
        }
        _ => MinimumUpdateIntervalSettings::Default,
    }
}

struct HandlerFlags {
    sink: SharedSink,
    first: Sender<SourceInfo>,
    fps: u32,
}

/// Runs on the capture thread: copies each frame out of its D3D texture and hands it to the sink.
struct Handler {
    sink: SharedSink,
    first: Option<Sender<SourceInfo>>,
    fps: u32,
}

impl GraphicsCaptureApiHandler for Handler {
    type Flags = HandlerFlags;
    type Error = CaptureError;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        Ok(Self {
            sink: ctx.flags.sink,
            first: Some(ctx.flags.first),
            fps: ctx.flags.fps,
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        _: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        let mut buffer = frame
            .buffer()
            .map_err(|error| CaptureError::Windows(format!("frame readback failed: {error}")))?;
        let captured = CaptureFrame {
            width: buffer.width(),
            height: buffer.height(),
            stride: buffer.row_pitch() as usize,
            format: PixelFormat::Bgra,
            data: buffer.as_raw_buffer().to_vec(),
            capture_ts_us: monotonic_us(),
        };
        if let Some(first) = self.first.take() {
            // The receiver is gone once the fallback driver stopped waiting; a late first frame
            // is not an error.
            let _ = first.send(SourceInfo {
                width: captured.width,
                height: captured.height,
                fps: self.fps,
            });
        }
        let mut sink = self.sink.lock().unwrap_or_else(PoisonError::into_inner);
        (*sink)(captured);
        Ok(())
    }

    fn on_closed(&mut self) -> Result<(), Self::Error> {
        tracing::warn!("captured window closed; the live keeps its last frame until stopped");
        Ok(())
    }
}

struct GcStarted {
    control: Option<Control>,
    first: Receiver<SourceInfo>,
}

impl Started for GcStarted {
    fn wait_first_frame(&mut self, timeout: Duration) -> Result<Option<SourceInfo>, CaptureError> {
        match self.first.recv_timeout(timeout) {
            Ok(info) => Ok(Some(info)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(CaptureError::SourceLost(
                "graphics capture ended before its first frame".into(),
            )),
        }
    }

    fn into_session(mut self: Box<Self>, info: SourceInfo) -> Box<dyn CaptureSession> {
        Box::new(GcSession {
            info,
            control: self.control.take(),
        })
    }
}

impl Drop for GcStarted {
    fn drop(&mut self) {
        stop(self.control.take());
    }
}

struct GcSession {
    info: SourceInfo,
    control: Option<Control>,
}

impl CaptureSession for GcSession {
    fn info(&self) -> SourceInfo {
        self.info
    }

    fn stop(mut self: Box<Self>) {
        stop(self.control.take());
    }
}

impl Drop for GcSession {
    fn drop(&mut self) {
        stop(self.control.take());
    }
}

fn stop(control: Option<Control>) {
    if let Some(control) = control
        && let Err(error) = control.stop()
    {
        tracing::warn!(%error, "graphics capture did not stop cleanly");
    }
}
```

- [ ] **Step 4: The desktop duplication session**

Create `crates/capture/src/windows/duplication.rs`:

```rust
//! DXGI desktop duplication on a thread of our own: the fallback for monitors that Graphics
//! Capture cannot serve, typically exclusive-fullscreen games.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use brp_proto::{PixelFormat, monotonic_us};
use windows_capture::dxgi_duplication_api::{DxgiDuplicationApi, DxgiDuplicationFormat, Error};
use windows_capture::monitor::Monitor;

use super::SharedSink;
use crate::error::CaptureError;
use crate::fallback::Started;
use crate::frame::{CaptureFrame, CaptureSession, SourceInfo};

/// The converter expects BGRA; asking for it up front avoids a per-frame format check.
const FORMATS: [DxgiDuplicationFormat; 1] = [DxgiDuplicationFormat::Bgra8];

type FirstFrame = Result<SourceInfo, CaptureError>;

pub(super) fn start(
    monitor: Monitor,
    fps: u32,
    sink: SharedSink,
) -> Result<Box<dyn Started>, CaptureError> {
    let stop = Arc::new(AtomicBool::new(false));
    let (first_tx, first_rx) = mpsc::channel();
    let thread = thread::Builder::new()
        .name("brp-duplication".into())
        .spawn({
            let stop = stop.clone();
            move || run(monitor, fps, sink, first_tx, stop)
        })
        .map_err(|error| {
            CaptureError::Windows(format!("failed to spawn the duplication thread: {error}"))
        })?;
    Ok(Box::new(DupStarted {
        stop,
        thread: Some(thread),
        first: first_rx,
    }))
}

/// The duplication is created on this thread so every D3D call for it happens in one place.
fn run(
    monitor: Monitor,
    fps: u32,
    sink: SharedSink,
    first: Sender<FirstFrame>,
    stop: Arc<AtomicBool>,
) {
    let mut duplication = match DxgiDuplicationApi::new_options(monitor, &FORMATS) {
        Ok(duplication) => duplication,
        Err(error) => {
            let _ = first.send(Err(CaptureError::Windows(format!(
                "desktop duplication failed to start: {error}"
            ))));
            return;
        }
    };
    let timeout_ms = (1_000 / fps.max(1)).max(1);
    let mut first = Some(first);
    while !stop.load(Ordering::Relaxed) {
        match next_frame(&mut duplication, timeout_ms) {
            Ok(Some(frame)) => {
                if let Some(first) = first.take() {
                    let _ = first.send(Ok(SourceInfo {
                        width: frame.width,
                        height: frame.height,
                        fps,
                    }));
                }
                let mut sink = sink.lock().unwrap_or_else(PoisonError::into_inner);
                (*sink)(frame);
            }
            Ok(None) => {}
            // Windows drops the duplication on mode changes and desktop switches; a new one
            // carries on from the current desktop.
            Err(Error::AccessLost) => match duplication.recreate_options(&FORMATS) {
                Ok(recreated) => duplication = recreated,
                Err(error) => {
                    tracing::warn!(%error, "desktop duplication could not be recreated; the live keeps its last frame");
                    return;
                }
            },
            Err(error) => {
                tracing::warn!(%error, "desktop duplication ended; the live keeps its last frame");
                return;
            }
        }
    }
}

/// `Ok(None)` when nothing changed within the timeout. Returns an owned frame so the caller can
/// recreate the duplication with no borrow outstanding.
fn next_frame(
    duplication: &mut DxgiDuplicationApi,
    timeout_ms: u32,
) -> Result<Option<CaptureFrame>, Error> {
    let mut frame = match duplication.acquire_next_frame(timeout_ms) {
        Ok(frame) => frame,
        Err(Error::Timeout) => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut buffer = frame.buffer()?;
    Ok(Some(CaptureFrame {
        width: buffer.width(),
        height: buffer.height(),
        stride: buffer.row_pitch() as usize,
        format: PixelFormat::Bgra,
        data: buffer.as_raw_buffer().to_vec(),
        capture_ts_us: monotonic_us(),
    }))
}

struct DupStarted {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    first: Receiver<FirstFrame>,
}

impl Started for DupStarted {
    fn wait_first_frame(&mut self, timeout: Duration) -> Result<Option<SourceInfo>, CaptureError> {
        match self.first.recv_timeout(timeout) {
            Ok(result) => result.map(Some),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(CaptureError::SourceLost(
                "desktop duplication ended before its first frame".into(),
            )),
        }
    }

    fn into_session(mut self: Box<Self>, info: SourceInfo) -> Box<dyn CaptureSession> {
        Box::new(DupSession {
            info,
            stop: self.stop.clone(),
            thread: self.thread.take(),
        })
    }
}

impl Drop for DupStarted {
    fn drop(&mut self) {
        shutdown(&self.stop, self.thread.take());
    }
}

struct DupSession {
    info: SourceInfo,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl CaptureSession for DupSession {
    fn info(&self) -> SourceInfo {
        self.info
    }

    fn stop(mut self: Box<Self>) {
        shutdown(&self.stop, self.thread.take());
    }
}

impl Drop for DupSession {
    fn drop(&mut self) {
        shutdown(&self.stop, self.thread.take());
    }
}

fn shutdown(stop: &AtomicBool, thread: Option<JoinHandle<()>>) {
    stop.store(true, Ordering::Relaxed);
    if let Some(thread) = thread
        && thread.join().is_err()
    {
        tracing::warn!("duplication thread panicked");
    }
}
```

- [ ] **Step 5: The backend**

Create `crates/capture/src/windows/mod.rs`:

```rust
//! Windows capture: Graphics Capture for monitors and windows, desktop duplication as the monitor
//! fallback, and a source list for the in-app picker.

mod duplication;
mod graphics_capture;
mod sources;

use std::sync::{Arc, Mutex};

use brp_proto::SourceKind;
use brp_proto::constants::CAPTURE_FALLBACK_TIMEOUT;

use crate::error::CaptureError;
use crate::fallback::{Attempt, start_with_fallback};
use crate::frame::{
    CaptureBackend, CaptureSession, FrameSink, SourceListing, SourceRequest, StartFuture,
};

use self::sources::Target;

/// The sink is shared between the primary and the fallback attempt. Only one capture thread runs
/// at a time, so the lock is never contended.
pub(crate) type SharedSink = Arc<Mutex<FrameSink>>;

/// Captures one monitor or window chosen from [`CaptureBackend::sources`].
pub struct WindowsCapture;

impl CaptureBackend for WindowsCapture {
    fn sources(&self, kind: SourceKind) -> Result<SourceListing, CaptureError> {
        sources::list(kind).map(SourceListing::Choices)
    }

    fn start(&self, request: SourceRequest, sink: FrameSink) -> StartFuture<'_> {
        Box::pin(async move {
            // Both attempts block on their first frame for up to the fallback timeout each.
            tokio::task::spawn_blocking(move || start_blocking(request, sink))
                .await
                .map_err(|error| {
                    CaptureError::Windows(format!("capture start task failed: {error}"))
                })?
        })
    }
}

fn start_blocking(
    request: SourceRequest,
    sink: FrameSink,
) -> Result<Box<dyn CaptureSession>, CaptureError> {
    let target = sources::resolve(request.kind, request.source)?;
    let refresh_rate = target.refresh_rate();
    let sink: SharedSink = Arc::new(Mutex::new(sink));
    let primary = Attempt {
        name: "graphics capture",
        start: Box::new({
            let sink = sink.clone();
            move || graphics_capture::start(target, request.target_fps, refresh_rate, sink)
        }),
    };
    let fallback = match target {
        Target::Monitor(monitor) => Some(Attempt {
            name: "desktop duplication",
            start: Box::new(move || duplication::start(monitor, refresh_rate, sink)),
        }),
        Target::Window(_) => None,
    };
    let session = start_with_fallback(CAPTURE_FALLBACK_TIMEOUT, primary, fallback)?;
    tracing::info!(kind = ?request.kind, info = ?session.info(), "windows capture started");
    Ok(session)
}
```

- [ ] **Step 6: Cross-check and Linux lints**

Run the cross-check command from Global Constraints (skip if the directories are missing).
Expected: `brp-capture` checks for `x86_64-pc-windows-msvc` with no errors. Fix any compile error in the three new files before moving on; do not change the trait or the driver to make Windows code fit.

Run: `cargo test --workspace && cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`
Expected: unchanged green on Linux (the module is compiled out).

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock crates/capture
git commit -m "feat: add the Windows capture backend with a desktop duplication fallback"
```

---

### Task 6: The in-app picker

**Files:**
- Modify: `crates/app/src/commands.rs`
- Modify: `crates/app/src/ui/state.rs`
- Create: `crates/app/src/ui/picker.rs`
- Modify: `crates/app/src/ui/mod.rs`
- Modify: `crates/app/src/ui/own_lives.rs:22-42`
- Modify: `crates/app/src/window.rs`
- Modify: `crates/app/src/participant.rs`
- Modify: `crates/app/src/publish.rs`

**Interfaces:**
- Consumes: `brp_capture::{PlatformCapture, SourceDescriptor, SourceId, SourceListing}`, `Room::{sources, start_live(kind, source, title)}`.
- Produces: `RoomCommand::Share { kind: SourceKind, source: Option<SourceId> }`; `UiState { picker: Option<SourcePicker>, .. }` with `open_picker(&mut self, kind, choices) -> bool`, `pick_source(&mut self, id) -> Option<RoomCommand>`, `cancel_picker(&mut self)`; `ui::picker::draw(ctx: &egui::Context, state: &mut UiState, commands: &mut Vec<RoomCommand>)`.

- [ ] **Step 1: Write the failing tests**

Append to the `tests` module in `crates/app/src/ui/state.rs`:

```rust
    fn descriptor(id: u64) -> SourceDescriptor {
        SourceDescriptor {
            id: SourceId(id),
            kind: SourceKind::Monitor,
            name: format!("Monitor {id}"),
            width: 1920,
            height: 1080,
        }
    }

    #[test]
    fn picking_a_listed_source_issues_the_share_and_closes_the_picker() {
        let mut state = UiState::new();
        assert!(state.open_picker(SourceKind::Monitor, vec![descriptor(1), descriptor(2)]));
        assert_eq!(
            state.pick_source(SourceId(2)),
            Some(RoomCommand::Share {
                kind: SourceKind::Monitor,
                source: Some(SourceId(2)),
            })
        );
        assert!(state.picker.is_none());
    }

    #[test]
    fn an_unlisted_id_or_a_closed_picker_yields_no_command() {
        let mut state = UiState::new();
        assert_eq!(state.pick_source(SourceId(1)), None);
        assert!(state.open_picker(SourceKind::Window, vec![descriptor(1)]));
        assert_eq!(state.pick_source(SourceId(9)), None);
        assert!(state.picker.is_some());
        state.cancel_picker();
        assert!(state.picker.is_none());
    }

    #[test]
    fn the_picker_does_not_open_while_a_share_is_pending() {
        let mut state = UiState::new();
        state.share_pending = true;
        assert!(!state.open_picker(SourceKind::Monitor, vec![descriptor(1)]));
        assert!(state.picker.is_none());
    }
```

Add to that tests module's imports:

```rust
    use brp_capture::{SourceDescriptor, SourceId};
    use crate::commands::RoomCommand;
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p brp picker`
Expected: compile error, no `open_picker` on `UiState` and no `Share { .. }` variant.

- [ ] **Step 3: The command and the state**

In `crates/app/src/commands.rs` change the import and the variant:

```rust
use brp_capture::SourceId;
use brp_proto::{Preset, SourceKind};
```

```rust
    /// Starts a new live of this kind. Without a source the window asks the room for the
    /// platform's listing and either starts at once or opens the picker.
    Share {
        kind: SourceKind,
        source: Option<SourceId>,
    },
```

In `crates/app/src/ui/state.rs` add imports:

```rust
use brp_capture::{SourceDescriptor, SourceId};

use crate::commands::RoomCommand;
```

Add above `UiState`:

```rust
/// The source list the user is choosing from, on platforms without a picker of their own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourcePicker {
    pub kind: SourceKind,
    pub choices: Vec<SourceDescriptor>,
}
```

In `UiState`, change the `share_pending` doc and add the field right after it:

```rust
    /// True from the share click until the live started or failed: the portal dialog on Linux,
    /// capture start with its fallback on Windows.
    pub share_pending: bool,
    /// Open while the user picks a source from the platform's list.
    pub picker: Option<SourcePicker>,
```

Add methods to `impl UiState` after `next_title`:

```rust
    /// Opens the picker unless a share is already under way. Returns whether it opened.
    pub fn open_picker(&mut self, kind: SourceKind, choices: Vec<SourceDescriptor>) -> bool {
        if self.share_pending {
            return false;
        }
        self.picker = Some(SourcePicker { kind, choices });
        true
    }

    /// The share command for a picked source, closing the picker. `None` when no picker is open
    /// or the id is not one of its choices.
    pub fn pick_source(&mut self, id: SourceId) -> Option<RoomCommand> {
        let picker = self.picker.as_ref()?;
        if !picker.choices.iter().any(|choice| choice.id == id) {
            return None;
        }
        let kind = picker.kind;
        self.picker = None;
        Some(RoomCommand::Share {
            kind,
            source: Some(id),
        })
    }

    pub fn cancel_picker(&mut self) {
        self.picker = None;
    }
```

- [ ] **Step 4: Run the state tests**

Run: `cargo test -p brp picker`
Expected: the three new tests PASS (the rest of the crate may not compile yet because `own_lives.rs` and `window.rs` still build `Share(kind)`; if so, finish step 5 first and rerun).

- [ ] **Step 5: The panel, the picker window, and the app**

In `crates/app/src/ui/own_lives.rs` replace the two `commands.push(...)` lines and the waiting text:

```rust
                    commands.push(RoomCommand::Share {
                        kind: SourceKind::Monitor,
                        source: None,
                    });
```

```rust
                    commands.push(RoomCommand::Share {
                        kind: SourceKind::Window,
                        source: None,
                    });
```

```rust
                if state.share_pending {
                    ui.weak("starting the share");
                }
```

Create `crates/app/src/ui/picker.rs`:

```rust
//! The source picker: a centred window listing what the platform can share, drawn only on
//! platforms without a picker of their own.

use brp_proto::SourceKind;

use super::state::UiState;
use crate::commands::RoomCommand;

const MAX_LIST_HEIGHT: f32 = 400.0;

/// Draws the picker when one is open and pushes the share command for a chosen source.
pub fn draw(ctx: &egui::Context, state: &mut UiState, commands: &mut Vec<RoomCommand>) {
    // Cloned so the window closure does not borrow `state` while it draws.
    let Some(picker) = state.picker.clone() else {
        return;
    };
    let title = match picker.kind {
        SourceKind::Monitor => "Share a monitor",
        SourceKind::Window => "Share a window",
    };
    let mut picked = None;
    let mut cancelled = false;
    egui::Window::new(title)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            if picker.choices.is_empty() {
                ui.weak("nothing to share");
            }
            egui::ScrollArea::vertical()
                .max_height(MAX_LIST_HEIGHT)
                .show(ui, |ui| {
                    for choice in &picker.choices {
                        let label =
                            format!("{} ({}x{})", choice.name, choice.width, choice.height);
                        if ui.selectable_label(false, label).clicked() {
                            picked = Some(choice.id);
                        }
                    }
                });
            ui.separator();
            if ui.button("Cancel").clicked() {
                cancelled = true;
            }
        });
    if let Some(id) = picked {
        commands.extend(state.pick_source(id));
    } else if cancelled {
        state.cancel_picker();
    }
}
```

In `crates/app/src/ui/mod.rs` add `pub mod picker;` to the module list (alphabetical, after `own_lives`) and call it in `draw` after the tiles:

```rust
    let tile_rects = tiles::draw(ui, snapshot, state, &mut commands);
    picker::draw(ui.ctx(), state, &mut commands);
```

In `crates/app/src/window.rs`:

Change the import block to add the listing type:

```rust
use brp_capture::SourceListing;
use brp_proto::SourceKind;
```

Update the two docs that mention the portal:

```rust
    /// The share task finished: the live started, or the error to show.
    ShareFinished(Result<(), String>),
```

```rust
    /// A share still waiting on capture holds an `Arc<Room>`; the caller aborts it before leaving.
    pub fn take_pending_share(&mut self) -> Option<JoinHandle<()>> {
```

Replace the `Share` arm in `apply`:

```rust
                RoomCommand::Share {
                    kind,
                    source: Some(source),
                } => {
                    self.share(kind, Some(source));
                    Ok(())
                }
                RoomCommand::Share { kind, source: None } => match self.room.sources(kind) {
                    Ok(SourceListing::PlatformPicker) => {
                        self.share(kind, None);
                        Ok(())
                    }
                    Ok(SourceListing::Choices(choices)) => {
                        self.state.open_picker(kind, choices);
                        Ok(())
                    }
                    Err(error) => Err(error),
                },
```

Replace `share`:

```rust
    fn share(&mut self, kind: SourceKind, source: Option<brp_capture::SourceId>) {
        if self.pending_share.is_some() {
            return;
        }
        let title = self.state.next_title(kind);
        self.state.share_pending = true;
        self.state.status.clear();
        let room = self.room.clone();
        let proxy = self.proxy.clone();
        self.pending_share = Some(self.runtime.spawn(async move {
            let outcome = room
                .start_live(kind, source, title)
                .await
                .map(|_live_id| ())
                .map_err(|error| error.to_string());
            let _ = proxy.send_event(AppEvent::ShareFinished(outcome));
        }));
    }
```

(Import `SourceId` alongside `SourceListing` instead of the path if you prefer; either is fine.)

In `crates/app/src/participant.rs` and `crates/app/src/publish.rs` replace the import and the construction:

```rust
use brp_capture::PlatformCapture;
```

```rust
            capture: Arc::new(PlatformCapture),
```

- [ ] **Step 6: Run the tests and lints**

Run: `cargo test --workspace && cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`
Expected: green.

Run: `cargo run -p brp -- create --no-relay` for a few seconds and click "Share monitor". On Linux the portal opens as before; no picker window appears. Close the app.

- [ ] **Step 7: Commit**

```bash
git add crates/app
git commit -m "feat: pick a source in the window where the platform lists them"
```

---

### Task 7: Windows CI job, artifact, and README

**Files:**
- Modify: `.github/workflows/ci.yml`
- Modify: `README.md` (the "Windows prerequisites" section)

**Interfaces:**
- Consumes: everything above. Produces the artifact `brp-windows-x86_64`.

- [ ] **Step 1: Add the Windows job**

Append to `.github/workflows/ci.yml` under `jobs:`:

```yaml
  windows:
    runs-on: windows-latest
    env:
      FFMPEG_DIR: ${{ github.workspace }}\ffmpeg
      LIBCLANG_PATH: C:\Program Files\LLVM\bin
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy
      - uses: Swatinem/rust-cache@v2
      - name: Ensure LLVM for bindgen
        shell: pwsh
        run: |
          if (-not (Test-Path "$env:LIBCLANG_PATH\libclang.dll")) { choco install llvm -y --no-progress }
      - name: Install FFmpeg (BtbN LGPL shared, pinned)
        shell: pwsh
        run: |
          $release = "autobuild-2026-09-05-13-10"
          $asset = "ffmpeg-n8.1.2-50-g1a748fe2cd-win64-lgpl-shared-8.1.zip"
          Invoke-WebRequest -Uri "https://github.com/BtbN/FFmpeg-Builds/releases/download/$release/$asset" -OutFile ffmpeg.zip
          Expand-Archive ffmpeg.zip -DestinationPath ffmpeg-extract
          Move-Item (Get-ChildItem ffmpeg-extract | Select-Object -First 1).FullName $env:FFMPEG_DIR
          Add-Content $env:GITHUB_PATH "$env:FFMPEG_DIR\bin"
      - run: cargo clippy --workspace --all-targets -- -D warnings
      - run: cargo test --workspace
      - run: cargo build --release -p brp
      - name: Stage the artifact
        shell: pwsh
        run: |
          New-Item -ItemType Directory dist | Out-Null
          Copy-Item target\release\brp.exe dist\
          foreach ($dll in "avcodec-62.dll", "avutil-60.dll", "swscale-9.dll", "swresample-6.dll") {
            Copy-Item "$env:FFMPEG_DIR\bin\$dll" dist\
          }
          Copy-Item "$env:FFMPEG_DIR\LICENSE.txt" dist\FFMPEG-LICENSE.txt
          Copy-Item LICENSE dist\LICENSE
      - uses: actions/upload-artifact@v4
        with:
          name: brp-windows-x86_64
          path: dist
```

The `linux` job is unchanged; rustfmt runs there only.

- [ ] **Step 2: README**

In `README.md`, extend the "Windows prerequisites" section. After the existing paragraph append:

````markdown
To build on Windows, install Visual Studio Build Tools with the C++ workload,
LLVM (bindgen needs `libclang`), and the FFmpeg LGPL shared build that CI pins,
BtbN release `autobuild-2026-09-05-13-10`, asset
`ffmpeg-n8.1.2-50-g1a748fe2cd-win64-lgpl-shared-8.1.zip`. Extract it and set:

```powershell
$env:FFMPEG_DIR = "C:\path\to\ffmpeg-n8.1.2-50-g1a748fe2cd-win64-lgpl-shared-8.1"
$env:LIBCLANG_PATH = "C:\Program Files\LLVM\bin"
$env:PATH = "$env:FFMPEG_DIR\bin;$env:PATH"
cargo build --release
```

Every push builds the same thing on GitHub Actions: the `windows` job uploads
`brp-windows-x86_64`, a zip with `brp.exe`, the four FFmpeg DLLs it links, and
both licences. Download it from the run's artifacts, extract, and run
`brp.exe` from that directory.

Sharing on Windows opens an in-app list of monitors or windows instead of a
system dialog. A monitor that Graphics Capture cannot deliver within two
seconds is captured through desktop duplication instead.
````

- [ ] **Step 3: Lints and commit**

Run: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: green.

```bash
git add .github/workflows/ci.yml README.md
git commit -m "ci: build, test, and package brp on Windows"
```

- [ ] **Step 4: Push and watch the Windows job**

Pushing to `main` triggers CI. Confirm with the user before the first push if they have not said the branch may be pushed. Then:

```bash
git push
gh run watch --exit-status
```

Expected: both jobs green; the `windows` job shows an uploaded artifact `brp-windows-x86_64`.

If the `windows` job fails:
- A bindgen error mentioning `libclang` means the LLVM path differs on the runner image; read the failing step's log, fix `LIBCLANG_PATH` in the workflow, commit as `ci: fix the LLVM path on the Windows runner`, and push again.
- A compile error in `crates/capture/src/windows/` is a real defect in Task 5; fix it in a `fix:` commit against the windows-capture 2.0.1 signatures listed in Global Constraints. Do not change the trait or driver.
- A failing room test with a socket bind error is the runner's firewall; report it to the user rather than disabling the test.

Repeat until green. The phase closes on CI evidence; runtime checks stay deferred as the spec says.

## Plan self-review notes

- Spec coverage: 5.1 → Task 1; 5.2 and 5.3 → Tasks 2, 5; 5.4 → Task 4; 5.5 → Tasks 3, 6; 5.6 → Task 7; section 9 error texts → Task 5 `sources.rs`, `mod.rs`, and Task 6 status line via `RoomError` display; section 10 hardware-free tests → Tasks 2, 3, 4, 6; section 11 constant → Task 1; deferred checks stated in Task 7 step 4.
- Type consistency: `Started::wait_first_frame` returns `Result<Option<SourceInfo>, CaptureError>` in Task 2 and both Task 5 implementations; `Attempt.start` returns `Result<Box<dyn Started>, CaptureError>` everywhere; `RoomCommand::Share { kind, source }` in Tasks 6 only, `start_live(kind, source, title)` from Task 3 onward; `SharedSink` is `Arc<Mutex<FrameSink>>` across the three Windows modules.
- The one thing this plan cannot prove locally is the windows-capture generic bound on `start_free_threaded`, `T: TryInto<GraphicsCaptureItemType> + Send + 'static`, being satisfied by `Monitor` and `Window`; both are `Copy + Send` and implement the conversion, and the cross-check in Task 5 step 6 verifies it before CI does.

---

## Addendum: launch without a terminal (spec section 13, approved 2026-09-06)

Tasks 8 to 10 implement spec section 13. They come after Task 7; the Windows CI job is green at their start. The Global Constraints above still apply; the app crate cannot be cross-checked locally, so Windows-only code in it is verified by the CI job in Task 10.

Additional verified facts: egui 0.36.1 `Ui::{text_edit_singleline, add_enabled, colored_label, heading, weak, label, separator, horizontal, vertical_centered}`, `TextEdit::singleline(&mut String).hint_text(..).desired_width(f32)`, `Color32::LIGHT_RED`, `CentralPanel::default().show(ui, ..)` as used in `crates/app/src/ui/tiles.rs`; winit 0.30 `Window::set_title(&str)`, `EventLoop::build` errors with `RecreationAttempt` on a second call; windows-sys 0.61 `Win32::System::Console::{AttachConsole, SetStdHandle, ATTACH_PARENT_PROCESS, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE}` with `HANDLE = *mut c_void` and `BOOL = i32`; `brp_proto::RoomTicket: FromStr + Display + Clone + PartialEq`, `RoomTicket::new(topic, bootstrap)`; `brp_net::RelaySetting: Copy + PartialEq`; `Room::snapshot().nickname`.

### File Structure (addendum)

```
crates/app/src/launch.rs          Intent, Launch (from WindowArgs), default_nickname, open_room
crates/app/src/ui/start.rs        StartAction, StartState (+ tests), draw
crates/app/src/ui/mod.rs          + pub mod start
crates/app/src/room_view.rs       RoomView: room, snapshot, ticket, watch handles, share task; refresh/upload/apply
crates/app/src/window.rs          App with Phase::{Start, Room(RoomView)}, RoomOpened event, finish()
crates/app/src/participant.rs     no pre-open; passes Option<Intent>; teardown via App::finish
crates/app/src/cli.rs             Option<Command>, DEFAULT_FPS, Default for WindowArgs
crates/app/src/main.rs            no-subcommand dispatch, console attach, windows_subsystem
crates/app/src/console.rs         attach_parent_console (Windows), no-op elsewhere
crates/app/src/lib.rs             + pub mod console; pub mod launch; pub mod room_view
crates/app/Cargo.toml             + windows-sys under [target.'cfg(windows)'.dependencies]
Cargo.toml                        + windows-sys workspace dependency
README.md                         Usage rewritten for the start screen; Windows section notes
```

---

### Task 8: Start-screen state and the shared room open

**Files:**
- Create: `crates/app/src/launch.rs`
- Create: `crates/app/src/ui/start.rs`
- Modify: `crates/app/src/ui/mod.rs`
- Modify: `crates/app/src/lib.rs`
- Modify: `crates/app/src/participant.rs`

**Interfaces:**
- Consumes: `WindowArgs`, `identity::load_or_create`, `AppEvent::{RoomChanged, NewFrame}`, `RoomConfig`.
- Produces: `launch::{Intent::{Create, Join(RoomTicket)}, Launch { nickname: Option<String>, fps: u32, relay: RelaySetting }, default_nickname(&Launch) -> Result<String, AppError>, open_room(&Launch, Intent, nickname: &str, EventLoopProxy<AppEvent>) -> Result<Arc<Room>, AppError>}`; `ui::start::{StartAction::{Create, Join}, StartState { nickname, ticket, connecting, error }, StartState::{new, submit, failed}, draw(&mut egui::Ui, &mut StartState) -> Option<StartAction>}`.

- [ ] **Step 1: Write the failing tests**

Create `crates/app/src/ui/start.rs` with the tests and `todo!()` bodies:

```rust
//! The start screen: nickname and ticket entry with Create and Join, shown until a room is open.

use std::str::FromStr;

use brp_proto::RoomTicket;

use crate::launch::Intent;

/// Which button the user clicked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartAction {
    Create,
    Join,
}

/// The form's fields, whether an open is in flight, and the last error to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartState {
    pub nickname: String,
    pub ticket: String,
    pub connecting: bool,
    pub error: String,
}

impl StartState {
    pub fn new(nickname: String) -> Self {
        Self {
            nickname,
            ticket: String::new(),
            connecting: false,
            error: String::new(),
        }
    }

    /// Turns a click into an intent, or refuses it: nothing while an open is in flight, and a join
    /// needs a ticket that parses. On success the screen is marked connecting.
    pub fn submit(&mut self, action: StartAction) -> Option<Intent> {
        todo!()
    }

    /// The open failed: back to the form with the reason shown.
    pub fn failed(&mut self, message: String) {
        todo!()
    }
}

/// Draws the start screen and returns the button clicked, if any.
pub fn draw(ui: &mut egui::Ui, state: &mut StartState) -> Option<StartAction> {
    todo!()
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use iroh::{EndpointAddr, SecretKey};

    use super::*;

    fn valid_ticket() -> RoomTicket {
        let id = SecretKey::from_bytes(&[7u8; 32]).public();
        let addr = EndpointAddr::new(id).with_ip_addr(SocketAddr::from(([192, 168, 1, 10], 4433)));
        RoomTicket::new([1u8; 32], vec![addr])
    }

    #[test]
    fn create_yields_the_create_intent_and_marks_connecting() {
        let mut state = StartState::new("alice".into());
        assert_eq!(state.submit(StartAction::Create), Some(Intent::Create));
        assert!(state.connecting);
        assert!(state.error.is_empty());
    }

    #[test]
    fn join_with_garbage_shows_an_error_and_stays_on_the_form() {
        let mut state = StartState::new("alice".into());
        state.ticket = "not a ticket".into();
        assert_eq!(state.submit(StartAction::Join), None);
        assert!(!state.connecting);
        assert!(state.error.starts_with("invalid ticket"), "{}", state.error);
    }

    #[test]
    fn join_with_a_valid_ticket_yields_the_join_intent() {
        let ticket = valid_ticket();
        let mut state = StartState::new("alice".into());
        state.ticket = format!("  {ticket}\n");
        assert_eq!(state.submit(StartAction::Join), Some(Intent::Join(ticket)));
        assert!(state.connecting);
    }

    #[test]
    fn nothing_is_accepted_while_connecting() {
        let mut state = StartState::new("alice".into());
        state.submit(StartAction::Create);
        assert_eq!(state.submit(StartAction::Create), None);
        assert_eq!(state.submit(StartAction::Join), None);
    }

    #[test]
    fn a_failure_returns_to_the_form_with_the_message() {
        let mut state = StartState::new("alice".into());
        state.submit(StartAction::Create);
        state.failed("no room member answered within the join timeout".into());
        assert!(!state.connecting);
        assert_eq!(state.error, "no room member answered within the join timeout");
        assert_eq!(state.submit(StartAction::Create), Some(Intent::Create));
    }
}
```

`iroh` is already a dependency of the app crate; `EndpointAddr` and `SecretKey` are re-exported at its root (the proto crate's ticket tests use the same construction through `iroh_base`).

Add `pub mod start;` to `crates/app/src/ui/mod.rs` (alphabetical, after `picker`) and `pub mod launch;` to `crates/app/src/lib.rs` (after `identity`).

Create `crates/app/src/launch.rs`:

```rust
//! Opening a room from the window: the settings the command line provides, and the async open
//! that the start screen and the `create`/`join` commands share.

use std::sync::Arc;

use brp_capture::PlatformCapture;
use brp_net::RelaySetting;
use brp_proto::RoomTicket;
use brp_proto::constants::RELAY_ONLINE_TIMEOUT;
use brp_room::codecs::FfmpegCodecs;
use brp_room::{Room, RoomConfig, RoomTimings};
use winit::event_loop::EventLoopProxy;

use crate::cli::WindowArgs;
use crate::error::AppError;
use crate::identity;
use crate::window::AppEvent;

/// What the user asked for: a fresh room, or a seat in an existing one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    Create,
    Join(RoomTicket),
}

/// Settings that apply to any room this window opens.
#[derive(Debug, Clone)]
pub struct Launch {
    pub nickname: Option<String>,
    pub fps: u32,
    pub relay: RelaySetting,
}

impl From<WindowArgs> for Launch {
    fn from(args: WindowArgs) -> Self {
        Self {
            nickname: args.nickname,
            fps: args.fps,
            relay: if args.no_relay {
                RelaySetting::Disabled
            } else {
                RelaySetting::Default
            },
        }
    }
}

/// The nickname the start screen offers: the `--nickname` flag, else the short peer id.
pub fn default_nickname(launch: &Launch) -> Result<String, AppError> {
    match &launch.nickname {
        Some(nickname) => Ok(nickname.clone()),
        None => Ok(identity::load_or_create()?.public().fmt_short().to_string()),
    }
}

/// Creates or joins the room and waits briefly for the relay so the ticket works off the LAN.
/// A blank `nickname` falls back to the short peer id. The ticket is not printed or logged.
pub async fn open_room(
    launch: &Launch,
    intent: Intent,
    nickname: &str,
    proxy: EventLoopProxy<AppEvent>,
) -> Result<Arc<Room>, AppError> {
    let secret = identity::load_or_create()?;
    let nickname = match nickname.trim() {
        "" => secret.public().fmt_short().to_string(),
        name => name.to_string(),
    };
    let change_proxy = proxy.clone();
    let config = RoomConfig {
        secret,
        relay: launch.relay,
        nickname,
        target_fps: launch.fps,
        capture: Arc::new(PlatformCapture),
        encoders: Arc::new(FfmpegCodecs::default()),
        decoders: Arc::new(FfmpegCodecs::default()),
        on_change: Arc::new(move || {
            let _ = change_proxy.send_event(AppEvent::RoomChanged);
        }),
        on_frame: Arc::new(move || {
            let _ = proxy.send_event(AppEvent::NewFrame);
        }),
        timings: RoomTimings::default(),
    };
    let room = match intent {
        Intent::Join(ticket) => Room::join(config, ticket).await?,
        Intent::Create => Room::create(config).await?,
    };
    if launch.relay == RelaySetting::Default && !room.online(RELAY_ONLINE_TIMEOUT).await {
        tracing::warn!(
            "relay registration timed out; the ticket may only work on the local network"
        );
    }
    Ok(Arc::new(room))
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p brp start`
Expected: five tests FAIL with `not yet implemented`.

- [ ] **Step 3: Implement the state and the panel**

Replace the three `todo!()` bodies in `crates/app/src/ui/start.rs`:

```rust
    pub fn submit(&mut self, action: StartAction) -> Option<Intent> {
        if self.connecting {
            return None;
        }
        let intent = match action {
            StartAction::Create => Intent::Create,
            StartAction::Join => match RoomTicket::from_str(self.ticket.trim()) {
                Ok(ticket) => Intent::Join(ticket),
                Err(error) => {
                    self.error = format!("invalid ticket: {error}");
                    return None;
                }
            },
        };
        self.error.clear();
        self.connecting = true;
        Some(intent)
    }
```

```rust
    pub fn failed(&mut self, message: String) {
        self.connecting = false;
        self.error = message;
    }
```

```rust
pub fn draw(ui: &mut egui::Ui, state: &mut StartState) -> Option<StartAction> {
    let mut action = None;
    egui::CentralPanel::default().show(ui, |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.25);
            ui.heading("brp");
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                ui.label("Nickname");
                ui.add_enabled(
                    !state.connecting,
                    egui::TextEdit::singleline(&mut state.nickname).desired_width(240.0),
                );
            });
            ui.add_space(8.0);
            if ui
                .add_enabled(!state.connecting, egui::Button::new("Create room"))
                .clicked()
            {
                action = Some(StartAction::Create);
            }
            ui.add_space(16.0);
            ui.separator();
            ui.add_space(8.0);
            ui.add_enabled(
                !state.connecting,
                egui::TextEdit::singleline(&mut state.ticket)
                    .hint_text("paste a ticket")
                    .desired_width(480.0),
            );
            let can_join = !state.connecting && !state.ticket.trim().is_empty();
            if ui
                .add_enabled(can_join, egui::Button::new("Join room"))
                .clicked()
            {
                action = Some(StartAction::Join);
            }
            ui.add_space(16.0);
            if state.connecting {
                ui.weak("connecting");
            }
            if !state.error.is_empty() {
                ui.colored_label(egui::Color32::LIGHT_RED, &state.error);
            }
        });
    });
    action
}
```

- [ ] **Step 4: Route the existing open through `launch`**

In `crates/app/src/participant.rs`, replace the imports and the room block so the behaviour is unchanged but the code lives in `launch`:

```rust
use std::str::FromStr;
use std::sync::Arc;

use brp_proto::RoomTicket;
use brp_proto::constants::STATS_LOG_INTERVAL;
use tokio::runtime::Runtime;
use winit::event_loop::EventLoop;

use crate::cli::WindowArgs;
use crate::error::AppError;
use crate::launch::{self, Intent, Launch};
use crate::window::{App, AppEvent};
```

```rust
pub fn run(runtime: &Runtime, ticket: Option<String>, args: WindowArgs) -> Result<(), AppError> {
    let intent = match ticket.as_deref().map(RoomTicket::from_str).transpose()? {
        Some(ticket) => Intent::Join(ticket),
        None => Intent::Create,
    };
    let launch = Launch::from(args);
    let nickname = launch::default_nickname(&launch)?;

    let event_loop = EventLoop::<AppEvent>::with_user_event()
        .build()
        .map_err(|e| AppError::Window(e.to_string()))?;
    let proxy = event_loop.create_proxy();

    let room = runtime.block_on(launch::open_room(&launch, intent, &nickname, proxy.clone()))?;
```

Delete the `println!("Ticket:...")` line that followed: the ticket is copied from the status bar (spec section 13). The rest of the function (ticker, `App::new`, teardown) is unchanged in this task.

- [ ] **Step 5: Run the tests and lints**

Run: `cargo test --workspace && cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`
Expected: green, including the five start-screen tests. `ui::start::draw` is not yet called anywhere; because the `ui` module is public in a library crate, that is not a dead-code warning.

- [ ] **Step 6: Commit**

```bash
git add crates/app/src/launch.rs crates/app/src/ui/start.rs crates/app/src/ui/mod.rs crates/app/src/lib.rs crates/app/src/participant.rs
git commit -m "feat: add the start-screen state and share the room open between window paths"
```

---

### Task 9: One window, two phases

**Files:**
- Create: `crates/app/src/room_view.rs`
- Modify: `crates/app/src/window.rs`
- Modify: `crates/app/src/participant.rs`
- Modify: `crates/app/src/cli.rs`
- Modify: `crates/app/src/main.rs`
- Modify: `crates/app/src/lib.rs`

**Interfaces:**
- Consumes: Task 8's `launch::{Intent, Launch, default_nickname, open_room}` and `ui::start::{StartAction, StartState, draw}`.
- Produces: `AppEvent::RoomOpened(Result<Arc<Room>, String>)`; `App::new(runtime: Handle, proxy: EventLoopProxy<AppEvent>, launch: Launch, nickname: String, intent: Option<Intent>)`; `App::finish(self) -> Shutdown { room: Option<Arc<Room>>, tasks: Vec<JoinHandle<()>> }`; `participant::run(runtime: &Runtime, intent: Option<Intent>, args: WindowArgs)`; `cli::Cli { command: Option<Command> }`, `cli::DEFAULT_FPS`, `impl Default for WindowArgs`.

There is no new unit test in this task: the phase switch is winit and egui wiring, verified by the manual check in step 5 and by the existing tests staying green. The room-specific code moves verbatim; do not change its behaviour.

- [ ] **Step 1: Extract the room view**

Create `crates/app/src/room_view.rs`:

```rust
//! Everything the window holds once a room is open: the room handle, its last snapshot, the
//! watch handles that feed tiles, and the share in flight. The window delegates room commands here.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use brp_capture::{SourceId, SourceListing};
use brp_proto::SourceKind;
use brp_room::{Room, RoomSnapshot, WatchHandle};
use tokio::runtime::Handle;
use tokio::task::JoinHandle;
use winit::event_loop::EventLoopProxy;

use crate::commands::RoomCommand;
use crate::render::GpuContext;
use crate::render::tiles::{TileKey, TileRenderer};
use crate::ui::state::UiState;
use crate::window::AppEvent;

pub struct RoomView {
    pub room: Arc<Room>,
    pub snapshot: RoomSnapshot,
    pub ticket: String,
    handles: HashMap<TileKey, WatchHandle>,
    pub pending_share: Option<JoinHandle<()>>,
}

impl RoomView {
    pub fn new(room: Arc<Room>) -> Self {
        let snapshot = room.snapshot();
        let ticket = room.ticket().to_string();
        Self {
            room,
            snapshot,
            ticket,
            handles: HashMap::new(),
            pending_share: None,
        }
    }

    /// Re-snapshots when the room's version moved, drops handles and tiles of ended watches, and
    /// refreshes the rate meters.
    pub fn refresh(&mut self, state: &mut UiState, tiles: Option<&mut TileRenderer>) {
        if self.room.version() != self.snapshot.version {
            self.snapshot = self.room.snapshot();
        }
        // The relay address can arrive after the first snapshot without bumping the version.
        self.ticket = self.room.ticket().to_string();
        let live: HashSet<TileKey> = self
            .snapshot
            .watches
            .iter()
            .map(|w| (w.publisher, w.live_id))
            .collect();
        self.handles.retain(|key, _| live.contains(key));
        if let Some(tiles) = tiles {
            tiles.retain(|key| live.contains(key));
        }
        state.refresh_rates(&self.snapshot, Instant::now());
    }

    /// Uploads the newest decoded frame of every watched live.
    pub fn upload_frames(&self, gpu: &GpuContext, tiles: &mut TileRenderer) {
        for (key, handle) in &self.handles {
            if let Some(frame) = handle.slot.try_take() {
                tiles.upload(&gpu.device, &gpu.queue, *key, &frame);
            }
        }
    }

    /// Applies the commands one egui pass produced. Errors land in the status line.
    pub fn apply(
        &mut self,
        commands: Vec<RoomCommand>,
        runtime: &Handle,
        proxy: &EventLoopProxy<AppEvent>,
        state: &mut UiState,
    ) {
        if commands.is_empty() {
            return;
        }
        // `Room::watch` spawns its task with `tokio::spawn`, which needs a runtime on this thread.
        let _guard = runtime.enter();
        state.status.clear();
        for command in commands {
            let result = match command {
                RoomCommand::Watch { key, preset_id } => {
                    self.room.watch(key.0, key.1, preset_id).map(|handle| {
                        self.handles.insert(key, handle);
                    })
                }
                RoomCommand::Unwatch(key) => self.room.unwatch(key.0, key.1).map(|()| {
                    self.handles.remove(&key);
                }),
                RoomCommand::StopLive(live_id) => self.room.stop_live(live_id),
                RoomCommand::SetPresets { live_id, presets } => {
                    self.room.set_presets(live_id, presets)
                }
                RoomCommand::Share {
                    kind,
                    source: Some(source),
                } => {
                    self.share(kind, Some(source), runtime, proxy, state);
                    Ok(())
                }
                RoomCommand::Share { kind, source: None } => match self.room.sources(kind) {
                    Ok(SourceListing::PlatformPicker) => {
                        self.share(kind, None, runtime, proxy, state);
                        Ok(())
                    }
                    Ok(SourceListing::Choices(choices)) => {
                        state.open_picker(kind, choices);
                        Ok(())
                    }
                    Err(error) => Err(error),
                },
            };
            if let Err(error) = result {
                state.status = error.to_string();
            }
        }
    }

    fn share(
        &mut self,
        kind: SourceKind,
        source: Option<SourceId>,
        runtime: &Handle,
        proxy: &EventLoopProxy<AppEvent>,
        state: &mut UiState,
    ) {
        if self.pending_share.is_some() {
            return;
        }
        let title = state.next_title(kind);
        state.share_pending = true;
        state.status.clear();
        let room = self.room.clone();
        let proxy = proxy.clone();
        self.pending_share = Some(runtime.spawn(async move {
            let outcome = room
                .start_live(kind, source, title)
                .await
                .map(|_live_id| ())
                .map_err(|error| error.to_string());
            let _ = proxy.send_event(AppEvent::ShareFinished(outcome));
        }));
    }
}
```

Add `pub mod room_view;` to `crates/app/src/lib.rs` (after `render`).

- [ ] **Step 2: Rewrite the window around phases**

Replace `crates/app/src/window.rs` entirely (the `repaint_deadline` helper and its test stay as they are at the bottom):

```rust
//! The participant window: a winit loop that shows the start screen until a room is open, then
//! draws the tile grid under the egui panels and hands panel commands to the room view.

use std::sync::Arc;
use std::time::{Duration, Instant};

use brp_room::Room;
use tokio::runtime::Handle;
use tokio::task::JoinHandle;
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy},
    window::{Window, WindowId},
};

use crate::error::AppError;
use crate::launch::{self, Intent, Launch};
use crate::render::grid::{self, PixelRect};
use crate::render::tiles::{TileKey, TileRenderer};
use crate::render::{GpuContext, ui::EguiLayer};
use crate::room_view::RoomView;
use crate::ui::start::{self, StartState};
use crate::ui::state::UiState;
use crate::ui::{self, UiOutput};

/// Wakes the winit event loop for a reason that does not arrive as a `WindowEvent`. Sent through
/// the `EventLoopProxy` from other threads (the room's background tasks, the open and share tasks).
pub enum AppEvent {
    /// The room's version counter moved; re-snapshot on the next redraw.
    RoomChanged,
    /// A watched live decoded a frame.
    NewFrame,
    /// Periodic wake so counters refresh while nothing is watched.
    Tick,
    /// The share task finished: the live started, or the error to show.
    ShareFinished(Result<(), String>),
    /// The open task finished: the room to show, or the error for the start screen.
    RoomOpened(Result<Arc<Room>, String>),
}

/// What the window shows: the start screen, or a room.
enum Phase {
    Start,
    Room(RoomView),
}

/// What must be torn down after the loop ends: tasks still holding room handles, then the room.
pub struct Shutdown {
    pub room: Option<Arc<Room>>,
    pub tasks: Vec<JoinHandle<()>>,
}

/// The winit `ApplicationHandler` for the participant window: owns the phase, the window-local UI
/// state, and the GPU and egui state.
pub struct App {
    runtime: Handle,
    proxy: EventLoopProxy<AppEvent>,
    launch: Launch,
    start: StartState,
    phase: Phase,
    state: UiState,
    pending_open: Option<JoinHandle<()>>,
    /// When egui asked for the next frame; `about_to_wait` sleeps until then instead of forever.
    next_repaint: Option<Instant>,
    window: Option<Arc<Window>>,
    gpu: Option<GpuContext>,
    tiles: Option<TileRenderer>,
    ui: Option<EguiLayer>,
}

impl App {
    /// An `intent` from the command line opens the room at once behind the connecting start
    /// screen; `None` waits for the user.
    pub fn new(
        runtime: Handle,
        proxy: EventLoopProxy<AppEvent>,
        launch: Launch,
        nickname: String,
        intent: Option<Intent>,
    ) -> Self {
        let mut app = Self {
            runtime,
            proxy,
            launch,
            start: StartState::new(nickname),
            phase: Phase::Start,
            state: UiState::new(),
            pending_open: None,
            next_repaint: None,
            window: None,
            gpu: None,
            tiles: None,
            ui: None,
        };
        if let Some(intent) = intent {
            app.start.connecting = true;
            app.open(intent);
        }
        app
    }

    pub fn finish(self) -> Shutdown {
        let mut tasks: Vec<JoinHandle<()>> = self.pending_open.into_iter().collect();
        let room = match self.phase {
            Phase::Room(view) => {
                tasks.extend(view.pending_share);
                Some(view.room)
            }
            Phase::Start => None,
        };
        Shutdown { room, tasks }
    }

    fn open(&mut self, intent: Intent) {
        let launch = self.launch.clone();
        let nickname = self.start.nickname.clone();
        let room_events = self.proxy.clone();
        let done = self.proxy.clone();
        self.pending_open = Some(self.runtime.spawn(async move {
            let outcome = launch::open_room(&launch, intent, &nickname, room_events)
                .await
                .map_err(|error| error.to_string());
            let _ = done.send_event(AppEvent::RoomOpened(outcome));
        }));
    }

    fn redraw(&mut self) {
        if let Phase::Room(view) = &mut self.phase {
            view.refresh(&mut self.state, self.tiles.as_mut());
        }
        let (Some(window), Some(gpu), Some(tiles), Some(ui)) = (
            self.window.as_ref(),
            self.gpu.as_mut(),
            self.tiles.as_mut(),
            self.ui.as_mut(),
        ) else {
            return;
        };
        if let Phase::Room(view) = &self.phase {
            view.upload_frames(gpu, tiles);
        }
        let surface = match gpu.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            _ => return,
        };
        let target = surface.texture.create_view(&Default::default());
        let size = (gpu.config.width, gpu.config.height);

        let mut output = UiOutput::default();
        let mut start_action = None;
        let mut ui_frame = ui.run(window, [size.0, size.1], |root| match &self.phase {
            Phase::Start => start_action = start::draw(root, &mut self.start),
            Phase::Room(view) => {
                output = ui::draw(root, &view.snapshot, &view.ticket, &mut self.state);
            }
        });
        let pixels_per_point = ui_frame.screen.pixels_per_point;
        let placements: Vec<(TileKey, PixelRect)> = output
            .tile_rects
            .iter()
            .map(|(key, rect)| (*key, grid::to_pixels(*rect, pixels_per_point, size)))
            .collect();
        tiles.update_fits(&gpu.queue, &placements);

        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let buffers = ui.prepare(&gpu.device, &gpu.queue, &mut encoder, &mut ui_frame);
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("tiles+ui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            tiles.draw(&mut pass, &placements);
            ui.paint(&mut pass, &ui_frame);
        }
        gpu.queue
            .submit(buffers.into_iter().chain(std::iter::once(encoder.finish())));
        ui.cleanup(&mut ui_frame);
        window.pre_present_notify();
        gpu.queue.present(surface);
        if ui_frame.repaint_delay.is_zero() {
            window.request_redraw();
            self.next_repaint = None;
        } else {
            self.next_repaint = repaint_deadline(Instant::now(), ui_frame.repaint_delay);
        }

        if let Some(action) = start_action
            && let Some(intent) = self.start.submit(action)
        {
            self.open(intent);
        }
        if let Phase::Room(view) = &mut self.phase
            && !output.commands.is_empty()
        {
            view.apply(output.commands, &self.runtime, &self.proxy, &mut self.state);
        }
        if start_action.is_some()
            && let Some(window) = &self.window
        {
            window.request_redraw();
        }
    }
}

impl ApplicationHandler<AppEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let window = match event_loop.create_window(
            Window::default_attributes()
                .with_title("brp")
                .with_inner_size(PhysicalSize::new(1280, 720)),
        ) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                tracing::error!(%error, "could not create window");
                event_loop.exit();
                return;
            }
        };
        let gpu = match GpuContext::new(event_loop, &window) {
            Ok(gpu) => gpu,
            Err(error) => {
                let _: AppError = error;
                event_loop.exit();
                return;
            }
        };
        self.tiles = Some(TileRenderer::new(&gpu.device, gpu.config.format));
        self.ui = Some(EguiLayer::new(&window, &gpu.device, gpu.config.format));
        self.gpu = Some(gpu);
        self.window = Some(window);
    }

    fn user_event(&mut self, _: &ActiveEventLoop, event: AppEvent) {
        match event {
            AppEvent::RoomOpened(Ok(room)) => {
                self.pending_open = None;
                self.state = UiState::new();
                if let Some(window) = &self.window {
                    window.set_title(&format!("brp: {}", room.snapshot().nickname));
                }
                self.phase = Phase::Room(RoomView::new(room));
            }
            AppEvent::RoomOpened(Err(message)) => {
                self.pending_open = None;
                self.start.failed(message);
            }
            AppEvent::ShareFinished(outcome) => {
                if let Phase::Room(view) = &mut self.phase {
                    view.pending_share = None;
                }
                self.state.share_pending = false;
                if let Err(message) = outcome {
                    self.state.status = format!("share failed: {message}");
                }
            }
            AppEvent::RoomChanged | AppEvent::NewFrame | AppEvent::Tick => {}
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some(window) = self.window.clone() else {
            return;
        };
        if let Some(ui) = self.ui.as_mut() {
            let response = ui.on_window_event(&window, &event);
            if response.repaint {
                window.request_redraw();
            }
            if response.consumed {
                return;
            }
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(gpu) = self.gpu.as_mut() {
                    gpu.resize(size.width, size.height);
                }
            }
            WindowEvent::RedrawRequested => self.redraw(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        match self.next_repaint {
            Some(deadline) if deadline <= Instant::now() => {
                self.next_repaint = None;
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
                event_loop.set_control_flow(ControlFlow::Wait);
            }
            Some(deadline) => event_loop.set_control_flow(ControlFlow::WaitUntil(deadline)),
            None => event_loop.set_control_flow(ControlFlow::Wait),
        }
    }
}
```

Keep the existing `repaint_deadline` function and its `tests` module below this, unchanged.

- [ ] **Step 3: The entry point and the command line**

Replace `crates/app/src/participant.rs` from the imports down:

```rust
//! The participant window: `brp` with no arguments shows the start screen, `brp create` and
//! `brp join` open a room at once. Owns the room's lifetime around the winit loop and leaves it
//! in an orderly fashion when the window closes.

use std::sync::Arc;

use brp_proto::constants::STATS_LOG_INTERVAL;
use tokio::runtime::Runtime;
use winit::event_loop::EventLoop;

use crate::cli::WindowArgs;
use crate::error::AppError;
use crate::launch::{self, Intent, Launch};
use crate::window::{App, AppEvent};

/// Runs the window to completion. `intent` from the command line opens the room immediately;
/// `None` shows the start screen.
pub fn run(runtime: &Runtime, intent: Option<Intent>, args: WindowArgs) -> Result<(), AppError> {
    let launch = Launch::from(args);
    let nickname = launch::default_nickname(&launch)?;

    let event_loop = EventLoop::<AppEvent>::with_user_event()
        .build()
        .map_err(|e| AppError::Window(e.to_string()))?;
    let proxy = event_loop.create_proxy();

    // Encoder byte counters and last-seen ages move without any frame arriving, so a slow tick
    // keeps the status bar honest when nothing is watched.
    let ticker = runtime.spawn({
        let proxy = proxy.clone();
        async move {
            let mut tick = tokio::time::interval(STATS_LOG_INTERVAL);
            loop {
                tick.tick().await;
                if proxy.send_event(AppEvent::Tick).is_err() {
                    break;
                }
            }
        }
    });

    let mut app = App::new(runtime.handle().clone(), proxy, launch, nickname, intent);
    let outcome = event_loop
        .run_app(&mut app)
        .map_err(|e| AppError::Window(e.to_string()));

    ticker.abort();
    let shutdown = app.finish();
    // Abort only requests cancellation; wait for every task so its Arc<Room> clone is gone
    // before the room is unwrapped (a cancelled JoinError is expected).
    let _ = runtime.block_on(ticker);
    for task in shutdown.tasks {
        task.abort();
        let _ = runtime.block_on(task);
    }
    if let Some(room) = shutdown.room {
        match Arc::try_unwrap(room) {
            Ok(room) => runtime.block_on(room.leave()),
            Err(_) => tracing::warn!("room still referenced at exit; skipping the orderly leave"),
        }
    }
    outcome
}
```

In `crates/app/src/cli.rs`:

```rust
/// Capture ceiling when no `--fps` is given, and the start screen's default.
pub const DEFAULT_FPS: u32 = 60;

#[derive(Parser, Debug)]
#[command(name = "brp", about = "Peer-to-peer screen sharing", version)]
pub struct Cli {
    /// Without a subcommand the participant window opens on its start screen.
    #[command(subcommand)]
    pub command: Option<Command>,
}
```

Both `#[arg(long, default_value_t = 60)]` fps fields become `#[arg(long, default_value_t = DEFAULT_FPS)]`, and after `WindowArgs` add:

```rust
impl Default for WindowArgs {
    fn default() -> Self {
        Self {
            nickname: None,
            fps: DEFAULT_FPS,
            no_relay: false,
        }
    }
}
```

In `crates/app/src/main.rs`, change the imports and the dispatch:

```rust
use std::str::FromStr;

use brp_app::cli::{Cli, Command, WindowArgs};
use brp_app::error::AppError;
use brp_app::launch::Intent;
use brp_app::{participant, publish};
use brp_proto::RoomTicket;
```

```rust
    let result = match cli.command {
        None => participant::run(&runtime, None, WindowArgs::default()),
        Some(Command::Publish(args)) => runtime.block_on(publish::run(args)),
        Some(Command::Create(args)) => participant::run(&runtime, Some(Intent::Create), args.window),
        Some(Command::Join(args)) => match RoomTicket::from_str(&args.ticket) {
            Ok(ticket) => participant::run(&runtime, Some(Intent::Join(ticket)), args.window),
            Err(error) => Err(AppError::Ticket(error)),
        },
    };
```

`brp-proto` is already a dependency of the app crate.

- [ ] **Step 4: Run the tests and lints**

Run: `cargo test --workspace && cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`
Expected: green; no test changed.

- [ ] **Step 5: Manual check on Linux**

Run `cargo run -p brp` with no arguments: the window opens on the start screen with the short id in the nickname field. Type garbage into the ticket box and click Join room: an "invalid ticket" line appears in red, the form stays. Click Create room: "connecting" shows, then the participant panels appear and the title gains the nickname. Close the window; it exits cleanly with no panic. Run `cargo run -p brp -- create --no-relay`: the room opens without a visible form (the connecting screen may flash). If the display is unavailable, say so in the report and rely on the suite.

- [ ] **Step 6: Commit**

```bash
git add crates/app/src/room_view.rs crates/app/src/window.rs crates/app/src/participant.rs crates/app/src/cli.rs crates/app/src/main.rs crates/app/src/lib.rs
git commit -m "feat: open the window on a start screen and switch to the room when it opens"
```

---

### Task 10: GUI subsystem on Windows, console when asked, docs, CI

**Files:**
- Create: `crates/app/src/console.rs`
- Modify: `crates/app/src/main.rs`
- Modify: `crates/app/src/lib.rs`
- Modify: `crates/app/Cargo.toml`
- Modify: `Cargo.toml`
- Modify: `README.md`

**Interfaces:**
- Produces: `console::attach_parent_console()`, a no-op off Windows.

- [ ] **Step 1: Dependencies**

Root `Cargo.toml`, `[workspace.dependencies]`:

```toml
windows-sys = { version = "0.61", features = ["Win32_Foundation", "Win32_System_Console"] }
```

`crates/app/Cargo.toml`, after `[dependencies]`:

```toml
[target.'cfg(windows)'.dependencies]
windows-sys.workspace = true
```

- [ ] **Step 2: The console module and the subsystem**

Create `crates/app/src/console.rs`:

```rust
//! On Windows the binary is a GUI-subsystem program so a double-click opens no console. Started
//! from a terminal with arguments, it borrows that terminal's console so `publish`, `--help`, and
//! errors still print. The shell does not wait for a GUI process, so output may follow the prompt.

/// Attaches to the parent console when there are arguments and a parent console exists. Must run
/// before anything prints. A no-op on other platforms and for a bare double-click.
pub fn attach_parent_console() {
    #[cfg(windows)]
    {
        use std::fs::OpenOptions;
        use std::os::windows::io::IntoRawHandle;

        use windows_sys::Win32::System::Console::{
            ATTACH_PARENT_PROCESS, AttachConsole, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE,
            SetStdHandle,
        };

        if std::env::args_os().len() < 2 {
            return;
        }
        // SAFETY: plain Win32 calls with constant arguments; failure only means there is no parent
        // console, in which case output has nowhere to go anyway.
        unsafe {
            if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
                return;
            }
            // A GUI process attached late has no standard handles; open the console's output
            // device and install it for both streams. The files are leaked on purpose so the
            // handles stay valid for the life of the process.
            for slot in [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
                if let Ok(file) = OpenOptions::new().write(true).open("CONOUT$") {
                    SetStdHandle(slot, file.into_raw_handle());
                }
            }
        }
    }
}
```

Add `pub mod console;` to `crates/app/src/lib.rs` (after `commands`).

In `crates/app/src/main.rs`, add as the first line of the file, above the module doc comment:

```rust
#![cfg_attr(windows, windows_subsystem = "windows")]
```

and make the first statement of `main` a call to `brp_app::console::attach_parent_console();`, before the tracing subscriber is installed and before `Cli::parse()` (which prints help and errors).

- [ ] **Step 3: README**

In `README.md`, replace the "Usage" code block and the two paragraphs after it (up to and including the sentence about `--fps`) with:

````markdown
Run `brp` with no arguments, or double-click `brp.exe` on Windows, to open the
start screen: pick a nickname, then Create room, or paste a ticket and Join
room. The status bar's Copy ticket button gives you the ticket to hand to
others. From a terminal the same window can be opened directly:

```
cargo build --release

# Open a new room in the participant window
./target/release/brp create [--nickname N] [--fps 60] [--no-relay]

# Join a room in the participant window
./target/release/brp join <ticket> [--nickname N] [--fps 60] [--no-relay]

# Share one live headlessly and print the ticket, creating a room or joining one
./target/release/brp publish --nickname alice [--ticket <ticket>] [--fps 60] [--bitrate-kbps N] [--codec hevc|h264|av1] [--source monitor|window] [--no-relay]
```

On Windows, releases bundle the FFmpeg DLLs, so `brp.exe` runs without a global
FFmpeg install, and the binary opens no console window. Started from a
terminal with arguments it prints to that terminal, but the shell does not
wait for it, so output can appear after the prompt.

In the window, tick a live in the left panel to watch it and pick its preset;
hover a tile for the preset selector and stats. The bottom panel lists your own
lives with a frame-rate control, a codec selector, template checkboxes, and a
bitrate per preset. `--fps` is the capture ceiling for lives shared from the
window and defaults to 60.
````

Leave the paragraphs about tickets, members, and `--no-relay` that follow as they are.

- [ ] **Step 4: Lints, commit, push, watch**

Run: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: green (the console module compiles to an empty function on Linux).

```bash
git add Cargo.toml Cargo.lock crates/app/Cargo.toml crates/app/src/console.rs crates/app/src/main.rs crates/app/src/lib.rs README.md
git commit -m "feat: run as a GUI program on Windows and borrow the console when started from one"
git push
```

Watch the `ci` workflow run for both jobs (`gh run list --workflow ci --limit 1`, then `gh run watch <id> --exit-status`; the repository also has a CodeQL workflow, pick the `ci` run). Expected: both green and the `brp-windows-x86_64` artifact present. A Windows compile error in `console.rs` is fixed with a `fix:` commit against the windows-sys 0.61 signatures in `~/.cargo/registry/src/*/windows-sys-0.61*/src/Windows/Win32/System/Console/mod.rs`; three attempts maximum.

Deferred, not done in this phase: on a Windows machine, double-click `brp.exe` and confirm no console appears; run `brp.exe --help` from PowerShell and confirm the help prints.

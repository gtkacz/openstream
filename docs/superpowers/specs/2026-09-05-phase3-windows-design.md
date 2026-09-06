# Phase 3: Windows

Status: approved design, 2026-09-05. Refines phase 3 of `2026-09-04-p2p-screen-sharing-design.md`, which remains the master spec. Where this document is silent, the master spec applies.

## 1. Goals

- A Windows participant creates or joins a room, watches lives, and publishes monitors and windows, interoperating with Linux participants over the unchanged protocol.
- Monitors and windows are captured through Windows Graphics Capture, with DXGI desktop duplication as the fallback for monitors that deliver no frame.
- Encoding uses NVENC, AMF, QSV, or Media Foundation in that order; decoding uses D3D11VA, then NVDEC, then software.
- Every push builds and tests on a Windows runner and publishes a runnable zip with the FFmpeg DLLs.

## 2. Non-goals for this phase

- Audio, pop-out windows, fullscreen, settings persistence, tagged releases, a GUI subsystem without a console. Those keep their phases.
- Source-lost notifications from a capture session up to the room. A closed window or a dead duplication freezes the live, as a lost PipeWire stream does on Linux today.
- Live titles taken from the picked source's name. Titles stay kind plus ordinal.
- Runtime verification on Windows hardware. No Windows machine is available in this phase; section 10 lists what is deferred.

## 3. Decisions and rationale

| Decision | Rationale |
|---|---|
| The duplication fallback lives inside the Windows capture backend, behind the existing session trait | Room and window see one ordinary session; the Linux crate stays untouched; the decision logic is a platform-neutral driver that the hardware-free suite tests on both runners. |
| An in-app picker built from the backend's source list, not the OS Graphics Capture picker | The fallback needs the exact monitor handle, which only enumeration provides. The trait gains one method with a default body so existing backends and fakes compile unchanged. |
| No platform gates in the encoder probe list | The probe already skips encoders FFmpeg does not know by name. Media Foundation entries are simply absent on Linux, VAAPI entries simply absent on Windows. |
| FFmpeg comes from one pinned BtbN LGPL shared release and is consumed through `FFMPEG_DIR` | The build script of `ffmpeg-sys-next` links prebuilt import libraries from that variable; no vcpkg, no source build in CI. LGPL keeps the project MIT and the build ships every encoder the spec names. |
| The Windows binary stays a console application | The ticket prints to stdout and `brp publish` is headless. The GUI subsystem is part of phase 5 polish. |
| CI artifact on every push, no release workflow | Anyone with repository access can fetch a runnable build; release packaging is phase 5. |

## 4. Product model additions

- **Source descriptor.** One capturable thing the platform can list: an opaque source id, its kind, a display name, and its pixel size. Only platforms without their own picker produce them.
- **Source listing.** Either "the platform shows its own picker" (Linux portal) or a list of descriptors (Windows).
- **Fallback.** A monitor session that Graphics Capture could not start, or that delivered no frame within the capture fallback timeout, is served by desktop duplication instead. Window sessions have no fallback.

## 5. Architecture

### 5.1 Capture trait

```
SourceId          (u64, the platform's raw handle)
SourceDescriptor  { id: SourceId, kind: SourceKind, name: String, width: u32, height: u32 }
SourceListing     PlatformPicker | Choices(Vec<SourceDescriptor>)
SourceRequest     { kind: SourceKind, source: Option<SourceId>, target_fps: u32 }

trait CaptureBackend {
    fn sources(&self, kind: SourceKind) -> Result<SourceListing, CaptureError>   // default: PlatformPicker
    fn start(&self, request: SourceRequest, sink: FrameSink) -> StartFuture<'_>
}
```

The Linux backend and the synthetic source keep the default `sources` and ignore `source`. The crate exports the platform backend under one alias, `PlatformCapture`, so the app names a single type on both operating systems.

`CaptureError` gains one variant, `Windows(String)`, for Win32 and WinRT failures with the failing call in the text. Stale ids and silent sources reuse `SourceLost`.

### 5.2 Windows backend

A `windows` module in the capture crate, compiled only on Windows, built on the `windows-capture` crate. Modules, each with one responsibility:

| Module | Responsibility |
|---|---|
| `sources` | Enumerates monitors and windows into descriptors. The id is the raw `HMONITOR` or `HWND` value. Windows owned by the brp process are left out. Resolves a request back to a monitor or window and rejects ids no longer present. A request without an id means the primary monitor for monitor kind and `SourceLost` for window kind, so `brp publish` works on Windows without new flags. |
| `graphics_capture` | The Graphics Capture session. A handler on the crate's own capture thread turns each frame into a `CaptureFrame` in BGRA with the row pitch as stride and the project's monotonic clock as timestamp, then calls the sink. Cursor on, border off where the OS supports it, minimum update interval set to the target frame interval where supported. The reported rate is the monitor's refresh rate, or the rate of the monitor the window is on. Stopping posts quit to the capture thread and joins it. |
| `duplication` | The desktop duplication session, monitors only. Our own thread loops on acquire-next-frame with the frame interval as timeout and produces the same frame type. Access lost, which Windows raises on mode changes and desktop switches, recreates the duplication. Any other error ends the thread and the live freezes. |
| `mod` | `WindowsCapture`, the backend. `start` resolves the source, then runs the fallback driver inside a blocking task, the way the Linux backend waits for the PipeWire format. |

### 5.3 Fallback driver

A platform-neutral `fallback` module in the capture crate, generic over a primary start, an optional fallback start, and a first-frame wait, so the hardware-free suite exercises it with fakes on both runners.

Rules, in order:

1. Start the primary. If it starts and delivers a frame within the capture fallback timeout, return it.
2. Otherwise stop whatever the primary opened. If there is no fallback, return `SourceLost` naming the primary and the timeout.
3. Start the fallback. If it starts and delivers a frame within the timeout, return it.
4. Otherwise stop it and return `SourceLost` naming both attempts.

The Windows backend passes duplication as the fallback for monitors and nothing for windows. The first frame's dimensions become the session's `SourceInfo`.

### 5.4 Codec

- The hardware decoder order becomes D3D11VA then CUDA on Windows, VAAPI then CUDA on Linux, selected at compile time. The existing device-context and frame-transfer path handles D3D11VA without new code; output stays CPU NV12.
- The encoder probe list adds `hevc_mf` and `h264_mf` after the QSV entries and before software AV1. Their low-latency options are hardware encoding only, CBR rate control, and the display-remoting scenario. There is no AV1 Media Foundation encoder.
- Nothing else in the codec crate changes. VAAPI symbols compile on Windows because FFmpeg's headers declare them on every platform; the VAAPI encoder simply fails to open there and the probe moves on.

### 5.5 Room and app

- `Room::start_live(kind, source, title)` takes the optional source id and puts it in the request. `Room::sources(kind)` is a one-line passthrough to the backend.
- The share buttons issue the share command without a source, as today. The window handles it: a platform-picker listing starts the live immediately; a choices listing opens the picker. The picker is a new `ui/picker` module drawing an egui window of names and sizes from an optional picker state held in the window-local UI state. Choosing an entry issues the share command with that source id and closes the picker; cancel closes it. The share command therefore carries `kind` and `Option<SourceId>`.
- `participant` and `publish` construct `PlatformCapture` instead of the Linux type. Identity storage already has a non-Unix branch; nothing else in the app is platform specific.
- The portal example in the capture crate is gated to Linux with an empty main elsewhere, so all-targets builds pass on Windows.

### 5.6 Build and CI

The Windows job on `windows-latest`:

1. Toolchain with clippy; the Rust cache action.
2. Download the pinned BtbN release zip (section 12), extract it, and export `FFMPEG_DIR` to its root. Add its `bin` directory to the path so tests that link FFmpeg can start, and `LIBCLANG_PATH` for bindgen.
3. `cargo clippy --workspace --all-targets -- -D warnings`, then `cargo test --workspace`.
4. `cargo build --release -p brp`, then stage `brp.exe`, `avcodec-62.dll`, `avutil-60.dll`, `swscale-9.dll`, `swresample-6.dll`, FFmpeg's `LICENSE.txt` renamed to make its origin clear, and the project licence into one directory, and upload it as the artifact `brp-windows-x86_64`.

The rustfmt check stays on the Linux job. The Linux job is unchanged. The four DLLs are the closure of what the binary imports: avcodec pulls in avutil and swresample, swscale pulls in avutil; every other import is a system library.

The README gains a Windows section: the BtbN download, the three environment variables, and where the artifact lives.

## 6. Protocol

Unchanged. Presence, tickets, media streams, and constants on the wire are identical, so a Linux and a Windows peer interoperate without negotiation.

## 7. Data flow

Unchanged from the room down. The Windows session hands BGRA frames to the live's capture fan; the publisher converts and encodes through the existing swscale and FFmpeg paths; viewers decode through the hardware order in 5.4 and render NV12 as before.

**Share on Windows.** Click share, the window asks the room for the listing, the picker opens, the user picks, the room starts the live with the id, the backend resolves the handle, the fallback driver runs, the session's first frame sets the live's source size and rate, presence is rebroadcast.

## 8. User interface

- Share monitor and share window open the picker on Windows and the portal on Linux; the waiting text no longer mentions the portal.
- The picker is a centred egui window titled by kind, listing each source as name and size, with a cancel button. It closes on choice, cancel, or when a share is already pending.

## 9. Error handling

- Enumeration failure and a stale source id surface as a status-bar line with the kind and, for a stale id, the id.
- Graphics Capture unsupported on the running Windows counts as a start failure: monitors fall through to duplication, windows report the failure.
- Both attempts silent: a status-bar line naming both, no live created.
- A closed captured window or a duplication error other than access lost is logged at warn; the live freezes until the user stops it.
- Tickets and secret keys stay out of logs as before.

## 10. Testing

- **Unit, hardware-free, both runners.** The fallback driver decision table: primary delivers, primary silent with fallback, primary start failure with fallback, primary silent without fallback, both silent, and that a silent primary is stopped before the fallback starts. The trait default returns the platform picker. Picker state transitions: open with choices, choose issues the share command with the id and closes, cancel closes, cannot open while a share is pending.
- **Room.** `start_live` with a source id passes it through to a recording fake backend; `sources` passes through.
- **CI.** The Windows job compiles the backend, the codec changes, and the app with warnings denied and runs the whole hardware-free suite. It is the compile oracle for code no machine here can run.
- **Local accelerator.** A cross compile-check from the Linux machine against the extracted Windows FFmpeg headers is attempted first; if it fails, plans rely on CI alone.
- **Deferred, not done in this phase.** Picking a monitor and a window; the fallback on an exclusive-fullscreen game; each hardware encoder and Media Foundation; D3D11VA decode; the two-machine glass-to-glass check with a Linux peer. They run when a Windows machine with a GPU exists.

## 11. Constants added in this phase

| Constant | Value | Rationale |
|---|---|---|
| Capture fallback timeout | 2 s | From the master spec; Graphics Capture normally delivers its first frame within milliseconds |

## 12. References

Verified on 2026-09-05:

- `windows-capture` 2.0.1 exposes `Monitor::enumerate`, `Window::enumerate`, raw handle round-trips for both, monitor refresh rate and names, window titles and process ids, a `GraphicsCaptureApiHandler` trait with `start_free_threaded` returning a `CaptureControl` whose `stop` posts quit and joins, `Settings` with cursor, border, minimum update interval, and `ColorFormat::Bgra8`, frames exposing width, height, row pitch, and a raw buffer, and `DxgiDuplicationApi` with `acquire_next_frame`, an `AccessLost` error, and `recreate`.
- `ffmpeg-sys-next` 9.0.0 links prebuilt libraries from `FFMPEG_DIR/lib` and reads headers from `FFMPEG_DIR/include` when the `build` feature is off.
- BtbN FFmpeg-Builds release `autobuild-2026-09-05-13-10`, asset `ffmpeg-n8.1.2-50-g1a748fe2cd-win64-lgpl-shared-8.1.zip`: ships MSVC import libraries and DLLs for avcodec 62, avutil 60, swscale 9, swresample 6; its avcodec contains `hevc_nvenc`, `h264_nvenc`, `av1_nvenc`, `hevc_amf`, `h264_amf`, `av1_amf`, `hevc_qsv`, `h264_qsv`, `av1_qsv`, `hevc_mf`, `h264_mf`, `hevc_d3d11va`, `libsvtav1`, and `libdav1d`.
- The dev machine has the `x86_64-pc-windows-msvc` Rust target installed but no Windows linker, so local verification is limited to `cargo check`.

## 13. Amendment: launch without a terminal

Approved 2026-09-06. Supersedes the section 3 decision that the Windows binary stays a console application, and brings the GUI subsystem forward from phase 5. Where this section is silent, the rest of the document applies.

**Goal.** A Windows participant double-clicks `brp.exe` and never sees a terminal: no console window, no printed ticket, no command-line arguments.

**Start screen.** `brp` with no arguments opens the window on a start screen: a nickname field prefilled with the `--nickname` value or the short peer id, a ticket text box, and two buttons, Create room and Join room. Either button opens the room in the background while the screen shows that it is connecting and disables both buttons. When the room is open, the participant view from slice 2 takes over the same native window and the title gains the nickname. A failure, including a ticket that does not parse, keeps the start screen and shows the reason as text.

**One window, two phases.** winit allows one event loop per process, so the app owns a phase: start, then room. The room-specific state and command handling move out of the window module into a room view the window delegates to. `brp create` and `brp join <ticket>` keep working and skip the form by opening the room as soon as the window exists; a ticket that does not parse on the command line is still reported before any window opens. `brp publish` stays a headless console command and still prints its ticket.

**Ticket.** The window paths no longer print the ticket; it is copied from the status bar as slice 2 designed. Tickets and secret keys still never appear in logs.

**Windows subsystem.** The Windows build declares the GUI subsystem, so a double-click opens no console. When started from a terminal with at least one argument, it attaches to the parent console and points standard output and error at it, so `publish`, `--help`, and errors still print. A shell does not wait for a GUI-subsystem process, so that output can interleave with the prompt; the README says so. Linux is unchanged.

**Testing.** Start-screen state transitions are unit-tested: create marks connecting and yields the create intent; join with a ticket that does not parse yields nothing and shows an error; join with a valid ticket yields the join intent; nothing is accepted while connecting; a failure clears connecting and shows the message. egui drawing is not unit-tested. Manual on Linux: launch with no arguments, create a room, join with garbage and see the error, `brp create` still opens a room directly. On Windows, deferred with the rest of section 10: the double-click launch shows no console, and a terminal launch with arguments still prints.

**Out of scope.** A log file for GUI-only sessions, a relay checkbox, settings persistence, and installers stay in phase 5; the start screen is where they will land.

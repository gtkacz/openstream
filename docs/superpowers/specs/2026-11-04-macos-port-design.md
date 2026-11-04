# macOS port — design

Closes #17. Make `brp` build, run, and ship natively on macOS, the third platform beside Linux
and Windows, with the same behaviour a Linux or Windows user gets: share a monitor or a window
with audio, watch others with hardware decode, open `brp://` links, and self-update.

## Decisions

Taken without a design conversation (the request asked for none); each line says what it costs if
wrong.

| Area | Decision | Why | Cost if wrong |
|---|---|---|---|
| Minimum OS | macOS 14.0 | ScreenCaptureKit's `contentRect`/`pointPixelScale` and its audio capture are stable there; 14 covers every Mac that still gets security updates | Users on 13 cannot run it; lowering is a deployment-target change plus a window-size fallback |
| Architectures shipped | `aarch64` (Apple Silicon) | GitHub's macOS runners are arm64; FFmpeg and its codec libraries are built natively there | Intel Macs build from source; the code itself is arch-neutral and `PLATFORM` already names `macos-x86_64` |
| Screen capture | ScreenCaptureKit (`SCStream`), monitors and windows from `SCShareableContent`, listed in the app's own picker | The only supported capture API on current macOS; CoreGraphics capture is deprecated | — |
| Audio capture | ScreenCaptureKit audio on the same kind of stream: `All` excludes brp (`excludesCurrentProcessAudio` plus excluding brp's own app), `Only` includes the selected applications and re-polls so apps launched later join | Same API and permission as video; per-application filtering is built in | Helper-process audio that SCK attributes oddly would be missed under `Only`; `All` is unaffected |
| Audible applications list | CoreAudio process objects (`kAudioHardwarePropertyProcessObjectList` + `IsRunningOutput`, macOS 14.2+), falling back to the running GUI applications from `SCShareableContent` | Matches "audible right now" like PipeWire/WASAPI sessions | On 14.0/14.1 the list shows running apps rather than audible ones |
| Application identity (`AppKey`) | Bundle identifier, lowercased; basename of a path otherwise | Stable across launches and renames; bundle ids are case-insensitive | Settings saved on another OS do not match by name, as today between Linux and Windows |
| Hardware encode | `hevc_videotoolbox`, `h264_videotoolbox` (hardware only, realtime, speed priority), probed after Media Foundation and before `libsvtav1` | VideoToolbox is the platform encoder on every Mac | — |
| Hardware decode | FFmpeg `videotoolbox` hwaccel, software fallback unchanged | Same | — |
| Bindings | `objc2` family (pure Rust, no Swift or C build step) | Builds with plain cargo on a macOS runner and can be type-checked from Linux with `-Zbuild-std` | — |
| Links | `CFBundleURLTypes` in the bundle's `Info.plist`; `LSRegisterURL` on launch keeps LaunchServices pointed at the running bundle; `brp://` arrives as a `GURL` Apple Event, handled before the event loop runs | macOS never passes the URL in argv | — |
| Opening a link while a room is open | Start screen and idle: join in this window. Otherwise: start a second `brp join <link>` process | Same outcome as Linux/Windows, where every click is a new process | — |
| Packaging | `OpenStream.app` (Info.plist, icns, `brp` and FFmpeg dylibs in `Contents/MacOS` with an `@executable_path` rpath), ad-hoc signed, zipped; plus a flat `brp-<v>-macos-aarch64.tar.gz` the updater consumes | The updater swaps files in the directory of the exe, which is `Contents/MacOS` inside the bundle, so the flat layout serves both | Without a Developer ID the first launch needs right-click → Open; notarisation is a later secrets change |
| FFmpeg | Built from the pinned 8.1.3 source in CI, LGPL, shared, with VideoToolbox, libopus, libdav1d, libsvtav1; install names `@rpath` | No LGPL shared macOS build is published; Homebrew's is GPL | CI minutes on a cache miss |

## Codepaths touched

- `brp-capture`: new `macos` module; `PlatformCapture` on macOS; error variant.
- `brp-audio`: new `macos` module; `PlatformAudioCapture` on macOS; cpal on macOS; `AppKey`
  normalisation; error variant.
- `brp-codec`: VideoToolbox in the probe order and options; decoder device order per platform.
- `brp-update`: `PLATFORM` for macOS (per arch).
- `brp` app: link handler (LaunchServices + Apple Event), `AppEvent::OpenLink`, bundle constants.
- CI: `macos` job in `ci.yml`; `macos` job in `release.yml`; `macos-ffmpeg` action; staging
  script; `Info.plist` template.

## Testing

Pure logic gets unit tests that run on every OS (frame copy, planar-to-interleaved audio, frame
status, source filtering and naming, key normalisation, link routing, bundle path discovery,
platform asset names, probe order, `Info.plist` contents). OS-bound code is type-checked for
`aarch64-apple-darwin` from Linux and built and tested for real by the new macOS CI job.

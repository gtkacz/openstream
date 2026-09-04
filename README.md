# brp_sharing

Native peer-to-peer screen sharing in Rust. `brp` captures a Linux monitor or
window, encodes the video with FFmpeg, and sends it directly to a viewer over
QUIC using [iroh](https://iroh.com/). The viewer decodes the stream and renders
it in a native `wgpu` window.

> **Status:** experimental phase-1 vertical slice. The current executable is
> Linux-only and supports one publisher, one live, one preset, and one viewer.
> Rooms, multiple lives, audio, preset switching, pop-out windows, and Windows
> support are planned phases rather than current features.

## How it works

```text
desktop portal + PipeWire  ->  BGRA frames  ->  FFmpeg encoder
                                                        |
                                      iroh QUIC / brp/media/1
                                                        |
       wgpu window  <-  NV12 frames  <-  FFmpeg decoder  <-
```

The publisher prints a ticket containing its bootstrap endpoint. A viewer uses
that ticket to connect, with iroh relays and hole punching enabled by default.
`--no-relay` disables relay use and is useful on a local network.

The implementation keeps latency bounded with single-slot or bounded queues:
when processing falls behind, older video frames are dropped instead of
allowing delay to accumulate. The stream uses the versioned media ALPN
`brp/media/1` and postcard-encoded control and frame metadata.

## Current commands

Build the release binary:

```sh
cargo build --release
```

On the publishing machine, start capture:

```sh
./target/release/brp publish
```

The desktop portal opens a picker for a monitor or window. The command prints
the selected encoder and a ticket. Share the complete ticket with the viewer.

On the viewing machine:

```sh
./target/release/brp watch '<ticket>'
```

The viewer currently subscribes to live `1` and preset `1`, then opens a native
window showing the decoded stream.

Available options:

```text
brp publish [--fps FPS] [--bitrate-kbps KBPS]
            [--codec h264|hevc|av1]
            [--source monitor|window] [--no-relay]
brp watch <ticket> [--no-relay]
```

Defaults are 60 FPS, monitor capture, automatic bitrate, automatic codec
selection, and iroh's default relay configuration. The publisher chooses a
hardware encoder when available (HEVC by default), then falls back through the
supported hardware/software probes. The default bitrate is scaled from the
source dimensions and frame rate; for example, 1080p60 is 20 Mbps and 1440p60
is 40 Mbps.

Stop publishing or viewing with `Ctrl-C` or by closing the viewer window.

## Linux prerequisites

The phase-1 build expects a Fedora-like system with FFmpeg 7.1 or newer,
PipeWire, and bindgen's Clang development files:

```sh
sudo dnf install gcc clang clang-devel pkgconf-pkg-config \
  pipewire-devel ffmpeg-devel
```

At runtime, install and run `xdg-desktop-portal` with a compositor backend and
PipeWire. The portal's ScreenCast interface supplies the monitor/window picker
and capture stream. A working Vulkan/GL-capable `wgpu` graphics backend is also
required for the viewer window.

The current development environment is Fedora 44 with KDE on Wayland. The
application is intended to work on Wayland and X11 where the desktop portal is
available.

## Identity and privacy

On first run, `brp` creates a persistent iroh identity in the platform config
directory (`brp/identity.key`). On Unix it is created with mode `0600`. The
public key identifies the participant to the transport; the private key and
tickets should be treated as sensitive.

Possession of a publisher's ticket is the access mechanism in phase 1. The
phase-1 media server authenticates the connecting iroh endpoint, while room
membership and signed presence are part of the later rooms phase. Media is
intended to flow peer-to-peer; a relayed connection is possible and may not
sustain high bitrates.

## Workspace crates

| Crate | Responsibility |
| --- | --- |
| `brp-proto` | Wire types, postcard encoding, frame headers, tickets, constants, and bitrate rules |
| `brp-codec` | Encoder/decoder traits, FFmpeg implementation, codec probing, and test fakes |
| `brp-capture` | Capture traits, synthetic source, and Linux portal/PipeWire backend |
| `brp-net` | iroh endpoint, QUIC media client/server, and stream framing |
| `brp-pipeline` | Bounded publisher/viewer pipelines, fan-out, reordering, and latest-frame slots |
| `brp` | CLI, identity management, winit event loop, wgpu video renderer, and egui stats overlay |

Platform-specific capture and codec code is isolated behind crate interfaces so
Windows and later macOS backends can be added without changing the pipeline.

## Development

Run the hardware-independent test suite:

```sh
cargo test --workspace
```

Run formatting and lint checks:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

The codec smoke test is opt-in because it requires installed FFmpeg codecs and
real hardware:

```sh
BRP_CODEC_TESTS=1 cargo test -p brp-codec --test codec_smoke -- --nocapture
```

To inspect the Linux portal capture path without running the full publisher:

```sh
cargo run -p brp-capture --example portal_dump
```

Set `RUST_LOG=debug` for diagnostic logging. Secret keys and tickets are not
written to logs.

## Roadmap

1. **Linux vertical slice** — current phase.
2. **Rooms and multiple lives** — gossip presence, live catalog, tile grid,
   presets, and recovery controls.
3. **Windows** — Windows Graphics Capture, desktop-duplication fallback,
   Windows hardware codecs, and CI packaging.
4. **Audio** — system loopback, Opus, mixing, and per-live volume.
5. **Window management and polish** — pop-outs, fullscreen, settings, and
   release packaging.

The full product model, protocol, constraints, and design rationale are in
[`docs/superpowers/specs/2026-09-04-p2p-screen-sharing-design.md`](docs/superpowers/specs/2026-09-04-p2p-screen-sharing-design.md).
The phase-1 implementation plan is in
[`docs/superpowers/plans/2026-09-04-phase1-linux-vertical-slice.md`](docs/superpowers/plans/2026-09-04-phase1-linux-vertical-slice.md).

## License

Licensed under the [MIT License](LICENSE).

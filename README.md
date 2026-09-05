# brp_sharing

Native peer-to-peer screen sharing in Rust. `brp` captures a Linux monitor or
window, encodes the video with FFmpeg, and sends it directly to a viewer over
QUIC using [iroh](https://iroh.com/). The viewer decodes the stream and renders
it in a native `wgpu` window.

> **Status:** experimental, Linux only. A participant creates or joins a room,
> shares several monitors or windows with presets, and watches other members'
> lives in a tile grid. Audio, pop-out windows, and Windows support are planned
> phases rather than current features.

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

## Usage

```
cargo build --release

# Open a room in the participant window and print its ticket
./target/release/brp create [--nickname N] [--fps 60] [--no-relay]

# Join that room in the participant window
./target/release/brp join <ticket> [--nickname N] [--fps 60] [--no-relay]

# Share one live headlessly, creating a room or joining one
./target/release/brp publish --nickname alice [--ticket <ticket>] [--fps 60] [--bitrate-kbps N] [--codec hevc|h264|av1] [--source monitor|window] [--no-relay]
```

In the window, tick a live in the left panel to watch it and pick its preset; hover a
tile for its preset selector and stats. The bottom panel lists your own lives with a
frame-rate control, a codec selector, template checkboxes, and a bitrate per preset.
`--fps` is the capture ceiling for lives shared from the window.

A ticket names the room and one member already in it. Anyone in the room can hand out a ticket.
Media connections are accepted only from room members. Encoders run only while someone watches.

`--no-relay` skips the public relay servers; use it on a LAN.

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
cargo test -p brp-room              # two rooms in one process, fake codecs
cargo test -p brp                   # grid, preset, and rate helpers
cargo test -p brp-pipeline          # slot, fan-out, reorder, pacing
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

1. **Linux vertical slice** — done.
2. **Rooms and multiple lives** — current phase.
3. **Windows** — Windows Graphics Capture, desktop-duplication fallback,
   Windows hardware codecs, and CI packaging.
4. **Audio** — system loopback, Opus, mixing, and per-live volume.
5. **Window management and polish** — pop-outs, fullscreen, settings, and
   release packaging.

The full product model, protocol, constraints, and design rationale are in
[`docs/superpowers/specs/2026-09-04-p2p-screen-sharing-design.md`](docs/superpowers/specs/2026-09-04-p2p-screen-sharing-design.md).
The phase-1 implementation plan is in
[`docs/superpowers/plans/2026-09-04-phase1-linux-vertical-slice.md`](docs/superpowers/plans/2026-09-04-phase1-linux-vertical-slice.md).
The slice 2 design is in
[`docs/superpowers/specs/2026-09-04-slice2-rooms-and-multi-live-design.md`](docs/superpowers/specs/2026-09-04-slice2-rooms-and-multi-live-design.md),
implemented by plans
[`2026-09-04-plan2a-room-layer.md`](docs/superpowers/plans/2026-09-04-plan2a-room-layer.md) and
[`2026-09-05-plan2b-participant-window.md`](docs/superpowers/plans/2026-09-05-plan2b-participant-window.md).

## License

Licensed under the [MIT License](LICENSE).

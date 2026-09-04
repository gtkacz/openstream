# brp_sharing

Peer-to-peer screen sharing for small groups. Native Rust, QUIC over iroh, with a Linux publisher
and viewer in the phase-1 vertical slice.

## Build prerequisites

Install FFmpeg 7.1+ headers, PipeWire headers, and clang for bindgen:

```sh
sudo dnf install gcc clang clang-devel pkgconf-pkg-config pipewire-devel ffmpeg-devel
```

Runtime capture requires `xdg-desktop-portal` with a compositor backend. The publisher prefers a
hardware H.264/HEVC encoder and falls back to software AV1 when available.

## Usage

```sh
cargo build --release
./target/release/brp publish [--fps 60] [--bitrate-kbps N] [--codec hevc|h264|av1] [--source monitor|window] [--no-relay]
./target/release/brp watch <ticket> [--no-relay]
```

The publisher prints a ticket to share with the viewer. `--no-relay` is useful on a LAN; the
default enables iroh relays and hole punching.

## Development

```sh
cargo test --workspace
BRP_CODEC_TESTS=1 cargo test -p brp-codec --test codec_smoke -- --nocapture
cargo run -p brp-capture --example portal_dump
RUST_LOG=debug ./target/release/brp publish
```

The design and roadmap are in `docs/superpowers/specs/`.

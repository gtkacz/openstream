# brp_sharing: native peer-to-peer screen sharing

Status: approved design, 2026-09-04. Implementation plan to follow.

## 1. Goals

- A participant creates a room and shares a ticket. Others join with it. No room server.
- Any participant publishes one or more lives. A live is a captured monitor or window plus the machine's audio.
- Any participant watches any subset of the room's lives, as tiles in one window or popped out into separate native windows.
- Quality is set by the participants, not by infrastructure. A live can run at the source's native resolution and frame rate at any bitrate the publisher chooses.
- Windows and Linux at launch. macOS later, behind the same capture and audio traits.

## 2. Non-goals for the first release

- Browser or mobile clients. The transport is QUIC without WebRTC.
- Rooms larger than about six participants, repeaters, or relay nodes for media.
- Voice chat, per-application audio, recording, remote input.
- Lossless or 4:4:4 encoding, GPU zero-copy, congestion-driven quality switching.

## 3. Decisions and rationale

| Decision | Rationale |
|---|---|
| Fully native Rust, no webview | Full control of capture and codecs so quality reaches source. Browser encoders cannot guarantee that. |
| QUIC peer-to-peer via iroh, native clients only | The only maintained Rust stack with hole punching, relay fallback, and multipath. Dropping browser support removes WebRTC and its codec constraints. |
| Small rooms, direct fan-out mesh | Each publisher streams straight to each viewer. Upload bandwidth is the only cap, and repeaters are not worth their complexity at six people. |
| Encode once per preset, fan out the bitstream | Encoder cost stays flat as viewers join. Only upload bandwidth grows. |
| Video plus system-wide audio, per-application audio later | System loopback works on both OSes today. Per-application capture is contained to the audio crate when it comes. |
| egui on a hand-owned winit loop | eframe's viewport system has open redraw bugs on Windows and Wayland. Owning the loop gives one native window per pop-out with egui drawing the controls. |
| FFmpeg as the single codec backend | One dependency covers hardware encode and decode on NVIDIA, AMD, and Intel across both OSes plus a software fallback. Vendor SDK crates remain a per-GPU optimisation behind the same trait. |
| Public relays for hole punching, self-hosted relay optional | Free relays are rate-limited and meant for development. Media must flow direct. Anyone stuck behind a hostile NAT self-hosts a relay. |

Rejected: Electron or Tauri wrappers, WebRTC, RustDesk's codec crate (unlicensed, git-only), building on iroh-live (early preview, no Windows, no NVIDIA, AMD, or Intel encoders). iroh-live remains the design reference for frame-per-stream transport.

## 4. Product model

- **Participant.** A persistent iroh key pair stored in the config file. The public key is the identity. A nickname is display-only.
- **Room.** A random 32-byte gossip topic. It exists while at least one member remains. Any member can mint a ticket listing themselves as bootstrap, so the creator leaving does not strand newcomers.
- **Ticket.** Implements the iroh-tickets `Ticket` trait with kind prefix `brp`. Payload: version, topic, one or more bootstrap endpoint addresses. Possessing the ticket is the only access control.
- **Live.** Owned by one publisher. Identified by the pair (publisher identity, live id). Carries a title, source kind (monitor or window), source resolution and frame rate, an audio flag, and a list of presets.
- **Preset.** An encoding configuration for a live: name, width, height, frame rate, bitrate, codec. Every live has a preset named Source at native resolution and frame rate. Publishers may add lower presets and change bitrate and codec on any preset.
- **Subscription.** A viewer watching one live at one preset, with or without audio.

## 5. Architecture

### 5.1 Process and thread model

One process per participant. Networking runs on a tokio runtime. Capture backends deliver frames on their own OS threads. Each active encoder and each active decoder owns a dedicated thread fed by a bounded channel that drops the oldest frame when full. The main thread owns the winit event loop and all rendering.

### 5.2 Workspace layout

| Crate | Responsibility | Key dependencies |
|---|---|---|
| `proto` | Wire types for room and media messages, ticket format, constants. No I/O. | serde, postcard, iroh-base, iroh-tickets |
| `net` | Endpoint, room gossip, media connections, per-viewer sender tasks | iroh, iroh-gossip, tokio |
| `capture` | Capture trait, source enumeration, Windows and Linux backends | windows-capture; ashpd, pipewire |
| `codec` | Encoder and decoder traits, FFmpeg implementation, hardware selection | ffmpeg-sys-next |
| `audio` | Loopback capture, Opus encode and decode, output mixer | cpal, opus |
| `pipeline` | Publisher and viewer pipelines wiring capture, codec, audio, and net together | the crates above |
| `app` | winit loop, wgpu renderer, egui chrome, settings, binary entry point | winit, wgpu, egui, egui-wgpu, egui-winit |

Platform-specific code lives only in `capture` and `audio` backends and in the codec's hardware device selection.

### 5.3 Capture

Trait with two operations: list sources, and start a session on a source. A session yields frames carrying pixel data, dimensions, and a monotonic capture timestamp in microseconds. The frame type has an optional GPU handle field, unused in the first release, so zero-copy can arrive without changing callers.

- **Windows.** Windows Graphics Capture for monitors and windows, cursor composited, yellow border disabled where the OS allows. Falls back to DXGI desktop duplication for a monitor when a Graphics Capture session delivers no frame within the capture fallback timeout, which happens with some exclusive-fullscreen games. Desktop duplication is monitor-only and limited to four concurrent duplications per process.
- **Linux.** Source selection through the xdg-desktop-portal ScreenCast interface, which shows the compositor's own picker for monitors and windows. Frames are read from the PipeWire stream the portal hands back, cursor embedded. Works on Wayland and X11 wherever the portal is available.

Frames reach the pipeline as CPU buffers in BGRA.

### 5.4 Codec

`VideoEncoder` and `VideoDecoder` traits with one implementation on the raw FFmpeg bindings plus a thin safe layer owned by this project. No dependency on the maintenance-mode safe wrapper.

- **Encoder selection** probes in order: NVENC, AMD AMF, Intel QSV, then Media Foundation on Windows or VAAPI on Linux, then software AV1. The first encoder that initialises for the requested codec wins. The chosen encoder is reported to the UI.
- **Codec default.** HEVC when the publisher's GPU encodes it, else H.264. AV1 is opt-in per preset. Chroma is 4:2:0.
- **Low-latency configuration** for every encoder: no B-frames, constant bitrate, rate-control buffer sized to one frame, infinite GOP, keyframes only on request, and the vendor's low-latency tuning where it exists.
- **Decoder selection** probes D3D11VA on Windows, VAAPI on Linux, and NVDEC on both, then falls back to FFmpeg's software decoders. Output is transferred to CPU NV12 in the first release.
- **Scaling and colour conversion** use libswscale: BGRA to NV12 with per-preset scaling in one pass.

Licensing: Linux builds link the system FFmpeg. Windows builds bundle an LGPL shared build, which limits the software fallback to AV1 or VP9 encoders and keeps the project MIT.

### 5.5 Audio

- **Capture.** cpal loopback: an input stream on the default render device on Windows, a sink monitor on PipeWire on Linux. Resampled to 48 kHz stereo when needed.
- **Encode.** Opus, 20 ms packets, 128 kbps.
- **Playback.** One cpal output stream. A mixer sums every subscribed live's decoded audio with a per-live gain and a master mute. Each live has an adaptive jitter buffer starting at 60 ms.

### 5.6 Rendering and windows

The app owns the winit loop. The main window and every pop-out are separate native windows sharing one wgpu device and queue. Each window has its own surface and egui context, integrated through egui-winit and egui-wgpu.

Decoded frames upload as two textures per live, luma at full resolution and interleaved chroma at half resolution. A shader converts BT.709 limited-range NV12 to RGB. A decoder thread signals the event loop through a proxy event when a new frame lands, and only windows showing that live redraw. Input also triggers redraws. There is no fixed timer.

Windows: the main window holds the room panel, the tile grid, and the publisher's own lives. A pop-out shows one live and can go borderless fullscreen. Closing a pop-out returns the live to the grid. The same decoded frame feeds whichever window shows the live; popping out never starts a second decode.

### 5.7 Persistence

TOML in the platform config directory: identity secret key, nickname, relay setting (default, custom URL, or disabled), default bitrate rule, audio output device, recent rooms.

## 6. Protocol

### 6.1 Encoding and versioning

All messages are serde structs encoded with postcard. Protocol version rides in the media ALPN string `brp/media/1` and in a version field on presence messages. Peers ignore messages with unknown versions.

### 6.2 Room control plane

Membership and the catalog of lives travel over iroh-gossip on the room topic. Every peer broadcasts a presence message on join, on any change, and on a heartbeat.

```
Signed      { author: EndpointId, payload: bytes, signature: [u8; 64] }
Presence    { version: u8, ts_unix_ms: u64, nickname: String, lives: Vec<LiveInfo> }
LiveInfo    { id: u32, title: String, kind: Monitor | Window,
              source_width: u32, source_height: u32, source_fps: u32,
              has_audio: bool, presets: Vec<Preset> }
Preset      { id: u32, name: String, width: u32, height: u32, fps: u32,
              bitrate_kbps: u32, codec: H264 | Hevc | Av1 }
```

Presence is signed with the author's iroh secret key because gossip relays through neighbours and reports only the last hop, not the author. Receivers verify the signature against the author identity and discard messages whose timestamp is not newer than the last accepted one from that author. Members not heard from within the expiry window are removed from the UI and from the membership set that gates media connections.

### 6.3 Media plane

A viewer opens one QUIC connection per publisher they watch, ALPN `brp/media/1`. The publisher accepts only callers whose identity is in its current membership set. A viewer refused because presence has not yet propagated retries through the normal resubscribe backoff. Every subscription is one bidirectional control stream on that connection. Video and audio frames each travel on their own unidirectional stream.

Viewer to publisher on the control stream:

```
Subscribe        { live_id: u32, preset_id: u32, want_audio: bool }
SwitchPreset     { preset_id: u32 }
RequestKeyframe
Unsubscribe
Stats            { frames_received: u32, frames_dropped: u32, decode_fps: u16, rtt_ms: u16 }
```

Publisher to viewer:

```
SubscribeAck     { video: CodecParams, audio: Option<AudioParams> }
SubscribeError   { reason: String }
PresetSwitched   { preset_id: u32, video: CodecParams }
LiveEnded
CodecParams      { codec, width: u32, height: u32, fps: u32, extradata: bytes }
AudioParams      { sample_rate: u32, channels: u8 }
```

Every frame stream begins with a header followed by the payload:

```
FrameHeader { live_id: u32, preset_id: u32, kind: Video | Audio, seq: u64,
              capture_ts_us: u64, keyframe: bool, len: u32 }
```

Audio frames use their own sequence space and a preset id of zero. Audio streams are sent at higher QUIC priority than video where the library exposes priority.

### 6.4 Quality model

The publisher owns presets and the viewer picks one per live. An encoder for a preset starts when its first viewer subscribes and stops after the idle grace period once the last one leaves. A new subscriber to a running preset triggers a keyframe request so it can start decoding immediately. Switching preset re-sends codec parameters and the new bitstream begins with a keyframe.

Default bitrate follows Moonlight's rule and scales linearly with pixel count and frame rate:

| Source preset | Default bitrate |
|---|---|
| 1280x720 at 60 | 10 Mbps |
| 1920x1080 at 60 | 20 Mbps |
| 2560x1440 at 60 | 40 Mbps |
| 3840x2160 at 60 | 80 Mbps |

The publisher adjusts bitrate anywhere from 1 Mbps to 250 Mbps per preset.

### 6.5 Latency protection

- **Encoder input.** The channel from capture to each encoder holds one frame and drops the oldest when encoding falls behind capture.
- **Per-viewer sender.** Each viewer has a sender task fed by a bounded channel of two frames. Writes to QUIC apply back-pressure through flow and congestion control. When the channel is full on arrival of a new frame, the sender marks the viewer as waiting for a keyframe, discards frames until one arrives, and requests one from the encoder. Forced keyframes are rate-limited per preset. Because the bitstream fans out, a forced keyframe reaches every viewer of that preset, which is acceptable at six participants.
- **Receiver.** Frames are decoded in sequence order. On a gap the receiver waits for the missing frame, since streams are reliable, unless a later keyframe has already arrived, in which case everything before it is discarded. If the wait exceeds the reorder cap the receiver discards up to the next keyframe and sends a keyframe request. The renderer always shows the newest decoded frame and never queues video.

### 6.6 Constants

| Constant | Value | Rationale |
|---|---|---|
| Presence heartbeat | 5 s | Cheap and keeps the catalog fresh |
| Member expiry | 20 s | Four missed heartbeats |
| Max lives per participant | 8 | Keeps presence under the 4 KB gossip message limit |
| Max presets per live | 6 | Same |
| Sender backlog channel | 2 frames | Bounds queueing delay to two frame intervals |
| Forced keyframe rate limit | 1 per second per preset | Caps the bitrate spikes viewers can trigger |
| Encoder idle stop grace | 5 s | Survives a viewer briefly toggling a tile |
| Receiver reorder cap | 200 ms | Longer than any plausible retransmit at internet RTTs |
| Resubscribe backoff | 1 s doubling to 30 s | Standard exponential backoff |
| Opus | 48 kHz stereo, 20 ms, 128 kbps | Transparent for game audio, 50 packets per second |
| Capture fallback timeout | 2 s | Graphics Capture normally delivers its first frame within milliseconds |
| Jitter buffer initial depth | 60 ms | Three audio packets |

## 7. Data flow

**Publish.** Capture thread emits BGRA frames with timestamps. For each active preset a converter scales and converts to NV12, the preset's encoder thread produces a compressed frame, and the fan-out hands it to every subscriber's sender task, which opens one QUIC stream, writes header and payload, and finishes it. Audio capture runs in parallel through the Opus encoder and the same fan-out.

**View.** A receive task per connection accepts incoming streams, reads headers, and routes video to the live's reorder buffer and audio to its jitter buffer. The decoder thread pulls frames in order, decodes, transfers to CPU NV12, and stores the result in a single-slot buffer, then signals the event loop. On redraw the renderer uploads the slot's frame to the live's textures and draws it in every window showing that live. The audio mixer pulls from every jitter buffer inside the cpal output callback.

## 8. User interface

- **Main window.** Left panel lists members and their lives with a watch toggle and preset selector per live. Centre holds the tile grid, auto-laid out for one, two, four, six, or nine tiles. Bottom panel holds the participant's own lives with a share button, a per-live preset editor, and a stop button. Status bar shows connection state per peer, direct or relayed, and aggregate bitrate.
- **Tile overlay on hover.** Title, preset selector, volume slider, stats toggle, pop-out button, fullscreen button.
- **Share flow.** On Windows an in-app picker lists monitors and windows. On Linux the portal dialog appears. The new live starts with the Source preset.
- **Settings.** Nickname, relay setting, default bitrate rule, audio output device.
- **Join flow.** Paste a ticket or create a room and copy the ticket. Recent rooms are listed.

## 9. Error handling

Every crate exposes a typed error enum built with thiserror. Nothing on the media path panics. Errors propagate to the app crate, which shows them next to the affected live or in the status bar.

- **Capture loss.** A closed window or unplugged monitor ends the live. Presence updates, viewers receive a live-ended message, and their tile shows a placeholder.
- **Encoder or decoder init failure.** The codec layer walks its fallback chain to software. The UI names the encoder actually running so a silent drop to CPU encoding is visible.
- **Connection loss.** Viewers resubscribe with exponential backoff while the publisher remains in the membership set. Publishers drop the sender task on disconnect.
- **Relayed path.** The transport reports direct versus relayed per connection. A relayed live shows a warning because the relay cannot sustain high bitrates. Preset switching stays manual.
- **Hostile input.** Malformed or unverifiable messages are logged and dropped. Media connections from identities outside the room are refused before any handshake. Presence with a stale timestamp is discarded.
- **Portal denied.** A clear message with the fix, no retry loop.
- **Logging** through tracing. Tickets and secret keys never appear in logs.

## 10. Testing

- **Unit tests.** Wire round-trips for every message, ticket parse and format, presence signing and verification, membership expiry, receiver ordering and gap rules, the sender backlog rule, jitter buffer behaviour, preset validation.
- **Integration tests without hardware.** A synthetic capture source generating a test pattern and a pass-through fake codec drive two in-process endpoints over loopback with relays disabled through subscribe, preset switch, forced keyframe recovery, and unsubscribe.
- **Codec smoke tests.** Behind an environment flag, on machines with real GPUs: an encode and decode round-trip for every encoder the machine offers, asserting frame counts and dimensions.
- **Manual checks.** Glass-to-glass latency with an on-screen timer captured and viewed side by side. Exclusive-fullscreen games on Windows. Multi-monitor capture.
- **CI.** GitHub Actions on Linux and Windows: rustfmt, clippy with warnings denied, the hardware-free suite. Coverage target of 80 percent on `proto`, `net`, and `pipeline`. Platform backends are covered by the gated smoke tests.

## 11. Phasing

Each phase ends with something runnable. Implementation plans are written one phase at a time, starting with phase 1.

1. **Vertical slice on Linux.** One publisher, one viewer, monitor capture through the portal, hardware encode, hardware or software decode, single window, ticket printed on the command line. Proves the transport, the FFmpeg layer, and NV12 rendering end to end.
2. **Rooms and multiple lives.** Gossip presence, the catalog, several lives per participant, tile grid, preset editing and switching, keyframe recovery, stats overlay.
3. **Windows.** Graphics Capture with the duplication fallback, Windows encoders and D3D11VA decode, FFmpeg DLL bundling, Windows CI build.
4. **Audio.** Loopback capture, Opus, the mixer, per-live volume.
5. **Window management and polish.** Pop-out windows, fullscreen, settings UI, release packaging.

Backlog, unordered: per-application audio, zero-copy GPU paths on both OSes, lossless and 4:4:4 presets, congestion-driven preset switching, self-hosted relay documentation, macOS.

## 12. Risks

| Risk | Mitigation |
|---|---|
| iroh has open issues on very high throughput links and on datagram bursts with many connections | Streams, not datagrams. Six peers is far below the reported thresholds. Phase 1 measures real throughput first. |
| Linux GPU import in wgpu is unfinished for multi-plane buffers | CPU copies in the first release. Six 1440p60 streams cost roughly 2 GB/s of texture upload, within PCIe budgets. |
| Windows Graphics Capture is not guaranteed for exclusive-fullscreen games | Desktop duplication fallback for monitors. Documented limit of four duplications per process. |
| FFmpeg distribution and licensing on Windows | Ship an LGPL shared build. Software fallback limited to AV1 and VP9. |
| NVIDIA caps GeForce cards at 12 concurrent encode sessions by driver policy | Encode once per preset. A participant publishing several lives at two presets each stays well under the cap. |
| Free relays are rate-limited | Direct-first design, relayed warning in the UI, self-hosted relay documented in the backlog. |
| Home upload bandwidth | Publishers see aggregate upload in the status bar and choose bitrate accordingly. |

## 13. Assumptions

- A Windows machine with a GPU is available for phase 3 testing. Development happens on Fedora 44 with KDE on Wayland, an RTX 4080 SUPER, FFmpeg 8.1 headers, and PipeWire 1.6 headers.
- Participants have GPUs with HEVC or H.264 encoders. Software encoding is a fallback, not a target.
- Linux participants run a desktop with xdg-desktop-portal and PipeWire.
- Windows participants run Windows 10 version 2004 or later.

## 14. References

Crate versions verified on 2026-09-04: iroh 1.1.0, iroh-gossip 0.101.0, iroh-tickets 1.0.0, windows-capture 2.0.1, ashpd 0.13.13, pipewire 0.10.1, ffmpeg-sys-next 9.0.0 supporting FFmpeg 3.4 through 9.0, cpal 0.18.2, opus 0.4.0, egui and egui-wgpu 0.36.1, wgpu 30.0.1, winit 0.30.13.

Prior art consulted: iroh-live for frame-per-stream transport over iroh; Sunshine and Moonlight for low-latency encoder settings and default bitrates; RustDesk for capture and hardware codec architecture, reference only.

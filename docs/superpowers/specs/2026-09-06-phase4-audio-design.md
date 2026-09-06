# Phase 4: Audio

Status: approved design, 2026-09-06. Refines phase 4 of `2026-09-04-p2p-screen-sharing-design.md`, which remains the master spec. Where this document is silent, the master spec applies. Where the two differ, this document wins for audio.

## 1. Goals

- A publisher sends the audio their machine plays, minus what brp itself plays, alongside their lives. A viewer hears it through one output stream with a volume per publisher and a master mute.
- Audio is Opus at 48 kHz stereo, 20 ms packets, 128 kbps, carried on the frame streams the protocol already reserves for it.
- Both operating systems are covered: PipeWire on Linux, WASAPI process loopback on Windows. Playback goes through cpal on both.
- Everything above the platform I/O is unit-tested without hardware, and the two-room integration test exercises audio end to end with a synthetic source and a fake codec.

## 2. Non-goals for this phase

- Choosing which applications to send. The capture is "everything except brp". The Linux backend is built so that per-application selection can reuse it, but no picker, no presence change, and no per-application UI ship now.
- Voice chat, microphone capture, recording.
- Audio and video synchronisation. Video renders the newest decoded frame; audio plays at jitter-buffer depth. See section 7.
- Choosing the output device. The default device is used; the settings UI is phase 5.
- Persisting volumes across sessions. Settings persistence is phase 5.
- Runtime verification on Windows hardware. Section 10 lists what is deferred, as phase 3 did.

## 3. Decisions and rationale

| Decision | Rationale |
|---|---|
| Capture everything the machine plays except brp's own playback | Plain system loopback records the audio brp plays from other participants and sends it straight back. In a room where two people share with audio and watch each other, which is the primary scenario, that is a delayed echo loop. Excluding our own process removes it. |
| Windows uses WASAPI process loopback in exclude mode | One activation call for "every process except this one", available since Windows 10 version 2004, already the README minimum. The `wasapi` crate exposes it directly. cpal cannot, so the Windows capture backend does not use cpal. |
| Linux links a PipeWire capture stream to every application output node except ours | PipeWire's sink monitor cannot exclude a client. Linking to application nodes directly can, and it is the same machinery per-application capture needs later. OBS's PipeWire audio capture works this way. |
| Playback through cpal on both operating systems | One playback code path. cpal 0.18's PipeWire host depends on the same `pipewire` 0.10 crate the video capture uses, so no second PipeWire binding enters the tree. |
| Opus through FFmpeg's libopus, not the `opus` crate | The FFmpeg builds already linked on every target include libopus: the dev machine's ffmpeg, Fedora's ffmpeg-free in CI, and the BtbN LGPL zip the Windows job downloads. No new native library, no cmake in CI, no extra DLL in the artifact. Encoder and decoder both work in 48 kHz stereo float, so swresample is not needed. |
| Audio is one stream per publisher, not one per live | The capture is system-wide, so every live of one publisher would carry identical audio. A single publisher-level flag, deduplicated on the viewer side, means nothing is ever heard twice and there is one slider per publisher. The per-live `has_audio` field stays on the wire and carries the same value on every live. |
| Moving the audio carrier reuses the preset-switch path | When the watch carrying a publisher's audio closes while another watch of that publisher remains, the survivor is resubscribed with audio requested. That costs one keyframe on a tile that was being rearranged anyway and needs no new control message. An in-band toggle would have added protocol for a rare event. |
| Capture starts on the first audio subscriber and stops after the encoder idle grace | Same lifecycle as video encoders: a publisher with the flag on but nobody listening costs nothing. |
| The jitter buffer never stalls the mixer | QUIC streams are reliable, so packets are late, never lost. A slot with no packet at playout time is silence; late packets are dropped and counted; depth adapts by rule. Simple to test and impossible to deadlock the device callback. |

## 4. Product model additions

- **Share audio.** A publisher-level flag, on by default. While on, every own live advertises `has_audio` in presence, provided capture is healthy.
- **Audio carrier.** Among a viewer's watches of one publisher, the single watch whose subscription requested audio. The carrier's tile shows the volume controls.
- **Publisher volume.** A gain from 0 to 1 per publisher, held by the room's mixer for the session.
- **Master mute.** One switch silencing all playback.

## 5. Architecture

Platform-specific code lives only in the new `audio` crate. Everything above it is pure Rust tested on both CI runners.

### 5.1 `proto`

New constants (section 11) and nothing else. `AudioParams`, `FrameKind::Audio`, `has_audio` on `LiveInfo`, `want_audio` on `Subscribe`, and `audio` on `SubscribeAck` already exist and keep their shapes.

### 5.2 `codec`

```
AudioFrame        { samples: Vec<f32> /* interleaved stereo, 960 per channel */, capture_ts_us: u64 }

trait AudioEncoder: Send {
    fn name(&self) -> &'static str
    fn params(&self) -> AudioParams
    fn encode(&mut self, frame: &AudioFrame) -> Result<Vec<EncodedFrame>, CodecError>
}
trait AudioDecoder: Send {
    fn decode(&mut self, packet: &EncodedFrame) -> Result<Vec<AudioFrame>, CodecError>
}
```

`ffmpeg::opus` holds `OpusEncoder` and `OpusDecoder` over `libopus` with `AV_SAMPLE_FMT_FLT`, 48 kHz, two channels, frame size 960, bitrate from the constant, application set to audio. `fake` gains `FakeAudioEncoder` and `FakeAudioDecoder` that carry the float samples through as bytes, so the integration test can assert on what reaches the output. `open_audio_encoder` and `open_audio_decoder` sit next to the video selectors. `EncodedFrame` is reused for packets with `keyframe: true` always.

### 5.3 `audio` (new crate)

```
AudioChunk        { samples: Vec<f32> /* interleaved stereo at 48 kHz, any length */, capture_ts_us: u64 }
AudioSink         Box<dyn FnMut(AudioChunk) + Send>

trait AudioCapture: Send + Sync {
    fn start(&self, sink: AudioSink) -> Result<Box<dyn AudioCaptureSession>, AudioError>
}
trait AudioCaptureSession: Send {
    fn stop(self: Box<Self>)
}

RenderFn          Box<dyn FnMut(&mut [f32]) + Send>   // interleaved stereo at 48 kHz, fill entirely

trait AudioOutput: Send + Sync {
    fn start(&self, render: RenderFn) -> Result<Box<dyn AudioOutputSession>, AudioError>
}
trait AudioOutputSession: Send {}
```

Modules:

| Module | Responsibility |
|---|---|
| `linux` | PipeWire capture of application output nodes, excluding brp's process. Section 5.4. |
| `windows` | WASAPI process loopback capture, exclude mode. Section 5.5. |
| `cpal_output` | Opens the default output device at 48 kHz stereo float and calls the render closure from the device callback. PipeWire host on Linux, WASAPI on Windows. |
| `synthetic` | A capture source emitting a sine tone in 10 ms chunks on a thread, for tests. |
| `fake_output` | An output whose test handle pulls the render closure on demand and returns the samples, for tests. |

The crate exports `PlatformAudioCapture` as one alias, as the capture crate does. Both backends take brp's process id at construction. `AudioError` has variants for the platform (`PipeWire(String)`, `Windows(String)`), `Unsupported(String)` for a platform that cannot exclude a process, `Device(String)` for output failures, and `Format(String)` when the requested format is refused.

### 5.4 Linux capture backend

One dedicated thread, `brp-audio-pw`, running its own PipeWire main loop with a quit channel, structured like the video capture thread.

- **Stream.** A capture stream with media type Audio and category Capture, autoconnect off, requesting F32LE, 48 kHz, two channels. It negotiates against its own adapter, so ports exist before any link is made. The server converts each linked node to this format.
- **Registry listener.** Tracks globals of two kinds: nodes whose `media.class` is `Stream/Output/Audio`, and ports whose direction is output and whose node id is a tracked node. Nodes carrying an `application.process.id` equal to brp's are ignored. Nothing else in the graph is touched: sink monitors, sources, and hardware nodes are not linked.
- **Linking.** For each tracked node, one link per channel through the `link-factory`: the node's FL port to our FL input, FR to FR. A node with a single output port is linked to both inputs. Extra channels beyond stereo are not linked. Nodes appearing later are linked on their global event; links of removed nodes disappear with them, so removal needs no work. Link failures are logged once per node and that node is skipped.
- **Process callback.** Copies the dequeued buffer into an `AudioChunk` stamped with the monotonic clock in microseconds and hands it to the sink. When no linked node is producing, the stream idles and no chunks flow.
- **Failure.** Missing session socket, stream connect error, or a `link-factory` that is absent surface as `AudioError::PipeWire` from `start`.

### 5.5 Windows capture backend

One dedicated thread, `brp-audio-wasapi`. Through the `wasapi` crate it creates an application loopback client for brp's own process id with the include-tree flag false, which is exclude mode; initialises it in shared, event-driven mode for 48 kHz stereo float; and loops on the event handle, draining the capture client into chunks stamped with the monotonic clock. Process loopback delivers the requested format, so there is no conversion. Activation failing because the OS predates version 2004 surfaces as `AudioError::Unsupported`; other failures as `AudioError::Windows` with the failing call named.

### 5.6 `pipeline`

Three new pieces, all free of platform code.

- **`AudioPublisher`.** Owns a thread `brp-audio-encode`. Receives `AudioChunk`s through a bounded channel, appends samples to an accumulator, and for every 960 samples per channel builds an `AudioFrame` stamped with the capture time of its first sample, encodes it, and pushes the packet to a `FanOut`. Stats: packets encoded, bytes encoded. Implements the audio side of the net source trait (section 6).
- **`FanOut` in audio mode.** The existing fan-out with a constructor flag that disables keyframe gating and sizes the channel with the audio backlog constant. Video behaviour is unchanged.
- **`JitterBuffer`.** Pure, driven by a caller-supplied clock so it is testable. Packets are keyed by sequence. Playout starts once the queued duration reaches the target depth. `pop(now)` returns the next packet, `Silence` when the next slot has no packet, or `Waiting` before playout has started. A packet whose slot has already been played is dropped and counted late; each late arrival grows the target by one step up to the maximum; ten seconds without one shrinks it by one step down to the initial depth. When queued audio exceeds twice the target the oldest packet is dropped and counted as trimmed, which absorbs clock drift between machines.
- **`Mixer`.** One `Track` per publisher: a ring buffer holding half a second of interleaved samples, an atomic gain, an underrun counter. `Mixer::render(&mut [f32])` sums every track times its gain, applies the master mute, clamps to the unit range, and writes silence for tracks without enough samples while counting the underrun. Tracks are added and removed by publisher key under a mutex the callback takes with `try_lock`; if the lock is contended for a callback, that callback renders silence rather than blocking the device thread.
- **`AudioViewer`.** Owns a thread `brp-audio-decode` per audio subscription. Pulls `ReceivedFrame`s from the net receiver, pushes them into the jitter buffer, and every 20 ms pops one slot, decodes it or produces 960 stereo samples of silence, and writes into the publisher's mixer track. Stats: packets received, late, trimmed, underruns read from the track.

### 5.7 `net`

- `LiveSource` gains `fn subscribe_audio(&self, live_id: u32) -> Result<AudioSubscription, SubscribeRejected>` where `AudioSubscription { params: AudioParams, packets: Receiver<Arc<EncodedFrame>> }`, and `SubscribeRejected` gains `NoAudio`.
- The server, on `Subscribe { want_audio: true }`, calls `subscribe_audio` after the video subscription succeeds. On success the ack carries `audio: Some(params)` and a second send task writes each packet on its own unidirectional stream with `FrameKind::Audio`, `preset_id: AUDIO_PRESET_ID`, `keyframe: true`, and the stream priority raised above video. On `NoAudio` the ack carries `audio: None` and video proceeds. Both send tasks end with the subscription.
- The client's `subscribe` takes `want_audio`. When the ack grants audio it registers a second route under `(live_id, AUDIO_PRESET_ID)` and returns `ViewerSubscription { audio: Option<AudioStream> }` where `AudioStream { params, packets: Receiver<ReceivedFrame> }`. The audio route is removed with the video one.
- `MediaClient::path_kind` and everything else is unchanged.

### 5.8 `room`

- **Registry.** Holds `audio: AudioState { enabled: bool, capture: Option<RunningAudio>, last_error: Option<String> }` where `RunningAudio { session, publisher: AudioPublisher, idle_since }`. `set_audio(enabled)` flips the flag, stops capture when turning off, and notifies. `live_infos` sets `has_audio` on every live to `enabled && last_error.is_none()`. `subscribe_audio(live_id)` rejects with `NoAudio` when the live is unknown, the flag is off, or capture previously failed; otherwise starts capture and the publisher on first use and returns a fan-out receiver. `housekeeping` stops capture after the grace with no subscribers, like encoders. Capture start failure records the error, clears `has_audio` through the next presence, and notifies.
- **Room.** Constructs the `Mixer` and opens the `AudioOutput` at start. An output failure is recorded in `audio_output_error` and disables audio requests for the room's lifetime. New methods: `set_audio(bool)`, `set_volume(PublicKey, f32)`, `volume(PublicKey) -> f32`, `set_master_mute(bool)`, `master_mute() -> bool`. `RoomConfig` gains `audio_capture: Arc<dyn AudioCapture>` and `audio_output: Arc<dyn AudioOutput>`.
- **Watcher.** `WatchEntry` gains `audio: bool`. On `watch`, `want_audio` is true when the output is healthy, the publisher advertises `has_audio`, and no other entry of that publisher has `audio` set. When the subscription is granted audio, the task starts an `AudioViewer` writing into the publisher's mixer track and creates the track if absent. On `unwatch`, on `Ended`, and on `member_left`, if the removed entry carried audio and another entry of the same publisher remains, that entry is replaced through the preset-switch path with the same preset id, which subscribes with audio. The mixer track is removed when the publisher's last watch goes. The rule is a pure function over the entries, tested directly.
- **Snapshot.** `OwnAudioView { enabled: bool, state: AudioCaptureState /* Off | Idle | Capturing | Failed(String) */, subscribers: usize, packets_encoded: u64 }` on `RoomSnapshot`; `audio_output_error: Option<String>` on `RoomSnapshot`; `audio: bool` on `WatchView`; `has_audio: bool` on `MemberView`.

### 5.9 `app`

- `RoomCommand` gains `SetAudio(bool)`, `SetVolume { publisher: PublicKey, gain: f32 }`, `SetMasterMute(bool)`.
- `publish` passes `PlatformAudioCapture` and the cpal output like the window path does; the terminal path shares audio by default like the window path.
- The panels described in section 8.

### 5.10 Build and CI

- `audio` crate: `cpal` 0.18 with the `pipewire` feature on Linux, `wasapi` 0.24 on Windows. Both compile on the existing runners; the Fedora job needs no new packages because `pipewire-devel` is already installed and cpal's PipeWire host uses the same binding. The `alsa-lib-devel` package is added to the Fedora job for cpal's ALSA fallback host, which cpal always compiles on Linux.
- The Windows artifact is unchanged: libopus is inside the BtbN `avcodec-62.dll`.
- The first CI run must confirm that Fedora's `ffmpeg-free` exposes `libopus`. If it does not, the Fedora job switches to the RPM Fusion `ffmpeg-devel` package, which the dev machine already uses. This is the only open dependency question and it resolves in the plan's first task.

## 6. Protocol

No new message variants. The existing fields carry the feature:

```
Subscribe        { live_id, preset_id, want_audio }     want_audio set by the carrier rule
SubscribeAck     { video, audio: Option<AudioParams> }  Some({48000, 2}) when granted, None otherwise
FrameHeader      { live_id, preset_id: 0, kind: Audio, seq, capture_ts_us, keyframe: true, len }
LiveInfo         { .., has_audio }                       same value on every live of a publisher
```

Audio packets have their own sequence space starting at zero per audio subscription. Audio streams are opened with a higher QUIC priority than video streams. A viewer receiving `audio: None` after asking for audio proceeds with video only and shows no controls. A publisher receiving `want_audio: true` while its flag is off answers `None` without error.

## 7. Data flow

**Publish.** The platform capture thread delivers chunks to the audio publisher's channel. The publisher thread accumulates, encodes one Opus packet per 20 ms, and pushes it to the audio fan-out. Each subscriber's audio send task writes the packet on a fresh unidirectional stream. With no application producing sound, no chunks arrive, no packets are sent, and the fan-out idles.

**View.** The client's receive task routes audio frames to the subscription's audio receiver. The audio viewer thread feeds the jitter buffer and, on its 20 ms cadence, decodes one slot into the publisher's mixer track. The cpal callback renders the mix into the device buffer.

**Latency.** Audio trails the newest video frame by roughly the jitter-buffer depth plus the mixer track's cushion: about 140 ms at rest, 60 ms of jitter depth and an 80 ms cushion ahead of the device (section 13). There is no synchronisation logic and none is planned for this phase; game sharing tolerates this margin.

## 8. User interface

- **Own lives panel.** A "Share audio" checkbox in the panel header, on by default, bound to `SetAudio`. Beside it the state: off, idle, capturing with the subscriber count, or the error text.
- **Tile overlay.** The carrier tile's hover bar gains a volume slider from 0 to 100 percent and a mute toggle, both bound to the publisher's gain. Non-carrier tiles and tiles of publishers without audio are unchanged.
- **Members panel.** A volume slider next to each member whose lives advertise audio, bound to the same gain.
- **Status bar.** A master mute toggle. When the output device failed, the reason.
- Sliders read their value from the room each frame, so the UI holds no audio state of its own, matching how the panels work today.

## 9. Error handling

- **Capture fails to start.** The registry records the error, presence drops `has_audio` from every live so viewers stop asking, subscribers get `NoAudio`, and the panel shows the text. Toggling share audio off and on retries. This covers exclude mode on an old Windows build and a PipeWire session without the link factory.
- **Capture dies mid-stream.** The session's thread ends with an error; the registry treats it as a start failure: the publisher stops, subscribers' packet receivers close and their audio send tasks end, viewers hear silence, presence drops the flag.
- **Output device fails.** Recorded once at room start, shown in the status bar, and every subscribe goes out with `want_audio: false`. No retry in this phase.
- **Audio granted but decoder fails to open.** The watch continues with video only; the audio receiver is dropped, the publisher's send task ends on the closed channel. Logged once.
- **Late or malformed audio frames.** Counted and dropped, never logged per packet.
- **Mixer lock contention in the device callback.** The callback renders silence for that period rather than blocking. Track additions and removals happen only on watch changes, so this is rare.

## 10. Testing

- **Unit.** Jitter buffer: playout starts at target depth; a late packet is dropped and counted; depth grows by one step per late arrival up to the maximum; ten quiet seconds shrink it one step; queued audio over twice the target trims the oldest; a missing slot yields silence. Mixer: gain scaling, master mute, summing two tracks, clamping, underrun counting, contended lock renders silence. Audio publisher: chunks of odd sizes become 960-sample frames with the first sample's timestamp; a partial tail waits. Fan-out in audio mode: no keyframe gating, audio backlog size. Watcher carrier rule: first watch of a publisher requests audio, second does not, closing the carrier moves audio to the survivor, closing the non-carrier moves nothing, an unhealthy output requests nothing. Registry: toggling audio flips `has_audio` on every live; the first audio subscriber starts capture; the grace stops it; a failing capture clears `has_audio` and rejects with `NoAudio`. Opus: a sine encoded and decoded through the FFmpeg codec keeps its sample count and its dominant frequency. Wire: `AudioParams` inside `SubscribeAck` round trips, `FrameHeader` with the audio kind round trips.
- **Integration.** Two in-process rooms over loopback with relays disabled, the synthetic tone source, the fake audio codec, and the fake output: a watch with audio delivers non-silent samples to the fake output; a second watch of the same publisher is granted no audio; closing the first moves audio to the second within one resubscribe; toggling share audio off ends the packets.
- **Manual on Linux.** Two instances on the dev machine: share audio in one, watch in the other, hear the desktop's sound. Then share from both and watch both ways, and confirm no echo returns, which is the exclusion test. Exercise the tile slider, the members slider, and the master mute.
- **Deferred to Windows hardware.** Process loopback capture, exclude mode with a Linux peer, WASAPI playback, the sliders on Windows.

## 11. Constants added in this phase

| Constant | Value | Rationale |
|---|---|---|
| `AUDIO_SAMPLE_RATE` | 48 000 Hz | Opus's native rate; both backends deliver it without conversion |
| `AUDIO_CHANNELS` | 2 | Stereo game audio; the master spec's choice |
| `AUDIO_FRAME_SAMPLES` | 960 per channel | 20 ms at 48 kHz, the Opus frame the master spec picked |
| `OPUS_BITRATE_KBPS` | 128 | Transparent for game audio at fifty packets per second |
| `AUDIO_PRESET_ID` | 0 | Reserved by the master spec for audio frames |
| `AUDIO_SENDER_BACKLOG_PACKETS` | 10 | 200 ms of slack before a stalled viewer drops audio; video's backlog of two frames is tuned for keyframe recovery, which audio has no need of |
| `JITTER_INITIAL_DEPTH` | 60 ms | Three packets, from the master spec |
| `JITTER_STEP` | 20 ms | One packet per adjustment |
| `JITTER_MAX_DEPTH` | 200 ms | Beyond this the delay is more annoying than the dropouts it prevents |
| `JITTER_SHRINK_AFTER` | 10 s | Long enough that a burst of lateness does not oscillate the depth |
| `MIXER_TRACK_CAPACITY` | 500 ms | Room for the jitter maximum plus decode scheduling slack |
| `MIXER_TRACK_CUSHION` | 80 ms | Silence pre-rolled into a track ahead of its first packet, so a device quantum larger than one packet does not underrun every callback: a 1024-frame quantum plus scheduling jitter |
| `AUDIO_CAPTURE_START_TIMEOUT` | 5 s | Bounds a platform backend's ready wait, and the PipeWire backend's wait for its own stream to become linkable |
| `MAX_AUDIO_PACKET_BYTES` | 4096 | Opus's largest single-frame packet is 1275 bytes; without this an audio route would accept `MAX_FRAME_BYTES` |

The audio capture reuses `ENCODER_IDLE_STOP_GRACE` through `RoomTimings::encoder_grace`; no separate constant exists.

## 12. References

Verified on 2026-09-06:

- `cpal` 0.18.2: a `pipewire` feature adds a PipeWire host depending on `pipewire` 0.10, selected by default when the PipeWire socket exists and falling back to ALSA otherwise; the WASAPI host sets the loopback flag when an input stream is built on a render device. The PipeWire host lists sinks as duplex devices and sets `stream.capture.sink` on their input side, which this design does not use for capture but confirms the binding's coverage.
- `wasapi` 0.24.0 exposes `AudioClient::new_application_loopback_client(process_id, include_tree)` built on `AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK` with both include and exclude modes.
- The dev machine's FFmpeg 8.1.2 links `libopus.so.0` and lists the `libopus` encoder and decoder. The BtbN LGPL shared build pinned in CI includes libopus in `avcodec-62.dll`. Fedora's `ffmpeg-free` is expected to as well; section 5.10 covers the check.
- `pipewire` 0.10 ships an `audio-capture` example and a `create-delete-remote-objects` example covering the stream, registry, and factory calls this design relies on. `libspa` 0.10 has audio format helpers under `param::audio`.
- The dev machine runs PipeWire with a PulseAudio compatibility server and one USB stereo sink at 48 kHz.

## 13. Amendments from the implementation run

- **Linux node identification (5.4).** `application.process.id` is absent from a node's registry properties at announce time on PipeWire 1.6 with WirePlumber, so the backend resolves a node's owner through its `client.id` to the Client global's kernel-verified `pipewire.sec.pid`. brp's own capture node is recognised only when both `node.name == brp-audio-capture` and that resolved pid match; a node whose pid cannot be resolved is never linked (fail closed) and logged once. Links carry `link.passive = true` so a passive link does not keep an idle application node running.
- **Stream node id (5.4).** `Stream::node_id` is unassigned until the stream is paused, so the backend learns its own node id from the registry, via the graph's verdict for our own node, rather than from the stream object.
- **PipeWire core errors (5.4).** Only an error on the core object ends the capture session; per-object errors, such as a link whose target vanished, are logged and ignored.
- **Jitter buffer (5.6).** A packet below the next sequence is late regardless of whether playout is primed. After the buffer runs dry it re-primes to the target depth. The shrink check runs before the priming check, so a long idle stretch still relaxes the depth even mid re-prime.
- **Mixer (5.6).** Tracks and remembered gains live under one mutex so a gain set concurrently with a track being added cannot be lost. A track with fewer samples than the callback contributes silence for that callback and is cleared, rather than kept for the next one.
- **Audio publisher (5.6).** The capture-chunk sender lives on the publisher handle, not on the shared thread state, so the encode thread exits once every handle and sink is dropped, with no explicit `stop()` required.
- **Net (5.7).** Route removal on the client is guarded so only the subscription that registered a route removes it: a stale teardown cannot evict a newer subscription's audio route. Stream priority via `SendStream::set_priority` was available and is used to rank audio streams above video.
- **Registry (5.8).** Capture and encoder threads are stopped after the registry lock is released, not while held. `views()` re-derives `has_audio` the same way `live_infos()` does, rather than relying on a stored value that could go stale.
- **Audio output trait (5.3).** `AudioOutputSession` is `Send + Sync`, not just `Send`, so the room stays `Sync`.
- **Windows (5.5).** Buffers flagged `AUDCLNT_BUFFERFLAGS_SILENT` are zeroed before reaching the sink rather than passed through undefined, and draining keeps byte alignment to whole stereo frames so a partial frame cannot swap channels for the rest of the session.
- **Mixer track cushion (5.6).** The decode loop pushes one 20 ms slot per 20 ms tick, so a track that starts empty holds less than a device callback larger than one packet ever asks for, and every callback underruns. The first packet of each run — the initial prime and every re-prime after the jitter buffer runs dry — pre-rolls `MIXER_TRACK_CUSHION` of silence. A short track now contributes what it holds and the rest stays silent instead of being cleared, a decode that yields no frames still pushes one frame of silence, and `render` only tries each track's buffer lock. Playout latency is therefore about 140 ms rather than the 60 ms of section 7's original wording.
- **Carrier re-acquisition (5.8).** Spec 5.8's migration applies to every path that loses a carrier, not only `unwatch`: a watch that ends, an audio decode thread that finishes under a live watch, and a publisher whose presence turns `has_audio` back on. The watcher's `reacquire_audio` picks the publisher's lowest watched live id, skipping ended watches, and re-watches it through the preset-switch path. A publisher that stops sharing goes quiet rather than closing anything the viewer can observe — an idle publisher sends nothing either — so presence is also what clears a stale carrier flag. The presence loop carries both directions on an `audio_changed` channel beside `expired`.
- **Capture start (5.8).** `subscribe_audio` opens the encoder and starts the platform capture with the registry lock released, serialised by a lock of its own, so `Room::snapshot` and every presence broadcast stay quick while a daemon is slow; a second subscriber waits there and then finds the capture installed. Both backends bound their ready wait with `AUDIO_CAPTURE_START_TIMEOUT` and quit their thread on a timeout.
- **Audio payload cap (5.7).** The client drops an audio frame larger than `MAX_AUDIO_PACKET_BYTES`, and any frame whose `kind` contradicts the route its preset id selects, warning once per connection. `interleaved_samples` rejects a decoded frame longer than `AUDIO_FRAME_SAMPLES` per channel. The fake audio codec carries 16-bit PCM so its packets fit the same cap.
- **PipeWire linkability deadline (5.4).** Every link plan waits for our own node and both of its input ports. If the graph has not produced them `AUDIO_CAPTURE_START_TIMEOUT` after the loop starts, the thread records "capture stream never became linkable" in the error slot, warns once, and quits, so housekeeping drops `has_audio` instead of advertising a capture that can never link.

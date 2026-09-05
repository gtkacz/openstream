# Slice 2: rooms, multiple lives, and the participant window

Status: approved design, 2026-09-04. Frame-rate amendment approved 2026-09-05 (section 3, 5.2, 7, 10). Refines phase 2 of `2026-09-04-p2p-screen-sharing-design.md`, which remains the master spec. Where this document is silent, the master spec applies.

## 1. Goals

- A participant creates a room or joins one with a ticket, and sees who else is in it.
- Every participant can publish several lives at once and watch several lives at once, in a tile grid inside one window.
- A live offers presets the publisher controls; a viewer picks one per live.
- Encoders exist only while someone watches them.
- The media server serves members only.

## 2. Non-goals for this slice

- Audio, pop-out windows, fullscreen, settings persistence, Windows and macOS. Those keep their phases.
- Seamless mid-stream preset switching over one control stream. See section 6.5.
- Congestion-driven preset changes, repeaters, relayed-path quality adaptation.

## 3. Decisions and rationale

| Decision | Rationale |
|---|---|
| New `room` crate holding membership, catalog, live registry, and watches behind a snapshot and command API | Two rooms can be tested in one process with fake codecs; the window stays a thin renderer; the headless publisher reuses the same model. |
| Membership is application level, from signed presence with heartbeat and expiry | The gossip library only exposes direct overlay neighbours and reports the last hop, not the author. |
| Bootstrap addresses are registered in the transport's in-memory address lookup before subscribing | iroh 1.1 has no method to add a peer address to an endpoint; the lookup service is the supported path. Peers beyond the bootstrap become reachable through gossip's own address exchange. |
| Preset switching is unsubscribe plus subscribe | Avoids decoder and reorder reset logic on the viewer for a user-initiated action that costs one extra round trip. `SwitchPreset` and `PresetSwitched` stay reserved. |
| Preset templates rather than free-form fields | No invalid presets can be typed; the UI stays small. |
| Two implementation plans, 2a headless room layer and 2b window | Each ends runnable and testable; 2a is verified with headless publishers and the phase 1 viewer. |
| Frame rate is a per-live control that sets every preset's `fps`, enforced by frame pacing in the publisher | The catalog already advertises a preset frame rate; pacing makes it true. Capture keeps the rate PipeWire negotiated at start, bounded by `--fps`, because changing it would reopen the portal picker. |

## 4. Product model additions

- **Member.** A peer whose signed presence was verified within the expiry window. Carries nickname, short id, lives, last-seen time, and path kind (direct, relayed, unknown).
- **Own live.** A capture session plus a set of presets. Each preset may have a running encoder.
- **Watch.** This participant's subscription to one remote live at one preset, with a state of connecting, live, reconnecting, or ended.
- **Preset templates.** Source at even source dimensions, plus optional derived presets at 1080, 720, and 480 lines, offered only when strictly smaller than the source, width scaled to keep the aspect and rounded down to even.

## 5. Architecture

### 5.1 The `room` crate

`Room::create(config)` and `Room::join(config, ticket)` bind the endpoint, spawn gossip and the media server behind one router, subscribe the topic, and start the loops. `Room::snapshot()` returns a `RoomSnapshot` clone; `Room::version()` returns a counter that increments on every change so the window redraws only when needed. Commands are async methods: `start_live(kind, title)`, `stop_live(live_id)`, `set_presets(live_id, presets)`, `watch(publisher, live_id, preset_id)`, `unwatch(publisher, live_id)`, `ticket()`, `leave()`.

`RoomConfig` carries the secret key, relay setting, nickname, a capture backend, an encoder factory, and a decoder factory. The factories are traits so tests inject the fake codec and the synthetic source.

Modules, each with one responsibility:

| Module | Responsibility |
|---|---|
| `membership` | Pure state. `apply(verified presence, now)` accepts only newer timestamps per author. `expire(now)` drops silent peers. `is_member(id)`. |
| `gossip` | Joins the topic, broadcasts signed presence on join, on change, and on heartbeat; feeds received presence into membership; logs neighbour and lag events. |
| `registry` | Own lives. Per live: capture session, a capture fan into one slot per running encoder, presets with optional publishers. Implements the media server's `LiveSource` and its connection policy. Lazy start on first subscription, stop after the idle grace on a one second housekeeping tick. |
| `watcher` | Remote lives. One media connection per publisher reused across lives. Watches yield a frame slot and stats. Reconnects with backoff while the publisher is a member; ends when the member expires. |
| `snapshot` | `RoomSnapshot`, `MemberView`, `OwnLiveView`, `WatchView`, `PresetView`. |
| `codecs` | `EncoderFactory` and `DecoderFactory` traits, the FFmpeg-backed implementations, and fake ones behind a `fake` module for tests. |

### 5.2 Refactors to phase 1 crates

- `pipeline::Publisher::start` no longer receives the capture session and reads `Arc<CaptureFrame>` from its slot, so several encoders share one capture without copying pixels. The registry owns sessions.
- `net::MediaServer::new(source, policy)` takes a `ConnectionPolicy` with `allows(peer) -> bool`, checked before any stream is accepted. Refused connections close with an application code that the client maps to a retriable error.
- `proto` gains `LiveInfo`, `Presence`, `Signed<T>` with `sign(secret, &T)` and `verify() -> Result<(author, T)>`, preset template derivation, and the constants in section 9.
- `pipeline::Publisher` paces to the preset frame rate: a captured frame is skipped when its timestamp falls before the next slot at `1 / fps` after the last encoded frame. Presets at the source rate pass every frame.
- `app::publish` is rebuilt on `Room`. `app::watch` is deleted in plan 2b.

### 5.3 Data flow

**Presence out.** Registry or nickname changes bump a dirty flag; the gossip loop broadcasts a fresh signed presence immediately and otherwise every heartbeat.

**Presence in.** Gossip receive loop verifies the envelope, hands it to membership, bumps the snapshot version when the member set or a member's lives changed.

**Subscription in.** Media server checks the policy, reads Subscribe, asks the registry. The registry finds the live and preset, starts an encoder if none is running by opening a converter and encoder through the factory and a `Publisher` fed by a new slot registered with the live's capture fan, then adds a fan-out subscriber. Housekeeping stops publishers idle past the grace and unregisters their slots.

**Watch out.** The watcher looks up the member's address from membership, connects or reuses the connection, subscribes, opens a decoder through the factory, and starts a `Viewer`. The tile renders from the viewer's slot. On disconnect while the member remains, it retries with backoff and reports reconnecting.

## 6. Protocol

### 6.1 Presence

```
Signed<T>  { author: EndpointId, payload: Vec<u8>, signature: [u8; 64] }
Presence   { version: u8, ts_unix_ms: u64, nickname: String, lives: Vec<LiveInfo> }
LiveInfo   { id: u32, title: String, kind: SourceKind,
             source_width: u32, source_height: u32, source_fps: u32,
             has_audio: bool, presets: Vec<Preset> }
```

The signature covers the payload bytes. Receivers verify with the author's public key, reject unknown versions, and reject timestamps not newer than the last accepted from that author. Nicknames are truncated to the maximum length before signing.

### 6.2 Gossip transport

Topic id is the ticket's 32 bytes. Messages use the library default size cap of 4 KB, which the limits of 8 lives and 6 presets keep comfortably. Broadcasts before the first neighbour is connected are not delivered; the heartbeat covers that window.

### 6.3 Tickets

Create mints a random topic and lists our own address as bootstrap. Any member can mint a ticket listing itself. Join registers every bootstrap address in the endpoint's in-memory lookup, then subscribes with the bootstrap ids and waits for the first neighbour with a timeout before reporting joined.

### 6.4 Membership gating

The media server admits a connection only when membership currently contains the caller. A viewer refused because presence has not propagated yet retries through the watcher's backoff.

### 6.5 Preset switching

The viewer unsubscribes from the old preset and subscribes to the new one. The tile keeps its last frame until the first keyframe of the new preset decodes. `SwitchPreset` is never sent in this slice.

### 6.6 Preset changes by the publisher

Adding a preset rebroadcasts presence. Removing a preset stops its encoder and ends its subscriptions with live-ended on those control streams; viewers fall back to the live's Source preset automatically. Changing a preset's bitrate or codec restarts its encoder on the next subscription and, if one is running, immediately.

## 7. The participant window

- **Left panel.** Members with nickname, short id, and a path badge. Under each, their lives with a watch checkbox and a preset selector.
- **Centre.** Tile grid: columns are the ceiling of the square root of the watch count, rows follow, which yields the master spec's one to nine layouts. Each tile draws letterboxed video and, on hover, an overlay with title, preset selector, and stats toggle. Reconnecting and ended states show as a status line over the last frame.
- **Bottom panel.** Own lives with title, per-preset encoder state including the encoder name and bitrate, template checkboxes, a bitrate control within the allowed range, a frame-rate control from 1 to the source rate that applies to every preset of the live, a codec selector, and a stop button. Share monitor and share window buttons open the portal picker; new lives are titled by kind and ordinal.
- **Status bar.** Copy ticket, member count, aggregate encode bitrate, nickname.

Rendering generalises the phase 1 video renderer to a set of tiles, each with its own planes, letterbox uniform, and viewport inside the central area, drawn in one render pass before egui. Redraw triggers: any watch's new frame, a snapshot version bump, input.

Commands: `brp create [--nickname N] [--fps F] [--no-relay]`, `brp join <ticket> [--nickname N] [--fps F] [--no-relay]`, and `brp publish` with its phase 1 flags plus `--ticket` and `--nickname`. Nickname defaults to the short peer id. `--fps` is the capture ceiling for lives started from the window and defaults to 60.

## 8. Error handling

- Unverifiable, stale, or unknown-version presence is dropped at debug level.
- A refused media connection is a transient failure to the watcher; it retries while the publisher is a member.
- Member expiry while watched ends the watch and clears the checkbox.
- Encoder start failure rejects that subscription with the codec error text; the live stays up on other presets and the bottom panel marks the preset failed.
- Capture loss removes the live from presence and ends its subscriptions.
- Portal denied leaves a status line and creates no live.
- A gossip lag event is logged at warn; the next heartbeat repairs the catalog.

## 9. Constants added in this slice

| Constant | Value | Rationale |
|---|---|---|
| Presence heartbeat | 5 s | From the master spec |
| Member expiry | 20 s | From the master spec |
| Max lives per participant | 8 | From the master spec |
| Max presets per live | 6 | From the master spec |
| Registry housekeeping tick | 1 s | Bounds how late an idle encoder is noticed relative to the 5 s grace |
| Join timeout | 15 s | Three heartbeats to reach the first neighbour before reporting failure |
| Template heights | 1080, 720, 480 | Common display heights below typical sources |
| Nickname max length | 32 chars | Keeps presence small and panels legible |

## 10. Testing

- **Unit.** Membership apply and expiry; presence signing, tampering, replay; template derivation including aspect and even rounding; grid layout for one to nine; publisher frame pacing; registry lazy start and idle stop with an injected clock; watcher backoff schedule.
- **Integration, hardware-free.** Two rooms in one process with relays disabled, fake codecs, synthetic capture: mutual presence; catalog propagation; a watch that decodes frames; refusal of a third endpoint outside the room; live stop ending the watch; preset add and remove propagating.
- **Backfill.** Phase 1 unit tests for slot, fan-out, reorder, synthetic source, ticket, and fake codec, taken from the phase 1 plan.
- **Manual.** Two machines: create and join, three lives across both, a three-tile grid, a preset switch, publisher exit, ticket minted by the joiner used from a third machine.

## 11. Plans

1. **2a, room layer.** Backfill tests, refactors, proto presence, room modules with tests, `publish` on `Room`. Verified with two headless publishers and the phase 1 viewer.
2. **2b, participant window.** Snapshot-driven panels, tile renderer, commands wired, `create` and `join`, removal of `watch`. Verified with the two-machine check.

## 12. References

Verified on 2026-09-04 against iroh-gossip 0.101.0 and iroh 1.1.0: `Gossip::builder().spawn(endpoint)` implements the router's protocol handler under `iroh_gossip::ALPN`; `subscribe(topic, bootstrap_ids)` returns a topic that splits into sender and receiver, with `joined()` resolving on the first neighbour; events are `NeighborUp`, `NeighborDown`, `Received { content, delivered_from, scope }`, and `Lagged`, where `delivered_from` is the last hop; the default message cap is 4 KB; bootstrap addresses are registered through `MemoryLookup` in `iroh::address_lookup`; `SecretKey::sign` and `PublicKey::verify` exist with a serde-capable 64-byte `Signature`. A two-endpoint loopback join measured about 16 ms.

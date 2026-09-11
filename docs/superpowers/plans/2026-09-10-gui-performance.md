# GUI and Screen-sharing Performance Plan

Status: proposed implementation plan. No optimization has been implemented or benchmarked as part of this document.

## Goal and scope

Keep the Linux desktop responsive while sharing a screen, and reduce GUI overhead while watching streams or using pop-outs. The reported symptom is whole-desktop slowdown during sharing on a powerful Linux PC. Whether it begins before a viewer connects is unknown.

Proceed with the optimization candidates without requiring the user to reproduce or confirm a bottleneck first. Collect comparative measurements during implementation; do not present any candidate as the established cause of the reported slowdown.

## Existing behavior to preserve

- Room snapshots are already cached by version.
- Video frame slots already provide latest-frame consumption; consuming a frame prevents another window from uploading that same slot value again.
- Tile textures are already reused until the frame dimensions change.
- Encoders start on subscription, rather than for every advertised preset. Multiple subscribed presets can run separate conversion and encoding pipelines.
- The event loop already waits between events; there is no unconditional polling loop to remove.
- Hardware encoder selection can fall back to software AV1 (`libsvtav1`). The sharing panel already exposes the encoder name.

Recheck the relevant live code before implementation because other changes may land after this plan.

## Delivery order

| Stage | Work | Intended result |
|---|---|---|
| 1 | Publishing controls, targeted redraws, housekeeping separation | Bound background work and remove unnecessary GUI activity |
| 2 | Buffer reuse and shared conversion | Reduce allocation, memory traffic, and duplicate CPU work |
| 3 | Linux GPU frame interoperability | Avoid CPU round trips where supported |

Each candidate can be delivered separately. Stage 3 is an exploratory implementation with capability-based fallback, not a prerequisite for completing stages 1 and 2.

## 1. Control publishing resource use

Primary files: `crates/app/src/settings.rs`, `crates/app/src/ui/settings.rs`, `crates/app/src/launch.rs`, `crates/codec/src/select.rs`, `crates/codec/src/ffmpeg/encoder.rs`, `crates/pipeline/src/publisher.rs`, `crates/pipeline/src/pacer.rs`.

- [ ] Make a 30 FPS sharing option easy to select using the existing FPS setting; preserve explicit user choices and avoid introducing a second conflicting setting.
- [ ] Explain software fallback in the sharing UI in plain language, using the existing encoder identity. Keep detailed diagnostics in logs.
- [ ] Bound software encoder parallelism using controls supported by the installed FFmpeg/SVT-AV1 versions. Account for several active encoders so each cannot independently consume the full machine.
- [ ] Add overload pacing based on sustained conversion/encoding time, with hysteresis and gradual recovery up to the configured FPS ceiling. Keep queues bounded and prefer recent frames.
- [ ] Preserve keyframe requests, timestamps, audio continuity, and encoder startup/shutdown behavior when pacing changes.

Acceptance: software fallback has a defined aggregate concurrency policy; explicit FPS settings still work; sustained overload does not accumulate frame latency; frame-rate recovery does not oscillate. Compare CPU usage, delivered FPS, and responsiveness at 30 and 60 FPS.

Tradeoff: reducing concurrency or admission rate can lower delivered FPS. Do not promise the same quality and throughput at a lower resource budget.

## 2. Reduce unnecessary window redraws

Primary files: `crates/app/src/window.rs`, `crates/app/src/launch.rs`, `crates/app/src/render/surface.rs`, `crates/room/src/watcher.rs`, and the room frame-notification contract.

- [ ] Carry stream identity through frame notifications, or maintain a shared dirty-stream set with a single pending wakeup.
- [ ] Coalesce notifications without losing a wakeup when a frame arrives while the UI drains pending work. Bound pending state by active streams rather than incoming frame count.
- [ ] Map dirty streams to their displaying windows. Redraw the main grid only when an affected tile is there; redraw only the relevant pop-out otherwise.
- [ ] Keep repaint deadlines per window. A cursor blink or animation in one window must not redraw every window.
- [ ] Track minimized, zero-sized, and supported occlusion state. Suspend unnecessary presentation while hidden and request a fresh frame immediately when restored. Treat unknown visibility conservatively.
- [ ] Preserve independent redraw requests for input, resize, expose, dialogs, animations, and room changes.

Acceptance: frames for one popped-out stream do not repeatedly redraw unrelated windows; notification bursts have bounded pending work; minimize/restore and moving a tile between grid and pop-out do not freeze video; input remains responsive.

Tradeoff: scheduling requires more state and careful lifecycle handling. Include deterministic tests for dirty-state draining, concurrent notification arrival, window closure, and repaint deadlines.

## 3. Move housekeeping out of frame rendering

Primary files: `crates/app/src/room_view.rs`, `crates/app/src/window.rs`, `crates/app/src/participant.rs`, `crates/app/src/ui/state.rs`.

- [ ] Refresh room snapshots and reconcile watches, handles, textures, and pop-outs once per relevant state change.
- [ ] Refresh rate meters and age labels on the existing statistics timer rather than every video redraw.
- [ ] Cache the serialized ticket. Invalidate it on endpoint/relay changes, or refresh it on a bounded timer if no reliable event exists. Room version alone is insufficient for relay address changes.
- [ ] Separate metadata preparation from rendering so drawing a second window does not repeat housekeeping.
- [ ] Keep the snapshot coherent across a rendering batch and apply lifecycle updates before using removed stream handles.

Acceptance: steady video redraws do not regenerate tickets or rebuild watch membership sets; statistics still advance without video; relay changes update the ticket; ended watches close their pop-outs promptly.

Tradeoff: invalidation errors can leave stale UI state. Test endpoint updates without room-version changes and watch removal while frames are pending.

## 4. Reduce video memory transfers

Primary files: `crates/codec/src/raw.rs`, `crates/codec/src/ffmpeg/decoder.rs`, `crates/codec/src/ffmpeg/convert.rs`, `crates/app/src/room_view.rs`, `crates/app/src/render/tiles.rs`.

- [ ] Trace ownership across decoding, frame slots, and GPU upload; remove intermediate copies only when buffer lifetime permits it.
- [ ] Reuse decoded-frame and conversion allocations through bounded pools. Return buffers only after all consumers release them.
- [ ] Avoid redundant initialization of buffers that are fully overwritten, while keeping padding and partially written regions safe.
- [ ] Preserve existing texture reuse and latest-frame consumption. Avoid uploading frames for windows that cannot present them, while retaining access to the newest frame on restoration.
- [ ] Compare direct texture writes with reusable staging storage before changing upload strategy; adopt staging only when it improves the measured workload.
- [ ] Handle stride, chroma layout, resolution changes, and pool exhaustion explicitly; never allow unbounded retained buffers.

Acceptance: stable-resolution streaming reuses frame allocations; obsolete frames do not accumulate; resolution changes and odd dimensions remain correct; upload counts track consumed new frames rather than window redraw count.

Tradeoff: pooling changes ownership and increases retained memory. Bound memory by stream count and frame size, and release resources when watches end.

## 5. Reduce repeated publishing conversion

Primary files: `crates/room/src/codecs.rs`, `crates/room/src/registry.rs`, `crates/pipeline/src/publisher.rs`, `crates/codec/src/ffmpeg/convert.rs`.

- [ ] Reuse scaling and conversion buffers before introducing shared processing.
- [ ] Identify active presets with compatible output dimensions, pixel format, color properties, and source frame identity; convert once and share immutable results where compatible.
- [ ] Skip conversion when the captured format and dimensions already satisfy the encoder input contract.
- [ ] Keep independent pacing and bounded latest-frame delivery for each preset. One slow consumer must not block another.
- [ ] Evaluate hardware scaling/color conversion as a separate backend option, preserving CPU conversion fallback.
- [ ] Retain subscription-driven encoder startup and existing idle cleanup. Advertising an unused preset must not create conversion work.

Acceptance: compatible presets share conversion work; incompatible presets remain correct; disconnecting a viewer releases unused processing resources; slow presets cannot increase latency for other viewers.

Tradeoff: sharing intermediates may require more synchronization and memory. Separate output resolutions generally still require separate scaling work.

## 6. Keep video frames on the GPU where supported

Primary areas: `crates/capture/src/linux/pipewire.rs`, `crates/capture/src/frame.rs`, codec frame abstractions and hardware backends, and `crates/app/src/render`.

- [ ] Document supported import/export capabilities for the actual capture, encoder, decoder, and rendering devices before selecting an interoperability design.
- [ ] Prototype capture-to-encoder and decoder-to-renderer independently using Linux DMA-BUF where supported.
- [ ] Define ownership, synchronization, plane layout, modifiers, color metadata, and device compatibility explicitly.
- [ ] Negotiate capabilities at runtime; retain the CPU path for unsupported formats, devices, drivers, and import failures.
- [ ] Handle resize, stream shutdown, device loss, and buffers still in use without premature release or stale frames.
- [ ] Land each accelerated path only with a repeatable comparison showing reduced copies or resource use and correct output on supported hardware.

Acceptance: supported paths avoid the intended CPU round trip; unsupported systems continue working; no frame corruption, synchronization stalls, or resource leaks occur in resize and shutdown testing.

Tradeoff: this is the highest-complexity candidate. Cross-device copies may remain necessary, and hardware encoding alone does not guarantee a copy-free pipeline.

## Measurement and verification

Instrumentation accompanies implementation and does not block starting it. Use aggregate counters and sampled timings rather than per-frame log output.

- [ ] Record redraws per window, coalesced notifications, UI preparation time, surface acquisition time, conversion/encoding duration, upload bytes, frame drops, active encoder, and buffer-pool usage.
- [ ] Compare release builds with the same machine, source resolution, content, selected presets, viewers, and display configuration. Separate capture, CPU processing, video engine, and rendering load where tooling permits.
- [ ] Exercise idle GUI; sharing without viewers; one viewer; several viewers on the same and different presets; viewing with pop-outs; minimized/restored windows; and software fallback.
- [ ] Include high-resolution/high-refresh capture, static content, motion, resize, viewer disconnection, and audio-enabled sharing.
- [ ] Record CPU/GPU usage and delivered FPS alongside latency and desktop responsiveness. Report quality or throughput reductions explicitly instead of counting them as free performance gains.
- [ ] Add behavioral tests for scheduling, invalidation, pacing, and ownership where those contracts change. Run relevant crate tests plus repository-required formatting, lint, and integration checks before merging implementation changes.
- [ ] Inspect Repowise rationale/risk before changing existing architectural patterns or hotspot files; refresh and check affected code health after meaningful implementation work.

## Completion criteria

Stages 1 and 2 are complete when their behavioral acceptance criteria pass and comparative results document benefits and regressions. Stage 3 completes per supported path, with a tested fallback and a documented compatibility boundary. No fixed percentage speedup is promised in advance.

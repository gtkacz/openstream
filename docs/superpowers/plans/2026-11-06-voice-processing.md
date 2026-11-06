# Voice Processing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Echo cancellation, Light/Strong noise suppression and automatic gain on each person's microphone before it is encoded for voice. Each can be switched off in Settings.

**Architecture:**
- A new crate, `brp-voiceproc`, owns the processing:
  - a `VoiceProcessor` that runs a high-pass filter and AEC3 (`sonora`), then RNNoise or DeepFilterNet3, then AGC2 (`sonora`), on 10 ms mono hops;
  - an `EchoReference` that collects what brp's two mixers render, as two channels, without blocking their device threads.
- The voice `AudioPublisher` runs a per-frame hook on its encode thread, just before Opus. `VoiceSend` installs the processor and the level meter there.
- Settings carry three new audio fields to the voice join.

**Tech Stack:**
- Rust 2024 (workspace MSRV 1.95), egui;
- `sonora` 0.2, `nnnoiseless` 0.5, DeepFilterNet libDF over tract 0.21.4 (Task 6 only).

**Spec:** `docs/superpowers/specs/2026-11-06-voice-processing-design.md`. Read it before any task. Its section 9 holds the measured figures the test thresholds below are drawn from.

## Global Constraints

- **Dependencies**, added to `[workspace.dependencies]` and nothing else:
  - `sonora = "0.2"`
  - `nnnoiseless = { version = "0.5", default-features = false }`
  - Task 6 only: `deep_filter = { git = "https://github.com/Rikorose/DeepFilterNet", rev = "7b14b895fe7fc42cab72b4a4c1e49c982bd6404b", default-features = false, features = ["tract", "default-model"] }`, plus `tract-core`, `tract-onnx`, `tract-pulse` and `tract-hir` each at `"=0.21.4"`, plus `ndarray = "0.15"`. The tract pins carry a comment: libDF does not compile against later tract 0.21 releases.
- **Processing format:** 48 kHz, mono, 10 ms hops of 480 samples. Order: high-pass filter and echo cancellation, then noise suppression, then automatic gain.
- **`sonora` configuration:**
  - `Pipeline { maximum_internal_processing_rate: MaxProcessingRate::Rate48kHz, .. }`.
  - Echo-cancelling instance: render stream of 2 channels (voice, streams) and capture stream of 1 channel.
  - AGC2: `input_volume_controller: false`, `adaptive_digital: Some(AdaptiveDigital::default())`, `fixed_digital: FixedDigital::default()`.
- **`nnnoiseless` scaling:** it expects samples on the i16 scale. Multiply by 32768 going in and divide by 32768 coming out.
- **Defaults:** `echo_cancellation = true`, `noise_suppression = Light`, `automatic_gain = true`. They apply to new settings files and to files written before this change.
- **Unchanged when off:** with all three off, the frame reaching the encoder is bit-for-bit what it is today.
- **Threads:** processing never runs on an audio device thread. Device threads never wait on a lock that the encode thread holds.
- **Failures:** a stage that fails to start, returns an error, or panics is dropped for the rest of that voice session. It logs one `warn!`, and its input passes through.
- **Releases and pushes:** commit locally after each task and never push. A push to `main` with a commit that is not a `fix:` releases automatically. Task 6 ends by asking the requester before anything leaves the machine.
- **Commits:** Conventional Commits, matching `git log`, for example `feat(voiceproc): …` or `feat(app): …`. No attribution trailers.
- **Gate before every commit:** `cargo fmt --all` and `cargo clippy --workspace --all-targets -- -D warnings` are clean, and the tests of every crate the task touched pass.

## Review Focus

1. **Muting and unmuting.** While the microphone is closed, no frames reach the processor, but the mixers keep pushing reference audio. On unmute, cancellation resumes without unbounded memory or a long stale burst. Covered by the cap test in Task 1.
2. **A stream output that never opened.** `room.rs` tolerates `audio_output.start` failing. Then the streams channel never receives anything, and cancellation of voice must not wait for it. Covered in Tasks 1 and 3.
3. **Odd callback sizes.** Devices render 441, 1024 or other counts per callback, never a neat 480. The reference must come out as whole hops in order, with no samples lost or duplicated. Covered in Task 1.
4. **Leaving and rejoining voice.** The reference stops collecting when the processor is dropped, and a rejoin starts clean. Covered in Tasks 1 and 3.
5. **A microphone that carries voice on one channel.** The downmix averages L and R; the output must carry the processed voice on both channels. Covered in Task 2.

---

### Task 1: The echo reference

The reference buffers what the two mixers render until the encode thread asks for whole hops. It is a crate of its own because the processor (Task 2) and the room (Task 4) both use it.

**Files:**
- Create: `crates/voiceproc/Cargo.toml`, `crates/voiceproc/src/lib.rs`, `crates/voiceproc/src/reference.rs`
- Modify: `Cargo.toml` (workspace dependencies: `brp-voiceproc = { path = "crates/voiceproc" }`, `sonora`, `nnnoiseless`)
- Modify: `crates/proto/src/constants.rs` (add `ECHO_REFERENCE_MAX`)

**Interfaces:**
- Produces:
  - `pub const ECHO_REFERENCE_MAX: Duration = Duration::from_millis(200);` in `brp_proto::constants`. Its doc comment gives the reason: "Ten times the 20 ms the encode thread takes between drains. Older reference audio is stale, and only confuses the canceller's delay estimate."
  - `#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum ReferenceChannel { Voice, Streams }`
  - `#[derive(Clone, Default)] pub struct EchoReference`, a handle around `Arc<…>`:
    - `pub fn new() -> Self`
    - `pub fn push(&self, channel: ReferenceChannel, stereo: &[f32])`: averages interleaved stereo to mono and appends it. It returns at once while inactive, or if its lock is contended, dropping that callback's audio. Each channel keeps at most `ECHO_REFERENCE_MAX` (9 600 samples) and drops the oldest.
    - `pub fn tap(&self, channel: ReferenceChannel, render: impl FnMut(&mut [f32]) + Send + 'static) -> impl FnMut(&mut [f32]) + Send + 'static`: calls `render`, then `push`es what it wrote.
    - `pub(crate) fn set_active(&self, active: bool)`: deactivating empties both channels.
    - `pub(crate) fn drain_hops(&self) -> Vec<[[f32; 480]; 2]>`: every complete hop available, as `[voice, streams]`, in order. See step 3 for the rule.
  - `brp_voiceproc` re-exports `EchoReference` and `ReferenceChannel`.

- [ ] **Step 1: Create the crate.** Package `brp-voiceproc`, using workspace version, edition, license and `rust-version`. Its dependencies are `brp-proto`, `sonora`, `nnnoiseless` and `tracing`, all `.workspace = true`. `lib.rs` declares `mod reference;` and the re-exports.

- [ ] **Step 2: Write the failing tests** in `reference.rs`. Run them with `cargo test -p brp-voiceproc reference`; expected FAIL, because `EchoReference` is not defined. The tests:

```rust
#[test] fn nothing_is_kept_while_inactive()
    // push 4800 stereo samples to Voice without set_active → drain_hops() is empty
#[test] fn odd_callback_sizes_come_out_as_whole_hops_in_order()
    // active; push a ramp (sample i == i as f32, L == R) to Voice in callbacks of 441 and 1024 frames,
    // 4000 frames in total → drain_hops() yields 8 hops whose voice halves concatenate to 0.0..3840.0,
    // streams halves all 0.0; a second drain_hops() after pushing 320 more frames yields 1 hop starting at 3840.0
#[test] fn a_channel_that_never_played_is_zero_filled()
    // only Voice pushed (960 frames) → 2 hops, streams all 0.0
#[test] fn both_channels_wait_for_each_other_once_both_play()
    // Voice 960 frames, Streams 480 frames → 1 hop; push 480 more to Streams → 1 more hop, both halves non-zero
#[test] fn a_stopped_channel_stops_holding_back_the_other()
    // Streams gets 100 frames then nothing; Voice gets ECHO_REFERENCE_MAX + 960 frames
    // → hops come out, and the streams half of the later ones is 0.0
#[test] fn the_buffer_never_holds_more_than_the_cap()
    // active; push 2 s to both channels without draining → drain_hops().len() * 480 <= 9600
#[test] fn deactivating_empties_both_channels()
    // active; push 960 frames to both; set_active(false); set_active(true) → drain_hops() is empty
#[test] fn the_tap_renders_then_records()
    // tap(Voice, |out| out.fill(0.25)); call it with a 960-sample buffer → buffer is all 0.25, and drain_hops() yields 1 hop of 0.25
```

- [ ] **Step 3: Implement `EchoReference`.** Use a `Mutex` around both channels' `VecDeque<f32>` and a per-channel `playing` flag, with `try_lock` in `push` and `lock` in `drain_hops`, plus an `AtomicBool` for active. The draining rule:
  - A channel is *playing* from its first `push` after activation.
  - The number of hops drained is the smallest `len / 480` among playing channels. A channel that is not playing is zero-filled.
  - When `push` overflows one channel's cap while the other playing channel holds less than one hop, that other channel stops playing and its partial hop is discarded. A device that stopped then no longer holds the other back.
  - Deactivating clears both channels and both flags.
  - Never zero-fill a playing channel that is merely between callbacks. That would shift its audio in time against the microphone.

- [ ] **Step 4: Run** `cargo test -p brp-voiceproc reference`. Expected: PASS, 8 tests.

- [ ] **Step 5: Commit.** `git add Cargo.toml Cargo.lock crates/voiceproc crates/proto/src/constants.rs && git commit -m "feat(voiceproc): collect what brp plays as a two-channel echo reference"`

---

### Task 2: The processor, with Light and automatic gain

**Files:**
- Create: `crates/voiceproc/src/config.rs`, `crates/voiceproc/src/processor.rs`, `crates/voiceproc/src/test_signals.rs` (`#[cfg(test)]`), `crates/voiceproc/tests/fixtures/speech-48k-mono-s16le.raw`
- Modify: `crates/voiceproc/src/lib.rs`

**Interfaces:**
- Consumes: `EchoReference` (Task 1).
- Produces:
  - `#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum Suppression { Off, Light }`. Task 6 adds `Strong`.
  - `#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub struct ProcessingConfig { pub echo_cancellation: bool, pub noise_suppression: Suppression, pub automatic_gain: bool }`, with:
    - `pub const OFF: ProcessingConfig`, all three off;
    - `pub fn is_off(self) -> bool`.
  - `pub struct VoiceProcessor`, which is `Send`:
    - `pub fn new(config: ProcessingConfig, reference: &EchoReference) -> Self`
    - `pub fn process(&mut self, stereo: &mut [f32])`: one `AudioFrame` worth, 1 920 interleaved samples. It returns untouched when every stage is absent. Otherwise it averages to mono, runs two hops through the stages, and writes the result to both channels.
  - `brp_voiceproc` re-exports all three.
  - `fn run_stage<S>(stage: &mut Option<S>, name: &'static str, hop: &mut [f32; 480], f: impl FnOnce(&mut S, &mut [f32; 480]) -> Result<(), String>)`, private to `processor.rs`:
    - It runs `f` on a copy of `hop`, under `std::panic::catch_unwind(AssertUnwindSafe(..))`.
    - On `Ok`, it writes the copy back.
    - On `Err` or a panic, it logs `tracing::warn!(stage = name, …)` once, sets `*stage = None` and leaves `hop` as it was.

- [ ] **Step 1: Generate the speech fixture.**

```bash
espeak-ng -v en-us -s 160 -w /tmp/brp-speech.wav "Can you hear me clearly, or is my voice still far too quiet for you?"
sox /tmp/brp-speech.wav -r 48000 -c 1 -b 16 -e signed-integer -t raw crates/voiceproc/tests/fixtures/speech-48k-mono-s16le.raw gain -n -1
```

  Expected: a file of roughly 300–400 KB. `test_signals::speech() -> Vec<f32>` loads it with `include_bytes!` and divides by 32768.

  `test_signals` also holds:
  - `noise(len, seed) -> Vec<f32>`: a deterministic LCG passed through a one-pole low-pass (`y = 0.97*y + 0.03*x`);
  - `at_peak(x, dbfs)` and `rms_db(x)`;
  - `looped(x, seconds)`;
  - `run(&mut VoiceProcessor, mono) -> Vec<f32>`, which duplicates mono to stereo, processes whole 960-frame frames and returns the left channel.

- [ ] **Step 2: Write the failing tests** in `processor.rs`. Run them with `cargo test -p brp-voiceproc processor`; expected FAIL, because the types are missing. The tests:

```rust
#[test] fn with_everything_off_the_frame_is_untouched()
    // ProcessingConfig::OFF; a stereo frame whose L and R are different noise → identical after process
#[test] fn light_removes_steady_noise()
    // {echo: false, Light, gain: false}; 3 s of noise() at -40 dBFS RMS
    // → rms_db(last 1 s out) <= rms_db(last 1 s in) - 10.0
#[test] fn automatic_gain_raises_quiet_speech()
    // {echo: false, Off, gain: true}; looped(at_peak(speech(), -30.0), 40.0)
    // → rms_db(last 10 s out) >= rms_db(last 10 s in) + 10.0   (measured +17)
#[test] fn automatic_gain_leaves_loud_speech_within_3_db()
    // same with at_peak(.., -1.0) → |rms_db(out) - rms_db(in)| over the last 10 s <= 3.0   (measured -0.8)
#[test] fn a_one_sided_microphone_comes_out_on_both_channels()
    // {echo: false, Light, gain: true}; stereo frames with speech on L and 0.0 on R
    // → every output frame has L == R and every sample is finite
#[test] fn a_stage_that_errors_is_dropped_and_its_input_passes_through()
    // run_stage(&mut Some(()), "test", &mut hop, |_, h| { h.fill(9.0); Err("no".into()) })
    // → stage is None, hop unchanged
#[test] fn a_stage_that_panics_is_dropped_and_its_input_passes_through()
    // same with |_, _| panic!("boom")
```

- [ ] **Step 3: Implement `ProcessingConfig`, `Suppression` and `VoiceProcessor`** without echo cancellation; Task 3 adds it.
  - When `!config.is_off()`, `new` builds a capture `sonora::AudioProcessing` with only `high_pass_filter: Some(HighPassFilter::default())` for now.
  - `Suppression::Light` holds `nnnoiseless::DenoiseState::new()`.
  - `automatic_gain` adds a second `AudioProcessing` with only AGC2, configured as in Global Constraints.
  - Each stage runs through `run_stage`.

- [ ] **Step 4: Run** `cargo test -p brp-voiceproc`. Expected: PASS, every test in Tasks 1–2, in under 60 s in a debug build. `sonora` costs about 1.5 ms per 10 ms hop in debug.

- [ ] **Step 5: Commit.** `git add crates/voiceproc && git commit -m "feat(voiceproc): suppress noise with RNNoise and level the microphone with AGC2"`

---

### Task 3: Echo cancellation

**Files:**
- Modify: `crates/voiceproc/src/processor.rs`, `crates/voiceproc/src/test_signals.rs`

**Interfaces:**
- Consumes: `EchoReference::{set_active, drain_hops}` (Task 1).
- Produces: `VoiceProcessor::new` honours `echo_cancellation`, and `Drop for VoiceProcessor` deactivates the reference it activated. No signature changes.

- [ ] **Step 1: Add `test_signals::sparse_echo(far: &[f32], delay_ms: usize, seed: u64) -> Vec<f32>`.**
  - It builds a direct path of gain 0.5 at `delay_ms`, plus 20 reflections. Reflection `k` sits at `delay_ms + 2 + 3k` ms plus up to 40 random samples, with gain `0.25 * r * exp(-k/6)`, where `r` is in [-1, 1) from the LCG.
  - It adds LCG noise at amplitude 3e-4.
  - This is the benchmark's room, and the spec's section 9 measures it.

- [ ] **Step 2: Write the failing tests.** Run them with `cargo test -p brp-voiceproc echo`; expected FAIL, because the echo stays at full level. Each test feeds a 20 ms frame of what is "playing" to `reference.push(...)` before it processes the matching 20 ms microphone frame, the way device threads would.

```rust
#[test] fn echo_cancellation_removes_a_speaker_echo()
    // {echo: true, Off, gain: false}; far = looped(speech(), 15.0) pushed to Voice only (Streams never plays);
    // mic = sparse_echo(far, 40, 1) → over frames after 5 s where far is not silent,
    // rms_db(mic) - rms_db(out) >= 30.0   (measured 54)
#[test] fn only_the_stream_echo_is_cancelled_while_voice_plays_on_headphones()
    // voice = looped(speech(), 15.0) to Voice; streams = 0.8 * the same rotated by 1.3 s to Streams;
    // mic = sparse_echo(streams, 60, 2) only → after 5 s, rms_db(mic) - rms_db(out) >= 30.0   (measured 51)
#[test] fn dropping_the_processor_stops_the_reference_collecting()
    // new with echo on; drop it; push 960 frames → drain_hops() is empty
#[test] fn without_echo_cancellation_the_reference_stays_inactive()
    // new with {echo: false, Light, gain: true}; push 960 frames → drain_hops() is empty
```

- [ ] **Step 3: Implement.**
  - With `echo_cancellation`, the capture instance also sets `echo_canceller: Some(EchoCanceller::default())`, with a render `StreamConfig::new(48_000, 2)`. `new` calls `reference.set_active(true)` and keeps a clone.
  - Before each capture hop, every `drain_hops()` pair goes to `process_render_f32(&[&voice, &streams], ..)`.
  - If the capture stage is dropped by `run_stage`, the reference is deactivated with it.

- [ ] **Step 4: Run** `cargo test -p brp-voiceproc`. Expected: PASS.

- [ ] **Step 5: Commit.** `git add crates/voiceproc && git commit -m "feat(voiceproc): cancel the echo of what brp plays"`

---

### Task 4: Processing on the voice path

**Files:**
- Modify: `crates/pipeline/src/audio_publisher.rs`, `crates/pipeline/src/lib.rs`
- Modify: `crates/room/Cargo.toml` and `crates/app/Cargo.toml` (`brp-voiceproc.workspace = true` in both), `crates/room/src/voice/send.rs`, `crates/room/src/voice/mod.rs`, `crates/room/src/room.rs`
- Modify: `crates/room/tests/voice.rs`, `crates/app/src/room_view.rs`. Every caller of `join_voice(`, `VoiceSend::start(` and `Voice::new(`; `rg` them first. Today they are `room_view.rs:254,299,502,513`, `room/tests/voice.rs:104,256,258,272,285`, `voice/mod.rs:115`, `voice/send.rs:232` and `room.rs:176`.

**Interfaces:**
- Consumes: `VoiceProcessor`, `ProcessingConfig`, `EchoReference`, `ReferenceChannel` (Tasks 1–3).
- Produces:
  - **In `brp_pipeline`:**
    - `pub type FrameHook = Box<dyn FnMut(&mut AudioFrame) + Send + 'static>;`
    - `pub fn start_with_hook(encoder: Box<dyn AudioEncoder>, make_hook: Box<dyn FnOnce() -> FrameHook + Send>) -> AudioPublisher`. It calls `make_hook` on the encode thread before its loop, so loading a model never blocks the caller. It runs the hook on every assembled frame just before `encoder.encode`.
    - `AudioPublisher::start(encoder)` keeps its behaviour and installs no hook.
  - **`VoiceSend::start`** becomes `(encoder, silence, processing: ProcessingConfig, reference: EchoReference, membership, not_sending, runtime)`.
  - **`Voice::new`** gains a trailing `echo: EchoReference`. `Voice::join(capture, output, processing: ProcessingConfig)` starts the voice output with `Box::new(echo.tap(ReferenceChannel::Voice, mixer.render_fn()))`.
  - **`Room::join_voice(capture, output, processing: ProcessingConfig)`.** `Room::new` creates one `EchoReference`, starts the stream output with `Box::new(echo.tap(ReferenceChannel::Streams, mixer.render_fn()))`, and passes a clone to `Voice::new`.
  - **`RoomView::join_voice(capture, output, processing, state)`.** Task 5 supplies the real config; until then the `JoinVoice` command passes `ProcessingConfig::OFF`.

- [ ] **Step 1: Write the failing tests.**
  - In `audio_publisher.rs`:

```rust
#[test] fn a_hook_sees_every_frame_before_it_is_encoded()
    // start_with_hook(FakeAudioEncoder, || Box::new(|f| f.samples.fill(0.0))); push two frames of 0.5
    // → both packets' data are all zero bytes
#[test] fn the_hook_is_made_on_the_encode_thread()
    // make_hook records std::thread::current().name() → "brp-audio-encode"
```

  - In `voice/send.rs`, the fixture gains a `processing` parameter and passes `EchoReference::new()`. Existing tests pass `ProcessingConfig::OFF`; Task 2's untouched-frame test already covers that path.

```rust
async fn a_deafened_listener_gets_silence_at_the_same_rate_with_processing_on()
    // the existing deafened test, with {echo: true, Light, gain: true}
```

  - `closing_the_mic_stops_packets_and_zeroes_the_level` must keep passing unchanged.
  - Run `cargo test -p brp-pipeline -p brp-room`. Expected FAIL: compile errors on the new signatures.

- [ ] **Step 2: Implement.**
  - Add the hook in `encode_loop`; frames become `for mut frame in assembler.push(chunk)`.
  - `VoiceSend` gains `metering: Arc<AtomicBool>`. It builds its `make_hook` from `VoiceProcessor::new(processing, &reference)`.
  - The hook processes `frame.samples`, then stores the frame's peak in `level` only while `metering` is set.
  - `open_mic` passes `self.publisher.sink()` straight to the capture, with no wrapper meter, and sets `metering`. `close_mic` clears `metering` before it zeroes `level`.
  - `SPEAKING_LEVEL` (`crates/app/src/ui/voice.rs`, 0.05, about −26 dBFS) stays as it is. Processed speech peaks near −7 dBFS and suppressed noise sits far below it (spec section 9).
  - Update every caller listed above.

- [ ] **Step 3: Run** `cargo test -p brp-pipeline -p brp-room -p brp`. Expected: PASS, including `crates/room/tests/voice.rs`.

- [ ] **Step 4: Commit.** `git add crates/pipeline crates/room crates/app Cargo.lock && git commit -m "feat(room): process the microphone before voice is encoded"`

---

### Task 5: Settings

**Files:**
- Modify: `crates/app/src/settings.rs`, `crates/app/src/ui/settings.rs`, `crates/app/src/room_view.rs`

**Interfaces:**
- Consumes: `ProcessingConfig`, `Suppression` (Task 2); `RoomView::join_voice(.., processing, ..)` (Task 4).
- Produces:
  - **Three new `AudioSettings` fields:** `echo_cancellation: bool`, `noise_suppression: NoiseSuppression` and `automatic_gain: bool`. They are serialised in `[audio]` as `echo_cancellation`, `noise_suppression = "off" | "light"` and `automatic_gain`.
  - **The enum:** `#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all = "lowercase")] pub enum NoiseSuppression { Off, #[default] Light }`.
  - **`Default`:** `AudioSettings` gets a hand-written `impl Default` with the defaults from Global Constraints.
  - **`pub fn voice_processing(&self) -> ProcessingConfig` on `AudioSettings`.** This follows the existing `to_selection` and `to_relay_setting` pattern.

- [ ] **Step 1: Write the failing tests.**
  - In `settings.rs`:
    - Extend `defaults_and_every_field_round_trip_through_toml` with non-default values (`false`, `Off`, `false`).
    - Add `an_older_settings_file_turns_voice_processing_on`: a TOML file without the three keys loads as `true`, `Light`, `true`.
    - Add `voice_processing_follows_the_three_settings`.
  - In `ui/settings.rs`:
    - Extend `the_rows_come_in_the_specs_order` so the labels read `"Microphone", "Echo cancellation", "Noise suppression", "Automatic gain", "Audio output"`.
    - Change the footnote test to look for `"Voice devices and processing apply the next time you join voice."`.
    - Add `the_suppression_hint_follows_the_choice`: with `Off` the dialog shows `"Your microphone as it is"`, and with `Light` it shows `"Removes steady noise such as fans and hum, at almost no CPU cost"`.
  - Run `cargo test -p brp settings`. Expected: FAIL.

- [ ] **Step 2: Implement the rows** directly after the Microphone row, each separated by `ui.add_space(ROW_GAP)`:
  - **Echo cancellation:** `theme::switch(ui, &mut draft.audio.echo_cancellation, "Cancel what your speakers play")`. Below it, a small secondary-coloured hint, styled like the Updates row's: `"Not needed with headphones, or when your system already cancels echo"`.
  - **Noise suppression:** `theme::segmented(ui, "settings-noise-suppression", &SUPPRESSION_CHOICES, selected, ui.available_width() / 3.0)`, where `SUPPRESSION_CHOICES` is `[(Off, "Off"), (Light, "Light")]`. The hint for the current choice goes below it, using the exact copy above.
  - **Automatic gain:** `theme::switch(ui, &mut draft.audio.automatic_gain, "Even out your volume")`.
  - **Footnote:** `"Changes apply to the next room you open. Voice devices and processing apply the next time you join voice."`
  - **Join:** `RoomCommand::JoinVoice` passes `audio.voice_processing()`.

- [ ] **Step 3: Run** `cargo test -p brp`. Expected: PASS.

- [ ] **Step 4: Check the dialog by eye.** Run `cargo run -p brp --example ui_preview` and open the settings render it writes (the example's doc comment names the output path). Expected: the three rows sit under Microphone, and the hint text does not overlap anything. Do not launch `brp` itself: it rotates `~/.config/brp/brp.log` and opens a real window.

- [ ] **Step 5: Commit.** `git add crates/app && git commit -m "feat(app): switch echo cancellation, noise suppression and gain in Settings"`

---

### Task 6: Strong (DeepFilterNet3)

This task comes last on purpose. Tasks 1–5 stand on their own.

**Files:**
- Modify: `Cargo.toml` (the workspace dependencies in Global Constraints), `crates/voiceproc/Cargo.toml`, `crates/voiceproc/src/config.rs`, `crates/voiceproc/src/processor.rs`, `crates/app/src/settings.rs`, `crates/app/src/ui/settings.rs`

**Interfaces:**
- Produces:
  - `Suppression::Strong`;
  - `NoiseSuppression::Strong`, serialised as `"strong"`;
  - a third `SUPPRESSION_CHOICES` entry, `(Strong, "Strong")`, with the hint `"Also removes keyboards and sudden noise. Uses more CPU and delays your voice by 30 ms"`.

- [ ] **Step 1: Write the failing tests.** Run them with `cargo test -p brp-voiceproc strong` and `cargo test -p brp settings`; expected FAIL.

```rust
// processor.rs
#[test] fn strong_removes_keyboard_clicks()
    // {echo: false, Strong, gain: false}; 3 s of test_signals::clicks(seed) at -14 dBFS peak
    // (press: 8 ms of differenced LCG noise decaying with tau 1.2 ms; release 70–110 ms later at half amplitude;
    //  next key 60–260 ms later — the benchmark's generator)
    // → rms_db(last 2 s out) <= rms_db(last 2 s in) - 10.0   (measured 16 dB with speech over it)
#[test] fn strong_delays_by_its_lookahead_only()
    // a single impulse in → the output's largest sample lands 30 ms (1440 samples) ± 480 later
// settings.rs: extend the round-trip test with Strong; ui/settings.rs: the Strong hint appears when selected
```

- [ ] **Step 2: Implement.**
  - `Suppression::Strong` builds `df::tract::DfTract::new(DfParams::default(), &RuntimeParams::default_with_ch(1))`. If that fails, the stage is skipped through the same path as `run_stage`.
  - Each hop goes through `process(ArrayView2::from_shape((1, 480), ..), ArrayViewMut2::from_shape((1, 480), ..))`.
  - The model loads inside `VoiceProcessor::new`, which runs on the encode thread (Task 4). It took about 175 ms on the requester's desktop.

- [ ] **Step 3: Run** `cargo test -p brp-voiceproc`. Expected: PASS within 2 minutes. If the Strong tests alone take longer, add `opt-level = 3` for `tract-linalg` and `tract-core` under `[profile.dev.package.<name>]` in the workspace `Cargo.toml`, then re-run.

- [ ] **Step 4: Run the full gate.** `cargo build --release -p brp` and the whole workspace's tests and clippy. Expected: PASS. Record the release binary's size against `HEAD~` (the spec estimates about +25 MB).

- [ ] **Step 5: Commit.** `git add Cargo.toml Cargo.lock crates/voiceproc crates/app && git commit -m "feat(voiceproc): offer DeepFilterNet3 as Strong noise suppression"`

- [ ] **Step 6: Stop and ask the requester.** The Windows and macOS CI jobs run only on pull requests and on pushes to `main`, and a push to `main` releases. Ask whether to open a pull request for CI. Strong is not done until both jobs build it.
  - If either fails on DeepFilterNet or tract, do not work around it per OS. Report it; the spec says the work stops there.
  - The manual Linux checks in the spec's section 12 are the requester's to run.

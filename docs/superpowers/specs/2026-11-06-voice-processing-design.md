# Voice processing: echo cancellation, noise suppression, automatic gain

Status: design, 2026-11-06. Closes #30 and #16. Builds on the voice design (`2026-11-05-voice-design.md`), whose first version sends the microphone unprocessed.

## 1. Intent

People in voice sound quiet, and the requester's group compares brp with Discord, where the same microphones sound normal. The difference is processing: Discord raises quiet microphones, removes background noise and cancels speaker echo; brp sends the raw signal. Measured on the requester's machine, their own microphone (a Razer USB sound card at 100% system volume) peaks between about −23 and −11 dBFS while they talk, and nothing between the microphone and the listener's speakers changes the level.

Success is a session where nobody asks anyone to speak up or move closer to their microphone, typing does not reach the room, and someone on speakers does not send the room back to itself.

## 2. Goals

- **Automatic gain.** Every voice arrives at about the same loudness without anyone touching a slider.
- **Noise suppression,** with a choice between options that trade how much noise they remove, keyboard noise in particular, against CPU and delay.
- **Echo cancellation.** Whatever brp plays through your speakers does not return to the room through your microphone.
- Each of the three can be turned off by the person whose microphone it processes.
- Linux, Windows and macOS, with the same processing on all three.

## 3. Non-goals

- **Sending only while you speak** (a voice-activity gate) and push-to-talk. The requester chose an open microphone that is cleaned up; muting stays the only way to stop transmitting.
- **Cancelling sound that brp does not play:** games, browsers, music, and the application audio you share yourself. Each needs an OS loopback feed as the reference. brp already captures shared application audio, so feeding it in later is possible, but it is not part of this work.
- **OS voice processing** (macOS Voice Processing I/O, Windows driver effects, PipeWire's echo-cancel module). The same in-app processing runs everywhere so it behaves and is tested the same way.
- **Hearing yourself** through a microphone test or loopback.
- **Turning another person up past 100%.** Automatic gain on the sender is the fix for quiet voices; the per-person slider stays a cut.
- **Automatically stepping down from Strong** when a machine is too slow for it.
- **Opus DTX.** It changes the packet rate in silence, which the voice design forbids (a deafened listener must receive encoded silence at the normal rate).
- **The near-silent Windows microphone** seen in the field test: a Windows friend arrived at −68 dBFS at most, about 50 dB below normal speech. Automatic gain cannot rescue that (measured in section 9). The likely cause is that cpal 0.18 opens Windows' default device in the console role, not the communications role that Discord and Teams use. It is a separate bug.

## 4. Anti-goals

The feature fails, however well it otherwise works, if any of these happen:

- Processing mutes or cuts off a person who is talking. A stage that cannot run lets the audio through unprocessed rather than silencing it.
- With all three switched off, what is sent differs from today's unprocessed path.
- The chosen processing changes anything a listener can observe beyond the sound itself: packet rate, packet sizes beyond what Opus naturally varies, or anything the voice design's privacy rules cover.
- Noise suppression removes enough speech that a quiet talker becomes harder to understand than with it off.

## 5. What the person sees

Three new rows in the Settings dialog, directly under **Microphone**, in this order:

| Row | Control | Default |
|---|---|---|
| Echo cancellation | Switch | On |
| Noise suppression | Off / Light / Strong | Light |
| Automatic gain | Switch | On |

- Light and Strong carry a short description of what each is for: Light handles steady noise such as fans and hum at almost no CPU cost; Strong also handles keyboard and other sudden noise, uses more CPU, and delays your voice slightly. The exact wording is left to the builder.
- Echo cancellation's description says it matters when you use speakers and can be turned off when you use headphones or when your system already cancels echo.
- The settings apply the next time you join voice, like the voice devices. The dialog's footnote, "Voice devices apply the next time you join voice", grows to cover them.
- The settings are saved with the other audio settings. A settings file without them reads as the defaults, so people who already use voice get the processing on their next join.
- The speaking indicator and your own level reflect the processed signal, which is what the room hears, so a keyboard that suppression removes no longer lights your indicator.

## 6. The processing

Order, for each 10 ms of microphone audio, mono at 48 kHz:

1. High-pass filter and echo cancellation (WebRTC's AEC3).
2. Noise suppression: none, Light (RNNoise) or Strong (DeepFilterNet3).
3. Automatic gain with its limiter (WebRTC's AGC2, adaptive digital mode).
4. The level meter, then the existing Opus encoder, two 10 ms hops per 20 ms packet.

Echo cancellation comes first because it models a linear path from the speaker to the microphone, which suppression and gain would break. Gain comes last so it neither amplifies noise before suppression nor fights the canceller.

- **Echo reference.** The reference is what brp's two mixers render: voice, and the application audio of streams you watch. Either can reach your speakers and return through your microphone.
  - The canceller receives them as two separate channels, one per mixer, each averaged to mono. AEC3 then keeps a separate echo filter for each one. So when voice plays on headphones and stream audio on speakers, it learns that only the stream echoes, and the processing never needs to know which device is which. Summing the two into one reference fails exactly there (measured in section 9).
  - The reference is taken as the mixers produce it on their device threads, which must never wait for it.
- **Delay.** AEC3's own delay estimator finds the speaker-to-microphone delay. brp does not measure device latency.
- **Threads.** Processing runs on the voice publisher's encode thread, never in the microphone's device callback. Strong costs up to about a millisecond per 10 ms on a fast desktop and several times that on a laptop, which the device callback cannot afford.
- **Added delay** before encoding: about 5 ms for the filter and the canceller, plus 10 ms for Light or 30 ms for Strong.
- **High-pass filter.** It runs whenever any of the three is on. With all three off the path is today's.

## 7. Libraries

These were chosen by the designer after research the requester asked for. The measurements are in section 9.

| Part | Library | Why |
|---|---|---|
| High-pass filter, echo cancellation, automatic gain | `sonora` 0.2 (crates.io, BSD-3-Clause) | A pure-Rust port of WebRTC's audio processing module (M145 plus later fixes) that passes the module's 2,400-test C++ suite. It is actively maintained and builds for Linux, Windows and macOS with no native code. The C++ binding (`webrtc-audio-processing`) would add a meson/C++ build to all three platforms. |
| Light | `nnnoiseless` 0.5 (crates.io, BSD-3-Clause), RNNoise in pure Rust with the model compiled in | It matches WebRTC's own suppressor on steady noise and removes three times as much keyboard noise at the same CPU, so WebRTC's suppressor is not offered. |
| Strong | DeepFilterNet3 through `deep_filter` (libDF) from github.com/Rikorose/DeepFilterNet, pinned to commit `7b14b895fe7fc42cab72b4a4c1e49c982bd6404b`, features `tract` and `default-model` only | It is the best keyboard and sudden-noise suppressor that runs at full 48 kHz without a native inference runtime. Code and weights are MIT/Apache-2.0 (the README states this for the weights as of that commit). |

DeepFilterNet comes with conditions the builder must keep:

- **No crates.io release.** The crates.io release of `deep_filter` is 0.2.5 from 2022 and has no streaming model, so it is a git dependency on the pinned commit.
- **tract must stay at exactly 0.21.4.** Upstream has not released since 2023, and its code no longer compiles against the newest tract 0.21 patch releases. It builds with `tract-core`, `tract-onnx`, `tract-pulse` and `tract-hir` pinned to exactly 0.21.4, the version in its own lockfile. The pins need a comment saying why, because removing them breaks the build.
- **Size.** The model is embedded. Strong adds about 25 MB to the binary and about 13 MB to a compressed release, on top of today's 60 to 67 MB.
- **Untested on Windows and macOS.** tract assembles its own kernels, so local cross-checks for Windows and macOS fail for lack of Microsoft's and Apple's tools. The Windows and macOS CI jobs will be the first builds there.
- **No anonymous repackaging.** `deep-filter-rt` on crates.io repackages the same model with a current tract, but it is days old, has no source repository and has one unknown publisher. It is not used.

Strong also needs `ndarray` 0.15 directly for libDF's API. It is already in the tree through libDF. No other dependency is added. Only Strong pulls in an inference engine, and it is pure Rust apart from tract's assembly kernels.

DeepFilterNet runs at its library defaults: no attenuation limit, no post-filter. The table in section 9 measures that configuration.

## 8. Constraints from the existing system

- **Microphone path.** `VoiceSend::open_mic` wraps the capture sink with a peak meter and feeds `AudioPublisher`. The publisher queues chunks for its encode thread (`CHUNK_QUEUE`, half a second), cuts 20 ms frames and encodes them. Processing belongs on that thread, between the frame cutting and the encoder. The meter has to move after processing.
- **Stereo throughout.** Capture delivers interleaved stereo at 48 kHz. The voice Opus encoder averages the two channels to mono itself. Processing works on mono, so the downmix moves ahead of it.
- **Two mixers, two output streams.** Room-level application audio has its own mixer and output stream, started in `room.rs`. Voice has another, started on joining. Either may open a different device when "Same as audio output" is off. Both `render` functions run on device threads, take nothing but try-locks, and must stay that way.
- **Settings.** Voice devices live in `Settings.audio`. The dialog is `crates/app/src/ui/settings.rs`, and its tests check row order and the footnote. An older settings file must keep loading.
- **Privacy rules from the voice design.** Encoded silence goes to a deafened listener at the normal packet rate, and nothing at the default log level may differ for a muted or deafened listener. Processing happens before the encoder and before the deafen decision, so it must not change either.
- **Hardware verification.** Only Linux is available to test on. Windows and macOS runtime checks are listed as deferred, as in earlier phases.
- **Temporary diagnostics.** The requester's working tree has uncommitted "TEMP diagnostic" level logging in `crates/pipeline/src/mixer.rs` and `crates/audio/src/cpal_input.rs`, from the investigation that led here. It must not ship. The decoded-level line logs at info level and would read differently for someone who has deafened you, which breaks the voice design's privacy rule. Build from `HEAD`, not from that tree.
- **Releases.** A push to `main` with a commit that is not a `fix:` releases automatically.

## 9. Measurements behind the choices

These come from a throwaway benchmark on the requester's machine (Ryzen 9 9950X3D), outside the repository. It used 58 s of synthesised speech (espeak-ng and flite) with low-passed fan noise and synthesised keyboard clicks. Treat the dB figures as a ranking, not a promise about real rooms.

| Option | Fan noise removed in pauses | Keyboard noise removed in pauses | Speech level change | Time per 10 ms (mean / 99th percentile) | Added delay | Binary size |
|---|---|---|---|---|---|---|
| WebRTC suppressor, very high | 18.1 dB | 2.7 dB | −1.0 dB | 29 µs / 39 µs | ~5 ms | part of `sonora` (+0.7 MB) |
| RNNoise (Light) | 18.1 dB | 8.6 dB | −0.1 dB | 26 µs / 32 µs | 10 ms | +1.2 MB |
| DeepFilterNet3 (Strong) | 25.6 dB | 16.1 dB | −1.3 to −1.6 dB | 202 µs / 349 µs | 30 ms | +24.8 MB (+13.2 MB gzip) |

- **Echo cancellation:**
  - The speech was played through a simulated room (a sparse impulse response) at 40 ms and 150 ms of delay.
  - It removed 53–54 dB of echo after 5 s, and 26–28 dB later in the run.
  - The local voice dropped 2 dB when talking alone and 5–6 dB when both sides talked at once.
  - It costs 72 µs per 10 ms.
  - The test had one clock and a linear echo path, unlike separate real devices.
- **Voice on headphones, stream audio on speakers.** Only the stream echoes, at 60 ms. The run had three phases: stream audio alone, voice alone, then both. Each figure is the microphone level going in and the canceller's output coming out:

  | Reference | Stream audio alone | Voice alone (no echo to remove) | Both at once |
  |---|---|---|---|
  | Both mixers summed into one channel | −32.5 → −78.2 dBFS | −75.2 → −83.1 dBFS | −31.1 → −48.4 dBFS |
  | One channel per mixer | −32.5 → −75.5 dBFS | −75.2 → −82.9 dBFS | −31.1 → −82.8 dBFS |

  Neither version invented sound while only the headphone voice played. The summed one removed only 17 dB of echo whenever both played. Separate channels removed about 51 dB.
- **Automatic gain** (adaptive digital, `sonora` defaults; speech averaged over the last 20 s):

  | Input | Input speech average | Output speech average | Output peak |
  |---|---|---|---|
  | Peaking at −15 dBFS | −30.8 dBFS | −21.2 dBFS | −6.6 dBFS |
  | Peaking at −30 dBFS | −45.7 dBFS | −28.8 dBFS | −14.9 dBFS |
  | Peaking at −1 dBFS | −16.8 dBFS | −17.6 dBFS | −3.7 dBFS |
  | Peaking at −68 dBFS, over 16-bit noise | −82.7 dBFS | −49.2 dBFS | −35.2 dBFS |

  The −68 dBFS case got only 34 dB because the gain stops raising noise past about −50 dBFS. That is why the Windows bug in section 3 is separate.
- **Laptops** will be several times slower than this desktop. Published figures put DeepFilterNet3 at about 1.7 ms per 10 ms on a laptop CPU, which still fits the 20 ms encode budget.

## 10. Calls made without the requester

| Call | Why |
|---|---|
| Defaults: echo cancellation on, Light, automatic gain on, for new and existing settings files | It fixes the reported quiet voices without anyone visiting Settings, and Light adds 10 ms and almost no CPU. Strong's 30 ms and CPU cost should be a choice. |
| Echo cancellation and automatic gain are on/off with no choice of provider | The other echo cancellers in the research (DTLN-aec, NKF-AEC) process 16 kHz audio and need a neural-network runtime. There is no second gain controller worth shipping. A choice only earns its place for noise suppression, where options differ on the noises the requester named. |
| Two suppression options, not more | GTCRN and DTLN process 16 kHz audio, so voices lose everything above 8 kHz. DPDFNet needs ONNX Runtime, a large native library on every platform. WebRTC's suppressor is dominated by RNNoise. |
| The echo reference covers both mixers, voice and stream audio, as two separate channels | Both play through brp, and stream audio on speakers would otherwise return to the room through your microphone. Separate channels work whichever device each plays on, without the processing knowing about devices (section 9). |
| Strong lands last, and the rest does not wait for it. If Strong cannot build or run on Windows or macOS, the work stops there and returns to the requester rather than shipping Strong on some systems only. | Automatic gain is the fix for the reported problem. Strong is the only part with an unproven Windows and macOS build. |
| Settings apply on the next voice join, not live | It matches how voice devices already apply and avoids rebuilding the processing mid-stream. The cost is that switching between Light and Strong to compare means leaving and rejoining voice. |
| The same processing on every OS, no Voice Processing I/O on macOS | One behaviour to reason about and test, and macOS cannot be tested here. |
| A stage that fails, whether it fails to start, returns an error or panics while running, is dropped for the rest of the voice session and logged. Its input passes through and voice continues. | It keeps the anti-goal that processing never mutes anyone. A panic on the encode thread would otherwise stop your voice for the session. A crash inside tract's assembly kernels cannot be caught and remains a risk. |
| The meter and speaking indicator read the processed signal | They should show what the room hears. |

## 11. Left to the builder

- Where the processing lives: a new crate or an existing one. Keeping tract out of crates whose tests do not need it keeps their build time down.
- How rendered audio reaches the encode thread for the echo reference, provided the device threads never block.
- The settings field names and serialised forms, the dialog's control for the three-way choice, and its wording.
- `sonora` configuration details beyond those stated here, such as the internal processing rate (48 kHz keeps the full band) and AEC3's transparent mode.
- Whether `sonora` runs as one instance around the external suppressor or as two.
- Re-checking the speaking threshold (`SPEAKING_LEVEL`, 0.05) against processed audio.
- Where Strong's model is loaded, provided joining voice does not stall on it. Its start-up time was not measured.
- The speech fixture for the automatic-gain tests.

## 12. Verification

- **Unit tests,** needing no hardware:
  - With all three off, the output is sample-for-sample what today's path sends.
  - Each stage, switched on alone, changes the output; switched off, it is absent.
  - Echo cancellation removes a synthetic echo of a known reference by a large margin. The benchmark's sparse-impulse-response setup is a model for this.
  - It also does so when stream audio echoes and voice does not, while both play.
  - A stage that panics is dropped and its input passes through.
  - Automatic gain raises quiet speech and leaves loud speech within a few dB. AGC2 detects speech with a neural network that does not trigger on tones, so this needs a short speech fixture rather than a synthetic tone.
  - Settings round-trip, and an older settings file reads as the defaults.
  - The dialog's row order and footnote, extending the existing tests.
  - A deafened listener still receives encoded silence at the normal packet rate with processing on.
- **CI:** the Windows and macOS jobs are the first real builds of Strong there.
- **Manual,** on Linux with two instances:
  - Speaking from speakers with echo cancellation on and off.
  - Typing while talking with Light and with Strong.
  - A quiet microphone with automatic gain on and off.
  - Windows and macOS runtime checks are deferred.

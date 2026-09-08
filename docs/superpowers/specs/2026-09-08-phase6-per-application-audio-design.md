# Phase 6: Per-application audio

Status: approved design, 2026-09-08. Refines [`2026-09-06-phase4-audio-design.md`](2026-09-06-phase4-audio-design.md), which remains the audio spec, which in turn refines `2026-09-04-p2p-screen-sharing-design.md`. Where this document is silent, phase 4 applies. Where the two differ, this document wins for application selection.

## 1. Goals

- A publisher chooses what the room hears: every application except brp itself, which is phase 4's behaviour and stays the default, or only a named set of applications.
- A selection is stored by executable identity, so it survives brp restarting, the application restarting, and the application not running at all. An application selected before it launches becomes audible when it starts, with no further interaction.
- No protocol change. Audio remains one publisher-level Opus stream with one `has_audio` flag, so a viewer needs no new code and an old viewer is unaffected.
- Linux is implemented and verified in plan 1. Windows is designed here and implemented in plan 2, when hardware is available.

## 2. Non-goals

Recorded in the README backlog:

- **A send volume per shared application.** Independent gains need each application's samples separately, which the Linux design cannot give: one capture stream with N links has PipeWire summing them inside our input ports. It would mean one capture stream per application — each with its own adapter, negotiation, node, ports, and linkability deadline — or a fixed 2·N-channel layout de-interleaved by us. On Windows it is nearly free once the capture-side mixer of section 5.4 exists, since each application is already its own client. Because a gain of zero subsumes deselection, building this later revisits the product model, not just the UI.
- **Viewer-side visibility of which applications a publisher is mixing.** Small in code — a presence field, plumbing, one list — but it broadcasts running application names to the room on every presence tick, and it invites per-application volume on the viewer side, which is impossible once the mix is summed and would otherwise mean one Opus stream per application.
- **Microphone capture and voice chat.** A phase of its own: an input path, a second audio stream per publisher so muting the mic does not mute the game, protocol work for it, a second jitter buffer and mixer track, push-to-talk, and Opus in voip mode. Gated on acoustic echo cancellation, which means a new native dependency on both platforms and in the Windows artifact; that decision deserves its own spike first.
- **A self-updating application list in the picker.** Refresh on demand only. Polling the enumeration about once a second while the picker is open is small and self-contained, and can be added later without touching anything built here — but the manual test is what should decide whether the button feels bad.

Also out of scope: per-application video (window capture already covers it), and any change to the carrier rule, the jitter buffer, the viewer-side `pipeline::Mixer`, or the wire format.

## 3. Decisions and rationale

| Decision | Rationale |
|---|---|
| An allow-list, not a deny-list | A deny-list shares every application the user has not thought of, including one launched mid-session. An allow-list is predictable and privacy-safe by construction: audio you did not name is never sent. |
| An explicit mode, not "an empty list means all" | With the implicit rule, unchecking every box shares *everything* — the opposite of what the gesture suggests, and the kind of trap discovered mid-game. Two radio buttons cost one field in settings. |
| Identity is the executable basename, never a pid | Pids do not survive a restart, and one application owns several audio streams and often several processes. A basename is stable across restarts and updates and naturally catches multi-process applications such as browsers. |
| `sources()` on the `AudioCapture` trait, not a free function like `capture::list_sources` | It needs brp's own pid to exclude itself, and putting it on the trait lets the test capture return a canned list, so the picker's union logic and the integration test both run without hardware. |
| Enumeration returns only what is audible now | Unioning that with the persisted selection, so a closed application can still be deselected, is a UI concern. The platform layer stays a thin report of the present. |
| The selection is an argument to `start`, not state on the backend | The backend never learns that a selection can change; the registry swaps sessions. This is the decision below, expressed in the type. |
| A selection change swaps the platform session and keeps the `AudioPublisher` | The fan-out, the audio sequence space, and every viewer's carrier survive. The jitter buffer emits silence for the gap and re-primes when packets resume, which is what it was built to do. Restarting the whole `RunningAudio` would instead route recovery through the capture-*failure* path: presence drops `has_audio`, viewers lose the carrier, then re-acquire — an audible gap plus a resubscribe for one edit. |
| Stop the old session before starting the new one | Starting first to shrink the gap leaves both sessions feeding one publisher for the overlap, summing the desktop into itself at double amplitude. A brief silence is the better artefact. |
| A failed swap has no fallback to the old session | Reviving it would keep sharing applications the user just deselected. Sharing more than was asked for is the failure mode this feature exists to prevent, so it fails loudly through phase 4's capture-failure path. |
| Draft-then-apply in the picker | Every apply is a session swap with an audible gap; applying per checkbox click would gap the room once per click. |
| brp's own executable is excluded in both modes | Exclusion by our own pid alone leaves a second brp instance on the same machine a foreign application node, so its playback is captured — an echo path in exactly the environment the manual test runs in. One clause in the predicate also keeps brp out of the picker. |
| Windows opens one include-mode client per topmost matching ancestor | `new_application_loopback_client` offers only INCLUDE_TARGET_PROCESS_TREE and EXCLUDE_TARGET_PROCESS_TREE; there is no single-process include mode. One client per enumerated pid would capture an ancestor's tree and its descendants separately — the same audio at double amplitude. Walking parents while the basename matches collapses a browser's renderers onto its root, keeps two independent instances distinct, and stops at a differently-named launcher. |
| Windows polls the session enumerator | `IAudioSessionNotification` is too unreliable to hang a feature on. A one-second poll makes a late-launching application audible without being noticed, at the cost of one cheap COM call. |
| Interim Windows refuses `Only` rather than capturing everything | Between plan 1 and plan 2 the Windows backend answers `Unsupported` for both `sources()` and `start(Only(..))`. Silently sharing more than was asked for is worse than a visible failure; `start(All, ..)` is untouched, so Windows keeps working as it does today. |
| Settings keeps the names in both modes | So flipping to "all applications" for a movie and back does not lose the set. This deliberately deviates from `RelayChoice`'s adjacently-tagged shape, whose variants carry genuinely different data; here the set is meaningful in both modes. |
| One spec, two plans | The interfaces are designed against both platforms so they are right, but Linux is implemented and verified against real hardware first. Windows lands as its own plan rather than as unverifiable code shipped alongside verified code. |

## 4. Product model additions

- **Audio scope.** A publisher-level setting beside share audio: *all applications* (except brp) or *only the applications I select*. Persisted.
- **Application identity.** An executable basename, matched case-insensitively on Windows and exactly on Linux. What the user sees is a friendly label; what is stored is the identity.
- **The application picker.** A window listing what is playing now, unioned with what is selected but absent, edited as a draft and applied on Done.

## 5. Architecture

Platform code stays inside `audio`, as phase 4 established. Everything above it is platform-neutral and tested on the Linux runner.

### 5.1 `audio` contracts

```rust
/// The identity a selection is stored under: an executable basename,
/// normalised lowercase on Windows. Never a pid — pids do not survive a
/// restart, and one application may own several audio streams.
pub struct AppKey(String);

/// One application the platform reports as producing audio right now.
pub struct AudioSource { pub key: AppKey, pub label: String }

/// What a capture session carries.
pub enum AudioSelection {
    /// Everything the machine plays except brp itself — phase 4's behaviour.
    All,
    /// Only these. An empty set is silence, which is what the mode says.
    Only(BTreeSet<AppKey>),
}

pub trait AudioCapture: Send + Sync {
    fn sources(&self) -> Result<Vec<AudioSource>, AudioError>;
    fn start(&self, selection: AudioSelection, sink: AudioSink)
        -> Result<Box<dyn AudioCaptureSession>, AudioError>;
}
```

`AudioError` needs no new variant: enumeration failures are `PipeWire`, `Windows`, or `Unsupported`. `AudioCaptureSession` is unchanged.

### 5.2 Linux backend

The capture stream, the links, the linkability deadline, and the core-error handling in `linux/mod.rs` are untouched. What changes is which nodes `graph::Graph` consents to link, and a new enumeration path.

- **Identity resolution.** `Graph` already resolves a node's owner through `client.id` to the Client global's kernel-verified `pipewire.sec.pid`. It also records, per client, `application.process.binary` as the `AppKey` and `application.name` as the label. Phase 4 section 13 found node properties thin at announce time, so where the binary is absent the resolved pid gives `/proc/<pid>/exe` and its basename — not `comm`, which the kernel truncates to fifteen characters.
- **Construction.** `Graph::new(own_process_id, own_binary, selection)`, where `own_binary` comes from `std::env::current_exe()`.
- **The predicate.** `NodeVerdict` gains `NotSelected`, distinct from `Ignored` so logs and tests can tell "not audio" from "not chosen". The own-pid check and the own-binary check precede the predicate, so neither brp's own playback node nor a second brp instance can ever be linked, even if a hand-edited settings file names it. Under `All` a key is not load-bearing and behaviour is identical to today: an unresolvable binary still links, an unresolvable pid stays `Unresolved` and unlinked. Under `Only` a key is load-bearing, so an unresolvable binary means not selected — fail closed, logged once per node, consistent with phase 4's stance that missing a participant's audio beats leaking the wrong audio.
- **Enumeration.** `sources()` connects, registers a registry listener, issues one `core.sync` roundtrip, and quits its loop on the matching `done` — the `pw-dump` idiom rather than the long-lived listeners capture uses. Nodes collapse by key, so three Firefox streams are one row, and brp's own key is never listed. Bounded by `AUDIO_SOURCE_LIST_TIMEOUT`.
- **Unchanged by construction.** A selected application that launches later links on its node's global event. A selection whose applications are all silent leaves the stream idle with no chunks and no error, exactly as a silent desktop does today.

### 5.3 Windows backend (plan 2)

`All` keeps today's single exclude-mode client, untouched. Until plan 2 lands, the Windows backend answers `Unsupported` to both `sources()` and `start(Only(..))`, and `start(All, ..)` behaves exactly as it does today.

- **Enumeration.** The default render device's `IAudioSessionManager2` session enumerator; for each session, `get_process_id`, then the executable path through `OpenProcess` and `QueryFullProcessImageNameW`, and a label from `get_display_name` falling back to the basename. Sessions in `Active` and `Inactive` are listed and `Expired` ones are not, so an application that has just gone quiet does not vanish mid-decision. brp's own pid and basename are excluded. Rows collapse by key.
- **Client roots.** For each selected key, the candidate pids are walked up their parent chain while the parent's basename still matches; the topmost such ancestor is the root, roots are deduplicated, and one include-tree client is opened per root. The walk is a pure function over an injected `pid -> (parent, basename)` map; only the `CreateToolhelp32Snapshot` pass that fills the map is Windows code.
- **Clients.** One thread per client, each initialised and drained exactly as phase 4 does, including the `AUDCLNT_BUFFERFLAGS_SILENT` zeroing and the whole-stereo-frame alignment its section 13 amended in.
- **Reconciliation.** A `brp-audio-wasapi-watch` thread re-enumerates every `AUDIO_SESSION_POLL_INTERVAL`, opens clients for newly matching roots, and joins those whose process has exited or whose session has expired.
- **Ready.** With zero matching processes `start` succeeds with no clients and emits nothing, mirroring Linux's idle stream. *Ready* therefore means the first reconciliation pass finished, not that a client is streaming.
- **Per-pid failures.** An activation that fails because the process exited between enumeration and activation is logged once and skipped, mirroring phase 4's stance that per-object errors are routine.

### 5.4 Capture-side mixer (plan 2)

`audio::mix`, platform-neutral and compiled on both targets so the Linux runner tests it. Per-source ring buffers feeding one chunk stream: a source starved for a quantum contributes silence and counts an underrun, and a source lagging past `CAPTURE_MIX_MAX_LAG` is trimmed — `pipeline::Mixer`'s discipline verbatim. Every process-loopback client rides the same audio-engine clock, so this absorbs thread scheduling rather than rate drift. An emitted chunk carries the earliest contributing sample's `capture_ts_us`; audio's jitter buffer keys on sequence, so that field is informational and wants no precision engineering.

### 5.5 `room` registry

- `AudioState` gains `selection: AudioSelection`, and `start_audio` passes it to the backend.
- `set_audio_applications(selection)` stores it and clears `last_error`, so editing a selection also retries a capture that previously failed. With nothing running that is all it does, and the next `subscribe_audio` starts with the new value.
- With capture running it swaps the session: serialised by the existing `audio_start` mutex, performed with the `inner` lock released as phase 4 section 13 requires, stopping the old session before starting the new one, drawing a fresh sink from the same `AudioPublisher` handle, and re-checking `advertises()` before reinstalling — share audio may have been switched off while a slow daemon was answering. A new session that fails to start goes through the existing capture-failure path.
- `sources()` delegates to the backend and takes no registry lock.
- `OwnAudioView` gains `selection`, which the picker uses to pre-check rows and the panel uses for the one state that would otherwise mislead: mode `Only` with an empty set reads "no applications selected", not "Capturing".

### 5.6 `room`

`Room::set_audio_applications(AudioSelection)` and `Room::audio_sources() -> Result<Vec<AudioSource>, RoomError>`, with one new transparent variant `RoomError::Audio(#[from] AudioError)` beside `Capture` and `Codec`. `RoomConfig` gains `audio_applications: AudioSelection`, so a persisted `Only` is in force from the first capture rather than from the first time the picker is opened.

### 5.7 `app`

- **Settings.** `AudioSettings` gains a plain struct holding `mode` (`"all"` or `"only"`) and `names`, always both, converted to an `AudioSelection` through `AppKey` so a hand-edited file with odd casing still matches on Windows. `brp-audio` gains no serde dependency; the conversion mirrors `RelayChoice::to_relay_setting`.

  ```toml
  [audio]
  output_device = "USB Audio"

  [audio.applications]
  mode = "only"
  names = ["firefox", "game.exe"]
  ```

- **Commands.** `RoomCommand::SetAudioApplications(AudioSelection)` and `RoomCommand::ChooseApplications`. The latter mirrors `Share { source: None }` verbatim: `room_view` calls `audio_sources()`, opens the picker on `Ok`, and sets the status line on `Err` — which is how the interim Windows `Unsupported` message reaches the user with no capability flag anywhere in the tree.
- **Picker.** A new `ui/applications.rs` beside `picker.rs`; the two shapes differ enough that merging them would tangle both. `UiState` gains `applications: Option<ApplicationPicker>` holding the reported list plus the draft mode and draft set. Done applies and persists, Cancel discards. Refresh re-issues the enumeration and merges the result by replacing the reported list while preserving the draft, since a refresh mid-edit must not discard the edit. The window registers with the popup-open state added in commit `9ee7d1d`, so Esc dismisses the picker and cannot leak through to the fullscreen handler.
- **Persistence.** Done updates the settings and calls phase 5's `save_unless_load_failed`, so a corrupt settings file stays untouched. A save failure is a status line, not a blocked selection: the selection still applies for the session.

### 5.8 Build and CI

Plan 1 adds no dependency. Plan 2 adds two `windows-sys` features — `Win32_System_Threading` for `OpenProcess` and `QueryFullProcessImageNameW`, and `Win32_System_Diagnostics_ToolHelp` for the process snapshot — and no new crate: `wasapi` 0.24 already exposes session enumeration, `get_process_id`, `get_display_name`, and `get_state`. Both runners are unchanged.

The one open dependency question, to be resolved in plan 1's first task as phase 4 resolved libopus: whether PipeWire clients on the dev machine carry `application.process.binary`. If they do not, the `/proc/<pid>/exe` fallback of section 5.2 becomes the primary route rather than the fallback.

## 6. Protocol

Unchanged. Audio stays one publisher-level Opus stream with one `has_audio` flag per live; nothing on the wire describes which applications are in the mix, and no viewer-side code changes. A publisher whose `Only` set is empty simply sends nothing, which is indistinguishable from a silent desktop and already handled.

## 7. Data flow

**Publish.** Unchanged from phase 4 except at the graph: with `All`, every foreign application node is linked as before; with `Only`, only nodes whose resolved basename is in the set. On Windows with `Only`, one include-mode client per root feeds the capture-side mixer, whose single chunk stream reaches the publisher's channel exactly as one backend session does today.

**A selection edit.** Picker Done sends `SetAudioApplications`, the registry stores it, and, if capture is running, stops the platform session and starts a new one with a fresh sink from the same publisher. Chunks stop for the length of one backend start, the publisher's accumulator holds a partial frame across the gap, and the fan-out, the sequence space, and every subscriber's carrier are untouched. Viewers' jitter buffers emit silence for the gap and re-prime when packets resume.

**An application appearing later.** On Linux its node's global event runs the predicate and links it. On Windows the next reconciliation pass opens a client for its root. Neither path involves the registry or the UI.

## 8. User interface

- **Own lives panel.** Beside the existing "Share audio" checkbox, a "Choose applications…" button and a one-line summary: "all applications", "3 applications", or "no applications selected". Enabled while share audio is off. On interim Windows the click produces the `Unsupported` status line.
- **The picker.** Two radio buttons — "All applications (except brp)" and "Only the applications I select". Under "All" the check-list stays visible but disabled, so what was chosen is still visible and a flip back loses nothing. Rows are the union of what is playing and what is selected: playing first, then selected-but-absent marked as not playing, each group alphabetical by label so a Refresh does not reshuffle. A selected-but-absent row is labelled with its stored key, since a closed application has no friendly name to report. An empty platform list reads "nothing is playing audio", mirroring `picker.rs`. Refresh, Done, Cancel.
- Nothing on the viewer side changes.

## 9. Error handling

Phase 4 section 9 continues to apply. Four cases are new.

- **Enumeration fails or times out.** The picker does not open, the message goes to the status line, and a running capture is untouched. Clicking again retries.
- **A swap fails to start.** Phase 4's capture-failure path: the publisher stops, presence drops `has_audio`, subscribers get `NoAudio`, and the panel shows the text. No fallback to the old session, per section 3.
- **Mode `Only` with nothing selected, or a selected application that never runs.** Not errors. The stream idles, viewers hear silence, the panel says "no applications selected". Phase 4's linkability deadline covers our own node and ports, not whether anything got linked, so nothing times out.
- **An identity that cannot be resolved.** Linux fails closed under `Only`, logged once per node. Windows skips the session, logged once.

## 10. Known limitations

- **Elevated and protected processes on Windows.** `OpenProcess` from a medium-integrity brp fails against an elevated or anti-cheat-protected process, so it enumerates without a key and cannot be selected. Running brp elevated is the only workaround, and this design does not add one.
- **A selection is local to the machine.** Executable basenames differ between platforms, so a settings file carried from Linux to Windows may name applications that never match. It fails closed and silently: those keys simply never link.
- **The list is a snapshot.** An application that starts playing while the picker is open appears only after Refresh.

## 11. Testing

**Unit, all on the Linux runner.**

- `Graph` under `Only`: a selected binary links; an unselected one yields `NotSelected`; an unresolvable binary is not selected under `Only` but still links under `All`; brp's own pid and brp's own basename are excluded in both modes; several nodes of one binary collapse to one `AudioSource`; a label prefers `application.name` and falls back to the key.
- `AppKey`: normalisation is case-insensitive on Windows and exact on Linux; a basename is taken from a full path.
- Registry: an edit while idle only stores; an edit while running swaps the session and keeps the same publisher, so the fan-out, the sequence space, and every subscriber survive; a failing swap drops `has_audio` and rejects with `NoAudio`; an edit clears a stale `last_error`; `live_infos` is unaffected by the selection.
- Settings: both modes round trip; names survive a flip to `all`; a name with odd casing normalises; an unknown mode fails the parse, leaving phase 5's corrupt-file discipline in charge.
- Picker state: the union orders playing first then alphabetical; Refresh preserves the draft; Cancel discards; Done emits the selection.
- Plan 2 adds: the ancestry walk over an injected process map — a renderer collapses onto its browser root, two independent instances stay two roots, a differently-named launcher stops the walk; and `audio::mix` — two aligned sources sum, a starved source contributes silence and counts an underrun, a source past `CAPTURE_MIX_MAX_LAG` is trimmed, the emitted timestamp is the earliest contributor's.

**Integration**, extending phase 4's two-room test. `SyntheticTone` gains a canned `sources()` and honours the selection. Selecting one application delivers non-silent samples to the fake output; editing to an empty `Only` set stops the samples without disturbing the watch or the carrier, which is the assertion that proves the swap, since a publisher restart would have moved the carrier.

**Manual on Linux**, the phase's real verification. Two instances on the dev machine:

1. Select one noisy application and confirm the viewer hears only it while a second noisy application stays audible locally.
2. Flip to all applications, confirm both are heard, flip back.
3. Close a selected application; confirm its row persists as not playing and can still be deselected.
4. Launch a selected application after capture has started; confirm it becomes audible with no picker interaction.
5. Confirm the second brp instance is absent from the list and its playback never returns as echo.
6. Confirm a selection survives restarting brp.

**Deferred to Windows hardware (plan 2).** Include-tree activation per root pid, the ancestry walk against a real browser, the poll loop catching a late launch, the capture-side mixer on real clients, and the elevated-process limitation.

## 12. Constants added in this phase

All three live in `proto::constants` beside phase 4's audio constants.

| Constant | Value | Rationale |
|---|---|---|
| `AUDIO_SOURCE_LIST_TIMEOUT` | 1 s | Bounds a picker click. The enumeration runs on the window thread through the command-drain path, so borrowing capture's 5 s would freeze the UI on a sick daemon. |
| `AUDIO_SESSION_POLL_INTERVAL` | 1 s | How often Windows reconciles selected identities against live sessions: fast enough that a launched application becomes audible without being noticed, cheap enough for one COM call. |
| `CAPTURE_MIX_MAX_LAG` | 40 ms | Two frames. Capture-side buffering is pure added latency, and two frames absorb thread scheduling without being audible. |

## 13. Plan split

- **Plan 1, Linux.** The `audio` contracts, the Linux predicate and enumeration, the registry swap, the room and settings plumbing, the picker, every test above the backend, and the manual verification.
- **Plan 2, Windows.** Session enumeration, the ancestry walk, include-mode clients, the reconciliation loop, `audio::mix` with its tests, the two `windows-sys` features, and the deferred hardware verification. Removing the interim `Unsupported` answers is part of this plan.

## 14. References

Verified on 2026-09-08:

- `wasapi` 0.24.0 exposes `AudioSessionManager::get_audiosessionenumerator`, `AudioSessionEnumerator::get_count`/`get_session`, and on `AudioSessionControl` the methods `get_process_id`, `get_display_name`, and `get_state`. `new_application_loopback_client(process_id, include_tree)` maps to `INCLUDE_TARGET_PROCESS_TREE` and `EXCLUDE_TARGET_PROCESS_TREE`; there is no single-process include mode, which is what forces the ancestry walk of section 5.3.
- The video picker enumerates synchronously on the command-drain path, `crates/app/src/room_view.rs:115`, which is the pattern `ChooseApplications` follows and the reason `AUDIO_SOURCE_LIST_TIMEOUT` is a second rather than five.
- Phase 4 section 13 established that a node's owner is resolved through `client.id` to the Client global's `pipewire.sec.pid`, because `application.process.id` is absent from node properties at announce time on PipeWire 1.6 with WirePlumber. Whether the same Client global carries `application.process.binary` is section 5.8's open question.
- `windows-sys` 0.61 is already a workspace dependency; the two features named in section 5.8 are expected to cover `OpenProcess`, `QueryFullProcessImageNameW`, `CreateToolhelp32Snapshot`, and `Process32NextW`, and are confirmed in plan 2's first task.

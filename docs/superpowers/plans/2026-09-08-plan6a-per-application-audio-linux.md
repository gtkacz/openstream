# Plan 6a: Per-application audio, Linux Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A publisher chooses whether the room hears every application except brp, which stays the default, or only a named set of applications, stored by executable basename so the choice survives restarts of brp and of the applications themselves.

**Architecture:** `brp-audio` gains an identity type (`AppKey`), a reported row (`AudioSource`), a choice (`AudioSelection`), and two trait methods: `sources()` to enumerate what is audible now and `start(selection, sink)` to capture a subset. The Linux backend keeps its capture stream, links, and deadlines untouched and changes only which nodes `graph::Graph` consents to link, plus a new `pw-dump`-style enumeration roundtrip. The registry owns the selection, passes it to the backend, and on an edit swaps the platform session while keeping the same `AudioPublisher`, so the fan-out, the audio sequence space, and every viewer's carrier survive. The room plumbs a config field and two methods; the app persists the choice in `settings.toml` and edits it in a draft-then-apply picker window. No protocol change, no viewer change.

**Tech Stack:** Rust 2024, pipewire 0.10.1 / libspa 0.10.1, egui 0.36, toml 1.1, serde, tokio, iroh 1.1.

**Spec:** `docs/superpowers/specs/2026-09-08-phase6-per-application-audio-design.md`, refining `docs/superpowers/specs/2026-09-06-phase4-audio-design.md` (whose section 13 amendments are binding) and `docs/superpowers/specs/2026-09-04-p2p-screen-sharing-design.md`. Read the phase 6 spec and phase 4's sections 5.4, 5.8, and 13.

## Global Constraints

- **This plan is spec section 13's "Plan 1, Linux" only.** Windows is designed in spec section 5.3/5.4 and implemented by a later plan 6b when hardware exists. In this plan the Windows backend answers `AudioError::Unsupported` to `sources()` and to `start(Only(..), ..)`, and `start(All, ..)` behaves exactly as it does today. Never claim a Windows runtime check ran; the Windows CI job is the compile oracle.
- **Only `AUDIO_SOURCE_LIST_TIMEOUT` is added to `proto::constants` in this plan.** Spec section 12 also lists `AUDIO_SESSION_POLL_INTERVAL` and `CAPTURE_MIX_MAX_LAG`; both are Windows-only and belong to plan 6b, so adding them here would leave unused constants behind. Record this split in the spec amendment in Task 9.
- **No protocol change.** Nothing under `crates/proto/src` changes except one constant in `constants.rs`. No message variant, no presence field, no viewer-side change. `has_audio` keeps its phase 4 meaning: a publisher whose `Only` set is empty simply sends nothing.
- **Identity is an executable basename, never a pid.** Matched case-insensitively on Windows and exactly on Linux. brp's own executable is excluded in both modes, by resolved pid *and* by basename, before any selection predicate runs.
- **Fail closed under `Only`.** An audio node whose identity cannot be resolved is not linked when a selection is in force. Missing a participant's audio is safer than sharing audio the user did not name.
- **Platform code stays inside `audio`.** New PipeWire code lives only in `crates/audio/src/linux/` behind the existing `#[cfg(target_os = "linux")]`; `selection.rs` and `chunk.rs` are platform-neutral and compile on both targets.
- **`brp-audio` gains no dependency, and this plan adds no workspace dependency.** In particular `brp-audio` gains no `serde`: the settings conversion lives in the app crate and mirrors `RelayChoice::to_relay_setting`.
- Comments explain why. Doc comments state contracts on new public items. No task ids, branch names, or ticket numbers in code.
- One Conventional Commit per task, imperative subject. `cargo fmt --all` and `cargo clippy --workspace --all-targets -- -D warnings` pass on Linux before each commit, and `cargo test --workspace` passes at every commit — no task may leave the workspace uncompilable. Before committing run `git status --short`; if `.vscode/` files appear staged, run `git rm --cached -r .vscode` first (recurring environment quirk). The `Claude-Session:` trailer the harness requires is expected; no other trailers, no co-author lines.
- **Verified library facts this plan relies on.** pipewire 0.10.1: `pw::keys::APP_PROCESS_BINARY` (`application.process.binary`), `pw::keys::APP_NAME` (`application.name`), `pw::keys::SEC_PID` (`pipewire.sec.pid`), `pw::keys::CLIENT_ID`, `pw::keys::MEDIA_CLASS`, `pw::keys::NODE_NAME`; `core.sync(seq: i32) -> Result<AsyncSeq, Error>` asks the server to emit `done`; `core.add_listener_local().done(|id: u32, seq: AsyncSeq| ..)` receives it, with `pw::core::PW_ID_CORE` as the core's id and `AsyncSeq::seq() -> i32` the sequence to match; `mainloop.loop_().add_timer(|_expirations: u64| ..)` plus `timer.update_timer(Some(Duration), None).into_result()` arms a one-shot deadline; `registry.add_listener_local().global(..).register()` must be kept alive for the loop's lifetime.
- **The spec's one open dependency question is resolved.** On the dev machine (Fedora 44, PipeWire 1.6, WirePlumber) every Client global carries `application.process.binary` and `application.name`, so `/proc/<pid>/exe` stays a fallback rather than the primary route. Task 1 re-checks this in one command. Two observations from that probe that the code comments must carry: a client connected through `pipewire-pulse` has the *pulse daemon's* `pipewire.sec.pid`, so the `/proc` fallback can name `pipewire-pulse` rather than the application; and a node's own properties frequently lack the binary (Spotify's node had none while its client did), which is why the client global is the source of identity.

## File Structure

```
crates/proto/src/constants.rs               + AUDIO_SOURCE_LIST_TIMEOUT

crates/audio/src/selection.rs               new: AppKey, AudioSource, AudioSelection (platform-neutral, pure)
crates/audio/src/lib.rs                     + pub mod selection and its re-exports
crates/audio/src/chunk.rs                   AudioCapture gains sources(); start() takes a selection
crates/audio/src/synthetic.rs               SyntheticTone: canned sources(), honours the selection
crates/audio/src/linux/graph.rs             Client struct, NodeVerdict::NotSelected, the predicate, sources()
crates/audio/src/linux/mod.rs               own_binary(), client_of(), node_of(), binary_of_pid(), the listing roundtrip
crates/audio/src/windows/mod.rs             interim Unsupported for sources() and for Only

crates/room/src/registry.rs                 AudioState.selection, set_audio_applications() with the session swap, sources()
crates/room/src/snapshot.rs                 OwnAudioView.selection
crates/room/src/room.rs                     RoomConfig.audio_applications, Room::set_audio_applications, Room::audio_sources
crates/room/src/error.rs                    + RoomError::Audio
crates/room/tests/registry.rs               selection unit tests, fakes updated
crates/room/tests/two_rooms.rs              the swap integration test, config updated
crates/room/tests/hub_leaves.rs             config updated

crates/app/src/settings.rs                  AudioApplications, AudioMode, to_selection()
crates/app/src/launch.rs                    Launch.audio_applications, RoomConfig wiring
crates/app/src/publish.rs                   RoomConfig wiring (the headless publisher stays on All)
crates/app/src/commands.rs                  + SetAudioApplications, ChooseApplications
crates/app/src/room_view.rs                 both commands applied; apply() takes the stored choice
crates/app/src/ui/applications.rs           new: ApplicationPicker, rows, summary, the picker window
crates/app/src/ui/state.rs                  UiState.applications and its two methods
crates/app/src/ui/own_lives.rs              the button, the summary, the empty-selection state text
crates/app/src/ui/mod.rs                    + pub mod applications
crates/app/src/window.rs                    draws the picker, applies its outcome, persists the choice

README.md                                   roadmap, usage, backlog, links
```

`crates/audio/src/selection.rs` is deliberately separate from `chunk.rs`: `chunk.rs` is the backend-to-pipeline contract, while the identity and the choice are also the settings vocabulary and the UI vocabulary. `crates/app/src/ui/applications.rs` is separate from `picker.rs` because the two shapes differ enough that merging them would tangle both (spec 5.7).

---

### Task 1: The identity, the reported row, and the choice

**Files:**
- Create: `crates/audio/src/selection.rs`
- Modify: `crates/audio/src/lib.rs`
- Modify: `crates/proto/src/constants.rs`
- Test: `crates/audio/src/selection.rs` (inline `#[cfg(test)] mod tests`, as every other module in this crate does)

**Interfaces:**
- Consumes: nothing.
- Produces: `brp_audio::AppKey::new(&str) -> AppKey`, `AppKey::as_str(&self) -> &str`; `brp_audio::AudioSource { key: AppKey, label: String }`; `brp_audio::AudioSelection::{All, Only(BTreeSet<AppKey>)}` with `Default = All` and `AudioSelection::admits(&self, key: Option<&AppKey>) -> bool`; `brp_proto::constants::AUDIO_SOURCE_LIST_TIMEOUT: Duration`.

- [ ] **Step 1: Confirm the spec's open dependency question on this machine**

Run:

```bash
pw-dump | python3 -c "
import json,sys
clients=[o for o in json.load(sys.stdin) if o.get('type','').endswith('Client')]
props=[c.get('info',{}).get('props',{}) for c in clients]
have=sum(1 for p in props if p.get('application.process.binary'))
print(f'{have}/{len(props)} clients carry application.process.binary')
"
```

Expected: every client, or nearly every client, carries it. If the count is zero, stop and report: the `/proc/<pid>/exe` fallback of Task 2 becomes the primary route and the plan's Task 2 comment must be inverted. Anything else proceeds unchanged.

- [ ] **Step 2: Write the failing tests**

Create `crates/audio/src/selection.rs` with only the test module for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_the_basename_of_whatever_it_is_given() {
        assert_eq!(AppKey::new("firefox").as_str(), "firefox");
        assert_eq!(AppKey::new("/usr/bin/firefox").as_str(), "firefox");
        assert_eq!(AppKey::new("").as_str(), "");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_matches_a_basename_exactly() {
        assert_eq!(AppKey::new("Firefox").as_str(), "Firefox");
        assert_ne!(AppKey::new("Firefox"), AppKey::new("firefox"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_matches_case_insensitively_and_takes_either_separator() {
        assert_eq!(AppKey::new("Game.exe"), AppKey::new("GAME.EXE"));
        assert_eq!(AppKey::new(r"C:\Games\Game.exe"), AppKey::new("game.exe"));
        assert_eq!(AppKey::new("C:/Games/Game.exe"), AppKey::new("game.exe"));
    }

    #[test]
    fn all_is_the_default_and_admits_everything_including_an_unknown_identity() {
        let all = AudioSelection::default();
        assert_eq!(all, AudioSelection::All);
        assert!(all.admits(Some(&AppKey::new("firefox"))));
        assert!(all.admits(None), "under All a key is not load-bearing");
    }

    #[test]
    fn only_admits_its_members_and_never_an_unknown_identity() {
        let only = AudioSelection::Only(BTreeSet::from([AppKey::new("firefox")]));
        assert!(only.admits(Some(&AppKey::new("firefox"))));
        assert!(!only.admits(Some(&AppKey::new("spotify"))));
        assert!(
            !only.admits(None),
            "an unresolved identity fails closed under Only"
        );
    }

    #[test]
    fn an_empty_only_set_is_silence() {
        let nothing = AudioSelection::Only(BTreeSet::new());
        assert!(!nothing.admits(Some(&AppKey::new("firefox"))));
        assert!(!nothing.admits(None));
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p brp-audio selection`
Expected: FAIL to compile, `cannot find type AppKey in this scope` (the module is not declared yet either).

- [ ] **Step 4: Write the types**

Prepend to `crates/audio/src/selection.rs`:

```rust
//! What a capture session carries: the identity a selection is stored under, the applications the
//! platform reports as audible, and the choice itself. Platform-neutral and pure, so the picker's
//! logic and both backends' predicates are testable on one runner.

use std::collections::BTreeSet;

/// The identity a selection is stored under: an executable basename, normalised lowercase on
/// Windows. Never a pid — pids do not survive a restart, and one application often owns several
/// audio streams and several processes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AppKey(String);

impl AppKey {
    /// From a basename or a full path, whichever the platform reported or the settings file holds.
    pub fn new(name: &str) -> Self {
        Self(normalised(name))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Windows accepts either separator and ignores case in file names; Linux does neither, so the
/// same settings file behaves like the platform that reads it.
#[cfg(windows)]
fn normalised(name: &str) -> String {
    name.rsplit(['/', '\\'])
        .next()
        .unwrap_or(name)
        .to_lowercase()
}

#[cfg(not(windows))]
fn normalised(name: &str) -> String {
    name.rsplit('/').next().unwrap_or(name).to_string()
}

/// One application the platform reports as producing audio right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioSource {
    pub key: AppKey,
    /// What the user sees: the application's own name, or its identity when it has none.
    pub label: String,
}

/// What a capture session carries.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum AudioSelection {
    /// Everything the machine plays except brp itself — phase 4's behaviour, and the default.
    #[default]
    All,
    /// Only these. An empty set is silence, which is what the mode says.
    Only(BTreeSet<AppKey>),
}

impl AudioSelection {
    /// Whether audio owned by this identity is captured. A stream whose owner could not be
    /// identified has no key: it is captured under `All`, where the key is not load-bearing, and
    /// left out under `Only`, where sharing an application the user did not name is the failure
    /// this feature exists to prevent.
    pub fn admits(&self, key: Option<&AppKey>) -> bool {
        match self {
            Self::All => true,
            Self::Only(keys) => key.is_some_and(|key| keys.contains(key)),
        }
    }
}
```

Declare the module in `crates/audio/src/lib.rs`, beside the existing declarations and re-exports:

```rust
pub mod selection;
```

```rust
pub use selection::{AppKey, AudioSelection, AudioSource};
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p brp-audio selection`
Expected: PASS, five tests on Linux (`windows_matches_case_insensitively_and_takes_either_separator` is compiled out).

- [ ] **Step 6: Add the constant**

In `crates/proto/src/constants.rs`, after `AUDIO_CAPTURE_START_TIMEOUT`:

```rust
/// Bounds one picker click. The enumeration runs on the window thread through the command-drain
/// path, so borrowing capture's five seconds would freeze the UI on a sick daemon.
pub const AUDIO_SOURCE_LIST_TIMEOUT: Duration = Duration::from_secs(1);
```

- [ ] **Step 7: Verify the workspace and commit**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: PASS.

```bash
git add crates/audio/src/selection.rs crates/audio/src/lib.rs crates/proto/src/constants.rs
git commit -m "feat(audio): add the application identity and the capture selection"
```

---

### Task 2: The Linux link predicate and the source list, in the graph

**Files:**
- Modify: `crates/audio/src/linux/graph.rs`
- Modify: `crates/audio/src/linux/mod.rs`
- Test: `crates/audio/src/linux/graph.rs` (inline test module, extended)

**Interfaces:**
- Consumes: `AppKey`, `AudioSelection`, `AudioSource` from Task 1.
- Produces: `graph::Client { pid: Option<u32>, key: Option<AppKey>, label: Option<String> }` (derives `Debug, Clone, Default, PartialEq, Eq`); `Graph::new(own_process_id: u32, own_binary: AppKey, selection: AudioSelection) -> Graph`; `Graph::add_client(&mut self, id: u32, client: Client)`; `NodeVerdict::NotSelected`; `Graph::sources(&self) -> Vec<AudioSource>`; and in `linux/mod.rs` the private helpers `own_binary() -> AppKey`, `client_of(&DictRef) -> Client`, `node_of(u32, &DictRef) -> Node`, `binary_of_pid(u32) -> Option<AppKey>`.

This task changes behaviour in one way beyond the selection: a foreign node whose identity equals brp's own executable is no longer linked under `All` either. That is spec section 3's deliberate fix for the echo path a second brp instance opens on the machine the manual test runs on.

- [ ] **Step 1: Write the failing tests**

In `crates/audio/src/linux/graph.rs`, replace the existing test module's helpers and add the new tests. The new helper block, at the top of `mod tests`:

```rust
    use super::*;

    const OWN_PID: u32 = 4242;
    const OWN_BINARY: &str = "brp";

    fn graph(selection: AudioSelection) -> Graph {
        Graph::new(OWN_PID, AppKey::new(OWN_BINARY), selection)
    }

    fn only(binaries: [&str; 1]) -> AudioSelection {
        AudioSelection::Only(binaries.iter().map(|b| AppKey::new(b)).collect())
    }

    /// A client that reports a pid and nothing else, as phase 4's tests assumed.
    fn client(pid: u32) -> Client {
        Client {
            pid: Some(pid),
            ..Default::default()
        }
    }

    /// A client that also names its executable and itself.
    fn app(pid: u32, binary: &str, name: &str) -> Client {
        Client {
            pid: Some(pid),
            key: Some(AppKey::new(binary)),
            label: Some(name.into()),
        }
    }
```

Mechanically update the seven existing tests: every `Graph::new(4242)` becomes `graph(AudioSelection::All)`, `Graph::new(1)` becomes `Graph::new(1, AppKey::new(OWN_BINARY), AudioSelection::All)`, and every `graph.add_client(50, Some(p))` becomes `graph.add_client(50, client(p))`. Their assertions do not change: a client with no key is admitted under `All` exactly as before, which is what keeps `a_node_named_like_ours_from_a_foreign_pid_is_linked_as_foreign` green.

Then add:

```rust
    #[test]
    fn only_the_selected_binary_is_linked() {
        let mut graph = graph(only(["firefox"]));
        graph.add_client(50, app(1000, "firefox", "Firefox"));
        graph.add_client(51, app(1001, "spotify", "Spotify"));
        assert_eq!(graph.add_node(node(10, Some(50))), NodeVerdict::Linked);
        assert_eq!(graph.add_node(node(20, Some(51))), NodeVerdict::NotSelected);
        assert_eq!(
            graph.add_port(port(21, 20, "FL")),
            None,
            "an unselected node is not linked"
        );
    }

    #[test]
    fn an_unresolvable_binary_links_under_all_and_is_not_selected_under_only() {
        let mut all = graph(AudioSelection::All);
        all.add_client(50, client(1000));
        assert_eq!(all.add_node(node(10, Some(50))), NodeVerdict::Linked);

        let mut restricted = graph(only(["firefox"]));
        restricted.add_client(50, client(1000));
        assert_eq!(
            restricted.add_node(node(10, Some(50))),
            NodeVerdict::NotSelected,
            "no identity means not selected: fail closed"
        );
    }

    #[test]
    fn our_own_pid_and_our_own_binary_are_excluded_in_both_modes() {
        for selection in [AudioSelection::All, only([OWN_BINARY])] {
            let mut graph = graph(selection);
            graph.add_client(50, app(OWN_PID, OWN_BINARY, "brp"));
            graph.add_client(51, app(9999, OWN_BINARY, "brp"));
            graph.add_client(52, Client {
                pid: None,
                key: Some(AppKey::new(OWN_BINARY)),
                label: None,
            });
            assert_eq!(
                graph.add_node(node(10, Some(50))),
                NodeVerdict::Ignored,
                "our own playback"
            );
            assert_eq!(
                graph.add_node(node(20, Some(51))),
                NodeVerdict::Ignored,
                "a second brp instance is an echo path, not an application"
            );
            assert_eq!(
                graph.add_node(node(30, Some(52))),
                NodeVerdict::Ignored,
                "excluded by name even without a resolvable pid"
            );
            assert!(graph.sources().is_empty(), "brp is never listed");
        }
    }

    #[test]
    fn nodes_of_one_binary_collapse_into_one_source() {
        let mut graph = graph(AudioSelection::All);
        graph.add_client(50, app(1000, "firefox", "Firefox"));
        graph.add_client(51, app(1001, "firefox", "Firefox"));
        graph.add_client(52, app(1002, "spotify", ""));
        graph.add_client(53, client(1003));
        for (id, client) in [(10, 50), (11, 50), (12, 51), (20, 52), (30, 53)] {
            graph.add_node(node(id, Some(client)));
        }
        assert_eq!(
            graph.sources(),
            vec![
                AudioSource {
                    key: AppKey::new("firefox"),
                    label: "Firefox".into(),
                },
                AudioSource {
                    key: AppKey::new("spotify"),
                    label: "spotify".into(),
                },
            ],
            "three Firefox streams are one row, an empty name falls back to the key, and a stream \
             with no identity cannot be listed"
        );
    }

    #[test]
    fn an_unselected_node_is_still_listed() {
        let mut graph = graph(AudioSelection::All);
        graph.add_client(50, app(1000, "spotify", "Spotify"));
        graph.add_node(node(10, Some(50)));
        assert_eq!(graph.sources().len(), 1, "the list reports what is audible");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p brp-audio graph`
Expected: FAIL to compile, `expected 1 argument, found 3` on `Graph::new` and `no variant named NotSelected`.

- [ ] **Step 3: Rework the graph**

In `crates/audio/src/linux/graph.rs`, add the import and the client record:

```rust
use crate::selection::{AppKey, AudioSelection, AudioSource};
```

```rust
/// What a Client global says about the process behind the nodes that name it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Client {
    /// The client's kernel-verified pid (`pipewire.sec.pid`).
    pub pid: Option<u32>,
    /// `application.process.binary`, or the basename of `/proc/<pid>/exe` when that property is
    /// absent. `None` when neither is available, which fails closed under a selection.
    pub key: Option<AppKey>,
    /// `application.name`, the friendly label the picker shows.
    pub label: Option<String>,
}
```

Extend the verdict:

```rust
    /// An application output node whose identity is not in the selection, or has no identity while
    /// a selection is in force. Distinct from `Ignored` so logs and tests can tell "not audio"
    /// from "not chosen".
    NotSelected,
```

Replace the struct's fields and constructor:

```rust
pub struct Graph {
    own_process_id: u32,
    /// brp's own executable identity, excluded in both modes.
    own_binary: AppKey,
    selection: AudioSelection,
    /// `client.id` to what that Client global reported.
    clients: BTreeMap<u32, Client>,
    /// Application output nodes that are not ours and are selected.
    nodes: BTreeMap<u32, Node>,
    /// Every output port seen, by id; ports can arrive before their node.
    ports: BTreeMap<u32, Port>,
}

impl Graph {
    pub fn new(own_process_id: u32, own_binary: AppKey, selection: AudioSelection) -> Self {
        Self {
            own_process_id,
            own_binary,
            selection,
            clients: BTreeMap::new(),
            nodes: BTreeMap::new(),
            ports: BTreeMap::new(),
        }
    }

    /// Records what a Client global reported. A client with no resolvable pid leaves the nodes it
    /// owns `Unresolved` rather than silently matching none.
    pub fn add_client(&mut self, id: u32, client: Client) {
        self.clients.insert(id, client);
    }
```

Replace `add_node`:

```rust
    /// Classifies a node and, if it is a foreign application output this session carries, tracks
    /// it for linking.
    pub fn add_node(&mut self, node: Node) -> NodeVerdict {
        let client = node.client.and_then(|id| self.clients.get(&id));
        let pid = client.and_then(|client| client.pid);
        let key = client.and_then(|client| client.key.clone());
        if node.name.as_deref() == Some(OWN_STREAM_NAME) && pid == Some(self.own_process_id) {
            return NodeVerdict::Own;
        }
        if node.media_class != APP_OUTPUT_CLASS {
            return NodeVerdict::Ignored;
        }
        // Both exclusions run ahead of the predicate, so neither our own playback nor a second brp
        // instance on this machine can be linked even if a hand-edited settings file names brp.
        if pid == Some(self.own_process_id) || key.as_ref() == Some(&self.own_binary) {
            return NodeVerdict::Ignored;
        }
        if pid.is_none() {
            return NodeVerdict::Unresolved;
        }
        if !self.selection.admits(key.as_ref()) {
            return NodeVerdict::NotSelected;
        }
        self.nodes.insert(node.id, node);
        NodeVerdict::Linked
    }

    /// The applications behind the tracked nodes, collapsed by identity: what `sources()` reports.
    /// A node whose owner has no identity cannot be selected, so it cannot be a row either. The
    /// first label seen for an identity wins; nodes are keyed by id, so that is deterministic.
    pub fn sources(&self) -> Vec<AudioSource> {
        let mut labels: BTreeMap<AppKey, String> = BTreeMap::new();
        for node in self.nodes.values() {
            let Some(client) = node.client.and_then(|id| self.clients.get(&id)) else {
                continue;
            };
            let Some(key) = client.key.clone() else {
                continue;
            };
            let label = match client.label.as_deref() {
                Some(label) if !label.is_empty() => label.to_string(),
                _ => key.as_str().to_string(),
            };
            labels.entry(key).or_insert(label);
        }
        labels
            .into_iter()
            .map(|(key, label)| AudioSource { key, label })
            .collect()
    }
```

In `remove`, replace `self.client_pids.remove(&id);` with `self.clients.remove(&id);`.

- [ ] **Step 4: Feed the new constructor from the backend**

In `crates/audio/src/linux/mod.rs`, extend the imports:

```rust
use self::graph::{Client, Graph, Input, LinkPlan, Node, NodeVerdict, OWN_STREAM_NAME, Port};
use crate::selection::{AppKey, AudioSelection};
```

Add the three helpers at the bottom of the file, beside `pw_error`:

```rust
/// brp's own executable identity, excluded from capture in both modes. An unreadable
/// `current_exe` leaves it empty, which matches no reported binary, so exclusion then rests on the
/// pid alone.
fn own_binary() -> AppKey {
    match std::env::current_exe() {
        Ok(path) => AppKey::new(&path.to_string_lossy()),
        Err(error) => {
            tracing::warn!(
                %error,
                "could not read our own executable path; a second brp instance will not be \
                 recognised by name"
            );
            AppKey::new("")
        }
    }
}

/// What a Client global says about its owner. The binary property is the identity: a node's own
/// properties often lack it while its client carries it.
fn client_of(props: &pw::spa::utils::dict::DictRef) -> Client {
    let pid: Option<u32> = props
        .get(*pw::keys::SEC_PID)
        .and_then(|pid| pid.parse().ok());
    Client {
        pid,
        // The fallback is a fallback: every client observed on the dev machine carries the binary.
        // A client that reaches the daemon through pipewire-pulse has the pulse daemon's verified
        // pid, so falling back to it can name `pipewire-pulse` rather than the application.
        key: props
            .get(*pw::keys::APP_PROCESS_BINARY)
            .map(AppKey::new)
            .or_else(|| pid.and_then(binary_of_pid)),
        label: props.get(*pw::keys::APP_NAME).map(str::to_string),
    }
}

/// A node global as the graph wants it.
fn node_of(id: u32, props: &pw::spa::utils::dict::DictRef) -> Node {
    Node {
        id,
        media_class: props.get(*pw::keys::MEDIA_CLASS).unwrap_or("").to_string(),
        name: props.get(*pw::keys::NODE_NAME).map(str::to_string),
        client: props.get(*pw::keys::CLIENT_ID).and_then(|c| c.parse().ok()),
    }
}

/// The basename of a running process's executable, read through `/proc/<pid>/exe` rather than
/// `comm`, which the kernel truncates to fifteen characters.
fn binary_of_pid(pid: u32) -> Option<AppKey> {
    std::fs::read_link(format!("/proc/{pid}/exe"))
        .ok()
        .map(|path| AppKey::new(&path.to_string_lossy()))
}
```

Rewrite the two arms of `State::global` that parse properties, so both paths share the helpers:

```rust
            ObjectType::Client => {
                self.graph.borrow_mut().add_client(global.id, client_of(props));
            }
            ObjectType::Node => {
                let id = global.id;
                let node = node_of(id, props);
                let name = node.name.clone();
                let verdict = self.graph.borrow_mut().add_node(node);
                match verdict {
                    NodeVerdict::Own => {
                        *self.stream_node.borrow_mut() = Some(id);
                    }
                    NodeVerdict::Linked => {
                        let plans = self.graph.borrow().pending_links(id);
                        for plan in plans {
                            self.link(plan);
                        }
                    }
                    NodeVerdict::Unresolved => {
                        tracing::warn!(
                            node = id,
                            name = name.as_deref().unwrap_or(""),
                            "could not resolve the pid owning this audio output node; leaving it unlinked"
                        );
                    }
                    NodeVerdict::NotSelected => {
                        tracing::debug!(
                            node = id,
                            name = name.as_deref().unwrap_or(""),
                            "this application is not in the audio selection; leaving it unlinked"
                        );
                    }
                    NodeVerdict::Ignored => {}
                }
            }
```

In `run`, construct the graph with the new arguments — the selection stays `All` until Task 3 threads the real one through:

```rust
        graph: RefCell::new(Graph::new(process_id, own_binary(), AudioSelection::All)),
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p brp-audio`
Expected: PASS, including the seven reworked phase 4 tests and the five new ones.

- [ ] **Step 6: Verify the workspace and commit**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: PASS.

```bash
git add crates/audio/src/linux
git commit -m "feat(audio): decide links by application identity in the PipeWire graph"
```

---

### Task 3: The capture trait, the Linux enumeration, and the interim Windows answers

**Files:**
- Modify: `crates/audio/src/chunk.rs`
- Modify: `crates/audio/src/synthetic.rs`
- Modify: `crates/audio/src/linux/mod.rs`
- Modify: `crates/audio/src/windows/mod.rs`
- Modify: `crates/room/src/registry.rs` (the one call site; the registry's own selection lands in Task 4)
- Test: `crates/audio/src/synthetic.rs` (inline), `crates/room/tests/registry.rs` (fakes updated to the new trait)

**Interfaces:**
- Consumes: Task 1's `AudioSelection`/`AudioSource`, Task 2's `Graph::new`/`Graph::sources`/`client_of`/`node_of`/`own_binary`.
- Produces: `AudioCapture::sources(&self) -> Result<Vec<AudioSource>, AudioError>` and `AudioCapture::start(&self, selection: AudioSelection, sink: AudioSink) -> Result<Box<dyn AudioCaptureSession>, AudioError>`; `SyntheticTone::tone_key() -> AppKey` and `SyntheticTone::silent_key() -> AppKey`.

- [ ] **Step 1: Write the failing tests**

In `crates/audio/src/synthetic.rs`, update the existing test's `start` call to `.start(AudioSelection::All, Box::new(move |c| sink_chunks.lock().unwrap().push(c)))` and add:

```rust
    #[test]
    fn the_tone_lists_itself_and_a_silent_neighbour() {
        let tone = SyntheticTone {
            frequency_hz: 440.0,
            amplitude: 0.5,
        };
        let listed: Vec<(String, String)> = tone
            .sources()
            .unwrap()
            .into_iter()
            .map(|source| (source.key.as_str().to_string(), source.label))
            .collect();
        assert_eq!(
            listed,
            [
                ("tone".to_string(), "Tone".to_string()),
                ("silent".to_string(), "Silent".to_string()),
            ]
        );
    }

    #[test]
    fn a_selection_without_the_tone_delivers_no_chunks() {
        let chunks = Arc::new(Mutex::new(Vec::new()));
        let sink_chunks = chunks.clone();
        let session = SyntheticTone {
            frequency_hz: 440.0,
            amplitude: 0.5,
        }
        .start(
            AudioSelection::Only(std::collections::BTreeSet::from([
                SyntheticTone::silent_key(),
            ])),
            Box::new(move |c| sink_chunks.lock().unwrap().push(c)),
        )
        .unwrap();
        thread::sleep(Duration::from_millis(120));
        assert!(session.error().is_none(), "an idle selection is not a failure");
        session.stop();
        assert!(chunks.lock().unwrap().is_empty());
    }

    #[test]
    fn a_selection_naming_the_tone_delivers_chunks() {
        let chunks = Arc::new(Mutex::new(Vec::new()));
        let sink_chunks = chunks.clone();
        let session = SyntheticTone {
            frequency_hz: 440.0,
            amplitude: 0.5,
        }
        .start(
            AudioSelection::Only(std::collections::BTreeSet::from([SyntheticTone::tone_key()])),
            Box::new(move |c| sink_chunks.lock().unwrap().push(c)),
        )
        .unwrap();
        thread::sleep(Duration::from_millis(120));
        session.stop();
        assert!(!chunks.lock().unwrap().is_empty());
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p brp-audio synthetic`
Expected: FAIL to compile, `this method takes 1 argument but 2 arguments were supplied` and `no function or associated item named tone_key`.

- [ ] **Step 3: Change the trait**

In `crates/audio/src/chunk.rs`:

```rust
use crate::error::AudioError;
use crate::selection::{AudioSelection, AudioSource};
```

```rust
pub trait AudioCapture: Send + Sync {
    /// The applications the platform reports as producing audio right now, collapsed by identity
    /// and never including brp's own. Only what is audible now: unioning that with a selection
    /// whose applications are closed is the caller's business.
    fn sources(&self) -> Result<Vec<AudioSource>, AudioError>;

    /// Starts capturing what `selection` names. The selection is an argument, not state: a backend
    /// never learns that it can change, because a change swaps the session.
    fn start(
        &self,
        selection: AudioSelection,
        sink: AudioSink,
    ) -> Result<Box<dyn AudioCaptureSession>, AudioError>;
}
```

- [ ] **Step 4: Teach the synthetic tone the selection**

In `crates/audio/src/synthetic.rs`, add the imports (`use crate::selection::{AppKey, AudioSelection, AudioSource};`) and:

```rust
impl SyntheticTone {
    /// The identity the tone answers to: a selection naming it is audible.
    pub fn tone_key() -> AppKey {
        AppKey::new("tone")
    }

    /// A second listed identity that never produces samples, so a test can select something the
    /// platform reports and still hear nothing.
    pub fn silent_key() -> AppKey {
        AppKey::new("silent")
    }
}
```

Replace the `impl AudioCapture for SyntheticTone` header and opening lines:

```rust
impl AudioCapture for SyntheticTone {
    fn sources(&self) -> Result<Vec<AudioSource>, AudioError> {
        Ok(vec![
            AudioSource {
                key: Self::tone_key(),
                label: "Tone".into(),
            },
            AudioSource {
                key: Self::silent_key(),
                label: "Silent".into(),
            },
        ])
    }

    fn start(
        &self,
        selection: AudioSelection,
        mut sink: AudioSink,
    ) -> Result<Box<dyn AudioCaptureSession>, AudioError> {
        let audible = selection.admits(Some(&Self::tone_key()));
        let stop = Arc::new(AtomicBool::new(false));
```

and inside the loop, replace the `sink(AudioChunk { .. })` call with:

```rust
                // A selection that does not name the tone leaves the stream idle with no chunks
                // and no error, exactly as a silent desktop does.
                if audible {
                    sink(AudioChunk {
                        samples,
                        capture_ts_us: monotonic_us(),
                    });
                }
```

- [ ] **Step 5: Run the tone's tests to verify they pass**

Run: `cargo test -p brp-audio synthetic`
Expected: FAIL to compile — the two platform backends do not implement the new trait yet. That is the next step; the tone's own code is done.

- [ ] **Step 6: Thread the selection through the Linux backend and add the enumeration**

In `crates/audio/src/linux/mod.rs`, extend the two imports that need it — `Cell`, `RefCell`, `Rc`, and `RecvTimeoutError` are already there:

```rust
use brp_proto::constants::{
    AUDIO_CAPTURE_START_TIMEOUT, AUDIO_CHANNELS, AUDIO_SAMPLE_RATE, AUDIO_SOURCE_LIST_TIMEOUT,
};
use crate::selection::{AppKey, AudioSelection, AudioSource};
```

Replace the `impl AudioCapture for PipeWireCapture` block:

```rust
impl AudioCapture for PipeWireCapture {
    fn sources(&self) -> Result<Vec<AudioSource>, AudioError> {
        let (tx, rx) = mpsc::channel::<Result<Vec<AudioSource>, AudioError>>();
        let process_id = self.process_id;
        let thread = thread::Builder::new()
            .name("brp-audio-pw-list".into())
            .spawn(move || {
                let _ = tx.send(list(process_id));
            })
            .map_err(|e| {
                AudioError::PipeWire(format!("failed to spawn the PipeWire thread: {e}"))
            })?;
        // This runs on the window thread through the command-drain path, so it is bounded far more
        // tightly than a capture start. The loop arms the same deadline, so a thread still wedged
        // past it ends on its own rather than being joined here.
        match rx.recv_timeout(AUDIO_SOURCE_LIST_TIMEOUT) {
            Ok(result) => {
                let _ = thread.join();
                result
            }
            Err(RecvTimeoutError::Timeout) => Err(AudioError::PipeWire(format!(
                "the application list did not arrive within {AUDIO_SOURCE_LIST_TIMEOUT:?}"
            ))),
            Err(RecvTimeoutError::Disconnected) => Err(AudioError::PipeWire(
                "PipeWire thread exited before listing".into(),
            )),
        }
    }

    fn start(
        &self,
        selection: AudioSelection,
        sink: AudioSink,
    ) -> Result<Box<dyn AudioCaptureSession>, AudioError> {
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), AudioError>>();
        let (quit_tx, quit_rx) = pw::channel::channel();
        let error = Arc::new(Mutex::new(None));
        let error_slot = error.clone();
        let process_id = self.process_id;
        let thread = thread::Builder::new()
            .name("brp-audio-pw".into())
            .spawn(move || {
                if let Err(e) = run(process_id, selection, sink, quit_rx, ready_tx.clone()) {
                    let message = e.to_string();
                    let _ = ready_tx.send(Err(e));
                    *error_slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(message);
                }
            })
            .map_err(|e| {
                AudioError::PipeWire(format!("failed to spawn the PipeWire thread: {e}"))
            })?;
```

The rest of `start` — the `ready_rx.recv_timeout` match and the `Ok(Box::new(Session { .. }))` — is unchanged.

Change `run`'s signature and its graph construction:

```rust
fn run(
    process_id: u32,
    selection: AudioSelection,
    sink: AudioSink,
    quit: pw::channel::Receiver<()>,
    ready: mpsc::Sender<Result<(), AudioError>>,
) -> Result<(), AudioError> {
```

```rust
        graph: RefCell::new(Graph::new(process_id, own_binary(), selection)),
```

Add the listing roundtrip after `run`:

```rust
/// One `pw-dump`-style roundtrip: connect, listen, ask for a `done`, and quit the loop when it
/// arrives. Capture's long-lived listeners are the wrong shape for a question that has an answer.
fn list(process_id: u32) -> Result<Vec<AudioSource>, AudioError> {
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None).map_err(pw_error)?;
    let context = pw::context::ContextRc::new(&mainloop, None).map_err(pw_error)?;
    let core = context.connect_rc(None).map_err(pw_error)?;
    let registry = core.get_registry().map_err(pw_error)?;

    let seen_clients = Rc::new(RefCell::new(Vec::<(u32, Client)>::new()));
    let seen_nodes = Rc::new(RefCell::new(Vec::<Node>::new()));
    let _registry_listener = {
        let clients = seen_clients.clone();
        let nodes = seen_nodes.clone();
        registry
            .add_listener_local()
            .global(move |global| {
                let Some(props) = global.props else { return };
                match global.type_ {
                    ObjectType::Client => clients.borrow_mut().push((global.id, client_of(props))),
                    ObjectType::Node => nodes.borrow_mut().push(node_of(global.id, props)),
                    _ => {}
                }
            })
            .register()
    };

    // Issued after the registry bind, so every global the server already had arrives before the
    // matching `done`: methods are handled in order and events are delivered in order.
    let asked = core.sync(0).map_err(pw_error)?.seq();
    let done = Rc::new(Cell::new(false));
    let core_error = Rc::new(RefCell::new(None::<String>));
    let _core_listener = {
        let quit_on_done = mainloop.clone();
        let quit_on_error = mainloop.clone();
        let done = done.clone();
        let core_error = core_error.clone();
        core.add_listener_local()
            .done(move |id, seq| {
                if id == pw::core::PW_ID_CORE && seq.seq() == asked {
                    done.set(true);
                    quit_on_done.quit();
                }
            })
            .error(move |id, seq, res, message| {
                tracing::warn!(id, seq, res, message, "PipeWire object reported an error");
                if id == pw::core::PW_ID_CORE {
                    *core_error.borrow_mut() = Some(message.to_string());
                    quit_on_error.quit();
                }
            })
            .register()
    };

    // A daemon that never answers must not hold the window thread past its own deadline.
    let _deadline = {
        let quit = mainloop.clone();
        let timer = mainloop.loop_().add_timer(move |_| quit.quit());
        timer
            .update_timer(Some(AUDIO_SOURCE_LIST_TIMEOUT), None)
            .into_result()
            .map_err(|e| AudioError::PipeWire(format!("could not arm the list deadline: {e}")))?;
        timer
    };
    mainloop.run();

    if let Some(message) = core_error.borrow_mut().take() {
        return Err(AudioError::PipeWire(message));
    }
    if !done.get() {
        return Err(AudioError::PipeWire(format!(
            "the application list did not complete within {AUDIO_SOURCE_LIST_TIMEOUT:?}"
        )));
    }
    // A node can be announced before the Client global it names, so the graph is fed after the
    // roundtrip rather than during it. Everything foreign is tracked, whatever is selected: the
    // list reports what is playing, and our own pid and binary are what keep brp out of it.
    let mut graph = Graph::new(process_id, own_binary(), AudioSelection::All);
    for (id, client) in seen_clients.take() {
        graph.add_client(id, client);
    }
    for node in seen_nodes.take() {
        graph.add_node(node);
    }
    Ok(graph.sources())
}
```

- [ ] **Step 7: Give the Windows backend its interim answers**

In `crates/audio/src/windows/mod.rs`, add `use crate::selection::{AudioSelection, AudioSource};` and replace the trait header:

```rust
impl AudioCapture for ProcessLoopbackCapture {
    fn sources(&self) -> Result<Vec<AudioSource>, AudioError> {
        Err(AudioError::Unsupported(
            "choosing which applications to share is not implemented on Windows yet".into(),
        ))
    }

    fn start(
        &self,
        selection: AudioSelection,
        sink: AudioSink,
    ) -> Result<Box<dyn AudioCaptureSession>, AudioError> {
        // Sharing more than was asked for is the failure this feature exists to prevent, so an
        // unimplemented selection fails visibly instead. `All` is untouched.
        if selection != AudioSelection::All {
            return Err(AudioError::Unsupported(
                "sharing only selected applications is not implemented on Windows yet".into(),
            ));
        }
```

The rest of `start` is unchanged.

- [ ] **Step 8: Update the one call site and the room's test fakes**

In `crates/room/src/registry.rs`, add `use brp_audio::AudioSelection;` to the existing `brp_audio` import and pass the only value the registry can know at this point:

```rust
        match self.audio_capture.start(AudioSelection::All, publisher.sink()) {
```

In `crates/room/tests/registry.rs`, add `AudioSelection, AudioSource` to the `brp_audio` import and give each fake the new shape:

```rust
impl AudioCapture for FailingCapture {
    fn sources(&self) -> Result<Vec<AudioSource>, AudioError> {
        Ok(Vec::new())
    }
    fn start(
        &self,
        _selection: AudioSelection,
        _sink: AudioSink,
    ) -> Result<Box<dyn AudioCaptureSession>, AudioError> {
        Err(AudioError::Unsupported("no loopback here".into()))
    }
}
```

```rust
impl AudioCapture for SlowCapture {
    fn sources(&self) -> Result<Vec<AudioSource>, AudioError> {
        Ok(Vec::new())
    }
    fn start(
        &self,
        selection: AudioSelection,
        sink: AudioSink,
    ) -> Result<Box<dyn AudioCaptureSession>, AudioError> {
        std::thread::sleep(Duration::from_millis(300));
        SyntheticTone {
            frequency_hz: 440.0,
            amplitude: 0.5,
        }
        .start(selection, sink)
    }
}
```

```rust
impl AudioCapture for DyingCapture {
    fn sources(&self) -> Result<Vec<AudioSource>, AudioError> {
        Ok(Vec::new())
    }
    fn start(
        &self,
        _selection: AudioSelection,
        _sink: AudioSink,
    ) -> Result<Box<dyn AudioCaptureSession>, AudioError> {
        Ok(Box::new(DeadSession))
    }
}
```

- [ ] **Step 9: Run the tests to verify they pass**

Run: `cargo test --workspace`
Expected: PASS, including the three new synthetic-tone tests.

The graph logic behind `sources()` is covered by Task 2; the roundtrip itself has no automated test, because it needs a live daemon. It is exercised by eye in Task 8 step 7 and in Task 9's manual checks. Do not claim it works before then.

- [ ] **Step 10: Verify the workspace and commit**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: PASS.

```bash
git add crates/audio crates/room/src/registry.rs crates/room/tests/registry.rs
git commit -m "feat(audio): capture a selection and list the applications playing audio"
```

---

### Task 4: The registry owns the selection and swaps the session

**Files:**
- Modify: `crates/room/src/registry.rs`
- Modify: `crates/room/src/snapshot.rs`
- Modify: `crates/room/src/room.rs:127-132` (the `LiveRegistry::new` call gains an argument; the config field lands in Task 5, so pass `AudioSelection::All` here)
- Test: `crates/room/tests/registry.rs`

**Interfaces:**
- Consumes: Task 3's `AudioCapture::sources`/`start(selection, sink)`, `SyntheticTone::tone_key`.
- Produces: `LiveRegistry::new(encoders, audio_capture, selection: AudioSelection, grace, on_change) -> Arc<LiveRegistry>`; `LiveRegistry::set_audio_applications(&self, selection: AudioSelection)`; `LiveRegistry::sources(&self) -> Result<Vec<AudioSource>, AudioError>`; `OwnAudioView.selection: AudioSelection`.

- [ ] **Step 1: Write the failing tests**

In `crates/room/tests/registry.rs`, add to the imports:

```rust
use std::collections::BTreeSet;
use brp_audio::{AppKey, AudioSelection, AudioSource};
```

Update the `registry` helper and every `LiveRegistry::new` call in the file to pass the selection:

```rust
fn registry(grace: Duration) -> Arc<LiveRegistry> {
    LiveRegistry::new(
        Arc::new(FakeCodecs),
        Arc::new(SyntheticTone {
            frequency_hz: 440.0,
            amplitude: 0.5,
        }),
        AudioSelection::All,
        grace,
        Arc::new(|| {}),
    )
}
```

Add the fake whose second start fails, beside the other fakes:

```rust
/// Succeeds once and fails afterwards: the swap a selection edit performs is the second start.
struct SwapFailsCapture {
    starts: AtomicUsize,
}

impl AudioCapture for SwapFailsCapture {
    fn sources(&self) -> Result<Vec<AudioSource>, AudioError> {
        Ok(Vec::new())
    }
    fn start(
        &self,
        selection: AudioSelection,
        sink: AudioSink,
    ) -> Result<Box<dyn AudioCaptureSession>, AudioError> {
        if self.starts.fetch_add(1, Ordering::SeqCst) == 0 {
            return SyntheticTone {
                frequency_hz: 440.0,
                amplitude: 0.5,
            }
            .start(selection, sink);
        }
        Err(AudioError::PipeWire("the daemon went away".into()))
    }
}
```

And the tests:

```rust
fn only(keys: [AppKey; 1]) -> AudioSelection {
    AudioSelection::Only(BTreeSet::from(keys))
}

#[tokio::test]
async fn an_edit_while_idle_only_stores_the_selection_and_the_next_capture_uses_it() {
    let registry = registry(GRACE);
    let live = synthetic_live(&registry, "desk").await;
    registry.set_audio_applications(AudioSelection::Only(BTreeSet::new()));
    assert_eq!(registry.audio_view().state, AudioCaptureState::Idle);
    assert_eq!(
        registry.audio_view().selection,
        AudioSelection::Only(BTreeSet::new())
    );
    assert!(
        registry.live_infos()[0].has_audio,
        "an empty selection is silence, not a failure, so presence is unaffected"
    );

    let mut audio = registry.subscribe_audio(live).unwrap();
    assert_eq!(registry.audio_view().state, AudioCaptureState::Capturing);
    assert!(
        registry.live_infos()[0].has_audio,
        "live_infos is unaffected by the selection, even while capturing nothing"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(300), audio.packets.recv())
            .await
            .is_err(),
        "the capture honours the stored selection, so nothing is sent"
    );
}

#[tokio::test]
async fn an_edit_while_capturing_swaps_the_session_and_keeps_the_publisher() {
    let registry = registry(GRACE);
    let live = synthetic_live(&registry, "desk").await;
    let mut audio = registry.subscribe_audio(live).unwrap();
    let first = tokio::time::timeout(Duration::from_secs(2), audio.packets.recv())
        .await
        .unwrap()
        .unwrap();

    registry.set_audio_applications(only([SyntheticTone::tone_key()]));
    let view = registry.audio_view();
    assert_eq!(
        (view.state, view.subscribers),
        (AudioCaptureState::Capturing, 1),
        "the same publisher, so the same subscriber"
    );
    let next = tokio::time::timeout(Duration::from_secs(2), audio.packets.recv())
        .await
        .unwrap()
        .expect("the subscription survives the swap");
    assert!(
        next.seq > first.seq,
        "the sequence space continued: {} then {}",
        first.seq,
        next.seq
    );
    assert!(registry.live_infos()[0].has_audio);
}

#[tokio::test]
async fn a_swap_that_fails_to_start_drops_has_audio_and_rejects_new_subscribers() {
    let registry = LiveRegistry::new(
        Arc::new(FakeCodecs),
        Arc::new(SwapFailsCapture {
            starts: AtomicUsize::new(0),
        }),
        AudioSelection::All,
        GRACE,
        Arc::new(|| {}),
    );
    let live = synthetic_live(&registry, "desk").await;
    let mut audio = registry.subscribe_audio(live).unwrap();
    tokio::time::timeout(Duration::from_secs(2), audio.packets.recv())
        .await
        .unwrap()
        .unwrap();

    registry.set_audio_applications(only([SyntheticTone::tone_key()]));
    assert!(matches!(
        registry.audio_view().state,
        AudioCaptureState::Failed(ref m) if m.contains("went away")
    ));
    assert!(!registry.live_infos()[0].has_audio);
    assert!(matches!(
        registry.subscribe_audio(live),
        Err(SubscribeRejected::NoAudio)
    ));
    assert!(
        tokio::time::timeout(Duration::from_secs(2), audio.packets.recv())
            .await
            .unwrap()
            .is_none(),
        "no fallback to the old session: the publisher stops and the subscription ends"
    );
}

#[tokio::test]
async fn an_edit_clears_a_stale_capture_failure() {
    let registry = LiveRegistry::new(
        Arc::new(FakeCodecs),
        Arc::new(FailingCapture),
        AudioSelection::All,
        GRACE,
        Arc::new(|| {}),
    );
    let live = synthetic_live(&registry, "desk").await;
    assert!(matches!(
        registry.subscribe_audio(live),
        Err(SubscribeRejected::NoAudio)
    ));
    assert!(!registry.live_infos()[0].has_audio);

    registry.set_audio_applications(only([AppKey::new("firefox")]));
    assert_eq!(registry.audio_view().state, AudioCaptureState::Idle);
    assert!(
        registry.live_infos()[0].has_audio,
        "editing a selection is also the retry path"
    );
}

#[test]
fn the_registry_reports_what_the_backend_lists() {
    let registry = registry(GRACE);
    let listed: Vec<String> = registry
        .sources()
        .unwrap()
        .into_iter()
        .map(|source| source.label)
        .collect();
    assert_eq!(listed, ["Tone", "Silent"]);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p brp-room --test registry`
Expected: FAIL to compile, `this function takes 4 arguments but 5 arguments were supplied` and `no method named set_audio_applications`.

- [ ] **Step 3: Store the selection and add the view field**

In `crates/room/src/snapshot.rs`, add `use brp_audio::AudioSelection;` and the field:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnAudioView {
    pub enabled: bool,
    pub state: AudioCaptureState,
    /// What the room hears. The picker pre-checks its rows from this, and the panel reads mode
    /// `Only` with an empty set as "no applications selected" rather than "capturing".
    pub selection: AudioSelection,
    pub subscribers: usize,
    pub packets_encoded: u64,
}
```

In `crates/room/src/registry.rs`, extend the state and the constructor:

```rust
struct AudioState {
    enabled: bool,
    selection: AudioSelection,
    running: Option<RunningAudio>,
    last_error: Option<String>,
}
```

```rust
    pub fn new(
        encoders: Arc<dyn EncoderFactory>,
        audio_capture: Arc<dyn AudioCapture>,
        selection: AudioSelection,
        grace: Duration,
        on_change: ChangeNotify,
    ) -> Arc<Self> {
        Arc::new(Self {
            audio_start: Mutex::new(()),
            inner: Mutex::new(Inner {
                lives: BTreeMap::new(),
                next_live_id: 1,
                audio: AudioState {
                    enabled: true,
                    selection,
                    running: None,
                    last_error: None,
                },
            }),
            encoders,
            audio_capture,
            grace,
            on_change,
        })
    }
```

In `audio_view`, add the field to the returned struct:

```rust
        OwnAudioView {
            enabled: audio.enabled,
            state,
            selection: audio.selection.clone(),
```

- [ ] **Step 4: Read the selection when starting, and add the two new methods**

In `start_audio`, take the stored selection under a brief lock — never across the blocking start:

```rust
    /// Opens the encoder and starts the platform capture with the registry unlocked. A failure
    /// stops the publisher it already built, which joins its encode thread.
    fn start_audio(&self) -> Result<RunningAudio, String> {
        let selection = lock(&self.inner).audio.selection.clone();
        let encoder = self.encoders.open_audio().map_err(|e| e.to_string())?;
        let publisher = AudioPublisher::start(encoder);
        match self.audio_capture.start(selection, publisher.sink()) {
```

Add both methods to the first `impl LiveRegistry` block, after `set_audio`:

```rust
    /// Replaces which applications the capture carries. With nothing running that is all it does,
    /// and the next subscriber starts with the new value. Editing a selection also clears a
    /// recorded failure, so it is a retry path like the share-audio retoggle.
    pub fn set_audio_applications(&self, selection: AudioSelection) {
        // The same lock a first start holds: a swap and a start must not overlap, and the swap's
        // start blocks just as long.
        let _starting = lock(&self.audio_start);
        let mut inner = lock(&self.inner);
        inner.audio.selection = selection.clone();
        inner.audio.last_error = None;
        let Some(running) = take_audio(&mut inner.audio) else {
            drop(inner);
            (self.on_change)();
            return;
        };
        drop(inner);

        // The publisher outlives the swap, so the fan-out, the audio sequence space, and every
        // viewer's carrier survive it; the jitter buffer emits silence for the gap and re-primes.
        let RunningAudio {
            session,
            publisher,
            idle_since,
        } = running;
        // Stopped before the new one starts: two sessions feeding one publisher would sum the
        // desktop into itself at double amplitude for the overlap.
        session.stop();
        match self.audio_capture.start(selection, publisher.sink()) {
            Ok(session) => {
                let mut inner = lock(&self.inner);
                if !inner.audio.advertises() {
                    // Share audio was turned off while a slow daemon was answering.
                    drop(inner);
                    stop_audio(RunningAudio {
                        session,
                        publisher,
                        idle_since,
                    });
                    (self.on_change)();
                    return;
                }
                inner.audio.running = Some(RunningAudio {
                    session,
                    publisher,
                    idle_since,
                });
            }
            Err(error) => {
                // No fallback to the old session: reviving it would keep sharing applications the
                // user just deselected. This is phase 4's capture-failure path.
                publisher.stop();
                let message = error.to_string();
                tracing::warn!(%message, "audio capture failed to restart for a new selection");
                lock(&self.inner).audio.last_error = Some(message);
            }
        }
        (self.on_change)();
    }

    /// What the platform reports as playing audio now. Takes no registry lock: the backend's own
    /// deadline is what bounds it.
    pub fn sources(&self) -> Result<Vec<AudioSource>, AudioError> {
        self.audio_capture.sources()
    }
```

Extend the imports at the top of the file:

```rust
use brp_audio::{AudioCapture, AudioCaptureSession, AudioError, AudioSelection, AudioSource};
```

- [ ] **Step 5: Keep the room compiling**

In `crates/room/src/room.rs`, add `AudioSelection` to the `brp_audio` import and pass it at the call site; the config field arrives in Task 5:

```rust
        let registry = LiveRegistry::new(
            config.encoders.clone(),
            config.audio_capture.clone(),
            AudioSelection::All,
            config.timings.encoder_grace,
            registry_notify,
        );
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p brp-room --test registry`
Expected: PASS, all thirteen tests.

- [ ] **Step 7: Verify the workspace and commit**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: PASS.

```bash
git add crates/room/src crates/room/tests/registry.rs
git commit -m "feat(room): swap the audio session when the application selection changes"
```

---

### Task 5: The room's selection, its error variant, and the integration test

**Files:**
- Modify: `crates/room/src/room.rs`
- Modify: `crates/room/src/error.rs`
- Modify: `crates/room/tests/two_rooms.rs`
- Modify: `crates/room/tests/hub_leaves.rs:13-40` (the `config` helper gains the field)
- Modify: `crates/app/src/launch.rs:81-98`, `crates/app/src/publish.rs:27-40` (the config literals gain the field; the settings-derived value arrives in Task 6)
- Test: `crates/room/tests/two_rooms.rs`

**Interfaces:**
- Consumes: Task 4's `LiveRegistry::set_audio_applications`/`sources`, `OwnAudioView.selection`.
- Produces: `RoomConfig.audio_applications: AudioSelection`; `Room::set_audio_applications(&self, selection: AudioSelection)`; `Room::audio_sources(&self) -> Result<Vec<AudioSource>, RoomError>`; `RoomError::Audio(AudioError)`.

- [ ] **Step 1: Write the failing test**

In `crates/room/tests/two_rooms.rs`, add `use std::collections::BTreeSet;` and `AudioSelection` to the `brp_audio` import, add the field to the `config` helper:

```rust
        audio_applications: AudioSelection::All,
```

and add the test after `toggling_share_audio_off_and_on_gets_the_carrier_back`:

```rust
/// Spec 7's selection edit: chunks stop for one backend start, but the publisher, the sequence
/// space, and the viewer's carrier are untouched. A publisher restart would have moved the
/// carrier, which is what makes this the assertion that proves the swap.
#[tokio::test]
async fn a_selection_edit_swaps_the_capture_without_moving_the_carrier() {
    let a = Room::create(config("alice")).await.unwrap();
    let (bob_cfg, output) = config_with_output("bob");
    let b = Room::join(bob_cfg, a.ticket()).await.unwrap();
    wait_until("mutual presence", Duration::from_secs(5), || {
        a.snapshot().members.len() == 1 && b.snapshot().members.len() == 1
    })
    .await;
    let live = a
        .start_live(SourceKind::Monitor, None, "desk".into())
        .await
        .unwrap();
    wait_until("catalog", Duration::from_secs(5), || {
        b.snapshot().members[0].lives.len() == 1
    })
    .await;
    b.watch(a.id(), live, SOURCE_PRESET_ID).unwrap();
    wait_until("audible", Duration::from_secs(5), || {
        is_audible(&output.render(1024))
    })
    .await;

    let listed: Vec<String> = a
        .audio_sources()
        .unwrap()
        .into_iter()
        .map(|source| source.key.as_str().to_string())
        .collect();
    assert_eq!(listed, ["tone", "silent"]);

    a.set_audio_applications(AudioSelection::Only(BTreeSet::from([
        SyntheticTone::tone_key(),
    ])));
    wait_until("still audible", Duration::from_secs(5), || {
        is_audible(&output.render(1024))
    })
    .await;
    let carrier = |room: &Room| -> Vec<u32> {
        room.snapshot()
            .watches
            .iter()
            .filter(|w| w.audio)
            .map(|w| w.live_id)
            .collect()
    };
    assert_eq!(carrier(&b), vec![live]);

    a.set_audio_applications(AudioSelection::Only(BTreeSet::new()));
    wait_until("silence", Duration::from_secs(5), || {
        !is_audible(&output.render(1024))
    })
    .await;
    assert_eq!(carrier(&b), vec![live], "the carrier did not move");
    assert!(
        b.snapshot()
            .watches
            .iter()
            .all(|w| w.state == WatchState::Live),
        "the watch was not disturbed"
    );
    assert!(
        b.snapshot().members[0].has_audio,
        "an empty selection is silence, so presence still advertises audio"
    );

    b.leave().await;
    a.leave().await;
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p brp-room --test two_rooms a_selection_edit`
Expected: FAIL to compile, `struct RoomConfig has no field named audio_applications` and `no method named set_audio_applications`.

- [ ] **Step 3: Add the error variant**

In `crates/room/src/error.rs`, beside `Capture` and `Codec`:

```rust
    #[error(transparent)]
    Audio(#[from] brp_audio::AudioError),
```

- [ ] **Step 4: Add the config field and the two methods**

In `crates/room/src/room.rs`, extend the `brp_audio` import to `use brp_audio::{AudioCapture, AudioOutput, AudioOutputSession, AudioSelection, AudioSource};`, add the field to `RoomConfig`:

```rust
    /// Which applications this participant's audio carries, as persisted. In force from the first
    /// capture rather than from the first time the picker is opened.
    pub audio_applications: AudioSelection,
```

pass it to the registry, replacing the `AudioSelection::All` placeholder of Task 4:

```rust
            config.audio_applications,
```

and add the two methods beside `set_audio`:

```rust
    pub fn set_audio_applications(&self, selection: AudioSelection) {
        self.registry.set_audio_applications(selection);
    }

    /// Lists what is playing audio now, for the application picker.
    pub fn audio_sources(&self) -> Result<Vec<AudioSource>, RoomError> {
        Ok(self.registry.sources()?)
    }
```

- [ ] **Step 5: Fill in the field at every construction site**

`crates/room/tests/hub_leaves.rs`, in `config`:

```rust
        audio_applications: brp_audio::AudioSelection::All,
```

`crates/app/src/launch.rs`, in `open_room`'s `RoomConfig` — a placeholder this task, replaced in Task 6:

```rust
        audio_applications: brp_audio::AudioSelection::All,
```

`crates/app/src/publish.rs`, in its `RoomConfig`:

```rust
        // The headless publisher reads no settings, so it shares every application as it always has.
        audio_applications: brp_audio::AudioSelection::All,
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p brp-room --test two_rooms`
Expected: PASS. The new test takes a few seconds: two silence waits at a 200 ms jitter depth.

- [ ] **Step 7: Verify the workspace and commit**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: PASS.

```bash
git add crates/room crates/app/src/launch.rs crates/app/src/publish.rs
git commit -m "feat(room): expose the audio application selection on the room handle"
```

---

### Task 6: The persisted choice

**Files:**
- Modify: `crates/app/src/settings.rs`
- Modify: `crates/app/src/launch.rs`
- Test: `crates/app/src/settings.rs` (inline), `crates/app/src/launch.rs` (inline)

**Interfaces:**
- Consumes: `brp_audio::{AppKey, AudioSelection}`.
- Produces: `settings::AudioMode::{All, Only}` (`Default = All`); `settings::AudioApplications { mode: AudioMode, names: Vec<String> }` with `to_selection(&self) -> AudioSelection`; `AudioSettings.applications`; `Launch.audio_applications: AudioSelection`.

- [ ] **Step 1: Write the failing tests**

In `crates/app/src/settings.rs`, extend the `full()` fixture in the test module:

```rust
            audio: AudioSettings {
                output_device: Some("PipeWire:alsa_output.usb".into()),
                applications: AudioApplications {
                    mode: AudioMode::Only,
                    names: vec!["firefox".into(), "game.exe".into()],
                },
            },
```

extend `the_file_shape_matches_the_spec` with:

```rust
        assert!(text.contains("[audio.applications]"), "{text}");
        assert!(text.contains("mode = \"only\""), "{text}");
        assert!(text.contains("\"firefox\""), "{text}");
```

and add:

```rust
    #[test]
    fn the_mode_selects_all_or_only_the_named_applications() {
        assert_eq!(
            AudioApplications::default().to_selection(),
            AudioSelection::All
        );
        let only = AudioApplications {
            mode: AudioMode::Only,
            names: vec!["firefox".into(), "game.exe".into()],
        };
        assert_eq!(
            only.to_selection(),
            AudioSelection::Only(BTreeSet::from([
                AppKey::new("firefox"),
                AppKey::new("game.exe"),
            ]))
        );
        let kept = AudioApplications {
            mode: AudioMode::All,
            names: vec!["firefox".into()],
        };
        assert_eq!(
            kept.to_selection(),
            AudioSelection::All,
            "the names are kept in both modes but only in force under Only"
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_hand_edited_name_is_normalised_by_the_conversion() {
        let odd = AudioApplications {
            mode: AudioMode::Only,
            names: vec![r"C:\Games\GAME.EXE".into()],
        };
        assert_eq!(
            odd.to_selection(),
            AudioSelection::Only(BTreeSet::from([AppKey::new("game.exe")])),
            "the conversion goes through AppKey, so odd casing and a full path still match"
        );
    }

    #[test]
    fn an_unknown_application_mode_fails_the_parse() {
        let settings: Result<Settings, _> =
            toml::from_str("[audio.applications]\nmode = \"sometimes\"\n");
        assert!(settings.is_err());
    }

    #[test]
    fn a_file_without_an_applications_table_shares_every_application() {
        let settings: Settings = toml::from_str("[audio]\noutput_device = \"x\"\n").unwrap();
        assert_eq!(settings.audio.applications, AudioApplications::default());
        assert_eq!(settings.audio.applications.to_selection(), AudioSelection::All);
    }
```

The test module needs `use brp_audio::{AppKey, AudioSelection};` and `use std::collections::BTreeSet;`.

In `crates/app/src/launch.rs`, extend the `saved()` fixture with the same `applications` block as above and add to both existing assertions:

```rust
        assert_eq!(
            launch.audio_applications,
            AudioSelection::Only(BTreeSet::from([
                AppKey::new("firefox"),
                AppKey::new("game.exe"),
            ]))
        );
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p brp settings:: && cargo test -p brp launch::`
Expected: FAIL to compile, `cannot find struct AudioApplications` and `struct AudioSettings has no field named applications`.

- [ ] **Step 3: Add the settings types**

In `crates/app/src/settings.rs`, add `use brp_audio::{AppKey, AudioSelection};` and replace `AudioSettings`:

```rust
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioSettings {
    /// A cpal device id in its `Display` form; `None` is the system default.
    pub output_device: Option<String>,
    pub applications: AudioApplications,
}

/// Serialised as `[audio.applications] mode = "all" | "only"` with `names`. Both fields are always
/// written: flipping to every application for a film and back must not lose the set. This
/// deliberately differs from [`RelayChoice`]'s adjacently-tagged shape, whose variants carry
/// genuinely different data.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioApplications {
    pub mode: AudioMode,
    pub names: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioMode {
    #[default]
    All,
    Only,
}

impl AudioApplications {
    /// What the room captures with. Names go through [`AppKey`], so a hand-edited file with odd
    /// casing still matches on Windows; mirrors [`RelayChoice::to_relay_setting`].
    pub fn to_selection(&self) -> AudioSelection {
        match self.mode {
            AudioMode::All => AudioSelection::All,
            AudioMode::Only => {
                AudioSelection::Only(self.names.iter().map(|name| AppKey::new(name)).collect())
            }
        }
    }
}
```

`output_device` stays declared before `applications`: serde writes struct fields in order, and a TOML table's scalar keys must precede its sub-tables.

- [ ] **Step 4: Carry it into the launch**

In `crates/app/src/launch.rs`, add `AudioSelection` to the `brp_audio` import, add the field to `Launch`:

```rust
    /// Which applications the room's audio carries.
    pub audio_applications: AudioSelection,
```

fill it in `from_settings`:

```rust
            audio_applications: settings.audio.applications.to_selection(),
```

and replace the placeholder in `open_room`'s `RoomConfig`:

```rust
        audio_applications: launch.audio_applications.clone(),
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p brp settings:: && cargo test -p brp launch::`
Expected: PASS.

- [ ] **Step 6: Verify the workspace and commit**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: PASS.

```bash
git add crates/app/src/settings.rs crates/app/src/launch.rs
git commit -m "feat(app): persist which applications the room hears"
```

---

### Task 7: The picker's state and the two commands

**Files:**
- Create: `crates/app/src/ui/applications.rs`
- Modify: `crates/app/src/ui/mod.rs`
- Modify: `crates/app/src/ui/state.rs`
- Modify: `crates/app/src/commands.rs`
- Modify: `crates/app/src/room_view.rs`
- Modify: `crates/app/src/window.rs` (the two `view.apply(..)` call sites gain the stored choice)
- Test: `crates/app/src/ui/applications.rs` (inline), `crates/app/src/ui/state.rs` (inline)

**Interfaces:**
- Consumes: Task 5's `Room::audio_sources`/`Room::set_audio_applications`, Task 6's `AudioApplications`/`AudioMode`.
- Produces: `ui::applications::{ApplicationRow, ApplicationPicker, selection_summary}` with `ApplicationPicker::new(reported: Vec<AudioSource>, stored: &AudioApplications)`, `refresh(&mut self, Vec<AudioSource>)`, `rows(&self) -> Vec<ApplicationRow>`, `is_chosen(&self, &AppKey) -> bool`, `toggle(&mut self, &AppKey, bool)`, `applied(&self) -> AudioApplications`, and the public draft fields `only: bool` / `chosen: BTreeSet<AppKey>`; `UiState.applications: Option<ApplicationPicker>` with `open_applications`/`cancel_applications`; `RoomCommand::{SetAudioApplications(AudioSelection), ChooseApplications}`; `RoomView::apply(&mut self, commands, runtime, proxy, state, stored: &AudioApplications)`.

- [ ] **Step 1: Write the failing tests**

Create `crates/app/src/ui/applications.rs` with its test module only:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn source(key: &str, label: &str) -> AudioSource {
        AudioSource {
            key: AppKey::new(key),
            label: label.into(),
        }
    }

    fn stored(mode: AudioMode, names: [&str; 1]) -> AudioApplications {
        AudioApplications {
            mode,
            names: names.iter().map(|n| n.to_string()).collect(),
        }
    }

    #[test]
    fn the_draft_starts_from_the_stored_choice_in_both_modes() {
        let all = ApplicationPicker::new(Vec::new(), &stored(AudioMode::All, ["firefox"]));
        assert!(!all.only);
        assert!(
            all.is_chosen(&AppKey::new("firefox")),
            "under all the chosen set is still visible, so a flip back loses nothing"
        );
        let only = ApplicationPicker::new(Vec::new(), &stored(AudioMode::Only, ["firefox"]));
        assert!(only.only);
    }

    #[test]
    fn rows_list_what_is_playing_first_then_the_selected_but_absent() {
        let picker = ApplicationPicker::new(
            vec![source("spotify", "Spotify"), source("firefox", "Firefox")],
            &AudioApplications {
                mode: AudioMode::Only,
                names: vec!["game.exe".into(), "another.exe".into(), "firefox".into()],
            },
        );
        let rows: Vec<(String, bool)> = picker
            .rows()
            .into_iter()
            .map(|row| (row.label, row.playing))
            .collect();
        assert_eq!(
            rows,
            [
                ("Firefox".to_string(), true),
                ("Spotify".to_string(), true),
                ("another.exe".to_string(), false),
                ("game.exe".to_string(), false),
            ],
            "each group alphabetical by label, so a refresh does not reshuffle; an absent row is \
             labelled with its stored key"
        );
    }

    #[test]
    fn a_refresh_replaces_the_list_and_keeps_the_draft() {
        let mut picker =
            ApplicationPicker::new(vec![source("firefox", "Firefox")], &AudioApplications::default());
        picker.only = true;
        picker.toggle(&AppKey::new("firefox"), true);
        picker.refresh(vec![source("spotify", "Spotify")]);
        assert!(picker.only);
        assert!(picker.is_chosen(&AppKey::new("firefox")));
        let rows: Vec<(String, bool)> = picker
            .rows()
            .into_iter()
            .map(|row| (row.label, row.playing))
            .collect();
        assert_eq!(
            rows,
            [
                ("Spotify".to_string(), true),
                ("firefox".to_string(), false),
            ]
        );
    }

    #[test]
    fn what_done_applies_keeps_the_names_in_both_modes() {
        let mut picker = ApplicationPicker::new(
            vec![source("firefox", "Firefox")],
            &AudioApplications::default(),
        );
        picker.only = true;
        picker.toggle(&AppKey::new("firefox"), true);
        picker.toggle(&AppKey::new("spotify"), true);
        picker.toggle(&AppKey::new("spotify"), false);
        let applied = picker.applied();
        assert_eq!(
            applied,
            AudioApplications {
                mode: AudioMode::Only,
                names: vec!["firefox".into()],
            }
        );
        picker.only = false;
        assert_eq!(
            picker.applied(),
            AudioApplications {
                mode: AudioMode::All,
                names: vec!["firefox".into()],
            }
        );
    }

    #[test]
    fn the_summary_names_the_mode_and_counts_the_set() {
        assert_eq!(selection_summary(&AudioSelection::All), "all applications");
        assert_eq!(
            selection_summary(&AudioSelection::Only(BTreeSet::new())),
            "no applications selected"
        );
        assert_eq!(
            selection_summary(&AudioSelection::Only(BTreeSet::from([AppKey::new("a")]))),
            "1 application"
        );
        assert_eq!(
            selection_summary(&AudioSelection::Only(BTreeSet::from([
                AppKey::new("a"),
                AppKey::new("b"),
                AppKey::new("c"),
            ]))),
            "3 applications"
        );
    }
}
```

In `crates/app/src/ui/state.rs`, add to the test module:

```rust
    #[test]
    fn opening_the_application_picker_twice_refreshes_it_without_discarding_the_draft() {
        use crate::settings::AudioApplications;
        use brp_audio::{AppKey, AudioSource};

        let listed = |key: &str| {
            vec![AudioSource {
                key: AppKey::new(key),
                label: key.to_string(),
            }]
        };
        let mut state = UiState::new();
        state.open_applications(listed("firefox"), &AudioApplications::default());
        let picker = state.applications.as_mut().expect("opened");
        picker.only = true;
        picker.toggle(&AppKey::new("firefox"), true);

        state.open_applications(listed("spotify"), &AudioApplications::default());
        let picker = state.applications.as_ref().expect("still open");
        assert!(picker.only, "a refresh must not discard the edit");
        assert!(picker.is_chosen(&AppKey::new("firefox")));
        assert_eq!(picker.rows().len(), 2);

        state.cancel_applications();
        assert!(state.applications.is_none());
    }
```

Also update the two `OwnAudioView` literals in that file's tests with the new field:

```rust
                selection: brp_audio::AudioSelection::All,
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p brp ui::`
Expected: FAIL to compile, `file not found for module applications` and `no method named open_applications`.

- [ ] **Step 3: Write the picker's state**

Prepend to `crates/app/src/ui/applications.rs`:

```rust
//! The application picker: which applications the room hears. A draft edited in a window of its
//! own and applied on Done, because every apply swaps the capture session and gaps the room for
//! the length of one backend start.

use std::collections::BTreeSet;

use brp_audio::{AppKey, AudioSelection, AudioSource};

use crate::settings::{AudioApplications, AudioMode};

/// A row in the picker: an identity, what to call it, and whether the platform reports it playing
/// audio now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationRow {
    pub key: AppKey,
    pub label: String,
    pub playing: bool,
}

/// What the platform last reported, and the draft the user is editing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationPicker {
    reported: Vec<AudioSource>,
    /// Draft mode; `false` is every application except brp.
    pub only: bool,
    /// Draft set, kept in both modes so flipping to every application and back loses nothing.
    pub chosen: BTreeSet<AppKey>,
}

impl ApplicationPicker {
    /// Opens with what the platform reports and the stored choice as the draft. The stored choice,
    /// not the selection in force: under `all` only the settings still hold the names.
    pub fn new(reported: Vec<AudioSource>, stored: &AudioApplications) -> Self {
        Self {
            reported,
            only: stored.mode == AudioMode::Only,
            chosen: stored.names.iter().map(|name| AppKey::new(name)).collect(),
        }
    }

    /// Replaces the reported list and keeps the draft: a refresh mid-edit must not discard the edit.
    pub fn refresh(&mut self, reported: Vec<AudioSource>) {
        self.reported = reported;
    }

    /// The rows to draw: what is playing first, then what is chosen but absent, each group
    /// alphabetical by label so a refresh does not reshuffle the list.
    pub fn rows(&self) -> Vec<ApplicationRow> {
        let mut rows: Vec<ApplicationRow> = self
            .reported
            .iter()
            .map(|source| ApplicationRow {
                key: source.key.clone(),
                label: source.label.clone(),
                playing: true,
            })
            .collect();
        rows.sort_by(|a, b| a.label.cmp(&b.label).then_with(|| a.key.cmp(&b.key)));
        // A closed application has no friendly name to report, so its stored identity is the label.
        let mut absent: Vec<ApplicationRow> = self
            .chosen
            .iter()
            .filter(|key| !self.reported.iter().any(|source| &source.key == *key))
            .map(|key| ApplicationRow {
                key: key.clone(),
                label: key.as_str().to_string(),
                playing: false,
            })
            .collect();
        absent.sort_by(|a, b| a.label.cmp(&b.label));
        rows.append(&mut absent);
        rows
    }

    pub fn is_chosen(&self, key: &AppKey) -> bool {
        self.chosen.contains(key)
    }

    pub fn toggle(&mut self, key: &AppKey, chosen: bool) {
        if chosen {
            self.chosen.insert(key.clone());
        } else {
            self.chosen.remove(key);
        }
    }

    /// What Done applies to the room and stores in the settings.
    pub fn applied(&self) -> AudioApplications {
        AudioApplications {
            mode: if self.only {
                AudioMode::Only
            } else {
                AudioMode::All
            },
            names: self
                .chosen
                .iter()
                .map(|key| key.as_str().to_string())
                .collect(),
        }
    }
}

/// The one-line summary beside the button: what the room hears.
pub fn selection_summary(selection: &AudioSelection) -> String {
    match selection {
        AudioSelection::All => "all applications".to_string(),
        AudioSelection::Only(keys) if keys.is_empty() => "no applications selected".to_string(),
        AudioSelection::Only(keys) => {
            let plural = if keys.len() == 1 { "" } else { "s" };
            format!("{} application{plural}", keys.len())
        }
    }
}
```

Declare the module in `crates/app/src/ui/mod.rs`, in alphabetical order before `members`:

```rust
pub mod applications;
```

- [ ] **Step 4: Hold the picker in the UI state**

In `crates/app/src/ui/state.rs`, add the imports (`use brp_audio::AudioSource;`, `use crate::settings::AudioApplications;`, and `use super::applications::ApplicationPicker;` — sibling modules in `ui/` reach each other through `super`, as `picker.rs` and `own_lives.rs` do), the field:

```rust
    /// Open while the user edits which applications the room hears.
    pub applications: Option<ApplicationPicker>,
```

and the two methods in `impl UiState`:

```rust
    /// Opens the application picker, or replaces the list of one already open — Refresh takes the
    /// same path, and the draft must survive it.
    pub fn open_applications(&mut self, reported: Vec<AudioSource>, stored: &AudioApplications) {
        match &mut self.applications {
            Some(picker) => picker.refresh(reported),
            None => self.applications = Some(ApplicationPicker::new(reported, stored)),
        }
    }

    /// Closes it, discarding the draft.
    pub fn cancel_applications(&mut self) {
        self.applications = None;
    }
```

- [ ] **Step 5: Add the commands and apply them**

In `crates/app/src/commands.rs`, add `use brp_audio::AudioSelection;` and two variants:

```rust
    /// Replaces which applications this participant's audio carries.
    SetAudioApplications(AudioSelection),
    /// Lists what is playing audio and opens the application picker, or refreshes an open one.
    ChooseApplications,
```

In `crates/app/src/room_view.rs`, add `use crate::settings::AudioApplications;`, extend `apply`'s signature and add both arms:

```rust
    /// Applies the commands one egui pass produced. Errors land in the status line. `stored` seeds
    /// a picker these commands open: under `all` only the settings still hold the chosen names.
    pub fn apply(
        &mut self,
        commands: Vec<RoomCommand>,
        runtime: &Handle,
        proxy: &EventLoopProxy<AppEvent>,
        state: &mut UiState,
        stored: &AudioApplications,
    ) {
```

```rust
                RoomCommand::SetAudioApplications(selection) => {
                    self.room.set_audio_applications(selection);
                    Ok(())
                }
                // Mirrors `Share { source: None }`: the enumeration runs here on the command-drain
                // path, which is how the interim Windows `Unsupported` reaches the user as a
                // status line with no capability flag anywhere in the tree.
                RoomCommand::ChooseApplications => match self.room.audio_sources() {
                    Ok(sources) => {
                        state.open_applications(sources, stored);
                        Ok(())
                    }
                    Err(error) => Err(error),
                },
```

In `crates/app/src/window.rs`, pass the stored choice at both `view.apply(..)` call sites, in `redraw_main` and `redraw_popout`:

```rust
            view.apply(
                output.commands,
                &self.runtime,
                &self.proxy,
                &mut self.state,
                &self.store.settings.audio.applications,
            );
```

In `redraw_popout` the same call reads `self.store` while `self.phase` is mutably borrowed by the `if let Phase::Room(view) = &mut self.phase` guard; if the borrow checker refuses, clone the choice into a local before the guard:

```rust
        let stored = self.store.settings.audio.applications.clone();
```

and pass `&stored`.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p brp`
Expected: PASS, including the five picker tests and the state test.

- [ ] **Step 7: Verify the workspace and commit**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: PASS.

```bash
git add crates/app/src
git commit -m "feat(app): hold an application picker draft and the commands that apply it"
```

---

### Task 8: The picker window, the panel controls, and persistence

**Files:**
- Modify: `crates/app/src/ui/applications.rs`
- Modify: `crates/app/src/ui/own_lives.rs`
- Modify: `crates/app/src/window.rs`
- Test: `crates/app/src/ui/applications.rs` (inline, one added test)

**Interfaces:**
- Consumes: Task 7's `ApplicationPicker`, `selection_summary`, both commands; Task 6's `AudioApplications`.
- Produces: `ui::applications::PickerOutcome::{Refresh, Applied(AudioApplications)}` and `ui::applications::draw(ctx: &egui::Context, state: &mut UiState) -> Option<PickerOutcome>`.

`draw` returns its outcome rather than pushing commands, and `window.rs` accumulates it outside the egui pass, exactly as `settings_ui::draw` does: egui may run the pass closure twice, and Done closes the picker, so a second run would see nothing and a pushed command inside `UiOutput` would be discarded with it.

- [ ] **Step 1: Write the failing test**

In `crates/app/src/ui/applications.rs`'s test module:

```rust
    #[test]
    fn the_outcome_of_done_carries_the_choice_and_its_selection() {
        let mut picker = ApplicationPicker::new(Vec::new(), &AudioApplications::default());
        picker.only = true;
        picker.toggle(&AppKey::new("firefox"), true);
        let outcome = PickerOutcome::Applied(picker.applied());
        let PickerOutcome::Applied(applied) = outcome else {
            panic!("Done applies a choice");
        };
        assert_eq!(applied.mode, AudioMode::Only);
        assert_eq!(
            applied.to_selection(),
            AudioSelection::Only(BTreeSet::from([AppKey::new("firefox")]))
        );
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p brp applications::`
Expected: FAIL to compile, `cannot find type PickerOutcome in this scope`.

- [ ] **Step 3: Draw the picker**

Add `use super::state::UiState;` to the import block at the top of `crates/app/src/ui/applications.rs`, then append the rest:

```rust
const MAX_LIST_HEIGHT: f32 = 400.0;

/// What one pass of the picker produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerOutcome {
    /// Refresh was clicked: enumerate again and merge into the open draft.
    Refresh,
    /// Done was clicked: apply this to the room and store it.
    Applied(AudioApplications),
}

/// Draws the picker when one is open. Esc, Cancel, and the title bar's close button all discard
/// the draft; the main window has no other Esc consumer, since the fullscreen rule lives in the
/// pop-out windows and each has an egui context of its own.
pub fn draw(ctx: &egui::Context, state: &mut UiState) -> Option<PickerOutcome> {
    // Cloned so the window closure does not borrow `state` while it draws, as `picker.rs` does.
    let Some(mut picker) = state.applications.clone() else {
        return None;
    };
    let rows = picker.rows();
    let (mut done, mut cancelled, mut refresh) = (false, false, false);
    let mut open = true;
    egui::Window::new("Applications the room hears")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.radio_value(&mut picker.only, false, "All applications (except brp)");
            ui.radio_value(&mut picker.only, true, "Only the applications I select");
            if rows.is_empty() {
                ui.weak("nothing is playing audio");
            }
            let only = picker.only;
            egui::ScrollArea::vertical()
                .max_height(MAX_LIST_HEIGHT)
                .show(ui, |ui| {
                    for row in &rows {
                        let mut chosen = picker.is_chosen(&row.key);
                        let label = if row.playing {
                            row.label.clone()
                        } else {
                            format!("{} (not playing)", row.label)
                        };
                        // Under "all" the list stays visible but disabled: what was chosen is
                        // still there and a flip back loses nothing.
                        let response =
                            ui.add_enabled(only, egui::Checkbox::new(&mut chosen, label));
                        if response.changed() {
                            picker.toggle(&row.key, chosen);
                        }
                    }
                });
            ui.separator();
            ui.horizontal(|ui| {
                refresh = ui.button("Refresh").clicked();
                done = ui.button("Done").clicked();
                cancelled = ui.button("Cancel").clicked();
            });
        });
    let escape = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    if done {
        let applied = picker.applied();
        state.applications = None;
        return Some(PickerOutcome::Applied(applied));
    }
    if cancelled || escape || !open {
        state.applications = None;
        return None;
    }
    state.applications = Some(picker);
    refresh.then_some(PickerOutcome::Refresh)
}
```

- [ ] **Step 4: Add the button, the summary, and the empty-selection state**

In `crates/app/src/ui/own_lives.rs`, add `use brp_audio::AudioSelection;` and `use super::applications::selection_summary;`, then insert the button and the summary in the header `ui.horizontal`, between the share-audio checkbox's `if` block and the `match` on the capture state:

```rust
                if ui.button("Choose applications…").clicked() {
                    commands.push(RoomCommand::ChooseApplications);
                }
                ui.weak(selection_summary(&snapshot.own_audio.selection));
```

The button is not gated on share audio: choosing applications while audio is off is exactly how one prepares for turning it on.

Replace the `Capturing` arm of that match so the one state that would otherwise mislead reads honestly:

```rust
                    AudioCaptureState::Capturing => match &snapshot.own_audio.selection {
                        AudioSelection::Only(keys) if keys.is_empty() => {
                            ui.weak("no applications selected");
                        }
                        _ => {
                            let n = snapshot.own_audio.subscribers;
                            let plural = if n == 1 { "" } else { "s" };
                            ui.weak(format!("capturing · {n} listener{plural}"));
                        }
                    },
```

- [ ] **Step 5: Draw and apply it from the window**

In `crates/app/src/window.rs`, import the module (`use crate::ui::applications::{self as applications_ui, PickerOutcome};`), then in `redraw_main` add an accumulator beside `saved`:

```rust
        let mut picked = None;
```

inside the `ui.run` closure, after the `settings_ui::draw` block:

```rust
            // Accumulated outside the closure for the same reason `saved` is: egui may run this
            // pass twice, and Done closes the picker, so the second run would produce nothing.
            if let Some(outcome) = applications_ui::draw(root.ctx(), &mut self.state) {
                picked = Some(outcome);
            }
```

and after the existing command application, before the settings-dialog save block:

```rust
        if let Some(outcome) = picked {
            let (command, persist) = match outcome {
                PickerOutcome::Refresh => (RoomCommand::ChooseApplications, false),
                PickerOutcome::Applied(applications) => {
                    let command = RoomCommand::SetAudioApplications(applications.to_selection());
                    self.store.settings.audio.applications = applications;
                    (command, true)
                }
            };
            let stored = self.store.settings.audio.applications.clone();
            if let Phase::Room(view) = &mut self.phase {
                view.apply(
                    vec![command],
                    &self.runtime,
                    &self.proxy,
                    &mut self.state,
                    &stored,
                );
            }
            // Saved after the command is applied, because `RoomView::apply` clears the status line
            // and a save failure belongs on it. A failure is a status line, not a blocked
            // selection: the choice already applies for this session.
            if persist
                && let Err(error) = self.store.save_unless_load_failed()
            {
                self.state.status = format!("settings not saved: {error}");
            }
            if let Some(main) = &self.main {
                main.window.request_redraw();
            }
        }
```

`RoomCommand` needs importing in `window.rs` if it is not already there (`use crate::commands::{RoomCommand, WindowCommand};`).

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p brp`
Expected: PASS.

- [ ] **Step 7: Look at it**

Run: `cargo run -p brp`
Then: create a room, click "Choose applications…", and confirm by eye that the window lists what is playing, that the check-list is disabled under "All applications", that Refresh keeps a mid-edit draft, and that Esc and Cancel close it without changing the summary line. Done should change the summary to "N applications" and write `[audio.applications]` into the settings file:

```bash
cat "${XDG_CONFIG_HOME:-$HOME/.config}/brp/settings.toml"
```

Expected: `mode = "only"` and the names chosen.

- [ ] **Step 8: Verify the workspace and commit**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: PASS.

```bash
git add crates/app/src
git commit -m "feat(app): choose which applications the room hears from the own-lives panel"
```

---

### Task 9: Documentation, the spec amendment, and the manual verification

**Files:**
- Modify: `README.md`
- Modify: `docs/superpowers/specs/2026-09-08-phase6-per-application-audio-design.md`

**Interfaces:**
- Consumes: everything above.
- Produces: nothing in code.

- [ ] **Step 1: The manual two-instance verification**

This is the phase's real verification and it needs a human's ears. Run two instances on the dev machine, one publishing and one watching, and work through spec section 11's list. Present the results to the user and ask them to confirm each item; do not mark any of them passed on your own.

```bash
cargo build --release -p brp
# terminal one, publisher: create a room, share a monitor, keep share audio on
./target/release/brp
# terminal two, viewer: join with the ticket the first printed, watch the live
./target/release/brp
```

1. Select one noisy application; confirm the viewer hears only it while a second noisy application stays audible locally.
2. Flip to all applications, confirm both are heard, flip back.
3. Close a selected application; confirm its row persists as "not playing" and can still be deselected.
4. Launch a selected application after capture has started; confirm it becomes audible with no picker interaction.
5. Confirm the second brp instance is absent from the picker and its playback never returns as echo.
6. Restart brp and confirm the selection survived.

Record which items the user confirmed. If any fails, stop and debug with `superpowers:systematic-debugging` rather than adjusting the README's claim.

- [ ] **Step 2: README**

Add a phase 6 entry to the roadmap, after the phase 5 entry, in the same voice and hedged by whatever step 1 actually established:

```markdown
6. **Per-application audio** — done on Linux; Windows pending hardware: share
   every application except brp, which stays the default, or only the
   applications you select, stored by executable name so the choice survives
   restarts.
```

In the usage section's audio paragraph, add the button and the picker: "Choose applications…" beside the share-audio checkbox opens a window listing what is playing, with two radio buttons — all applications except brp, or only the ones you tick — applied on Done and remembered in `settings.toml`.

Remove `per-application audio` from the backlog sentence, leaving the three phase 6 non-goals (`a send volume per shared application`, `viewer-side visibility of which applications a publisher is mixing`, `microphone capture and voice chat behind an echo-cancellation dependency`, `a self-updating application list in the audio picker`) where they already are.

Add the links beside the phase 5 links:

```markdown
Phase 6 is designed in
[`docs/superpowers/specs/2026-09-08-phase6-per-application-audio-design.md`](docs/superpowers/specs/2026-09-08-phase6-per-application-audio-design.md);
the Linux half is implemented by
[`2026-09-08-plan6a-per-application-audio-linux.md`](docs/superpowers/plans/2026-09-08-plan6a-per-application-audio-linux.md).
```

- [ ] **Step 3: Spec amendment**

Append an "Amendments from the implementation run" section to the phase 6 spec, recording at least:

- **The open dependency question (5.8) is answered.** Every PipeWire Client global on the dev machine carries `application.process.binary` and `application.name`, so `/proc/<pid>/exe` stayed a fallback. Two observations from the probe: a client that reaches the daemon through `pipewire-pulse` carries the *pulse daemon's* `pipewire.sec.pid`, so the `/proc` fallback can name `pipewire-pulse` rather than the application; and a node's own properties often lack the binary while its client carries it, which is why identity is resolved through the client global.
- **Constants (12).** `AUDIO_SESSION_POLL_INTERVAL` and `CAPTURE_MIX_MAX_LAG` are Windows-only and land with plan 6b, so plan 6a added only `AUDIO_SOURCE_LIST_TIMEOUT`.
- **Enumeration (5.2).** The listing feeds its graph after the roundtrip rather than during it, because a Node global can be announced before the Client global it names.
- **The picker's plumbing (5.7).** The picker window is drawn from `window.rs` beside the settings dialog rather than from `ui::draw`, and returns its outcome instead of pushing a command: egui may run a pass twice, and Done closes the picker, so a command pushed from inside the pass would be discarded with it. `RoomCommand::SetAudioApplications` and `RoomCommand::ChooseApplications` are still what carry the change into the room. Esc dismisses the picker through the main window's context, which has no fullscreen handler to leak into — the pop-out Esc rule lives in its own context.
- Anything else the run changed.

- [ ] **Step 4: Commit**

```bash
git add README.md docs/superpowers/specs/2026-09-08-phase6-per-application-audio-design.md
git commit -m "docs: describe per-application audio and record the phase 6 Linux run"
```

Push and confirm CI is green on both jobs. The Windows job is the compile oracle for `windows/mod.rs`'s interim answers and for `AppKey`'s Windows-only test.

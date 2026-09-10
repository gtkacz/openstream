# Plan 7: Join links Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Whoever is in a room copies one `https://` link; clicking it on a machine with brp opens the participant window into the room, and on a machine without brp offers the download.

**Architecture:** A pure `app::link` module builds the share link and strips the two link prefixes before the existing `RoomTicket` parser, and every ticket entry point (the `join` command, `publish --ticket`, the start screen) goes through it. A new `app::handler` module registers the running binary as the `brp://` handler on a detached thread each time the participant window launches: a hidden `.desktop` entry plus `xdg-mime default` on Linux, current-user registry class keys on Windows. A `join` argument that fails to parse opens the start screen with the error instead of printing to a console a browser never gave us. One static page under `site/join/` forwards the fragment to `brp://join/<ticket>` and is deployed to GitHub Pages by a new workflow.

**Tech Stack:** Rust 2024, clap 4, egui 0.36, iroh-tickets 1.0, windows-sys 0.61 (`Win32_System_Registry`), directories 6, xdg-utils, GitHub Pages via `actions/deploy-pages`.

**Spec:** `docs/superpowers/specs/2026-09-10-phase7-join-links-design.md`. Read it first; the master spec `docs/superpowers/specs/2026-09-04-p2p-screen-sharing-design.md` applies where it is silent.

## Global Constraints

- **Link constants, verbatim.** `JOIN_PAGE` is `https://gtkacz.github.io/openstream/join/`; `SCHEME_JOIN_PREFIX` is `brp://join/`; the desktop entry is `brp.desktop`; the page's fallback delay is 1500 ms. The download button points at `https://github.com/gtkacz/openstream/releases/latest`.
- **`share_link` takes `&str`, not `&RoomTicket`.** Both callers (the status bar's `RoomView.ticket`, `publish`'s `room.ticket().to_string()`) already hold the string form. Task 7 records this deviation in the spec.
- **No new workspace dependency.** The only manifest change is the `Win32_System_Registry` feature on the existing `windows-sys` entry in the workspace `Cargo.toml`.
- **Validation stays in `proto`.** `link::parse_ticket` only strips prefixes; `RoomTicket::from_str` decides what is a ticket. There is no second error type; every failure is `iroh_tickets::ParseError`, shown as `AppError::Ticket` (`invalid ticket: …`).
- **Registration never blocks or fails the launch.** It runs on a detached `std::thread`, only from `participant::run`, never from `publish`, and every failure is `tracing::warn!`.
- **Platform code stays behind `cfg`.** Linux code under `#[cfg(target_os = "linux")]`, Windows under `#[cfg(windows)]`, and every other target gets a no-op. `cargo clippy --workspace --all-targets -- -D warnings` must pass on Linux at every commit. The Windows half is compiled by the Windows CI job; `cargo check -p brp --target x86_64-pc-windows-msvc` may fail in `ffmpeg-sys-next`'s build script on this machine, which is expected, not a defect. Never claim a Windows runtime check ran.
- **The page loads nothing external and sends nothing.** Inline CSS and JS only, `<meta name="referrer" content="no-referrer">`, `<meta name="robots" content="noindex">`, no analytics. The ticket is read from `location.hash` and never put in a query string or a request.
- Comments explain why. Doc comments state contracts on new public items. No task ids, branch names, or ticket numbers in code.
- One Conventional Commit per task, imperative subject. `cargo fmt --all` and `cargo clippy --workspace --all-targets -- -D warnings` pass on Linux before each commit, and `cargo test --workspace` passes at every commit. Before committing run `git status --short`; if `.vscode/` files appear staged, run `git rm --cached -r .vscode` first (recurring environment quirk). Commit messages end with the `Claude-Session:` trailer the harness requires; no other trailers, no co-author lines.

## File Structure

```
Cargo.toml                                  + "Win32_System_Registry" on windows-sys

crates/app/src/link.rs                      new: JOIN_PAGE, SCHEME_JOIN_PREFIX, share_link(), parse_ticket()
crates/app/src/handler.rs                   new: register_scheme_handler(); linux { desktop_entry, install, register }; windows { class_values, register }
crates/app/src/lib.rs                       + pub mod handler; pub mod link
crates/app/src/main.rs                      join parses through link; a bad argument opens the window with the error
crates/app/src/participant.rs               run() gains start_error; calls handler::register_scheme_handler()
crates/app/src/window.rs                    App::new gains start_error; both start-screen errors go through show_error
crates/app/src/publish.rs                   --ticket parses through link; prints the share link; `brp watch` hint corrected
crates/app/src/ui/start.rs                  submit() parses through link; show_error(); hint text; tests
crates/app/src/ui/status.rs                 Copy link button

site/join/index.html                        new: the join page
.github/workflows/pages.yml                 new: deploys site/ to GitHub Pages

README.md                                   usage, handler registration, security sentence, roadmap item 7, links
docs/superpowers/specs/2026-09-10-phase7-join-links-design.md   section 15 amendments
```

---

### Task 1: The link module

**Files:**
- Create: `crates/app/src/link.rs`
- Modify: `crates/app/src/lib.rs`

**Interfaces:**
- Consumes: `brp_proto::RoomTicket` (`FromStr<Err = iroh_tickets::ParseError>`, `Display`).
- Produces: `link::JOIN_PAGE: &str`, `link::SCHEME_JOIN_PREFIX: &str`, `link::share_link(ticket: &str) -> String`, `link::parse_ticket(input: &str) -> Result<RoomTicket, iroh_tickets::ParseError>`.

- [ ] **Step 1: Write the module with its tests, implementation left as `todo!()`**

Create `crates/app/src/link.rs`:

```rust
//! Join links: the share link the status bar copies, and the three forms every ticket entry
//! point accepts: a raw ticket, the `brp://` URL the operating system hands the binary, and the
//! share link itself.

use std::str::FromStr;

use brp_proto::RoomTicket;
use iroh_tickets::ParseError;

/// The static page that forwards a share link to the `brp://` handler.
pub const JOIN_PAGE: &str = "https://gtkacz.github.io/openstream/join/";
/// What the page hands the operating system, and so what the binary receives from a click.
pub const SCHEME_JOIN_PREFIX: &str = "brp://join/";

/// The share link for a ticket: the join page with the ticket in the fragment, which browsers
/// never send to the server.
pub fn share_link(ticket: &str) -> String {
    todo!()
}

/// A raw ticket, a scheme link, or a share link, whitespace-trimmed, to a ticket. Only the known
/// prefixes are stripped; whatever remains is judged by the ticket parser, so an empty or foreign
/// remainder fails exactly as a bad pasted ticket does.
pub fn parse_ticket(input: &str) -> Result<RoomTicket, ParseError> {
    todo!()
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use iroh::{EndpointAddr, SecretKey};

    use super::*;

    fn ticket() -> RoomTicket {
        let id = SecretKey::from_bytes(&[7u8; 32]).public();
        let addr = EndpointAddr::new(id).with_ip_addr(SocketAddr::from(([192, 168, 1, 10], 4433)));
        RoomTicket::new([1u8; 32], vec![addr])
    }

    #[test]
    fn every_link_form_parses_to_the_same_ticket() {
        let expected = ticket();
        let text = expected.to_string();
        for input in [
            text.clone(),
            format!("  {text}\n"),
            format!("brp://join/{text}"),
            format!("brp://join/{text}/"),
            format!("https://gtkacz.github.io/openstream/join/#{text}"),
            format!("https://gtkacz.github.io/openstream/join#{text}"),
        ] {
            assert_eq!(parse_ticket(&input).unwrap(), expected, "{input}");
        }
    }

    #[test]
    fn links_without_a_ticket_fail_like_a_bad_ticket() {
        for input in [
            "",
            "not a ticket",
            "brp://join/",
            "https://gtkacz.github.io/openstream/join/",
            "https://gtkacz.github.io/openstream/join/#",
            "https://example.com/#brpaaaa",
        ] {
            assert!(parse_ticket(input).is_err(), "{input}");
        }
    }

    #[test]
    fn the_share_link_round_trips_through_the_parser() {
        let expected = ticket();
        let link = share_link(&expected.to_string());
        assert!(
            link.starts_with("https://gtkacz.github.io/openstream/join/#brp"),
            "{link}"
        );
        assert_eq!(parse_ticket(&link).unwrap(), expected);
    }
}
```

In `crates/app/src/lib.rs`, add `pub mod link;` between `pub mod launch;` and `pub mod logging;` (the list is alphabetical).

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p brp link::`
Expected: three failures, each panicking at `not yet implemented`.

- [ ] **Step 3: Implement**

Replace the two `todo!()` bodies and add the private helper:

```rust
pub fn share_link(ticket: &str) -> String {
    format!("{JOIN_PAGE}#{ticket}")
}

pub fn parse_ticket(input: &str) -> Result<RoomTicket, ParseError> {
    RoomTicket::from_str(bare_ticket(input.trim()))
}

/// Strips a share-link or scheme-link prefix; anything else is returned as is.
fn bare_ticket(input: &str) -> &str {
    // Both `…/join/#t` and `…/join#t` are accepted: a redirect may drop the trailing slash.
    if let Some(rest) = input.strip_prefix(JOIN_PAGE.trim_end_matches('/')) {
        return rest.split_once('#').map_or("", |(_, fragment)| fragment);
    }
    if let Some(rest) = input.strip_prefix(SCHEME_JOIN_PREFIX) {
        return rest.trim_end_matches('/');
    }
    input
}
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p brp link::`
Expected: 3 passed.

- [ ] **Step 5: Format, lint, commit**

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
git status --short
git add crates/app/src/link.rs crates/app/src/lib.rs
git commit -m "feat(app): parse tickets from join links" -m "Claude-Session: https://claude.ai/code/session_0187tkPrKa1jRZWcMCACnkko"
```

---

### Task 2: Every ticket entry point accepts a link, and the link is offered

**Files:**
- Modify: `crates/app/src/ui/start.rs` (imports, `submit`, hint text, tests)
- Modify: `crates/app/src/ui/status.rs` (Copy link button)
- Modify: `crates/app/src/publish.rs` (imports, `--ticket`, the printed block)
- Modify: `crates/app/src/main.rs` (imports, the `Join` arm)

**Interfaces:**
- Consumes: `link::parse_ticket`, `link::share_link` from Task 1.
- Produces: nothing new; `StartState::submit` behaviour widens to links.

- [ ] **Step 1: Add the failing start-screen test**

In `crates/app/src/ui/start.rs`, inside `mod tests`, after `join_with_a_valid_ticket_yields_the_join_intent`:

```rust
    #[test]
    fn join_with_a_share_link_or_scheme_link_yields_the_join_intent() {
        let ticket = valid_ticket();
        for pasted in [
            format!("https://gtkacz.github.io/openstream/join/#{ticket}"),
            format!("brp://join/{ticket}"),
        ] {
            let mut state = StartState::new("alice".into());
            state.ticket = pasted.clone();
            assert_eq!(
                state.submit(StartAction::Join),
                Some(Intent::Join(ticket.clone())),
                "{pasted}"
            );
        }
    }
```

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test -p brp ui::start::tests::join_with_a_share_link`
Expected: FAIL, `submit` returned `None` because `RoomTicket::from_str` rejects the `https` prefix.

- [ ] **Step 3: Route the start screen through the link parser**

In `crates/app/src/ui/start.rs`:

Replace the imports at the top:

```rust
use crate::launch::Intent;
use crate::link;
use crate::settings::RecentRoom;
```

(remove `use std::str::FromStr;` and `use brp_proto::RoomTicket;`; the tests module still needs the ticket type, so add `use brp_proto::RoomTicket;` inside `mod tests` after `use iroh::{EndpointAddr, SecretKey};`).

In `submit`, replace

```rust
            StartAction::Join => match RoomTicket::from_str(self.ticket.trim()) {
```

with

```rust
            StartAction::Join => match link::parse_ticket(&self.ticket) {
```

In `draw`, change the hint:

```rust
                    .hint_text("paste a ticket or link")
```

- [ ] **Step 4: Run the start-screen tests to see them pass**

Run: `cargo test -p brp ui::start::`
Expected: all pass, including the new one.

- [ ] **Step 5: Add the Copy link button**

In `crates/app/src/ui/status.rs`, add `use crate::link;` after `use crate::commands::RoomCommand;`, and change the module doc's first line to `//! Bottom status bar: ticket and link, member count, upload rate, identity, last notice.`. Replace

```rust
            if ui.button("Copy ticket").clicked() {
                ui.ctx().copy_text(ticket.to_string());
            }
```

with

```rust
            if ui.button("Copy link").clicked() {
                ui.ctx().copy_text(link::share_link(ticket));
            }
            if ui.button("Copy ticket").clicked() {
                ui.ctx().copy_text(ticket.to_string());
            }
```

Update the function's doc comment to say `The two copy buttons write to the clipboard directly rather than queueing a `RoomCommand``.

- [ ] **Step 6: Route `publish --ticket` through the link parser and print the link**

In `crates/app/src/publish.rs`:

Remove `use std::str::FromStr;`. Change `use brp_proto::{RoomTicket, SourceKind};` to `use brp_proto::SourceKind;`. Add `use crate::link;` after `use crate::identity;`.

Replace

```rust
        Some(ticket) => Room::join(config, RoomTicket::from_str(ticket)?).await?,
```

with

```rust
        Some(ticket) => Room::join(config, link::parse_ticket(ticket)?).await?,
```

Replace the ticket `println!`:

```rust
    let ticket = room.ticket().to_string();
    println!(
        "Ticket:\n{ticket}\n\nLink:\n{}\n\nShare either: brp join <ticket-or-link>. Press Ctrl-C to stop.",
        link::share_link(&ticket)
    );
```

(The old text said `brp watch <ticket>`; there is no `watch` command.)

- [ ] **Step 7: Route `brp join` through the link parser**

In `crates/app/src/main.rs`, remove `use std::str::FromStr;` and `use brp_proto::RoomTicket;`, add `use brp_app::link;` after `use brp_app::launch::Intent;`, and replace

```rust
        Some(Command::Join(args)) => match RoomTicket::from_str(&args.ticket) {
```

with

```rust
        Some(Command::Join(args)) => match link::parse_ticket(&args.ticket) {
```

- [ ] **Step 8: Build, test, lint**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p brp`
Expected: clean, all tests pass. If clippy reports an unused import, remove exactly that import.

- [ ] **Step 9: Commit**

```bash
git status --short
git add crates/app/src/ui/start.rs crates/app/src/ui/status.rs crates/app/src/publish.rs crates/app/src/main.rs
git commit -m "feat(app): accept join links everywhere a ticket is and offer Copy link" -m "Claude-Session: https://claude.ai/code/session_0187tkPrKa1jRZWcMCACnkko"
```

---

### Task 3: A bad `join` argument opens the start screen with the error

**Files:**
- Modify: `crates/app/src/ui/start.rs` (`show_error`, test)
- Modify: `crates/app/src/window.rs:112-149` (`App::new`)
- Modify: `crates/app/src/participant.rs:17-27` (`run`)
- Modify: `crates/app/src/main.rs` (the match)

**Interfaces:**
- Consumes: `AppError::Ticket(ParseError)` whose `Display` is `invalid ticket: …`.
- Produces: `StartState::show_error(&mut self, message: &str)`; `participant::run(runtime: &Runtime, intent: Option<Intent>, start_error: Option<String>, args: WindowArgs)`; `App::new(runtime, proxy, args, secret, nickname, intent, start_error: Option<String>, store)`.

- [ ] **Step 1: Write the failing test for `show_error`**

In `crates/app/src/ui/start.rs` `mod tests`, after `a_failure_returns_to_the_form_with_the_message`:

```rust
    #[test]
    fn show_error_keeps_an_earlier_message_on_its_own_line() {
        let mut state = StartState::new("alice".into());
        state.show_error("settings not loaded");
        assert_eq!(state.error, "settings not loaded");
        state.show_error("invalid ticket: bad kind");
        assert_eq!(state.error, "settings not loaded\ninvalid ticket: bad kind");
    }
```

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test -p brp ui::start::tests::show_error`
Expected: compile error, no method `show_error`.

- [ ] **Step 3: Implement `show_error`**

In `impl StartState`, after `failed`:

```rust
    /// Adds a message to the error line. An earlier message stays on its own line: the settings
    /// load error and a bad link can both be true at launch, and neither should hide the other.
    pub fn show_error(&mut self, message: &str) {
        if !self.error.is_empty() {
            self.error.push('\n');
        }
        self.error.push_str(message);
    }
```

- [ ] **Step 4: Run it to see it pass**

Run: `cargo test -p brp ui::start::`
Expected: all pass.

- [ ] **Step 5: Thread `start_error` through `App::new`**

In `crates/app/src/window.rs`, change the `App::new` signature and body. The doc comment becomes:

```rust
    /// An `intent` from the command line opens the room at once behind the connecting start
    /// screen; `None` waits for the user. A `start_error` is shown on the start screen: it is how
    /// a `join` argument that did not parse reaches a user whose browser gave us no console.
    pub fn new(
        runtime: Handle,
        proxy: EventLoopProxy<AppEvent>,
        args: WindowArgs,
        secret: SecretKey,
        nickname: String,
        intent: Option<Intent>,
        start_error: Option<String>,
        store: SettingsStore,
    ) -> Self {
```

and replace

```rust
        if let Some(message) = &app.store.load_error {
            app.start.error = format!("settings not loaded, defaults in use: {message}");
        }
```

with

```rust
        if let Some(message) = &app.store.load_error {
            app.start
                .show_error(&format!("settings not loaded, defaults in use: {message}"));
        }
        if let Some(message) = &start_error {
            app.start.show_error(message);
        }
```

- [ ] **Step 6: Thread it through `participant::run`**

In `crates/app/src/participant.rs`, replace the function's doc comment and signature:

```rust
/// Runs the window to completion. `intent` from the command line opens the room immediately;
/// `None` shows the start screen, with `start_error` on it when the command line had a ticket
/// that did not parse.
pub fn run(
    runtime: &Runtime,
    intent: Option<Intent>,
    start_error: Option<String>,
    args: WindowArgs,
) -> Result<(), AppError> {
```

and pass it to `App::new`:

```rust
    let mut app = App::new(
        runtime.handle().clone(),
        proxy,
        args,
        secret,
        nickname,
        intent,
        start_error,
        store,
    );
```

- [ ] **Step 7: Make `main` open the window on a bad argument**

In `crates/app/src/main.rs`, replace the `match cli.command` block:

```rust
    let result = match cli.command {
        None => participant::run(&runtime, None, None, WindowArgs::default()),
        Some(Command::Publish(args)) => runtime.block_on(publish::run(args)),
        Some(Command::Create(args)) => {
            participant::run(&runtime, Some(Intent::Create), None, args.window)
        }
        Some(Command::Join(args)) => match link::parse_ticket(&args.ticket) {
            Ok(ticket) => {
                participant::run(&runtime, Some(Intent::Join(ticket)), None, args.window)
            }
            // A browser launches the binary without a console, so the error must reach the
            // window, not stderr.
            Err(error) => participant::run(
                &runtime,
                None,
                Some(AppError::Ticket(error).to_string()),
                args.window,
            ),
        },
    };
```

`AppError` stays imported; it is still used here.

- [ ] **Step 8: Build, test, lint**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p brp`
Expected: clean.

- [ ] **Step 9: Check by hand that the window shows the error**

Run: `cargo run -p brp -- join not-a-ticket`
Expected: the start screen opens with `invalid ticket: …` in red under the form and no error on the terminal. Close the window.

- [ ] **Step 10: Commit**

```bash
git status --short
git add crates/app/src/ui/start.rs crates/app/src/window.rs crates/app/src/participant.rs crates/app/src/main.rs
git commit -m "feat(app): show a bad join argument on the start screen" -m "Claude-Session: https://claude.ai/code/session_0187tkPrKa1jRZWcMCACnkko"
```

---

### Task 4: The Linux handler and the registration call

**Files:**
- Create: `crates/app/src/handler.rs`
- Modify: `crates/app/src/lib.rs`
- Modify: `crates/app/src/participant.rs` (one call)

**Interfaces:**
- Consumes: `directories::BaseDirs::new()?.data_local_dir()`, `std::env::current_exe()`.
- Produces: `handler::register_scheme_handler()`; on Linux `handler::DESKTOP_FILE: &str`, `handler::desktop_entry(exe: &Path) -> String`, `handler::install(dir: &Path, exe: &Path) -> io::Result<bool>`.

- [ ] **Step 1: Write the module with the Linux tests, implementation left as `todo!()`**

Create `crates/app/src/handler.rs`:

```rust
//! Registering the binary as the handler for `brp://` links, so a click in a browser opens the
//! participant window. There is no installer: the window registers itself on every launch,
//! pointing at whatever binary is running, so a moved or upgraded binary stays reachable. A
//! failure is a warning, never a reason to keep the window from opening.

use std::io;

#[cfg(target_os = "linux")]
pub use linux::{DESKTOP_FILE, desktop_entry, install};

/// Registers the running executable for `brp://` links on a detached thread. `xdg-mime` can be
/// slow or absent and a registry can be locked by policy, so nothing waits on the outcome and
/// every problem is logged at warn.
pub fn register_scheme_handler() {
    let spawned = std::thread::Builder::new()
        .name("scheme-handler".into())
        .spawn(|| {
            if let Err(error) = register() {
                tracing::warn!(%error, "could not register the brp:// link handler");
            }
        });
    if let Err(error) = spawned {
        tracing::warn!(%error, "could not start the brp:// link handler registration");
    }
}

#[cfg(target_os = "linux")]
fn register() -> io::Result<()> {
    linux::register()
}

#[cfg(not(target_os = "linux"))]
fn register() -> io::Result<()> {
    Ok(())
}

#[cfg(target_os = "linux")]
mod linux {
    use std::fs;
    use std::io;
    use std::path::Path;
    use std::process::Command;

    /// The handler-only desktop entry, under the user's applications directory.
    pub const DESKTOP_FILE: &str = "brp.desktop";
    const SCHEME_MIME: &str = "x-scheme-handler/brp";

    /// The desktop entry that hands `brp://` links to `exe`. `NoDisplay` keeps it out of
    /// launchers: started from a menu without a URL, `brp join` is a usage error.
    pub fn desktop_entry(exe: &Path) -> String {
        todo!()
    }

    /// Writes the entry into `dir` when its content differs from what is there, creating the
    /// directory if needed. Returns whether it wrote.
    pub fn install(dir: &Path, exe: &Path) -> io::Result<bool> {
        todo!()
    }

    pub fn register() -> io::Result<()> {
        todo!()
    }

    #[cfg(test)]
    mod tests {
        use std::path::PathBuf;

        use super::*;

        fn temp_dir(name: &str) -> PathBuf {
            let dir = std::env::temp_dir()
                .join(format!("brp-handler-test-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            dir
        }

        #[test]
        fn the_entry_is_a_hidden_handler_for_the_scheme() {
            let entry = desktop_entry(Path::new("/opt/brp/brp"));
            assert!(entry.starts_with("[Desktop Entry]\n"), "{entry}");
            assert!(entry.contains("\nExec=\"/opt/brp/brp\" join %u\n"), "{entry}");
            assert!(entry.contains("\nNoDisplay=true\n"), "{entry}");
            assert!(entry.contains("\nMimeType=x-scheme-handler/brp;\n"), "{entry}");
            assert!(entry.contains("\nTerminal=false\n"), "{entry}");
        }

        #[test]
        fn the_exec_path_is_quoted_and_escaped() {
            let entry = desktop_entry(Path::new("/opt/my apps/brp$1"));
            assert!(
                entry.contains("\nExec=\"/opt/my apps/brp\\\\$1\" join %u\n"),
                "{entry}"
            );
        }

        #[test]
        fn install_writes_once_and_again_when_the_exe_moves() {
            let dir = temp_dir("install");
            let path = dir.join(DESKTOP_FILE);
            assert!(install(&dir, Path::new("/a/brp")).unwrap());
            assert_eq!(fs::read_to_string(&path).unwrap(), desktop_entry(Path::new("/a/brp")));
            assert!(!install(&dir, Path::new("/a/brp")).unwrap());
            assert!(install(&dir, Path::new("/b/brp")).unwrap());
            assert_eq!(fs::read_to_string(&path).unwrap(), desktop_entry(Path::new("/b/brp")));
            let _ = fs::remove_dir_all(&dir);
        }
    }
}
```

In `crates/app/src/lib.rs`, add `pub mod handler;` between `pub mod error;` and `pub mod identity;`.

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p brp handler::`
Expected: three failures at `not yet implemented`.

- [ ] **Step 3: Implement the Linux half**

Replace the three `todo!()` bodies in `mod linux` and add the quoting helper:

```rust
    pub fn desktop_entry(exe: &Path) -> String {
        format!(
            "[Desktop Entry]\nType=Application\nName=brp\nComment=Peer-to-peer screen sharing\n\
             Exec={} join %u\nTerminal=false\nNoDisplay=true\nMimeType={SCHEME_MIME};\n",
            quote_exec(exe)
        )
    }

    /// Quotes a path for an `Exec` key. The desktop entry specification applies two rules in
    /// turn: the quoting rule wants `"`, `` ` ``, `$`, and `\` preceded by a backslash inside the
    /// quotes, and the string rule then wants each backslash itself doubled, so the file carries
    /// two backslashes before those characters and four for a literal backslash.
    fn quote_exec(path: &Path) -> String {
        let mut quoted = String::from("\"");
        for c in path.to_string_lossy().chars() {
            match c {
                '"' | '`' | '$' => quoted.push_str("\\\\"),
                '\\' => quoted.push_str("\\\\\\"),
                _ => {}
            }
            quoted.push(c);
        }
        quoted.push('"');
        quoted
    }

    pub fn install(dir: &Path, exe: &Path) -> io::Result<bool> {
        let path = dir.join(DESKTOP_FILE);
        let entry = desktop_entry(exe);
        if fs::read_to_string(&path).is_ok_and(|current| current == entry) {
            return Ok(false);
        }
        fs::create_dir_all(dir)?;
        fs::write(&path, entry)?;
        Ok(true)
    }

    pub fn register() -> io::Result<()> {
        let exe = std::env::current_exe()?;
        let dirs = directories::BaseDirs::new()
            .ok_or_else(|| io::Error::other("no home directory for the desktop entry"))?;
        let dir = dirs.data_local_dir().join("applications");
        let written = install(&dir, &exe)?;
        tracing::debug!(path = %dir.join(DESKTOP_FILE).display(), written, "desktop entry checked");
        // The default is claimed on every launch, not only after a write: another application or
        // a fresh desktop session can reset mimeapps.list while the entry itself is unchanged.
        let status = Command::new("xdg-mime")
            .args(["default", DESKTOP_FILE, SCHEME_MIME])
            .status()?;
        if !status.success() {
            return Err(io::Error::other(format!("xdg-mime default exited with {status}")));
        }
        Ok(())
    }
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p brp handler::`
Expected: 3 passed.

- [ ] **Step 5: Call it from the participant window**

In `crates/app/src/participant.rs`, add `use crate::handler;` after `use crate::error::AppError;`, and as the first line of `run`'s body, before `let store = SettingsStore::load()?;`:

```rust
    handler::register_scheme_handler();
```

- [ ] **Step 6: Build, test, lint**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p brp`
Expected: clean.

- [ ] **Step 7: Check the registration by hand**

```bash
rm -f ~/.local/share/applications/brp.desktop
cargo run -p brp &
sleep 3
cat ~/.local/share/applications/brp.desktop
xdg-mime query default x-scheme-handler/brp
```

Expected: the entry names the debug binary under `target/debug/`, and the query prints `brp.desktop`. Close the window.

- [ ] **Step 8: Commit**

```bash
git status --short
git add crates/app/src/handler.rs crates/app/src/lib.rs crates/app/src/participant.rs
git commit -m "feat(app): register the brp:// link handler on Linux at launch" -m "Claude-Session: https://claude.ai/code/session_0187tkPrKa1jRZWcMCACnkko"
```

---

### Task 5: The Windows handler

**Files:**
- Modify: `Cargo.toml` (workspace `windows-sys` features)
- Modify: `crates/app/src/handler.rs` (Windows module, `register` dispatch)

**Interfaces:**
- Consumes: `windows_sys::Win32::System::Registry::{RegCreateKeyExW, RegSetValueExW, RegCloseKey, HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ}`, `windows_sys::Win32::Foundation::ERROR_SUCCESS`.
- Produces: on Windows `handler::windows::class_values(exe: &Path) -> Vec<ClassValue>` and `handler::windows::register() -> io::Result<()>`.

- [ ] **Step 1: Enable the registry feature**

In the workspace `Cargo.toml`, the `windows-sys` entry becomes:

```toml
windows-sys = { version = "0.61", features = [
    "Win32_Foundation",
    "Win32_System_Console",
    "Win32_System_Diagnostics_ToolHelp",
    "Win32_System_Registry",
    "Win32_System_Threading",
] }
```

- [ ] **Step 2: Add the Windows module and dispatch**

In `crates/app/src/handler.rs`, replace the two `register` dispatch functions with three:

```rust
#[cfg(target_os = "linux")]
fn register() -> io::Result<()> {
    linux::register()
}

#[cfg(windows)]
fn register() -> io::Result<()> {
    windows::register()
}

#[cfg(not(any(target_os = "linux", windows)))]
fn register() -> io::Result<()> {
    Ok(())
}
```

and append the module at the end of the file:

```rust
#[cfg(windows)]
mod windows {
    use std::ffi::OsStr;
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::ptr;

    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
        RegCreateKeyExW, RegSetValueExW,
    };

    /// The per-user class for the scheme; no elevation is needed under HKEY_CURRENT_USER.
    const CLASS_KEY: &str = r"Software\Classes\brp";

    /// One string value under the class: `subkey` relative to the class (empty for the class
    /// itself), `name` of the value (`None` for the default value), and its data.
    pub struct ClassValue {
        pub subkey: &'static str,
        pub name: Option<&'static str>,
        pub data: String,
    }

    /// Everything the shell needs to hand `brp://` links to `exe`: what the scheme is, that it is
    /// a URL protocol, an icon, and the command. Pure, so the shape is checked without a registry.
    pub fn class_values(exe: &Path) -> Vec<ClassValue> {
        let exe = exe.display();
        vec![
            ClassValue { subkey: "", name: None, data: "URL:brp".to_string() },
            ClassValue { subkey: "", name: Some("URL Protocol"), data: String::new() },
            ClassValue { subkey: "DefaultIcon", name: None, data: format!("\"{exe}\",0") },
            ClassValue {
                subkey: r"shell\open\command",
                name: None,
                data: format!("\"{exe}\" join \"%1\""),
            },
        ]
    }

    pub fn register() -> io::Result<()> {
        let exe = std::env::current_exe()?;
        for value in class_values(&exe) {
            let path = if value.subkey.is_empty() {
                CLASS_KEY.to_string()
            } else {
                format!("{CLASS_KEY}\\{}", value.subkey)
            };
            set_string(&path, value.name, &value.data)?;
        }
        Ok(())
    }

    fn wide(text: &str) -> Vec<u16> {
        OsStr::new(text).encode_wide().chain(std::iter::once(0)).collect()
    }

    /// Creates `path` under HKEY_CURRENT_USER if needed and sets one REG_SZ value; `None` names
    /// the key's default value. Overwrites, so the same call is the update path.
    fn set_string(path: &str, name: Option<&str>, data: &str) -> io::Result<()> {
        let path = wide(path);
        let name = name.map(wide);
        let data = wide(data);
        let mut key: HKEY = ptr::null_mut();
        // SAFETY: every pointer is to a live NUL-terminated buffer, or null where the API allows
        // it; the key is closed before every return that follows its creation.
        unsafe {
            let created = RegCreateKeyExW(
                HKEY_CURRENT_USER,
                path.as_ptr(),
                0,
                ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                ptr::null(),
                &mut key,
                ptr::null_mut(),
            );
            if created != ERROR_SUCCESS {
                return Err(io::Error::from_raw_os_error(created as i32));
            }
            // cbData counts bytes including the terminating NUL, as REG_SZ requires.
            let set = RegSetValueExW(
                key,
                name.as_ref().map_or(ptr::null(), |n| n.as_ptr()),
                0,
                REG_SZ,
                data.as_ptr().cast(),
                (data.len() * 2) as u32,
            );
            RegCloseKey(key);
            if set != ERROR_SUCCESS {
                return Err(io::Error::from_raw_os_error(set as i32));
            }
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn the_command_quotes_the_exe_and_passes_the_url() {
            let values = class_values(Path::new(r"C:\Program Files\brp\brp.exe"));
            let command = values
                .iter()
                .find(|v| v.subkey == r"shell\open\command")
                .expect("command value");
            assert_eq!(command.data, r#""C:\Program Files\brp\brp.exe" join "%1""#);
            assert!(values.iter().any(|v| v.name == Some("URL Protocol")));
        }
    }
}
```

- [ ] **Step 3: Lint on Linux, then try the Windows check**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p brp`
Expected: clean; the Windows module is `cfg`-ed out here.

Run: `cargo check -p brp --target x86_64-pc-windows-msvc 2>&1 | tail -20`
Expected: either success, or a failure inside `ffmpeg-sys-next`'s build script about missing FFmpeg for the target. The latter is the known limitation of this machine; the Windows CI job on push is the compile oracle. Any error naming `handler.rs` must be fixed here.

- [ ] **Step 4: Commit**

```bash
git status --short
git add Cargo.toml crates/app/src/handler.rs
git commit -m "feat(windows): register the brp:// link handler in the user registry" -m "Claude-Session: https://claude.ai/code/session_0187tkPrKa1jRZWcMCACnkko"
```

Then check the Windows CI job on the pushed commit before Task 7 claims the phase is done.

---

### Task 6: The join page and its deployment

**Files:**
- Create: `site/join/index.html`
- Create: `.github/workflows/pages.yml`

**Interfaces:**
- Consumes: the fragment format `#<ticket>` from `link::share_link`; the scheme prefix `brp://join/`.
- Produces: the page at `https://gtkacz.github.io/openstream/join/`.

- [ ] **Step 1: Write the page**

Create `site/join/index.html`:

```html
<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="referrer" content="no-referrer">
<meta name="robots" content="noindex">
<title>Join a brp room</title>
<style>
  body { font: 16px/1.5 system-ui, sans-serif; margin: 0; min-height: 100vh; display: grid; place-items: center; background: #111; color: #eee; }
  main { max-width: 32rem; padding: 2rem; }
  h1 { font-size: 1.5rem; margin: 0 0 1rem; }
  p { margin: 0 0 1rem; }
  .actions { display: flex; flex-wrap: wrap; gap: .75rem; margin: 1.5rem 0; }
  a.button, button { font: inherit; padding: .6rem 1rem; border-radius: .4rem; border: 1px solid #555; background: #222; color: #eee; text-decoration: none; cursor: pointer; }
  a.button.primary { background: #3b6cf6; border-color: #3b6cf6; }
  code { display: block; word-break: break-all; padding: .75rem; background: #000; border-radius: .4rem; }
  [hidden] { display: none; }
</style>
</head>
<body>
<main>
  <h1>Join a brp room</h1>
  <section id="opening">
    <p>Opening brp…</p>
  </section>
  <section id="fallback" hidden>
    <p>If brp did not open, install it and click the link again, or paste the ticket into the start screen.</p>
    <div class="actions">
      <a class="button primary" href="https://github.com/gtkacz/openstream/releases/latest">Download brp</a>
      <a class="button" id="retry" href="#">Open in brp</a>
      <button id="copy" type="button">Copy ticket</button>
    </div>
    <code id="ticket"></code>
  </section>
  <section id="no-ticket" hidden>
    <p>This link carries no ticket. Ask someone in the room for a new one.</p>
    <div class="actions">
      <a class="button primary" href="https://github.com/gtkacz/openstream/releases/latest">Download brp</a>
    </div>
  </section>
</main>
<script>
  // Everything after # stays in the browser: the ticket never reaches the server this page is on.
  let ticket = "";
  try { ticket = decodeURIComponent(location.hash.slice(1)); } catch (_) { ticket = ""; }
  const looksLikeTicket = /^brp[a-z2-7]+$/i.test(ticket);
  const show = (id) => { for (const s of document.querySelectorAll("section")) { s.hidden = s.id !== id; } };
  if (!looksLikeTicket) {
    show("no-ticket");
  } else {
    const schemeLink = "brp://join/" + ticket;
    document.getElementById("retry").href = schemeLink;
    document.getElementById("ticket").textContent = ticket;
    document.getElementById("copy").addEventListener("click", () => navigator.clipboard.writeText(ticket));
    location.href = schemeLink;
    // A browser with a handler leaves this page; one without stays and gets the fallback.
    setTimeout(() => show("fallback"), 1500);
  }
</script>
</body>
</html>
```

- [ ] **Step 2: Write the workflow**

Create `.github/workflows/pages.yml`:

```yaml
name: pages
on:
  push:
    branches: [main]
    paths: ["site/**", ".github/workflows/pages.yml"]
permissions:
  contents: read
  pages: write
  id-token: write
concurrency:
  group: pages
  cancel-in-progress: true
jobs:
  deploy:
    runs-on: ubuntu-latest
    environment:
      name: github-pages
      url: ${{ steps.deploy.outputs.page_url }}
    steps:
      - uses: actions/checkout@v4
      - uses: actions/configure-pages@v5
      - uses: actions/upload-pages-artifact@v3
        with:
          path: site
      - id: deploy
        uses: actions/deploy-pages@v4
```

- [ ] **Step 3: Check the page locally, both states**

```bash
python3 -m http.server -d site 8765 &
```

Open `http://localhost:8765/join/` in a browser: expected the "carries no ticket" section with a Download button only. Then open `http://localhost:8765/join/#brpaaaabbbb2222` (a well-formed but meaningless ticket): expected the browser asks how to open `brp://` or, with Task 4's handler registered, launches brp, which shows `invalid ticket: …` on the start screen; the page shows the fallback with Download, Open in brp, Copy ticket, and the ticket text after 1.5 s. Confirm with the browser's network panel that only the page itself was requested. Stop the server with `kill %1`.

- [ ] **Step 4: Commit**

```bash
git status --short
git add site/join/index.html .github/workflows/pages.yml
git commit -m "feat: add the join page and its GitHub Pages deployment" -m "Claude-Session: https://claude.ai/code/session_0187tkPrKa1jRZWcMCACnkko"
```

- [ ] **Step 5: Turn on Pages for the repository, once**

Pages must be told to publish from Actions. This changes repository settings, so confirm with the user before running it:

```bash
gh api -X POST repos/gtkacz/openstream/pages -f build_type=workflow
```

If it answers `409` the site already exists; then `gh api -X PUT repos/gtkacz/openstream/pages -f build_type=workflow`. After the next push to `main`, `gh run list --workflow pages` should show a green run and `https://gtkacz.github.io/openstream/join/` should serve the page.

---

### Task 7: Documentation, spec amendment, and the Linux end-to-end check

**Files:**
- Modify: `README.md` (usage, security, roadmap, links)
- Modify: `docs/superpowers/specs/2026-09-10-phase7-join-links-design.md` (section 15)

- [ ] **Step 1: Usage**

In `README.md`, replace the paragraph that begins `Run \`brp\` with no arguments` with:

```markdown
Run `brp` with no arguments, or double-click `brp.exe` on Windows, to open the
start screen: pick a nickname, then Create room, or paste a ticket or a link and
Join room. The status bar's Copy link button gives you an `https://` link that
opens brp into the room on a machine that has it and offers the download on one
that does not; Copy ticket gives the bare ticket for the terminal. Each time
the window starts it registers brp as the handler for `brp://` links, pointing
at the binary that is running: a hidden `brp.desktop` under
`~/.local/share/applications` on Linux, the `brp` class under
`HKEY_CURRENT_USER\Software\Classes` on Windows. From a terminal the same window
can be opened directly:
```

In the code block below it, change the join line to:

```
# Join a room in the participant window; a ticket, a share link, or a brp:// link
./target/release/brp join <ticket-or-link> [--nickname N] [--fps 60] [--no-relay]
```

and the publish line's `[--ticket <ticket>]` to `[--ticket <ticket-or-link>]`.

- [ ] **Step 2: Security**

In the `## Identity and privacy` section, after the sentence ending `treat the file as you would the tickets themselves.`, insert:

```markdown
A share link carries the ticket in its fragment, which the browser never sends
to the page's server, but browser history, chat logs, and clipboard managers
keep the whole link, so treat a link exactly as you would the ticket. The join
page is one static file on GitHub Pages that loads nothing from third parties.
```

- [ ] **Step 3: Roadmap and links**

After roadmap item 6, add:

```markdown
7. **Join links** — done on Linux, Windows pending its hardware check: one
   `https://` link that opens the app into the room, the app registering itself
   as the `brp://` handler on Windows and Linux, and a static join page on
   GitHub Pages with the download as the fallback.
```

After the paragraph ending `and the spec's section 16 records the Windows implementation.`, add:

```markdown
Phase 7 is designed in
[`docs/superpowers/specs/2026-09-10-phase7-join-links-design.md`](docs/superpowers/specs/2026-09-10-phase7-join-links-design.md)
and implemented by
[`2026-09-10-plan7-join-links.md`](docs/superpowers/plans/2026-09-10-plan7-join-links.md).
```

- [ ] **Step 4: Spec amendment**

Append to `docs/superpowers/specs/2026-09-10-phase7-join-links-design.md`:

```markdown
## 15. Amendments from the implementation run

- `link::share_link` takes `&str`, not `&RoomTicket`: both callers already hold the string form, and the status bar's view stores the ticket as text.
- `StartState::show_error` appends newline-separated, and `App::new` routes both the settings load error and the `start_error` through it, so section 5.3's "appended after the settings load error" is a method rather than a format string.
- The Windows registry values are described by a pure `class_values(exe)` list with one unit test on the command string; the write itself is untested, as section 11 says.
- Windows runtime verification remains outstanding; the Windows CI job compiled the handler.
```

- [ ] **Step 5: The Linux end-to-end check**

With a release or debug build:

```bash
cargo build -p brp
./target/debug/brp create &
```

In the window, click Copy link. Then, from another terminal, with the clipboard's content:

```bash
xdg-open "$(wl-paste 2>/dev/null || xclip -o -selection clipboard)"
```

Expected: the browser opens the join page, asks once whether to open `brp` links with brp, and a second brp window opens already joined, with two members in the status bar. Repeat by pasting the link into Firefox's address bar. Then, with the first window still open, run `xdg-mime query default x-scheme-handler/brp` and confirm `brp.desktop`. Close both windows.

Record any deviation in the spec's section 15 before committing.

- [ ] **Step 6: Commit**

```bash
git status --short
git add README.md docs/superpowers/specs/2026-09-10-phase7-join-links-design.md
git commit -m "docs: describe join links and record the phase 7 run" -m "Claude-Session: https://claude.ai/code/session_0187tkPrKa1jRZWcMCACnkko"
```

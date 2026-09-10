# Phase 7: Join links

Status: approved design, 2026-09-10. Adds a phase to `2026-09-04-p2p-screen-sharing-design.md`, which remains the master spec. Where this document is silent, the master spec applies.

## 1. Goals

- Whoever is in a room hands out one `https://` link. Clicking it on a machine with brp installed opens the participant window straight into the room. On a machine without brp it offers the download.
- The link is clickable wherever tickets are pasted today: chat apps, email, issue trackers. Those only linkify `http(s)` URLs, which is why the link is not a bare `brp://` URL.
- The link works on Windows and Linux from the zip and tarball as shipped, with no installer. The app registers itself as the `brp://` handler.
- The ticket stays as private as it is today. Nothing about it reaches a server.
- Everything above the registry, the file system, and the browser is unit-tested without a display.

## 2. Non-goals

- A single running instance. A link clicked while brp is open starts a second process, as `brp join` does today. See section 10.
- macOS. The link format accommodates it; the bundle plist and the Apple Event that delivers the URL belong to the macOS phase.
- Flatpak and Snap, whose handler registration goes through their own manifests.
- A launcher entry or file associations. The `.desktop` file exists only as a handler.
- Link previews, short links, expiring links, or any server-side component. The page is one static file.
- Runtime verification on Windows hardware, deferred as in phases 3 to 6.

## 3. Decisions and rationale

| Decision | Rationale |
|---|---|
| The share link is an `https://` page that redirects to a `brp://` URL | Slack, Discord, Teams, WhatsApp, and most mail clients linkify only `http(s)`. A bare custom-scheme URL is text the recipient copies by hand, which is barely better than the ticket. Zoom, Discord, and Steam use the same page-then-scheme pattern, and a page gives non-users a download path. |
| The ticket travels in the URL fragment | Browsers never send the fragment to the server, so GitHub sees a request for the page and nothing else. Link unfurlers fetch the page URL without the fragment and get the generic page. |
| The page is hosted on GitHub Pages from this repository | The repository is public and releases already live on GitHub. One workflow deploys one directory; there is no domain to buy or renew. |
| The scheme URL is `brp://join/<ticket>` | The verb leaves room for other actions without a second scheme, and the OS hands the whole URL to the binary as one argument on both platforms. |
| The app registers the handler on every participant-window launch, idempotently | There is no installer to do it. Re-registering on launch keeps the registration pointing at the binary that actually runs after a move or an upgrade. The `publish` command does not register: a headless publisher is not the process a link should open. |
| Registration runs on a detached thread and never fails the launch | `xdg-mime` can be slow or absent and the registry can be locked down by policy. Neither is a reason to keep the window from opening; failures are logged at warn. |
| The `.desktop` file is `NoDisplay=true` | Its `Exec` is `brp join %u`. Launched from a menu without a URL that is a usage error, so the entry is a handler only. |
| Every ticket entry point accepts a raw ticket, the scheme URL, or the share link | A user pastes whatever they were given, and the browser hands the binary the scheme URL. One parser strips the known prefixes and defers to `RoomTicket::from_str`, so validation stays in `proto`. |
| A `join` argument that does not parse opens the start screen with the error shown | A browser launches the binary without a console, so today's `eprintln` would vanish. The start screen already has the error line a bad pasted ticket uses. A terminal typo also gets the window, which is acceptable: the window is the product's interface. |
| Recent rooms keep storing the raw ticket | The link is a presentation of the ticket, not a new identity. Settings and the recent-rooms list are unchanged. |

## 4. Product model additions

- **Share link.** `https://gtkacz.github.io/openstream/join/#<ticket>`, where `<ticket>` is the ticket string unchanged. Produced by the Copy link button in the status bar and printed by `publish`.
- **Scheme link.** `brp://join/<ticket>`. Produced by the page, consumed by the binary. Never shown in the UI.
- **Join page.** One static HTML file. It opens the scheme link, and after a short delay shows the fallback: download brp, retry, or copy the ticket.
- **Handler registration.** Per-user OS state that maps the `brp` scheme to `"<exe>" join "%1"` on Windows and `"<exe>" join %u` on Linux.

## 5. Architecture

### 5.1 `app::link`

New module `crates/app/src/link.rs`, pure:

```
pub const JOIN_PAGE: &str = "https://gtkacz.github.io/openstream/join/";
pub const SCHEME_JOIN_PREFIX: &str = "brp://join/";

/// The share link for a ticket: the join page with the ticket in the fragment.
pub fn share_link(ticket: &RoomTicket) -> String

/// A raw ticket, a scheme link, or a share link, whitespace-trimmed, to a ticket.
pub fn parse_ticket(input: &str) -> Result<RoomTicket, ParseError>
```

`parse_ticket` trims the input, then: if it starts with `JOIN_PAGE` with or without the trailing slash, the ticket is everything after the first `#`, or nothing when there is no `#`; if it starts with `SCHEME_JOIN_PREFIX`, the ticket is the rest with any trailing `/` removed; otherwise the input is the ticket. The remainder goes to `RoomTicket::from_str`, so an empty remainder or garbage yields the same `ParseError` a bad pasted ticket yields today. There is no second error type.

Call sites, all replacing a direct `RoomTicket::from_str`: `main.rs` for `brp join`, `publish.rs` for `--ticket`, and `ui/start.rs` for the Join button.

### 5.2 `app::handler`

New module `crates/app/src/handler.rs`:

```
/// Registers the running executable as the handler for brp:// links, on a detached thread.
/// Failures are logged and never reach the caller.
pub fn register_scheme_handler()
```

Called once from `participant::run`, before the event loop is built. Not called from `publish`.

**Windows** (`#[cfg(windows)]`, via `windows-sys` with the `Win32_System_Registry` feature added to the workspace dependency): under `HKEY_CURRENT_USER\Software\Classes\brp`, set the default value to `URL:brp`, an empty `URL Protocol` string value, `DefaultIcon` to `"<exe>",0`, and `shell\open\command` to `"<exe>" join "%1"`. Keys are created if missing and values overwritten, so the same call is the update path. No elevation is needed for the current-user hive.

**Linux** (`#[cfg(target_os = "linux")]`):

```
/// The desktop entry that hands brp:// links to `exe`. Pure.
pub fn desktop_entry(exe: &Path) -> String

/// Writes `brp.desktop` into `dir` if its content differs. Returns whether it wrote.
pub fn install(dir: &Path, exe: &Path) -> io::Result<bool>
```

`dir` is `BaseDirs::data_local_dir()/applications`, which honours `XDG_DATA_HOME`. The entry:

```
[Desktop Entry]
Type=Application
Name=brp
Comment=Peer-to-peer screen sharing
Exec="<exe>" join %u
Terminal=false
NoDisplay=true
MimeType=x-scheme-handler/brp;
```

The `Exec` path is double-quoted with `"`, `` ` ``, `$`, and `\` backslash-escaped, per the desktop entry specification. After `install`, whether or not it wrote, the thread runs `xdg-mime default brp.desktop x-scheme-handler/brp`; a missing `xdg-mime` or a non-zero exit is a warn. `install` is the tested part; the spawn is not.

**Other targets:** `register_scheme_handler` is a no-op.

### 5.3 `app` entry points

- `main.rs`: `Command::Join` parses through `link::parse_ticket`. On `Err`, the window opens with no intent and the error message, instead of printing and exiting.
- `participant::run` gains `start_error: Option<String>` and passes it to `App::new`, which appends it to `start.error` after the settings load error, newline-separated, so neither message hides the other.
- `publish.rs` prints the share link under the ticket. The stale `brp watch <ticket>` hint on the same line becomes `brp join <ticket>`.

### 5.4 Site and deployment

- `site/join/index.html`: one file, inline CSS and JS, no external resources, `<meta name="referrer" content="no-referrer">`, `<meta name="robots" content="noindex">`.
- `.github/workflows/pages.yml`: on pushes to `main` touching `site/**` or the workflow, upload `site/` as the Pages artifact and deploy it. `permissions: pages: write, id-token: write`, one concurrency group. The repository's Pages source must be set to GitHub Actions once by hand.

## 6. User interface

- **Status bar.** A Copy link button beside Copy ticket. Copy ticket stays for LAN use with `--no-relay` and for the terminal.
- **Start screen.** The ticket field's hint reads "paste a ticket or link". A pasted link joins exactly as a ticket does.
- **Join page.** On load it reads the fragment. If it matches `^brp[a-z2-7]+$` case-insensitively, the page sets `location.href` to the scheme link and, after 1.5 s, reveals the fallback: a Download brp button to the latest GitHub release, an Open in brp button that retries the scheme link, and the ticket in a box with a Copy button for pasting into the start screen. Without a fragment, or with one that does not look like a ticket, the page says the link carries no ticket and shows the download button only.

## 7. Data flow

Publisher clicks Copy link → clipboard holds `JOIN_PAGE#<ticket>` → pasted into a chat → recipient clicks → browser loads the page, fragment stays client-side → page navigates to `brp://join/<ticket>` → browser asks the OS for the `brp` handler → OS runs `"<exe>" join "brp://join/<ticket>"` → `link::parse_ticket` strips the prefix → `Intent::Join(ticket)` → the existing open path.

## 8. Error handling

- A link whose ticket does not parse opens the start screen with `invalid ticket: <reason>` on the error line, the same text a bad pasted ticket produces.
- Handler registration failures are `tracing::warn!` with the OS error. The window opens regardless.
- The page has nothing to fail loudly: a browser with no handler simply stays on the page, which is what the fallback is for.

## 9. Security

- A share link is exactly as sensitive as the ticket it carries. Browser history, chat logs, and clipboard managers will hold it. The README's security section says so.
- The fragment never reaches GitHub. The page loads no third-party resources and sends no referrer.
- Registration writes only to the current user's registry hive or data directory. Nothing runs elevated.

## 10. Known limitations

- **Second instance.** A link clicked while brp is open starts a second process that loads the same identity key. Whether two live endpoints with one key coexist on the relays is unverified; a single-instance hand-off over a local socket or named pipe is the follow-up if they do not.
- **Handler prompt.** Browsers ask once whether to open `brp` links with brp and may offer to remember the choice. That is browser behaviour the page cannot suppress.
- **First launch.** A fresh download cannot open links until the window has run once. The page's download button is the path for that case.

## 11. Testing

Unit, in the hardware-free suite:

- `link`: a raw ticket, the scheme link with and without a trailing slash, the share link with and without the trailing slash before `#`, and surrounding whitespace all parse to the same ticket; garbage, an empty fragment, and a share link with no `#` fail with `ParseError`; `share_link` round-trips through `parse_ticket`.
- `handler` (Linux): `desktop_entry` contains the MIME type, `NoDisplay=true`, and an `Exec` whose path is quoted and escaped for a path with a space and a `$`; `install` writes into a temp directory, returns `true`, returns `false` on the second call, and returns `true` again after the exe path changes.
- `ui::start`: a pasted share link produces `Intent::Join`; a pasted scheme link likewise.
- `App::new` with a `start_error` and a settings load error shows both. Skipped if `App::new` cannot be built without a display; the assignment is then covered by reading.

Not unit-tested: the Windows registry write, the `xdg-mime` spawn, and the page. These are manual.

Manual, Linux: run the window once, confirm `xdg-mime query default x-scheme-handler/brp` prints `brp.desktop`, copy a link from a room in one process, open it in Firefox and in Chromium, confirm a second process joins the room. Confirm the page's fallback appears with the handler unregistered. Windows: the same, deferred to the hardware run.

## 12. Constants added in this phase

| Constant | Where | Value |
|---|---|---|
| `JOIN_PAGE` | `app::link` | `https://gtkacz.github.io/openstream/join/` |
| `SCHEME_JOIN_PREFIX` | `app::link` | `brp://join/` |
| `DESKTOP_FILE` | `app::handler` | `brp.desktop` |
| fallback delay | `site/join/index.html` | 1500 ms |

## 13. Documentation

README: the Usage section describes Copy link and that a link opens the app; a sentence on what registration writes and where; the security paragraph's sentence on links; roadmap item 7; a pointer to this document beside the other phases.

## 14. References

- Desktop Entry Specification, `Exec` quoting and field codes; Shared MIME-info `x-scheme-handler/*`; `xdg-mime default`.
- Microsoft, "Registering an Application to a URI Scheme": `URL Protocol` value and `shell\open\command`.
- GitHub Pages, publishing with a custom GitHub Actions workflow: `actions/upload-pages-artifact`, `actions/deploy-pages`.

## 15. Amendments from the implementation run

- `link::share_link` takes `&str`, not `&RoomTicket`: both callers already hold the string form, and the status bar's view stores the ticket as text.
- `StartState::show_error` appends newline-separated, and `App::new` routes both the settings load error and the `start_error` through it, so section 5.3's "appended after the settings load error" is a method rather than a format string.
- The Windows registry values are described by a pure `class_values(exe)` list with one unit test on the command string; the write itself is untested, as section 11 says.
- Windows runtime verification remains outstanding; the Windows CI job compiled the handler.
- `App::new` gained `#[allow(clippy::too_many_arguments)]`: the `start_error` parameter made eight, the workspace lint gate is `-D warnings`, and the attribute already appears on `ui/tiles.rs` and `room/src/watcher.rs`; a parameter struct would have exceeded the task's footprint in the most bug-prone file.
- The `windows-sys` feature list also gained `Win32_Security`: `RegCreateKeyExW` takes a `SECURITY_ATTRIBUTES` pointer and windows-sys gates it behind that feature; without it the Windows build compiled only through feature unification from other dependencies. Section 5.2's "with the `Win32_System_Registry` feature added" therefore reads as "with the `Win32_Security` and `Win32_System_Registry` features added".

# OpenStream UI Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox syntax for tracking.

**Goal:** Apply the approved dark-only viewing-room direction and user-facing OpenStream name.

**Architecture:** Keep the native egui/wgpu stack and room-command boundaries. Install one shared theme per context, then restyle existing panels and dialogs without changing streaming behavior.

**Tech Stack:** Rust, egui 0.36, winit, wgpu.

**Spec:** [DESIGN.md](../../../DESIGN.md)

## Global Constraints

- Dark mode only, including native window decorations where supported.
- OpenStream is the display name; retain code, binary, config, URL, and protocol names.
- Preserve all existing sharing, watching, audio, update, and settings controls.
- No new runtime dependencies or continuous repaint loops.

## Review Focus

- Narrow windows: wrap controls and constrain text fields.
- Long member/source names: preserve actions and bound content.
- Keyboard operation: retain focus and native widget semantics.
- In-flight operations: preserve disabled states and command dispatch.
- Video: preserve transparent tile canvas and renderer placements.

### Task 1: Native preview and shared theme

- [x] Add an offscreen native UI preview using existing dependencies; capture the baseline.
- [x] Add the dark theme and reusable primary button, surface, and live-status helpers.
- [x] Apply it to every egui context; use native dark window decorations.

### Task 2: Screens and display identity

- [x] Restyle start, room, sharing, members, video overlays, settings, and pickers.
- [x] Rename visible application labels and window titles to OpenStream.
- [x] Preserve command logic and update states.

### Task 3: Verification

- [x] Render start, empty room, populated room, and settings at multiple widths.
- [x] Add behavioral UI checks for form actions and dark-only initialization.
- [x] Run formatting, app tests, workspace tests, and review the final diff.
- [x] Refresh Repowise and check touched-file health; report actual limitations.

## Verification record

- Workspace suite: 372 tests passed across 31 suites. The crash-log test intentionally prints a caught panic; it passed.
- `cargo clippy -p brp --all-targets -- -D warnings`: clean.
- `cargo fmt --all -- --check` and `git diff --check`: clean.
- Offscreen native renders: start, populated room, and settings at 1440, 768, and 390 points; empty room, narrow source/audio pickers, and connecting stream toolbar also inspected.
- UI interaction checks cover a narrow start-screen Create click, narrow settings Save with normalization, keyboard-only stream toolbar entry, visible encoding failures with advanced settings collapsed, and an OS light-theme signal against dark-only initialization.
- Independent code review found one regression: hidden encoding errors. Added a failing regression test, restored visible error/software summaries, and verified it passes.
- Join-page inspector: 0 failures at 1440/768/390. `font-not-dealt` explicitly allowed to retain the existing system font. One accepted empty-space warning: this is a brief join handoff/error utility, with no reason to invent content to fill the viewport.
- Repowise refreshed and touched-file health checked. Its stored history/coverage signals describe the committed baseline, not a measured health improvement from these uncommitted edits.
- Native rendering uses actual egui/wgpu output with isolated fixture data. No live capture session or Windows desktop visual check was performed. The browser inspector does not inspect the native egui UI.

## Final app-screen critique

1. The start screen answers “How do I share or join?” with a name field and the two existing actions; the room reserves its largest region for watched screens.
2. Start-form edges align, settings actions stay visible below a scrolling body, room controls wrap, and the sharing panel scrolls when needed.
3. Surfaces are neutral. Only live status and warnings/errors carry hue. OpenStream's overlapping-screen mark identifies the shell.
4. Counts and telemetry come from the room snapshot and existing meters. Preview-only Alex/Morgan names and encoder figures stay in the example, not production data.
5. Empty rooms explain the next action; upload zero remains zero; no invented activity fills an empty video canvas.
6. There are no charts or synthetic legends. Presets retain their real dimensions, rates, and encoding state.
7. Narrow forms remain usable; the sidebar stays available for stream selection. At very small tile widths, the toolbar scrolls horizontally and remains reachable by Tab.

## Decisions during implementation

- Kept the native bundled proportional face and existing join-page system font, as documented in DESIGN.md; external web-font loading would undermine offline operation.
- Kept advanced encoding controls behind a disclosure, with errors and CPU fallback summaries always outside it.
- Retained existing source, room-command, protocol, executable, URL-scheme, release, and storage identifiers.

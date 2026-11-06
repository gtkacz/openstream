# Voice

Status: design, 2026-11-05. Closes #15. Echo cancellation stays with #30 and noise suppression with #16; this document is the base both build on.

## 1. Intent

The people who use brp talk over Discord while they share. Voice inside brp removes that second application for the same reasons the video path exists: media flows directly between peers, with low latency, no third-party server and no external dependency. Success is a group running a whole session in brp with Discord closed.

## 2. Goals

- A room member can be in the room without being in voice. Joining voice is a deliberate act, and leaving voice keeps you in the room.
- Each person in voice sends their microphone straight to every other person in voice, as a stream separate from the application audio they share.
- You can mute and deafen yourself, and mute and deafen any other person. Every one of those controls is local: it changes only what you send and what you hear.
- Linux, Windows and macOS, through the same layers the existing audio uses.

## 3. Non-goals

- **Echo cancellation.** #30. Until it lands, headphones are what keep a speaker's microphone from returning everyone else.
- **Noise suppression, and any gate that transmits only while you speak.** #16. The first version is an open microphone, and muting is the only way to stop transmitting.
- **Push-to-talk.** A hotkey that works while a game has focus is a separate piece of work on each OS.
- **Room-wide moderation.** Rooms have no owner; nobody can silence anyone for anyone else.
- **Remembering per-person mutes, deafens and volumes across sessions.** Publisher volumes are not remembered today either.
- Recording, voice outside a room, and synchronising voice with video.

## 4. Anti-goals

The feature fails, however well it otherwise works, if any of these happen:

- Voice travels through a server. Relays are acceptable exactly where video already falls back to them.
- Muting your microphone silences the application audio you share, or stopping that share silences your voice.
- Someone you have muted or deafened can tell from brp that you did so.
- The microphone is open while you are out of voice or muted. "Muted" means the device is closed, so the OS microphone indicator goes off, not that silence is sent.
- Voice from other people is picked up by your application-audio share and sent back into the room.
- A peer running an older brp breaks, or its presence stops decoding, because a voice-capable peer is in the room.

## 5. Behaviour

### 5.1 Being in voice

- Opening or joining a room leaves you out of voice. The **Voice** switch in the dock (6.1) puts you in and takes you out.
- A new setting, **Join voice automatically**, sits beside the existing `auto_join_links` and is off by default. When on, entering a room by any route puts you in voice as if you had clicked. That includes a `brp://` link that `auto_join_links` accepts without asking. With both settings on, a link puts you in its room in voice, unmuted. The requester chose this deliberately: both settings are opt-in, and whoever turns both on has asked for exactly that.
- Leaving the room leaves voice.
- Each brp window is its own room and process, and each can be in voice independently, with no coordination between windows. That is the user's choice to make. Being in voice in two windows at once is accepted along with its effects: a second window in the same room plays your voice back to you, and on Windows a window playing voice holds another window's "all applications" share through `watch_other_windows`, as any brp window playing sound already does.
- Every member sees who is in voice, and the self-mute and self-deafen state of each person in voice.

### 5.2 Your own controls

| Control | Effect |
|---|---|
| Mute | Stops transmitting and closes the microphone. Shown to the room. |
| Deafen | You hear no voice from anyone. Deafening also mutes you; undeafening restores the mute state from before you deafened, and unmuting while deafened undeafens too. Shown to the room. |

### 5.3 Controls on another person

| Control | Effect |
|---|---|
| Mute them | You stop hearing their voice. Your brp keeps receiving it and drops it before decoding, so nothing changes on their side. That costs about 32 kbps per person you have muted. Their shared application audio is unaffected and keeps its own slider. |
| Deafen them | They stop hearing you. Instead of your voice they are sent encoded silence at the normal packet rate, so to them you look like someone who is not talking. |
| Voice volume | A gain from 0 to 100% for their voice, the same range as publisher volume, separate from their application-audio volume. |

Mute and deafen on the same person cut voice between you in both directions. None of these are shown to anyone else. They are keyed by the person's key and last as long as you stay in the room, surviving either of you leaving and rejoining voice; leaving the room forgets them.

"Not shown" is a promise about brp's interface and its traffic pattern. A modified client could still infer a deafen from the audio itself, because encoded silence is cleaner than the noise floor of an open microphone. Nothing in brp may surface it.

### 5.4 Other behaviour

- **Speaking indicator.** Each person in voice shows when they are speaking, judged from the audio you actually receive, so it needs no protocol. Your own row shows your microphone level while you transmit. How it is drawn is in 6.3.
- **Devices.** Settings gain a **Microphone** picker and a **Voice output** row whose **Same as audio output** switch is on by default. While it is on, the existing **Audio output** picker sets where both streams' audio and voice play. While it is off, a second picker sets the output device for voice alone, and the existing one keeps setting it for streams' audio. The new values sit next to `audio.output_device`; empty means the system default, and a settings file without them reads as the defaults.
- **When the microphone is open.** Exactly while you are in voice and unmuted, whether or not anyone is subscribed. Tying it to subscribers, as application-audio capture does, would turn your level meter off once everyone had muted you, and so tell you that they had.
- **Master mute** silences application audio only, and is renamed **Mute streams** (6.1). Voice answers to deafen alone, so what others see on your row matches what you hear.
- **Microphone failure.** No device, a denied permission, or a device that disappears leaves you in voice and listening, shown muted, with a message saying why.

## 6. Interface

Everything here follows `DESIGN.md`. Words come before icons. `[x]` marks a subscription. Mint marks only what is flowing now. The alarm colour marks only failure. Things the network does tick at 12 frames a second, and things you touch move on springs. The requester reviewed browser mockups and chose the dock surface in 6.1 and the always-visible controls in 6.2 over the alternatives, which were controls beside Mute streams and controls that unfold from the voice row.

### 6.1 Dock

- **One voice surface beside the Share audio surface**, built the way `own_lives::audio_controls` is: a switch reading `[ ] Voice` or `[x] Voice` that joins and leaves voice, then two quiet icon toggles that appear only while you are in voice. The microphone toggle mutes; the headphones toggle deafens. One raised surface sits under all of it.
- **On air.** The surface eases to `signal_surface` and the microphone glyph turns `signal` only while the microphone is actually sending: in voice, unmuted and healthy. When you are muted, the microphone toggle shows its slashed glyph and is selected, and the surface returns to `raised`. When you are deafened, both glyphs are slashed and selected. None of these states uses the alarm colour.
- **Icons.** Lucide `mic`, `mic-off`, `headphones` and `headphone-off` are added to the bundled subset as `crates/app/assets/fonts/README.md` describes. Each icon names itself in a tooltip and to screen readers: Mute microphone / Unmute microphone, and Deafen / Undeafen.
- **Mute all becomes Mute streams**, in its label and tooltip, because it no longer silences voice.
- **Width.** At full labels the voice surface is about 150 points wide, so every threshold in `DockFit` moves up by that much. The word Voice itself is never shed in the wide row. The existing test that the dock stays on one row from 396 to 1800 points has to cover every voice state.
- **Compact, below 800 points.** The Voice switch moves into the Room sheet, and while you are in voice the dock keeps only the microphone and headphones toggles. The dock still fits on one row at 396 points.
- **Microphone failure** goes on the dock's notice line in the alarm colour and says what to do next, in the style of the existing audio errors. For example: "No microphone. Check your input device or choose another in Settings, then turn Voice off and on. (<error>)"

### 6.2 Rail

- **Your voice row.** While you are in voice, it branches off you first among your own branches. It reads `Voice`, then a level strip of your real microphone level, then a state word at the right edge: *on air* in `signal`, *muted* or *deafened* in `secondary`, or *no microphone* in the alarm colour. It deliberately has no count of listeners, because the count would drop when someone mutes you. Its wire is drawn like those of your own streams.
- **Someone else in voice** gets a voice row first, above their application-audio volume and their streams. It reads `Voice`, then a level strip of what you receive, then `[x]` at the right edge, where a stream's watch mark sits. `[x]` means you are hearing them; unchecking it mutes them, and the row and its wire dim like an unwatched stream's. If they have muted or deafened themselves, the word *muted* or *deafened* in `secondary` replaces the strip. The tooltip and screen-reader name is "Hear <name>".
- **One line under that row, always shown.** It holds their voice volume as the slider half of `volume_control`; `[x]` already does what that control's mute toggle would. A `[ Deafen ]` toggle sits at the end of the line and stays pressed while you are not sending to them, with the tooltip "<name> stops hearing you".
- **Someone not in voice** has no voice row.
- **Packets.** Voice travels the wires as stream packets do. On a person's wire, their voice travels toward you and yours travels toward them. On their voice row's wire, packets travel only while you are hearing them. Packet density follows bytes, as it does today, so a quiet person and a person you have deafened look alike.
- **Ledger.** It logs *<name> joined voice* as a good event and *<name> left voice* as a neutral one. Mute and deafen are not logged, because they change too often for five lines.
- **Motion.** Voice rows reveal and collapse through `motion::reveal`, as stream rows do.

### 6.3 Level strips

- A level strip is built like `traffic_strip`: 3-point bars at a 4-point pitch, newest on the right, stepping at the wire tick (`WIRE_TICK`) and only while the window has focus. Each bar's height is the measured level. Bars above the speaking threshold are `signal` and bars below it are `muted`. A strip shows only real measurements.
- With reduced motion, the strip holds still. While someone is above the threshold, their row reads *speaking* in `signal`.

### 6.4 Tiles

- The name chip on a person's tile carries a three-bar level mark before the name. It is `signal` while they speak. Someone not in voice gets no mark. In a pop-out, the mark leaves with the chip, as the chip already does.

### 6.5 Room map

- Each person in voice gets a third label line: their level strip, or *muted* or *deafened*. Your own label in the centre gets the same state word your rail row shows.
- Wires to people in voice carry voice packets, so the map's mint patches strengthen while people talk. The map gets no voice controls; they live in the rail and the dock. The map's `fits` limits are unchanged.

### 6.6 Settings

- The rows, in order:
  1. Your name
  2. Connection
  3. Invite links
  4. **Voice:** a "Join voice automatically" switch, with the hint "Your microphone goes live in every room you enter"
  5. Frame rate ceiling
  6. **Microphone:** a dropdown of System default and the input devices, with an error line like the output picker's
  7. Audio output
  8. **Voice output:** a "Same as audio output" switch, checked by default; unchecking it unfolds a second dropdown through `motion::unfold`
  9. Updates
- Device changes apply the next time you join voice. The dialog's footnote, "Changes apply to the next room you open", must say so for the voice devices.

## 7. Constraints from the existing system

- **Presence.** It is signed, gossiped, and must stay under gossip's 4 KB cap. New fields go after the last existing field and are read with `absent_when_truncated`, so a peer that does not say is a peer that is not in voice. Your voice state (in voice, self-muted, self-deafened) travels this way.
- **Old peers.** A peer whose presence carries no voice state is never sent a voice request or a voice frame. That is what allows a new `ViewerMessage` variant and a new frame kind without changing `MEDIA_ALPN`.
- **Application audio is tied to a live.** It rides a live's subscription through `want_audio` and is routed by `(live_id, AUDIO_PRESET_ID)`. A person in voice may share nothing at all, so voice needs a subscription of its own, not tied to any live.
- **The mixer keys tracks by publisher.** `TrackKey` is the publisher's 32-byte key, one track and gain each, and its master mute silences everything in it. A person's voice and their application audio need separate gains, and the master mute must not reach voice. Section 8's separate voice mixer meets both without changing the existing mixer.
- **Self-exclusion of capture.** Application-audio capture already excludes brp's own playback on every OS (Linux by resolved pid, Windows by process loopback in exclude mode, macOS by bundle id). Voice plays through that same output, so this exclusion is what satisfies the fifth anti-goal and must hold.
- **Media connections refuse non-members** before any handshake. Voice inherits this unchanged.
- **The audio path is 48 kHz stereo throughout** (`AudioParams::STANDARD`, the jitter buffer, the mixer). Mono voice is upmixed to stereo after decoding, before it reaches the mixer.
- **`watching_me` counts every subscription** and the interface draws it. Voice subscriptions must not count toward it.
- **What a muted or deafened person sees.** Someone you have muted is still sent your normal subscription traffic. Someone you have deafened receives encoded silence at the normal packet rate. No signal-lost state, wire, packet count, underrun counter or log line at default level may differ for either of them from what a quiet speaker produces.
- **macOS** needs a microphone usage description in the bundle and goes through the system permission prompt. **Windows** can block microphone access in its privacy settings. Both arrive through the microphone-failure path in 5.4.
- **Verification on hardware.** Only Linux is available to test on. As in earlier phases, Windows and macOS hardware checks are listed as deferred, not claimed.

## 8. Calls made without the requester

These were left to the designer. Each one is open to change.

| Call | Why |
|---|---|
| A full mesh: each person in voice sends one stream to each other person in voice, over the existing media connections | It is what peer-to-peer means here, and voice at a low bitrate keeps the upload cost of a full mesh small at the room sizes brp is built for. |
| Opus in mono, VoIP mode, 20 ms frames, at a target of about 32 kbps | Speech needs neither stereo nor the 128 kbps that application audio uses. The existing FFmpeg libopus encoder and decoder carry it, so no new native library is needed. |
| Voice always has its own mixer and its own output stream; with **Same as audio output** on, both output streams open the same device | One shape for both settings. The existing mixer, its master mute and its track keys stay untouched, and the OS mixes two streams on one device on all three platforms. |
| **Same as audio output** is on by default | It keeps today's single-device behaviour for everyone who never opens the setting. |
| Reuse the existing jitter buffer, decode thread and mixer unchanged | They already work in production for application audio. Tuning latency specifically for voice can come after people have used it. |
| Muting someone keeps the subscription and drops their packets before decoding | Unsubscribing would stop the packets on their wire and lower their upload figure, which tells them. The cost is about 32 kbps per muted person. |
| Deafening someone sends them encoded silence at the normal packet rate | A refusal or an empty stream would tell them; silence looks the same as not talking. |
| Your voice row has no count of listeners | The count would drop when someone mutes you. |
| Joining voice, by click or automatically, starts unmuted and undeafened | It matches clicking Join, and gives the setting one predictable meaning, link joins included (5.1). |
| The sender enforces who hears it: voice goes only to subscribers whose presence says they are in voice and not deafened, re-checked whenever their presence changes, and a subscription that arrives before the presence permitting it is refused and retried, like the existing refusal of non-members | Otherwise a modified client could listen while showing itself out of voice. The cost is that someone who undeafens hears nothing until their presence reaches the speaker. |
| The first-version microphone path does no processing | Echo cancellation and noise suppression are #30 and #16. #30's "add microphone capture" is narrowed to the processing that goes on top of this path. |

## 9. Left to the builder

- Wire shapes: the presence field, the voice subscription message, the frame kind, and how a voice route is keyed on the client.
- The speaking threshold.
- Names and placement of new constants, following `crates/proto/src/constants.rs`, each with the reason for its value.

## 10. Verification

- Unit tests, needing no hardware, for:
  - presence round trips with and without voice state, including an old peer's shorter presence;
  - each control in section 5 and its restore rules;
  - voice and application-audio tracks of one person keeping separate gains;
  - the master mute leaving voice audible;
  - a deafened subscriber receiving encoded silence at the normal packet rate;
  - a muted person's packets still arriving and being dropped before decoding;
  - the sender refusing a subscriber whose presence is out of voice or deafened;
  - voice subscriptions not counting toward `watching_me`;
  - settings with and without the new device fields, including **Same as audio output** routing voice to the streams' device when on and to its own device when off.
- UI tests, in the existing style:
  - the dock stays on one row from 396 to 1800 points in every voice state;
  - voice rows hold the rail's alignment rules in `crates/app/tests/ui_flow.rs`;
  - with reduced motion, level strips hold still and *speaking* shows instead;
  - offscreen renders through `ui_preview` of a room with people in voice, at 1440, 768 and 390 points, both with a stream on screen and as the room map.
- An integration test in the style of phase 4's two-room test, with three rooms, a synthetic microphone and the fake codec. It checks that voice reaches every person in voice and nobody else; that leaving voice, muting, deafening and per-person controls change exactly what section 5 says they change; and that a room without voice support is never sent voice.
- A manual check on Linux with two machines or two instances: talking, muting, deafening, per-person controls, and the OS microphone indicator going off on mute and on leaving voice.

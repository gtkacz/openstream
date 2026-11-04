#!/usr/bin/env bash
# Checks OpenStream on a real Mac: everything the macOS CI job runs, plus what no CI runner can,
# because it needs the Screen & System Audio Recording permission, a display, speakers, and
# LaunchServices: ScreenCaptureKit capture of displays and windows, system and per-application
# audio, VideoToolbox encoding and decoding, the staged app bundle and its signature, the bundle
# refresh an in-app update performs, and brp:// links delivered as Apple Events.
#
#   curl -fsSL https://raw.githubusercontent.com/gtkacz/openstream/main/ci/macos/hardware-check.sh | bash
#
# BRP_REPO and BRP_REF choose what is checked (default: gtkacz/openstream main); BRP_CHECK_DIR is
# where everything goes (default ~/OpenStream-hardware-check): the checkout, the FFmpeg build,
# report.log, and summary.txt. Running it again resumes after the last step that passed. It needs
# the Xcode command line tools and Homebrew, and installs Rust with rustup if it is missing.
#
# It asks for one thing: when macOS asks whether Terminal may record the screen and system audio,
# allow it. It never installs anything outside BRP_CHECK_DIR except Homebrew packages, Rust, and
# OpenStream's own settings under ~/Library/Application Support/brp, which the link checks create.
set -uo pipefail

DIR="${BRP_CHECK_DIR:-$HOME/OpenStream-hardware-check}"
REPO="${BRP_REPO:-https://github.com/gtkacz/openstream}"
REF="${BRP_REF:-main}"
mkdir -p "$DIR"
LOG="$DIR/report.log"
SUMMARY="$DIR/summary.txt"
touch "$SUMMARY"
exec > >(tee -a "$LOG") 2>&1

APP="$DIR/OpenStream.app"
APP_EXE="$APP/Contents/MacOS/brp"
BUNDLE_ID="io.github.gtkacz.openstream"
LSREGISTER=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister
VIEWER_LOG="$HOME/Library/Application Support/brp/brp.log"
SOUND=/System/Library/Sounds/Submarine.aiff

note() { printf '\n== %s\n' "$*"; }
# One line per check in the summary, replaced when a check runs again.
record() {
  local status="$1" name="$2" detail="${3:-}"
  grep -v "^[A-Z]* $name:" "$SUMMARY" > "$SUMMARY.tmp" 2>/dev/null || true
  printf '%s %s: %s\n' "$status" "$name" "$detail" >> "$SUMMARY.tmp"
  mv "$SUMMARY.tmp" "$SUMMARY"
  printf '%s %s %s\n' "$status" "$name" "$detail"
}
done_before() { [ -e "$DIR/.done-$1" ]; }
mark_done() { touch "$DIR/.done-$1"; }
# Runs `name` once: skipped when an earlier run passed it.
step() {
  local name="$1"
  shift
  if done_before "$name"; then
    echo "-- $name passed in an earlier run"
    return 0
  fi
  note "$name"
  if "$@"; then
    mark_done "$name"
    record PASS "$name"
  else
    record FAIL "$name" "see report.log"
    return 1
  fi
}
# Waits up to `seconds` for Claude (or anyone) to create $DIR/continue-<name>, so a check that is
# looked at on screen can be held open; carries on by itself when nobody does.
checkpoint() {
  local name="$1" seconds="$2"
  echo "CHECKPOINT $name: waiting up to ${seconds}s (touch $DIR/continue-$name to go on)"
  local waited=0
  while [ "$waited" -lt "$seconds" ] && [ ! -e "$DIR/continue-$name" ]; do
    # The viewer logs under ~/Library; a copy here lets whoever is watching read it as it grows.
    cp "$VIEWER_LOG" "$DIR/viewer-live.log" 2>/dev/null || true
    sleep 2
    waited=$((waited + 2))
  done
  rm -f "$DIR/continue-$name"
}

# ---------------------------------------------------------------- prerequisites

prerequisites() {
  echo "macOS $(sw_vers -productVersion) on $(uname -m), $(sysctl -n machdep.cpu.brand_string)"
  local major
  major="$(sw_vers -productVersion | cut -d. -f1)"
  if [ "$major" -lt 14 ]; then
    echo "OpenStream needs macOS 14 or later" >&2
    return 1
  fi
  if ! xcode-select -p >/dev/null 2>&1; then
    echo "The Xcode command line tools are missing; a window to install them opens now." >&2
    echo "Run this command again when they are installed." >&2
    xcode-select --install || true
    exit 2
  fi
  if ! command -v brew >/dev/null 2>&1; then
    for candidate in /opt/homebrew/bin/brew /usr/local/bin/brew; do
      [ -x "$candidate" ] && eval "$("$candidate" shellenv)"
    done
  fi
  if ! command -v brew >/dev/null 2>&1; then
    echo "Homebrew is missing. Install it from https://brew.sh, then run this command again." >&2
    exit 2
  fi
  [ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
  if ! command -v cargo >/dev/null 2>&1; then
    echo "Installing Rust with rustup (into ~/.cargo and ~/.rustup)"
    curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal -c clippy
    . "$HOME/.cargo/env"
  fi
  rustup component add clippy >/dev/null 2>&1 || true
  cargo --version
  return 0
}

checkout() {
  if [ -d "$DIR/src/.git" ]; then
    git -C "$DIR/src" fetch --depth 50 origin "$REF" && git -C "$DIR/src" checkout -q --force FETCH_HEAD
  else
    git clone --depth 50 --branch "$REF" "$REPO" "$DIR/src"
  fi
  git -C "$DIR/src" log --oneline -1
}

ffmpeg() {
  if [ -e "$FFMPEG_DIR/lib/libavcodec.dylib" ]; then
    echo "FFmpeg already built in $FFMPEG_DIR"
    return 0
  fi
  ./ci/macos/build-ffmpeg.sh
}

# --------------------------------------------------------------- the CI job's steps

lint() { cargo clippy --workspace --all-targets -- -D warnings; }
unit_tests() { cargo test --workspace; }
videotoolbox() {
  BRP_CODEC_TESTS=1 cargo test -p brp-codec --test codec_smoke -- --nocapture --test-threads 1
}
release_build() {
  RUSTFLAGS="-C link-arg=-Wl,-rpath,@executable_path" cargo build --release -p brp
}

stage() {
  rm -rf dist
  ./ci/stage-macos.sh dist/brp dist/OpenStream.app || return 1
  codesign --verify --strict --deep --verbose=2 dist/OpenStream.app || return 1
  echo "-- what the staged binary and libraries load:"
  for file in dist/brp/brp dist/brp/*.dylib; do otool -L "$file" | tail -n +2; done | sort -u
  local dylib
  for dylib in dist/brp/*.dylib; do
    case "$(basename "$dylib")" in
      libavcodec.*.dylib | libavutil.*.dylib | libswscale.*.dylib | libswresample.*.dylib) ;;
      libopus.*.dylib | libdav1d.*.dylib | libSvtAv1Enc.*.dylib) ;;
      *)
        echo "unexpected library staged: $dylib" >&2
        return 1
        ;;
    esac
  done
  # The updater archive, made the way the release job makes it.
  rm -rf dist/archive && mkdir -p dist/archive/brp-0.0.0-macos-aarch64
  ditto dist/brp dist/archive/brp-0.0.0-macos-aarch64
  COPYFILE_DISABLE=1 tar --format ustar -C dist/archive -czf dist/update.tar.gz brp-0.0.0-macos-aarch64
  if tar -tzf dist/update.tar.gz | grep -q '/\._'; then
    echo "the update archive carries AppleDouble members" >&2
    return 1
  fi
  tar -tzf dist/update.tar.gz
}

# What an in-app update does to a bundle, then the refresh it runs: the seal must break, and the
# refresh must mend it, stamp the new version, and leave no .old files.
bundle_update() {
  local copy="$DIR/update-check/OpenStream.app" new="$DIR/update-check/new" file name
  rm -rf "$DIR/update-check" && mkdir -p "$new"
  ditto dist/OpenStream.app "$copy"
  tar -xzf dist/update.tar.gz -C "$new"
  for file in "$new"/brp-0.0.0-macos-aarch64/*; do
    name="$(basename "$file")"
    mv "$copy/Contents/MacOS/$name" "$copy/Contents/MacOS/$name.old"
    cp "$file" "$copy/Contents/MacOS/$name"
  done
  if codesign --verify --strict "$copy" 2>/dev/null; then
    echo "expected the swapped files to break the seal" >&2
    return 1
  fi
  echo "-- swapped files break the seal, as expected; refreshing"
  cargo run -q -p brp --example bundle_refresh -- "$copy" 9.9.9 || return 1
  codesign --verify --strict --deep --verbose=2 "$copy" || return 1
  local version
  version="$(plutil -extract CFBundleShortVersionString raw "$copy/Contents/Info.plist")"
  [ "$version" = "9.9.9" ] || { echo "Info.plist says $version" >&2; return 1; }
  for left in "$copy"/Contents/MacOS/*.old; do
    if [ -e "$left" ]; then
      echo ".old files left in the bundle: $left" >&2
      return 1
    fi
  done
  "$copy/Contents/MacOS/brp" --version
}

install_app() {
  # One copy only, so LaunchServices hands brp:// links to the one under test.
  "$LSREGISTER" -u dist/OpenStream.app >/dev/null 2>&1 || true
  "$LSREGISTER" -u "$DIR/update-check/OpenStream.app" >/dev/null 2>&1 || true
  rm -rf "$DIR/update-check" "$APP"
  ditto dist/OpenStream.app "$APP"
  rm -rf dist/OpenStream.app
}

# ------------------------------------------------------------ needs the permission

# Asks for the recording permission by using it, and waits while the user allows Terminal.
permission() {
  local probe=(cargo test -q -p brp-capture --test macos_hardware displays_and_windows -- --nocapture)
  if BRP_HARDWARE_TESTS=1 "${probe[@]}" >"$DIR/probe.log" 2>&1; then
    echo "Terminal may record the screen and audio"
    return 0
  fi
  cat "$DIR/probe.log"
  cat <<'MSG'

>>> macOS needs your permission for Terminal to record the screen and system audio.
>>> In System Settings › Privacy & Security › Screen & System Audio Recording, turn on Terminal.
>>> If macOS offers "Quit & Reopen", choose "Later". This check carries on by itself once allowed.
>>> (You can turn it off again when the check is done.)
MSG
  open "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture" || true
  local waited=0
  while [ "$waited" -lt 900 ]; do
    sleep 15
    waited=$((waited + 15))
    if BRP_HARDWARE_TESTS=1 "${probe[@]}" >"$DIR/probe.log" 2>&1; then
      echo "Permission granted"
      return 0
    fi
  done
  echo "Still not allowed. Quit Terminal (Cmd-Q), reopen it, and run the same command again." >&2
  exit 3
}

capture_hardware() {
  BRP_HARDWARE_TESTS=1 cargo test -p brp-capture --test macos_hardware -- --nocapture --test-threads 1
}
audio_hardware() {
  BRP_HARDWARE_TESTS=1 cargo test -p brp-audio --test macos_hardware -- --nocapture --test-threads 1
}

# ----------------------------------------------------- a room between two processes

viewer_pids() { pgrep -f "$APP_EXE" || true; }
viewer_count() { viewer_pids | grep -c . || true; }
quit_viewers() {
  osascript -e "tell application id \"$BUNDLE_ID\" to quit" >/dev/null 2>&1 || true
  sleep 3
  pkill -f "$APP_EXE" 2>/dev/null || true
  sleep 1
}
save_viewer_log() { cp "$VIEWER_LOG" "$DIR/viewer-$1.log" 2>/dev/null || true; }

# A headless publisher (its own identity, under its own HOME) shares the main display and the
# machine's audio; the bundle joins it through brp:// links three ways. Watching needs a click
# in the viewer, so the first checkpoint waits for one; the publisher starts capturing audio
# only once someone watches, which is when it must leave every OpenStream window out.
end_to_end() {
  local ok=0
  quit_viewers
  pkill -f "dist/brp/brp publish" 2>/dev/null || true
  rm -f "$DIR/publisher.out" "$DIR"/viewer-*.log
  (while true; do afplay "$SOUND"; done) &
  local player=$!
  HOME="$DIR/home-publisher" RUST_LOG="info,brp_room=debug,brp_audio=debug" \
    ./dist/brp/brp publish --nickname mac-publisher >"$DIR/publisher.out" 2>&1 &
  local publisher=$!
  local ticket="" waited=0
  while [ -z "$ticket" ] && [ "$waited" -lt 90 ] && kill -0 "$publisher" 2>/dev/null; do
    sleep 2
    waited=$((waited + 2))
    ticket="$(awk '/^Ticket:/{getline; print; exit}' "$DIR/publisher.out")"
  done
  if [ -z "$ticket" ]; then
    cat "$DIR/publisher.out"
    record FAIL publisher "no ticket; see publisher.out"
    kill "$player" "$publisher" 2>/dev/null
    return 1
  fi
  record PASS publisher "sharing the main display"
  grep -E "encoder opened|macos capture started" "$DIR/publisher.out" || true

  # 1. Idle start screen: the link joins in the window that is already open.
  open --env RUST_LOG="info,brp_room=debug" "$APP"
  sleep 8
  local before viewer
  before="$(viewer_count)"
  viewer="$(viewer_pids | head -1)"
  open "brp://join/$ticket"
  sleep 15
  if [ "$before" = 1 ] && [ "$(viewer_count)" = 1 ] && grep -q "room joined" "$VIEWER_LOG" 2>/dev/null; then
    record PASS link-idle-window "the open start screen joined"
  else
    record FAIL link-idle-window "viewers before=$before after=$(viewer_count)"
    ok=1
  fi
  checkpoint watch 1800
  if grep -q 'decoder opened.*hardware="\?videotoolbox' "$VIEWER_LOG" 2>/dev/null; then
    record PASS viewer-decode "$(grep -o 'decoder opened.*' "$VIEWER_LOG" | head -1)"
  else
    record FAIL viewer-decode "no VideoToolbox decoder in the viewer log"
    ok=1
  fi
  if grep "audio filter names these processes" "$DIR/publisher.out" | tail -1 | grep -qw "$viewer"; then
    record PASS audio-excludes-viewer "the publisher's audio leaves out viewer pid $viewer"
  else
    record FAIL audio-excludes-viewer "the publisher's audio filter does not name pid $viewer"
    ok=1
  fi
  save_viewer_log idle

  # 2. A room is open: the link opens a second window and leaves this one alone, and the
  # publisher's audio, already running, must leave the new window out too.
  open "brp://join/$ticket"
  sleep 10
  local second
  second="$(viewer_pids | grep -vx "$viewer" | head -1)"
  if [ "$(viewer_count)" = 2 ] && [ -n "$second" ]; then
    record PASS link-while-in-room "a second window opened (pid $second)"
  else
    record FAIL link-while-in-room "viewers=$(viewer_count)"
    ok=1
  fi
  sleep 4
  if [ -n "$second" ] && grep "retargeted the audio stream" "$DIR/publisher.out" | grep -qw "$second"; then
    record PASS audio-excludes-later-window "the running filter took in pid $second"
  else
    record FAIL audio-excludes-later-window "no retarget naming the second window"
    ok=1
  fi
  [ -n "$second" ] && kill "$second" 2>/dev/null
  sleep 2

  # 3. Not running: the link launches the bundle and the launch event joins.
  quit_viewers
  open "brp://join/$ticket"
  sleep 15
  if [ "$(viewer_count)" = 1 ] && grep -q "room joined" "$VIEWER_LOG" 2>/dev/null; then
    record PASS link-cold-start "the link launched the app into the room"
  else
    record FAIL link-cold-start "viewers=$(viewer_count)"
    ok=1
  fi
  checkpoint cold-start 60
  save_viewer_log cold-start

  quit_viewers
  kill "$publisher" "$player" 2>/dev/null
  pkill -f "afplay $SOUND" 2>/dev/null || true
  wait 2>/dev/null
  return "$ok"
}

# ---------------------------------------------------------------------------- run

note "OpenStream hardware check, $(date)"
echo "report: $LOG"
prerequisites || exit 1
export FFMPEG_DIR="$DIR/ffmpeg"
export MACOSX_DEPLOYMENT_TARGET=14.0
BREW_PREFIX="$(brew --prefix)"
export PKG_CONFIG_PATH="$FFMPEG_DIR/lib/pkgconfig:$BREW_PREFIX/lib/pkgconfig"
# Debug builds and tests load the FFmpeg dylibs straight from the build.
export RUSTFLAGS="-C link-arg=-Wl,-rpath,$FFMPEG_DIR/lib"

note checkout
checkout || exit 1
cd "$DIR/src" || exit 1
step ffmpeg ffmpeg || exit 1
step lint lint
step unit-tests unit_tests
step videotoolbox videotoolbox
step release-build release_build || exit 1
step stage stage || exit 1
step bundle-update bundle_update
step install-app install_app || exit 1
note permission
permission
step capture-hardware capture_hardware
step audio-hardware audio_hardware
note end-to-end
end_to_end

note summary
cat "$SUMMARY"
echo
echo "Everything is in $DIR. To undo: remove that folder, ~/Library/Application Support/brp,"
echo "and Terminal from Screen & System Audio Recording if you allowed it only for this check."

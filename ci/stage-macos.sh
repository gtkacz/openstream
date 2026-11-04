#!/usr/bin/env bash
# Stages the macOS build. Writes two things:
#   <destination>              the flat release directory the updater consumes: brp, the FFmpeg
#                              dylibs and the codec libraries they link, and licenses
#   <destination>.app (opt.)   OpenStream.app, whose Contents/MacOS holds the same files
# Every non-system library is copied beside brp with an @rpath install name, brp finds them
# through an @executable_path rpath, and everything is ad-hoc signed. The check at the end proves
# nothing still points outside the directory.
set -eu
destination="${1:?usage: stage-macos.sh <destination> [<OpenStream.app>]}"
bundle="${2:-}"
: "${FFMPEG_DIR:?FFMPEG_DIR must point at the FFmpeg build}"
version="${VERSION:-$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"] == "brp"))')}"

mkdir -p "$destination"
cp target/release/brp LICENSE README.md "$destination/"
cp "$FFMPEG_DIR/LICENSE.txt" "$destination/FFMPEG-LICENSE.txt"

# Libraries the system provides at a fixed path; everything else ships.
is_system() {
  case "$1" in
    /usr/lib/* | /System/*) return 0 ;;
    *) return 1 ;;
  esac
}

# Where a relative name (@rpath, @loader_path, @executable_path) is looked for: the FFmpeg
# build, then Homebrew's linked libraries, where the codec libraries FFmpeg links live.
search_dirs="$FFMPEG_DIR/lib"
if command -v brew >/dev/null 2>&1; then
  search_dirs="$search_dirs $(brew --prefix)/lib"
fi

# The file a dependency entry names: an absolute path as is, a relative one by its file name in
# the search directories, or nothing when it is in none of them.
resolve() {
  case "$1" in
    @rpath/* | @loader_path/* | @executable_path/*)
      for dir in $search_dirs; do
        if [ -e "$dir/$(basename "$1")" ]; then
          echo "$dir/$(basename "$1")"
          return
        fi
      done
      echo ""
      ;;
    *) echo "$1" ;;
  esac
}

dependencies() {
  otool -L "$1" | tail -n +2 | awk '{print $1}'
}

# Copies every non-system library `file` needs, transitively, rewriting each reference to @rpath.
# A newline-separated queue rather than an array: macOS ships bash 3.2, where an empty array
# trips `set -u`.
queue="$destination/brp"
while [ -n "$queue" ]; do
  file="${queue%%$'\n'*}"
  if [ "$queue" = "$file" ]; then queue=""; else queue="${queue#*$'\n'}"; fi
  while read -r dependency; do
    is_system "$dependency" && continue
    name="$(basename "$dependency")"
    [ "$name" = "$(basename "$file")" ] && continue
    if [ ! -e "$destination/$name" ]; then
      source="$(resolve "$dependency")"
      if [ -z "$source" ] || [ ! -e "$source" ]; then
        echo "cannot find $dependency, needed by $file" >&2
        exit 1
      fi
      # -L: brew and the FFmpeg build install versioned names as symlinks.
      cp -L "$source" "$destination/$name"
      chmod 644 "$destination/$name"
      install_name_tool -id "@rpath/$name" "$destination/$name"
      queue="${queue:+$queue$'\n'}$destination/$name"
    fi
    if [ "$dependency" != "@rpath/$name" ]; then
      install_name_tool -change "$dependency" "@rpath/$name" "$file"
    fi
  done < <(dependencies "$file")
done

# The dylibs find each other beside themselves, and brp finds them beside itself.
for dylib in "$destination"/*.dylib; do
  install_name_tool -add_rpath "@loader_path" "$dylib" 2>/dev/null || true
done
if ! otool -l "$destination/brp" | grep -A2 LC_RPATH | grep -q '@executable_path'; then
  install_name_tool -add_rpath "@executable_path" "$destination/brp"
fi

# install_name_tool invalidates signatures, and arm64 refuses unsigned code: sign again, ad hoc.
# Every file, not only the code: inside a bundle, Contents/MacOS is sealed as nested code, so the
# licences beside brp need signatures too or the bundle cannot be signed. The in-app update does
# the same after it swaps the files (crates/app/src/bundle.rs).
for file in "$destination"/*; do
  [ -f "$file" ] && codesign --force --sign - "$file"
done

# Nothing may point outside the directory.
for file in "$destination"/*.dylib "$destination/brp"; do
  while read -r dependency; do
    is_system "$dependency" && continue
    case "$dependency" in
      @rpath/*) [ -e "$destination/${dependency#@rpath/}" ] && continue ;;
    esac
    echo "staged $(basename "$file") still loads $dependency" >&2
    exit 1
  done < <(dependencies "$file")
done
# The binary must start from the staged directory with no library path set.
env -u DYLD_LIBRARY_PATH -u DYLD_FALLBACK_LIBRARY_PATH "$destination/brp" --version

if [ -n "$bundle" ]; then
  rm -rf "$bundle"
  mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
  # ditto keeps the signatures codesign stored in extended attributes on the non-code files.
  ditto "$destination" "$bundle/Contents/MacOS"
  sed "s/@VERSION@/$version/g" ci/macos/Info.plist > "$bundle/Contents/Info.plist"
  plutil -lint "$bundle/Contents/Info.plist"

  iconset="$(mktemp -d)/openstream.iconset"
  mkdir -p "$iconset"
  for size in 16 32 128 256 512; do
    rsvg-convert -w "$size" -h "$size" -o "$iconset/icon_${size}x${size}.png" crates/app/assets/icon/openstream.svg
    double=$((size * 2))
    rsvg-convert -w "$double" -h "$double" -o "$iconset/icon_${size}x${size}@2x.png" crates/app/assets/icon/openstream.svg
  done
  iconutil -c icns -o "$bundle/Contents/Resources/openstream.icns" "$iconset"

  codesign --force --sign - "$bundle"
  codesign --verify --strict --verbose=2 "$bundle"
fi

#!/usr/bin/env bash
# Stages the Linux build into one directory: the binary, the FFmpeg libraries it loads, and licenses.
# The binary must have been linked with an $ORIGIN rpath; the check at the end proves it finds the
# staged libraries without LD_LIBRARY_PATH.
set -eu
destination="${1:?usage: stage-linux.sh <destination>}"
mkdir -p "$destination"
cp target/release/brp LICENSE README.md "$destination/"
for lib in libavcodec.so.62 libavutil.so.60 libswscale.so.9 libswresample.so.6; do
  cp "$FFMPEG_DIR/lib/$lib" "$destination/"
done
cp "$FFMPEG_DIR/LICENSE.txt" "$destination/FFMPEG-LICENSE.txt"

resolved="$(env -u LD_LIBRARY_PATH ldd "$destination/brp")"
if grep -q 'not found' <<<"$resolved"; then
  echo "staged brp has unresolved libraries:" >&2
  grep 'not found' <<<"$resolved" >&2
  exit 1
fi
staged="$(realpath "$destination")"
for lib in libavcodec.so.62 libavutil.so.60 libswscale.so.9 libswresample.so.6; do
  if ! grep -Fq "$lib => $staged/$lib" <<<"$resolved"; then
    echo "staged brp does not load $lib from $staged; was it linked with -rpath \$ORIGIN?" >&2
    grep -F "$lib" <<<"$resolved" >&2 || true
    exit 1
  fi
done

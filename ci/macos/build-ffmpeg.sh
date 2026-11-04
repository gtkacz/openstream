#!/usr/bin/env bash
# Builds the pinned FFmpeg release as LGPL shared libraries for macOS, with VideoToolbox, libopus,
# libdav1d and libsvtav1, installed into $FFMPEG_DIR with @rpath install names. The macos-ffmpeg
# action and hardware-check.sh both run it. Needs the Xcode command line tools and Homebrew.
set -eu
: "${FFMPEG_DIR:?FFMPEG_DIR must name the install directory}"
: "${MACOSX_DEPLOYMENT_TARGET:=14.0}"
tag="n8.1.3"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

brew install --quiet nasm pkgconf opus dav1d svt-av1 librsvg
prefix="$(brew --prefix)"
export PKG_CONFIG_PATH="$prefix/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"

curl -fsSL -o "$work/ffmpeg.tar.gz" "https://github.com/FFmpeg/FFmpeg/archive/refs/tags/$tag.tar.gz"
mkdir -p "$work/src"
tar -xzf "$work/ffmpeg.tar.gz" -C "$work/src" --strip-components 1
cd "$work/src"
# Plain LGPL: no --enable-gpl or --enable-nonfree, so the libraries may ship beside an MIT binary
# as on the other platforms. --disable-autodetect keeps configure from linking whatever Homebrew
# happens to have installed (it found libX11 and made libavutil load it), so only the libraries
# named here are linked. brp uses libavcodec and libswscale; the rest is not built.
./configure \
  --prefix="$FFMPEG_DIR" \
  --install-name-dir='@rpath' \
  --enable-shared --disable-static \
  --disable-programs --disable-doc --disable-debug \
  --disable-autodetect \
  --disable-avdevice --disable-avformat --disable-avfilter \
  --enable-videotoolbox \
  --enable-libopus --enable-libdav1d --enable-libsvtav1 \
  --extra-cflags="-mmacosx-version-min=$MACOSX_DEPLOYMENT_TARGET" \
  --extra-ldflags="-mmacosx-version-min=$MACOSX_DEPLOYMENT_TARGET -Wl,-headerpad_max_install_names"
make -j"$(sysctl -n hw.ncpu)"
make install
cp COPYING.LGPLv2.1 "$FFMPEG_DIR/LICENSE.txt"

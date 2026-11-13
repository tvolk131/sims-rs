#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
if [ "$#" -ne 0 ] || [ "$(uname -s)" != Darwin ]; then
    printf '%s\n' 'Usage: sh scripts/build-window-present-macos.sh (macOS only)' >&2
    exit 2
fi
if [ -L target ]; then
    printf '%s\n' 'Refusing a symlinked target directory.' >&2
    exit 1
fi
mkdir -p target
staging=$(mktemp -d target/.window-present.XXXXXX)
trap 'rm -f "$staging/window-present-macos.dylib"; rmdir "$staging"' 0
trap 'exit 1' 1 2 15
clang -std=c11 -O2 -fobjc-arc -fblocks -fvisibility=hidden \
    -mmacosx-version-min=14.0 -dynamiclib \
    crates/sims-app/native/window_present_macos.m \
    -framework Foundation -framework QuartzCore -framework Metal -framework AppKit \
    -o "$staging/window-present-macos.dylib"
mv "$staging/window-present-macos.dylib" target/window-present-macos.dylib
printf '%s\n' 'Built target/window-present-macos.dylib; loaded only when explicitly requested.'

#!/bin/sh
# Builds the Windows tray app into app/target/release/kb16-mixer.exe, with build-machine paths
# kept out of the binary (see remap-paths.sh). Plain `cargo build --release` works too.
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
. "$ROOT/scripts/remap-paths.sh"
cd "$ROOT/app" || exit 1
cargo build --release --locked --config "$(remap_config build.rustflags "$ROOT")" || exit 1
echo "wrote $ROOT/app/target/release/kb16-mixer.exe"

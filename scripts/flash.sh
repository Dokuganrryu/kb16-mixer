#!/bin/sh
# Flashes the pad's app region only (DFU alt 2 = 0x08002000); the bootloader is never touched.
# Usage: scripts/flash.sh [firmware.bin]     (default: firmware/target/kb16-mixer-firmware.bin)
#
# dfu-util is taken from $DFU_UTIL, else PATH, else anywhere under the git-ignored tools/ folder.
# If the pad does not answer the bootloader request, unplug it, hold the top-left key,
# plug it in and keep holding until the download starts.
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${1:-$ROOT/firmware/target/kb16-mixer-firmware.bin}"
APP="$ROOT/app/target/release/kb16-mixer.exe"

DFU="$DFU_UTIL"
if [ -z "$DFU" ]; then
  for t in dfu-util dfu-util-static; do
    if command -v "$t" >/dev/null 2>&1; then DFU="$t"; break; fi
  done
fi
if [ -z "$DFU" ] && [ -d "$ROOT/tools" ]; then
  # The dfu-util release archive holds builds for several systems: take the first that runs here.
  DFU=$(find "$ROOT/tools" -type f \( -name 'dfu-util' -o -name 'dfu-util.exe' -o -name 'dfu-util-static.exe' \) 2>/dev/null |
    while IFS= read -r t; do
      if "$t" --version >/dev/null 2>&1; then echo "$t"; break; fi
    done)
fi
if [ -z "$DFU" ]; then
  echo "dfu-util not found. Install it (https://dfu-util.sourceforge.net), put it on PATH," >&2
  echo "set DFU_UTIL=/path/to/dfu-util, or unpack it anywhere under $ROOT/tools" >&2
  exit 1
fi
if [ ! -f "$BIN" ]; then
  echo "no firmware image at $BIN; run scripts/build-firmware.sh first" >&2
  exit 1
fi

echo "firmware: $BIN ($(wc -c < "$BIN") bytes)"
echo "dfu-util: $DFU"
if command -v taskkill >/dev/null 2>&1; then
  taskkill //IM kb16-mixer.exe //F >/dev/null 2>&1
fi
if [ -x "$APP" ] && "$APP" --bootloader; then
  echo "pad told to enter its bootloader"
else
  echo "waiting for the bootloader: unplug, hold the top-left key, plug in, keep holding"
fi
"$DFU" -w -d 1eaf:0003 -a 2 -D "$BIN" -R
echo "dfu-util exit $? (74 after 'Download done' is normal)"

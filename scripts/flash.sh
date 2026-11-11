#!/bin/sh
# Flashes the pad's app region only (DFU alt 2 = 0x08002000); the bootloader is never touched.
# Usage: scripts/flash.sh [firmware.bin]     (default: firmware/target/kb16-mixer-firmware.bin)
# For Git Bash and Linux. For Windows users, scripts/flash.ps1 does the same image check and more device checks.
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

# The same image check as flash.ps1: not empty, below the name page (0x1D400 bytes), and a
# vector table that starts with a stack pointer in RAM and a Thumb reset vector in the app area.
SIZE=$(wc -c < "$BIN" | tr -d ' ')
set -- $(od -An -t x4 -N 8 "$BIN")
SP=$((0x${1:-0})); RESET=$((0x${2:-0}))
if [ "$SIZE" -lt 8 ] || [ "$SIZE" -gt $((0x1D400)) ] ||
  [ "$SP" -le $((0x20000000)) ] || [ "$SP" -gt $((0x20005000)) ] ||
  [ "$RESET" -le $((0x08002000)) ] || [ "$RESET" -ge $((0x0801F400)) ] || [ $((RESET & 1)) -ne 1 ]; then
  printf 'REFUSED: %s is not a firmware image for this pad (%s bytes, stack pointer 0x%08x, reset 0x%08x)\n' \
    "$BIN" "$SIZE" "$SP" "$RESET" >&2
  exit 1
fi
echo "firmware: $BIN ($SIZE bytes, vectors OK)"
if command -v sha256sum >/dev/null 2>&1; then echo "  SHA-256: $(sha256sum < "$BIN" | cut -d' ' -f1)"; fi
echo "dfu-util: $DFU"
# The running app would take the pad back between the bootloader request and the download, so
# it is stopped here and started again from the same exe after the download.
RUNNING=""
if command -v powershell.exe >/dev/null 2>&1; then
  RUNNING=$(powershell.exe -NoProfile -Command \
    "Get-Process kb16-mixer -ErrorAction SilentlyContinue | ForEach-Object { \$_.Path } | Sort-Object -Unique" |
    tr -d '\r')
fi
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

echo "$RUNNING" | while IFS= read -r exe; do
  if [ -n "$exe" ]; then
    powershell.exe -NoProfile -Command "Start-Process -FilePath '$exe'" && echo "started the app again: $exe"
  fi
done
echo
echo "NEXT: unplug the pad and plug it in again. The bootloader starts the new firmware only after that."

#!/bin/sh
# Builds the firmware into firmware/target/kb16-mixer-firmware.bin and checks the image before
# it can be flashed.
# Usage: scripts/build-firmware.sh [--diag]   (--diag adds start-up breadcrumbs, see firmware/src/diag.rs)
#
# Needs the thumbv7m-none-eabi target and an objcopy/nm pair: $OBJCOPY/$NM, else
# arm-none-eabi-objcopy/-nm, rust-objcopy/-nm (cargo-binutils), or llvm-objcopy/-nm from the
# llvm-tools rustup component (`rustup component add llvm-tools`).
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
FW="$ROOT/firmware"
OUT="$FW/target/kb16-mixer-firmware.bin"
FEATURES=""
if [ "$1" = "--diag" ]; then FEATURES="--features diag"; OUT="$FW/target/kb16-mixer-firmware-diag.bin"; fi

# Finds one binutils tool: $1 = env override, $2 = tool (objcopy or nm).
find_tool() {
  if [ -n "$1" ]; then echo "$1"; return; fi
  for t in "arm-none-eabi-$2" "rust-$2" "llvm-$2"; do
    if command -v "$t" >/dev/null 2>&1; then echo "$t"; return; fi
  done
  host=$(rustc -vV | sed -n 's/^host: //p')
  for t in "$(rustc --print sysroot)/lib/rustlib/$host/bin/llvm-$2" "$(rustc --print sysroot)/lib/rustlib/$host/bin/llvm-$2.exe"; do
    if [ -x "$t" ]; then echo "$t"; return; fi
  done
}
OBJCOPY=$(find_tool "$OBJCOPY" objcopy)
NM=$(find_tool "$NM" nm)
if [ -z "$OBJCOPY" ] || [ -z "$NM" ]; then
  echo "no objcopy/nm found: run 'rustup component add llvm-tools' or install arm-none-eabi binutils" >&2
  exit 1
fi
PYTHON=""
for p in python3 python; do
  if "$p" -c "import struct" >/dev/null 2>&1; then PYTHON="$p"; break; fi
done
if [ -z "$PYTHON" ]; then echo "python 3 is needed for the image checks" >&2; exit 1; fi

. "$ROOT/scripts/remap-paths.sh"
# Merged with the -Tlink.x flag in firmware/.cargo/config.toml (cargo joins config arrays).
REMAP=$(remap_config target.thumbv7m-none-eabi.rustflags "$ROOT")

cd "$FW" || exit 1
# shellcheck disable=SC2086
cargo build --release --locked --config "$REMAP" $FEATURES || exit 1
E=target/thumbv7m-none-eabi/release/kb16-mixer-firmware
"$OBJCOPY" -O binary "$E" "$OUT.tmp" || exit 1
DEFAULT=$("$NM" "$E" | awk '$3=="DefaultHandler"{print $1}')
if [ -z "$DEFAULT" ]; then rm -f "$OUT.tmp"; echo "DefaultHandler not found in $E; IMAGE REJECTED" >&2; exit 1; fi
"$PYTHON" -I -c "
import struct, sys
b = open(sys.argv[1], 'rb').read()
default = int(sys.argv[2], 16) | 1
v = struct.unpack('<76I', b[:304])
sp, reset = v[0], v[1]
assert 0x20000000 < sp <= 0x20005000, 'bad stack pointer %#x' % sp
assert 0x08002000 < reset < 0x08020000, 'reset vector %#x is outside the app region' % reset
# IRQ n lives at vector 16 + n. An unbound handler means embassy-stm32's rt feature is missing
# and the first interrupt would hang the chip.
for name, irq in (('TIM2', 28), ('USB_LP_CAN1_RX0', 20), ('I2C2_EV', 33), ('DMA1_CHANNEL4', 14)):
    assert v[16 + irq] != default, name + ' interrupt has no handler (DefaultHandler)'
assert len(b) < 0x1D800, 'image overlaps the crash-log page'
print('%d bytes, vectors OK' % len(b))
" "$OUT.tmp" "$DEFAULT" || { rm -f "$OUT.tmp"; echo "IMAGE REJECTED" >&2; exit 1; }
mv "$OUT.tmp" "$OUT" && echo "wrote $OUT"

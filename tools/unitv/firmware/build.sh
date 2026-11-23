#!/usr/bin/env bash
# Catalyst Coprocessor: the UnitV's firmware (a MaixPy of our own), built
# from pinned sources on Linux (WSL on Windows).
#
#   tools/unitv/firmware/build.sh            # fetch, check, patch, build
#   tools/unitv/firmware/build.sh --sources  # only fetch, check and unpack
#
# What it does, every time from scratch:
#   1. Fetches Kendryte's K210 toolchain, CMake, pyserial, and MaixPy and its
#      submodules as GitHub tarballs at the commits in versions.lock, each
#      checked against its SHA-256 there (any other file stops the build).
#   2. Unpacks them into $WORK/src (never into the image build's tree), adds
#      patches/*.patch (the trimmed configuration, the sleeping delay and
#      continuous time base, quiet SPIFFS, the KPU taking a float buffer),
#      the `catalyst` C module (cmodule/ and tools/unitv/speech/fe.c), and
#      the frozen Python (frozen/: the boot that always runs the program, the
#      link, updates over Grove, the safe program).
#   3. Builds project catalyst_coprocessor (config_defaults.mk) and copies the
#      result next to this script: catalyst-coprocessor.bin, with
#      catalyst-coprocessor.txt (its SHA-256, size and memory figures).
#
# $WORK defaults to ~/catalyst-coprocessor-build and may not be inside
# ~/catalyst-linux-build (the image build's own tree).
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../../.." && pwd)
WORK=${CATALYST_COPROCESSOR_WORK:-$HOME/catalyst-coprocessor-build}
ONLY_SOURCES=0
[ "${1:-}" = "--sources" ] && ONLY_SOURCES=1

die() { echo "build.sh: $*" >&2; exit 1; }

mkdir -p "$WORK"
WORK=$(cd "$WORK" && pwd -P)
case "$WORK/" in
    "$(realpath -m "$HOME/catalyst-linux-build")"/*) die "\$WORK may not be inside ~/catalyst-linux-build" ;;
esac
for tool in curl sha256sum tar patch python3 make gcc; do
    command -v "$tool" >/dev/null || die "needs $tool"
done

# shellcheck source=versions.lock
. "$HERE/versions.lock"
# The firmware's own version: the hello says it, the handheld compares it
# with the one its image carries.
FIRMWARE=$(tr -d ' \n' <"$HERE/FIRMWARE")

DL=$WORK/dl
mkdir -p "$DL"

# fetch URL FILE SHA256: downloaded once, checked every time.
fetch() {
    local url=$1 file=$2 sum=$3
    if ! echo "$sum  $DL/$file" | sha256sum -c --quiet - >/dev/null 2>&1; then
        echo "-- fetching $file" >&2
        curl -fsSL --retry 3 -o "$DL/$file.part" "$url"
        mv "$DL/$file.part" "$DL/$file"
    fi
    echo "$sum  $DL/$file" | sha256sum -c --quiet - || die "$file is not the pinned file"
}

github() { # REPO COMMIT SHA256 -> the tarball's file name
    local name
    name=$(echo "$1" | tr '/' '-')-${2:0:12}.tar.gz
    fetch "https://codeload.github.com/$1/tar.gz/$2" "$name" "$3"
    echo "$name"
}

fetch "$TOOLCHAIN_URL" "$(basename "$TOOLCHAIN_URL")" "$TOOLCHAIN_SHA256"
fetch "$CMAKE_URL" "$(basename "$CMAKE_URL")" "$CMAKE_SHA256"
fetch "$PYSERIAL_URL" "$(basename "$PYSERIAL_URL")" "$PYSERIAL_SHA256"

TOOLS=$WORK/tools
if [ ! -x "$TOOLS/kendryte-toolchain/bin/riscv64-unknown-elf-gcc" ]; then
    mkdir -p "$TOOLS"
    tar -xJf "$DL/$(basename "$TOOLCHAIN_URL")" -C "$TOOLS"
fi
CMAKE_DIR=$TOOLS/$(basename "$CMAKE_URL" .tar.gz)
[ -x "$CMAKE_DIR/bin/cmake" ] || tar -xzf "$DL/$(basename "$CMAKE_URL")" -C "$TOOLS"

# ------------------------------------------------------------- sources ---
SRC=$WORK/src/MaixPy-v1
rm -rf "$WORK/src"
mkdir -p "$SRC"
tar -xzf "$DL/$(github sipeed/MaixPy-v1 "$MAIXPY_COMMIT" "$MAIXPY_SHA256")" --strip-components=1 -C "$SRC"
echo "$SUBMODULES" | while IFS='|' read -r path repo commit sum; do
    [ -n "$path" ] || continue
    rm -rf "${SRC:?}/$path"
    mkdir -p "$SRC/$path"
    tar -xzf "$DL/$(github "$repo" "$commit" "$sum")" --strip-components=1 -C "$SRC/$path"
done
[ "$ONLY_SOURCES" = 1 ] && { echo "sources in $SRC"; exit 0; }

for p in "$HERE"/patches/*.patch; do
    echo "-- $(basename "$p")"
    patch -d "$SRC" -p1 -s --no-backup-if-mismatch <"$p"
done

MOD=$SRC/components/micropython/port/src/catalyst
mkdir -p "$MOD"
cp "$HERE"/cmodule/*.c "$MOD/"
cp "$REPO/tools/unitv/speech/fe.c" "$REPO/tools/unitv/speech/fe.h" "$MOD/"

# The project: the M5StickV's (the UnitV runs its board support), with our
# configuration and our frozen Python only (no M5StickV demo, no PMU driver
# the UnitV does not have, no editor).
PROJ=$SRC/projects/catalyst_coprocessor
cp -r "$SRC/projects/maixpy_m5stickv" "$PROJ"
rm -rf "$PROJ/build" "$PROJ/builtin_py"
mkdir -p "$PROJ/builtin_py"
cp "$SRC/projects/maixpy_m5stickv/builtin_py/board.py" "$SRC/projects/maixpy_m5stickv/builtin_py/fpioa_manager.py" "$PROJ/builtin_py/"
cp "$HERE"/frozen/*.py "$PROJ/builtin_py/"
cp "$HERE/config_defaults.mk" "$PROJ/config_defaults.mk"

# --------------------------------------------------------------- build ---
export PATH="$CMAKE_DIR/bin:$PATH"
export PYTHONPATH="$DL/$(basename "$PYSERIAL_URL")"
# mpy-cross is built with the host's compiler, newer than MicroPython 1.11
# knew: its warnings are not errors here.
export CFLAGS_EXTRA="-Wno-error"
# Dates in the build come from MaixPy's commit, not the clock.
export SOURCE_DATE_EPOCH=1718610750
export CATALYST_FIRMWARE="$FIRMWARE"
cd "$PROJ"
python3 project.py \
    --toolchain "$TOOLS/kendryte-toolchain/bin" \
    --toolchain-prefix riscv64-unknown-elf- \
    build 2>&1 | tee "$WORK/build.log" | grep -E "error|Error|warning: implicit|firmware|text|maixpy.elf" || true
[ -f "$PROJ/build/maixpy.bin" ] || die "no firmware (see $WORK/build.log)"

# -------------------------------------------------------------- result ---
OUT=$HERE/catalyst-coprocessor.bin
cp "$PROJ/build/maixpy.bin" "$OUT"
SIZE=$(stat -c %s "$OUT")
SUM=$(sha256sum "$OUT" | cut -d' ' -f1)
read -r TEXT DATA BSS _ < <("$TOOLS/kendryte-toolchain/bin/riscv64-unknown-elf-size" "$PROJ/build/maixpy.elf" | tail -1)
GC=$(sed -n 's/^CONFIG_MAIXPY_GC_HEAP_SIZE=//p' "$PROJ/build/config/global_config.mk")
GC=$((GC))
# The K210's 8 MB of SRAM: 6 MB of main memory (the linker's `ram`), which
# the image is copied into whole and runs from, and 2 MB the KPU keeps for
# its feature maps.
RAM=$((6 * 1024 * 1024))
LEFT=$((RAM - TEXT - DATA - BSS - GC))
cat >"$HERE/catalyst-coprocessor.txt" <<EOF
Catalyst Coprocessor firmware $FIRMWARE (MaixPy ${MAIXPY_COMMIT:0:9} with patches/)
file    catalyst-coprocessor.bin
sha256  $SUM
bytes   $SIZE
# In the K210's 6 MB of main memory (its other 2 MB are the KPU's own):
text    $TEXT
data    $DATA
bss     $BSS
gc-heap $GC
# Left for everything allocated at run time: the image sensor's frame
# buffers (about 0.4 MB at QVGA, double-buffered, plus the KPU's copy), the
# models (face 0.37 MB, keywords 0.09 MB) and their working memory:
left    $LEFT
EOF
cat "$HERE/catalyst-coprocessor.txt"

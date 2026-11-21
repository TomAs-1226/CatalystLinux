#!/bin/bash
# Cross-compile catalyst-shell for the CardputerZero (aarch64, musl).
# Run inside WSL after wsl-setup.sh, once Buildroot has built its toolchain
# (build-image.sh). The binary lands in ~/catalyst-linux-build/out/catalyst-shell,
# where the image build picks it up.
#
# The shell is linked against the image's own musl, dynamically: it loads
# Mesa (the GPU's driver) when it starts, which a static binary cannot do.
# Linking goes through Buildroot's cross gcc so the binary asks for exactly
# the C library and loader the image has.
set -euo pipefail

HERE="$(cd "$(dirname "$0")/.." && pwd)"
BUILD="${CATALYST_BUILD:-$HOME/catalyst-linux-build}"
TARGET=aarch64-unknown-linux-musl
CC="$BUILD/output/host/bin/aarch64-linux-gcc"
export PATH="$HOME/.cargo/bin:$PATH"
# Build outside /mnt/c: the Windows filesystem is slow for many small files.
export CARGO_TARGET_DIR="$BUILD/target"
if [ ! -x "$CC" ]; then
    echo "no cross compiler at $CC: run build-image.sh first (it builds Buildroot's toolchain)" >&2
    exit 1
fi
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER="$CC"
# C code built into the shell (the AprilTag detector) goes through the same compiler.
export CC_aarch64_unknown_linux_musl="$CC"
export AR_aarch64_unknown_linux_musl="${CC%gcc}ar"
export CFLAGS_aarch64_unknown_linux_musl="-mcpu=cortex-a53"
# The CM0's Cortex-A53, and the C library shared rather than built in. Not
# its optional crypto instructions (AES, SHA-1/SHA-256): Raspberry Pi's chips
# leave them out, and LLVM's cortex-a53 assumes them, so SHA-256 compiled to
# them and the updater died with "illegal instruction" on its first install
# (0.4.1 and before; the emulator's A53 has them, so it never showed there).
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_RUSTFLAGS="-C target-cpu=cortex-a53 -C target-feature=-crt-static,-aes,-sha2"
# Checked before building: with a feature on here, a crate's "is the CPU
# able?" test answers yes without asking the CPU (crc32fast's PMULL was what
# died). Code that asks at run time may still carry the instructions.
# shellcheck disable=SC2086
if rustc --print cfg --target "$TARGET" $CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_RUSTFLAGS | grep -qE 'target_feature="(aes|sha2|sha3|lse)"'; then
	echo "the build flags assume processor features the CardputerZero lacks:" >&2
	rustc --print cfg --target "$TARGET" $CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_RUSTFLAGS | grep target_feature >&2
	exit 1
fi

cd "$HERE/userland"
# The shell, the system updater it starts, and `ai` for the Terminal.
cargo build --release --target "$TARGET" -p catalyst-shell -p catalyst-update -p catalyst-ai
mkdir -p "$BUILD/out"
for bin in catalyst-shell catalyst-update ai; do
	cp "$CARGO_TARGET_DIR/$TARGET/release/$bin" "$BUILD/out/"
	ls -l "$BUILD/out/$bin"
	file "$BUILD/out/$bin"
done

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
# The CM0's Cortex-A53, and the C library shared rather than built in.
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_RUSTFLAGS="-C target-cpu=cortex-a53 -C target-feature=-crt-static"

cd "$HERE/userland"
# The shell, and the system updater it starts.
cargo build --release --target "$TARGET" -p catalyst-shell -p catalyst-update
mkdir -p "$BUILD/out"
for bin in catalyst-shell catalyst-update; do
	cp "$CARGO_TARGET_DIR/$TARGET/release/$bin" "$BUILD/out/"
	ls -l "$BUILD/out/$bin"
	file "$BUILD/out/$bin"
done

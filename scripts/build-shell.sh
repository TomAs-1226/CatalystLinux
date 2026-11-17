#!/bin/bash
# Cross-compile catalyst-shell for the CardputerZero (aarch64, static musl).
# Run inside WSL after wsl-setup.sh. The binary lands in
# ~/catalyst-linux-build/out/catalyst-shell, where the image build picks it up.
set -euo pipefail

HERE="$(cd "$(dirname "$0")/.." && pwd)"
BUILD="${CATALYST_BUILD:-$HOME/catalyst-linux-build}"
TARGET=aarch64-unknown-linux-musl
export PATH="$HOME/.cargo/bin:$PATH"
# Build outside /mnt/c: the Windows filesystem is slow for many small files.
export CARGO_TARGET_DIR="$BUILD/target"
# A static musl binary needs no cross C toolchain, only Rust's own linker.
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=rust-lld
# The CM0's Cortex-A53.
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_RUSTFLAGS="-C target-cpu=cortex-a53"

cd "$HERE/userland"
cargo build --release --target "$TARGET" -p catalyst-shell
mkdir -p "$BUILD/out"
cp "$CARGO_TARGET_DIR/$TARGET/release/catalyst-shell" "$BUILD/out/"
ls -l "$BUILD/out/catalyst-shell"
file "$BUILD/out/catalyst-shell"

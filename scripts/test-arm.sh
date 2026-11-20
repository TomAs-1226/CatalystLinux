#!/bin/bash
# Run the shell's tests as the device's own ARM build, under user-mode
# emulation. This is what exercises the Linux-only parts no PC build has:
# the real shell on a pseudo-terminal behind the Terminal app, and the GPU
# path through the image's own Mesa (its software renderer standing in for
# the VideoCore, which emulation does not have).
# Run inside WSL after wsl-setup.sh and a Buildroot build (build-image.sh).
set -euo pipefail

HERE="$(cd "$(dirname "$0")/.." && pwd)"
BUILD="${CATALYST_BUILD:-$HOME/catalyst-linux-build}"
export PATH="$HOME/.cargo/bin:$PATH"
# Its own target folder when asked (CATALYST_ARM_TARGET), so builds from
# several checkouts at once do not wait on one another's lock.
export CARGO_TARGET_DIR="${CATALYST_ARM_TARGET:-$BUILD/target}"
# Linked as build-shell.sh links it, and run against the image's own root:
# its C library, its loader, its Mesa.
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER="$BUILD/output/host/bin/aarch64-linux-gcc"
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_RUSTFLAGS="-C target-cpu=cortex-a53 -C target-feature=-crt-static"
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_RUNNER="$HERE/scripts/arm-run.sh"

cd "$HERE/userland"
# The shell, and the renderer (whose NEON paths only an ARM build has).
cargo test --target aarch64-unknown-linux-musl -p catalyst-shell -p bezel -p catalyst-update "$@"

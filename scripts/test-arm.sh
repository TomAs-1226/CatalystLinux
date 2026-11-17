#!/bin/bash
# Run the shell's tests as the device's own ARM build, under user-mode
# emulation. This is what exercises the Linux-only parts no PC build has:
# the real shell on a pseudo-terminal behind the Terminal app.
# Run inside WSL after wsl-setup.sh.
set -euo pipefail

HERE="$(cd "$(dirname "$0")/.." && pwd)"
BUILD="${CATALYST_BUILD:-$HOME/catalyst-linux-build}"
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_TARGET_DIR="$BUILD/target"
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=rust-lld
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_RUNNER=qemu-aarch64-static

cd "$HERE/userland"
cargo test --target aarch64-unknown-linux-musl -p catalyst-shell "$@"

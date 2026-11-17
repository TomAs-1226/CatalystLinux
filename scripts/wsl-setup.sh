#!/bin/bash
# One-time build-machine setup, run inside WSL (Ubuntu). Needs no root:
# everything lands under ~/catalyst-linux-build and ~/.cargo.
#
#   wsl -d Ubuntu -- bash /mnt/c/Users/User/CatalystLinux/scripts/wsl-setup.sh
set -euo pipefail

BUILD="${CATALYST_BUILD:-$HOME/catalyst-linux-build}"
BUILDROOT_VERSION=2026.08.1
mkdir -p "$BUILD/tools/bin" "$BUILD/dl"
cd "$BUILD"

# Buildroot insists on unzip; fetch the .deb and unpack it locally rather
# than installing system packages.
if ! command -v unzip >/dev/null && [ ! -x "$BUILD/tools/bin/unzip" ]; then
    echo "== unzip (local copy)"
    tmp=$(mktemp -d)
    (cd "$tmp" && apt-get download unzip >/dev/null && dpkg -x unzip_*.deb x && cp x/usr/bin/unzip "$BUILD/tools/bin/")
    rm -rf "$tmp"
fi

if [ ! -d "buildroot-$BUILDROOT_VERSION" ]; then
    echo "== Buildroot $BUILDROOT_VERSION"
    wget -q "https://buildroot.org/downloads/buildroot-$BUILDROOT_VERSION.tar.xz"
    tar xf "buildroot-$BUILDROOT_VERSION.tar.xz"
    rm "buildroot-$BUILDROOT_VERSION.tar.xz"
fi
ln -sfn "buildroot-$BUILDROOT_VERSION" buildroot

if [ ! -x "$HOME/.cargo/bin/rustup" ]; then
    echo "== Rust"
    curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --no-modify-path >/dev/null
fi
"$HOME/.cargo/bin/rustup" target add aarch64-unknown-linux-musl >/dev/null

echo "== ready"
echo "buildroot: $(readlink -f buildroot)"
"$HOME/.cargo/bin/rustc" --version
ls "$BUILD/tools/bin"

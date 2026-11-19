#!/bin/bash
# Once: fetch a whole-machine emulator (QEMU) into the build folder, without
# root, so the image can be booted on this PC. Run inside WSL.
set -euo pipefail
BUILD="${CATALYST_BUILD:-$HOME/catalyst-linux-build}"
PREFIX="$BUILD/tools/qemu"
if [ -x "$PREFIX/usr/bin/qemu-system-aarch64" ]; then
	echo "already there: $PREFIX"
	exit 0
fi
mkdir -p "$PREFIX/debs"
cd "$PREFIX/debs"
# Everything apt would install for it, fetched as packages and unpacked beside
# the build instead of into the system.
PACKAGES=$(apt-get install -s --no-install-recommends qemu-system-arm | awk '/^Inst/ { print $2 }' | grep -v qemu-user-static)
# One at a time: the package index can be older than the mirror, and a
# package that has moved on should not stop the rest.
for package in $PACKAGES; do
	apt-get download "$package" >/dev/null 2>&1 || echo "could not fetch $package (checked below)"
done
for deb in *.deb; do
	dpkg -x "$deb" "$PREFIX"
done
cd "$PREFIX"
rm -rf "$PREFIX/debs"
echo "unpacked into $PREFIX"
export LD_LIBRARY_PATH="$PREFIX/usr/lib/x86_64-linux-gnu"
if ldd "$PREFIX/usr/bin/qemu-system-aarch64" | grep "not found"; then
	echo "the emulator is missing the libraries above" >&2
	exit 1
fi
"$PREFIX/usr/bin/qemu-system-aarch64" --version | head -1
"$PREFIX/usr/bin/qemu-system-aarch64" -M help | grep -i raspi

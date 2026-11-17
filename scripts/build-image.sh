#!/bin/bash
# Build the Catalyst Linux SD card image. Run inside WSL after wsl-setup.sh
# and build-shell.sh. The first build takes around an hour; the image lands
# in ~/catalyst-linux-build/output/images/sdcard.img.
set -euo pipefail

HERE="$(cd "$(dirname "$0")/.." && pwd)"
export CATALYST_BUILD="${CATALYST_BUILD:-$HOME/catalyst-linux-build}"
# Buildroot refuses a PATH with spaces, which the Windows entries WSL adds have.
export PATH="$CATALYST_BUILD/tools/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
export BR2_DL_DIR="$CATALYST_BUILD/dl"

# Buildroot wants its external tree on a filesystem with Unix permissions,
# so build from a copy of os/ rather than from /mnt/c.
rm -rf "$CATALYST_BUILD/os"
cp -r "$HERE/os" "$CATALYST_BUILD/os"
chmod +x "$CATALYST_BUILD/os/board/cardputer-zero/"*.sh

BR=(make -C "$CATALYST_BUILD/buildroot" O="$CATALYST_BUILD/output" BR2_EXTERNAL="$CATALYST_BUILD/os")
if [ ! -f "$CATALYST_BUILD/output/.config" ] || [ "${1:-}" = "reconfigure" ]; then
	"${BR[@]}" catalyst_cardputer_zero_defconfig
	[ "${1:-}" = "reconfigure" ] && shift
fi
"${BR[@]}" "${@:-all}"
ls -lh "$CATALYST_BUILD/output/images/" 2>/dev/null | grep -E "sdcard|rootfs|boot" || true

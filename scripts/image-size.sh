#!/bin/bash
# How big the image is, what fills it, and how small it compresses.
# Run inside WSL after build-image.sh.
BUILD="${CATALYST_BUILD:-$HOME/catalyst-linux-build}"
cd "$BUILD/output" || exit 1
mb() { awk -v b="$1" 'BEGIN { printf "%.1f MB", b / 1048576 }'; }

echo "sdcard.img (what you flash):   $(mb "$(stat -c %s images/sdcard.img)")"
echo "  boot partition:              $(mb "$(stat -c %s images/boot.vfat)")  (kernel $(mb "$(stat -c %s images/Image)"), firmware, overlays)"
echo "  root partition:              $(mb "$(stat -c %s images/rootfs.ext2)")  ($(du -sm target | cut -f1) MB used)"
echo "  data partition:              $(mb "$(stat -c %s images/data.ext4)")  (grows to fill the card on first boot)"
echo "compressed for download:       $(mb "$(gzip -1 -c images/sdcard.img | wc -c)") (gzip)"
echo
echo "what fills the root filesystem:"
du -sk target/lib/modules target/usr/bin target/usr/lib target/lib/firmware target/usr/sbin target/sbin target/bin target/lib 2>/dev/null |
	sort -rn | awk '{ printf "  %6.1f MB  /%s\n", $1 / 1024, substr($2, 8) }'
echo "  catalyst-shell alone: $(mb "$(stat -c %s target/usr/bin/catalyst-shell)") (about 4 MB of that is the two fonts)"

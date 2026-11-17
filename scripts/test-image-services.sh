#!/bin/bash
# Exercise the image's own first-boot and network scripts without a device,
# by running the target's BusyBox and e2fsprogs under user-mode emulation.
# Run inside WSL after build-image.sh.
set -uo pipefail

BUILD="${CATALYST_BUILD:-$HOME/catalyst-linux-build}"
T="$BUILD/output/target"
IMG="$BUILD/output/images/sdcard.img"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
run() { qemu-aarch64-static -L "$T" "$@"; }
# The scripts call fdisk, awk and friends by name: give them the target's BusyBox.
mkdir "$WORK/bin"
for applet in fdisk awk tail basename cat printf sh; do
	printf '#!/bin/sh\nexec qemu-aarch64-static -L %s %s/bin/busybox %s "$@"\n' "$T" "$T" "$applet" > "$WORK/bin/$applet"
	chmod +x "$WORK/bin/$applet"
done
fail=0
check() { if [ "$2" = "$3" ]; then echo "   ok    $1: $2"; else echo "   FAIL  $1: got '$2', wanted '$3'"; fail=1; fi; }

echo "== static address from the robot setting"
for case in "5805=10.58.5.77" "254=10.2.54.77" "10.58.5.2=10.58.5.77" "robot.local=" "192.168.1.9=" "="; do
	input="${case%%=*}"
	printf '%s' "$input" > "$WORK/robot"
	got=$(PATH="$WORK/bin:$PATH" run "$T/bin/busybox" sh "$T/usr/sbin/catalyst-net" --static-for "$WORK/robot")
	check "robot '$input'" "$got" "${case#*=}"
done

echo "== growing partition 3 on a card bigger than the image"
cp "$IMG" "$WORK/card.img"
truncate -s 2G "$WORK/card.img"
part3() { run "$T/bin/busybox" fdisk -l "$WORK/card.img" 2>/dev/null | awk '$1 ~ /img3$/ { print $(NF-5), $(NF-4) }'; }
before=$(part3)
PATH="$WORK/bin:$PATH" run "$T/bin/busybox" sh "$T/usr/sbin/catalyst-grow-data" --table-only "$WORK/card.img"
after=$(part3)
echo "   partition 3 start/end sectors: before $before, after $after"
check "start sector unchanged" "${after%% *}" "${before%% *}"
check "ends at the last sector of the 2 GB card" "${after##* }" "$((2 * 1024 * 1024 * 1024 / 512 - 1))"
p1=$(run "$T/bin/busybox" fdisk -l "$WORK/card.img" 2>/dev/null | awk '$1 ~ /img[12]$/' | wc -l)
check "partitions 1 and 2 still present" "$p1" "2"

echo "== growing the filesystem to the new partition size"
start=${after%% *}
end=${after##* }
dd if="$WORK/card.img" of="$WORK/data.ext4" bs=512 skip="$start" count=$((end - start + 1)) status=none
blocks_before=$(run "$T/sbin/dumpe2fs" -h "$WORK/data.ext4" 2>/dev/null | awk -F: '/^Block count/ { gsub(/ /, "", $2); print $2 }')
run "$T/sbin/resize2fs" "$WORK/data.ext4" >/dev/null 2>&1
blocks_after=$(run "$T/sbin/dumpe2fs" -h "$WORK/data.ext4" 2>/dev/null | awk -F: '/^Block count/ { gsub(/ /, "", $2); print $2 }')
bs=$(run "$T/sbin/dumpe2fs" -h "$WORK/data.ext4" 2>/dev/null | awk -F: '/^Block size/ { gsub(/ /, "", $2); print $2 }')
echo "   filesystem: $((blocks_before * bs / 1048576)) MB before, $((blocks_after * bs / 1048576)) MB after"
[ "$blocks_after" -gt "$((blocks_before * 10))" ] && echo "   ok    the filesystem grew to fill the partition" || { echo "   FAIL  the filesystem did not grow"; fail=1; }
run "$T/sbin/e2fsck" -fn "$WORK/data.ext4" >/dev/null 2>&1 && echo "   ok    e2fsck finds it clean" || { echo "   FAIL  e2fsck reports problems"; fail=1; }

exit $fail

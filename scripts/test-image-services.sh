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

echo "== growing partition 4 (data) on a card bigger than the image"
cp "$IMG" "$WORK/card.img"
truncate -s 2G "$WORK/card.img"
part4() { run "$T/bin/busybox" fdisk -l "$WORK/card.img" 2>/dev/null | awk '$1 ~ /img4$/ { print $(NF-5), $(NF-4) }'; }
before=$(part4)
PATH="$WORK/bin:$PATH" run "$T/bin/busybox" sh "$T/usr/sbin/catalyst-grow-data" --table-only "$WORK/card.img"
after=$(part4)
echo "   partition 4 start/end sectors: before $before, after $after"
check "start sector unchanged" "${after%% *}" "${before%% *}"
check "ends at the last sector of the 2 GB card" "${after##* }" "$((2 * 1024 * 1024 * 1024 / 512 - 1))"
p1=$(run "$T/bin/busybox" fdisk -l "$WORK/card.img" 2>/dev/null | awk '$1 ~ /img[123]$/' | wc -l)
check "boot and both systems still present" "$p1" "3"

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

echo "== first boot's decisions: rewrite and restart, retry, give up, grow"
# A pretend card: /sys says how big partition 4 and the disk are; fdisk,
# resize2fs, umount and the restart are stand-ins that record being called.
# fdisk fails as the real one does on a disk in use: it writes the table,
# then cannot get the kernel to re-read it.
STUB="$WORK/stub"
mkdir -p "$STUB/bin" "$STUB/sys/block/mmcblk0/mmcblk0p4" "$STUB/data"
for tool in fdisk resize2fs umount busybox sync; do
	printf '#!/bin/sh\necho "%s $*" >> %s/calls\n[ "%s" = resize2fs ] && exit "${RESIZE_FAILS:-0}"\n[ "%s" = fdisk ] && exit 1\nexit 0\n' "$tool" "$STUB" "$tool" "$tool" > "$STUB/bin/$tool"
	chmod +x "$STUB/bin/$tool"
done
for applet in basename cat; do
	printf '#!/bin/sh\nexec qemu-aarch64-static -L %s %s/bin/busybox %s "$@"\n' "$T" "$T" "$applet" > "$STUB/bin/$applet"
	chmod +x "$STUB/bin/$applet"
done
card() { echo "$1" > "$STUB/sys/block/mmcblk0/mmcblk0p4/start"; echo "$2" > "$STUB/sys/block/mmcblk0/mmcblk0p4/size"; echo "$3" > "$STUB/sys/block/mmcblk0/size"; }
boot() {
	rm -f "$STUB/calls"
	PATH="$STUB/bin:$PATH" CATALYST_SYS="$STUB/sys" CATALYST_DATA="$STUB/data" RESIZE_FAILS="${1:-0}" \
		run "$T/bin/busybox" sh "$T/usr/sbin/catalyst-grow-data" 2>/dev/null
	calls=$(grep -o '^[a-z0-9]*' "$STUB/calls" 2>/dev/null | grep -v '^sync$' | tr '\n' ' ' | sed 's/ $//')
}
# The image's 64 MB partition on a 2 GB card.
card 401408 131072 4194304
boot
check "first boot rewrites the table and restarts" "$calls" "fdisk umount busybox"
check "  and counts the try" "$(cat "$STUB/data/.grow-pending")" "1"
# The rewrite did not take (still 64 MB): tried again, up to three times in all.
boot; boot
check "a rewrite that did not take is tried again" "$(cat "$STUB/data/.grow-pending")" "3"
boot
check "after three tries it gives up without restarting" "$calls" ""
check "  and does not try again" "$([ -e "$STUB/data/.grown" ] && echo done)" "done"
# A card where the rewrite took: the filesystem is grown, and that is the end of it.
rm -f "$STUB/data/.grown" "$STUB/data/.grow-pending"
card 401408 3792896 4194304
boot 1
check "a failed resize is left to try next boot" "$calls/$([ -e "$STUB/data/.grown" ] && echo done)" "resize2fs/"
boot 0
check "a resize that works finishes the job" "$calls/$([ -e "$STUB/data/.grown" ] && echo done)" "resize2fs/done"
boot 0
check "  and nothing runs on later boots" "$calls" ""

exit $fail

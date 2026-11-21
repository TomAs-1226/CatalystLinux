#!/bin/sh
# catalyst-power's trials against a config.txt in a scratch folder: the
# blocks go in above the system block and come out whole, a trial set is
# tried at the next start, confirmed or (never confirmed) undone at the one
# after. Mounting and restarting are stood in for. Run anywhere with a POSIX
# shell, sed and awk (WSL is fine): sh scripts/test-catalyst-power.sh
set -eu
HERE=$(cd "$(dirname "$0")/.." && pwd)
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
mkdir -p "$T/boot" "$T/data"

# The script, pointed at the scratch folder, with mount and reboot stood in for.
sed -e "s|^MNT=.*|MNT=$T/boot|" \
	-e "s|/data/catalyst|$T/data|g" \
	-e 's|mount -t vfat -o rw "$BOOT_DEV" "$MNT" \|\| return 1|:|' \
	-e 's|^\tumount "$MNT"|\t:|' \
	-e 's|busybox reboot -f|echo REBOOT|' \
	-e 's|^\t\treboot$|\t\techo REBOOT|' \
	"$HERE/os/board/cardputer-zero/rootfs-overlay/usr/sbin/catalyst-power" >"$T/catalyst-power"
# CATALYST_TEST_SH runs it with another shell: the image's own BusyBox under
# user-mode emulation, say (with its awk first on PATH).
P="${CATALYST_TEST_SH:-sh} $T/catalyst-power"

cat >"$T/boot/config.txt" <<'EOF'
kernel=Image
dtoverlay=cardputerzero-v5-overlay
dtoverlay=bq27220_v5

# Which system starts: written by catalyst-update. [all] is the system in
# use; [tryboot] the one an update is tried in, once, after reboot "0 tryboot".
[all]
os_prefix=slot-a/
[tryboot]
os_prefix=slot-b/
EOF
cp "$T/boot/config.txt" "$T/original.txt"

fail=0
check() {
	if [ "$2" = "$3" ]; then echo "   ok    $1"; else echo "   FAIL  $1: got [$2], want [$3]"; fail=1; fi
}
line_of() { grep -n "^$1\$" "$T/boot/config.txt" | cut -d: -f1 | head -n1; }

echo "== the tear-free panel"
$P panel on
check "its state: set, not yet started with" "$(cat "$T/data/panel")" "trying 0"
check "status says it is being tried" "$($P panel status)" "trying"
check "the overlay line is in" "$(grep -c '^dtoverlay=cardputerzero-portrait$' "$T/boot/config.txt")" "1"
v5=$(line_of dtoverlay=cardputerzero-v5-overlay)
portrait=$(line_of dtoverlay=cardputerzero-portrait)
slots=$(grep -n '^# Which system starts:' "$T/boot/config.txt" | cut -d: -f1)
check "after M5Stack's overlay, before the system block" "$([ "$v5" -lt "$portrait" ] && [ "$portrait" -lt "$slots" ] && echo yes)" "yes"
$P panel on
check "set twice, it is in once" "$(grep -c '^dtoverlay=cardputerzero-portrait$' "$T/boot/config.txt")" "1"

$P boot >/dev/null
check "the next start tries it" "$(cat "$T/data/panel")" "trying 1"
$P panel confirm
check "Enter keeps it" "$(cat "$T/data/panel")" "on"
check "kept, a start leaves it be" "$($P boot)" ""
$P panel off
check "off: config.txt as it was" "$(cmp -s "$T/boot/config.txt" "$T/original.txt" && echo same)" "same"
check "off: no state left" "$([ -e "$T/data/panel" ] && echo left || echo none)" "none"

echo "== a trial nobody answered"
$P panel on
$P boot >/dev/null
check "tried" "$(cat "$T/data/panel")" "trying 1"
out=$($P boot 2>/dev/null)
check "the start after an unanswered trial restarts" "$out" "REBOOT"
check "and the overlay is out again" "$(cmp -s "$T/boot/config.txt" "$T/original.txt" && echo same)" "same"
check "said as failed" "$(cat "$T/data/panel")" "failed"

echo "== no answer within the countdown (revert)"
$P panel on
$P boot >/dev/null
out=$($P panel revert)
check "revert restarts" "$out" "REBOOT"
check "revert takes the overlay out" "$(cmp -s "$T/boot/config.txt" "$T/original.txt" && echo same)" "same"

echo "== both trials at once"
$P panel on
$P undervolt on
check "both blocks in" "$(grep -c '^# catalyst-power: end$' "$T/boot/config.txt")" "2"
$P panel off
check "the panel's out, the voltage's stays" "$(grep -c '^over_voltage=-1$' "$T/boot/config.txt"):$(grep -c '^dtoverlay=cardputerzero-portrait$' "$T/boot/config.txt")" "1:0"
$P undervolt off
check "both out: config.txt as it was" "$(cmp -s "$T/boot/config.txt" "$T/original.txt" && echo same)" "same"

exit $fail

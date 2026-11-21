#!/bin/sh
# catalyst-bluetooth against a config.txt in a scratch folder and a stand-in
# bluetoothctl: switching on puts its block above the system block (once,
# however often), off takes it out whole; the radio asks for a restart until
# it is there; devices are listed audio-only, paired first, nameless ones
# left out; pairing, connecting and forgetting say the right things to
# catalyst-audio. Mounting, the daemons and the radio are stood in for.
# Run anywhere with a POSIX shell, sed and awk (WSL is fine):
#   sh scripts/test-catalyst-bluetooth.sh
set -eu
HERE=$(cd "$(dirname "$0")/.." && pwd)
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
mkdir -p "$T/boot" "$T/data/catalyst" "$T/bin" "$T/sys"

# bluetoothctl: a few devices, and what it answers to pair and connect.
cat >"$T/bin/bluetoothctl" <<EOF
#!/bin/sh
echo "\$*" >>"$T/bluetoothctl.log"
case "\$*" in
*" pair "*) cat "$T/pair-answer" ;;
*" connect "*) echo "Attempting to connect"; echo "Connection successful" ;;
devices) printf 'Device 0C:A6:94:12:34:56 WH-1000XM4\nDevice 11:22:33:44:55:66 Kitchen Speaker\nDevice AA:BB:CC:DD:EE:01 AA-BB-CC-DD-EE-01\nDevice 70:F0:87:00:00:01 Fitness Band\nDevice 98:00:00:00:00:02 Old Earbuds\n' ;;
"info 0C:A6:94:12:34:56") printf '\tName: WH-1000XM4\n\tIcon: audio-headphones\n\tPaired: no\n\tConnected: no\n' ;;
"info 11:22:33:44:55:66") printf '\tIcon: audio-card\n\tPaired: yes\n\tConnected: yes\n' ;;
"info AA:BB:CC:DD:EE:01") printf '\tIcon: audio-headset\n\tPaired: no\n\tConnected: no\n' ;;
"info 70:F0:87:00:00:01") printf '\tIcon: watch\n\tPaired: no\n\tConnected: no\n' ;;
"info 98:00:00:00:00:02") printf '\tIcon: audio-headset\n\tPaired: yes\n\tConnected: no\n' ;;
show) printf '\tAlias: atlas\n\tPowered: yes\n' ;;
esac
exit 0
EOF
cat >"$T/bin/catalyst-audio" <<EOF
#!/bin/sh
echo "\$*" >>"$T/audio.log"
case "\$1" in route) echo bluetooth ;; output) [ -z "\${2:-}" ] && cat "$T/output" 2>/dev/null ;; esac
[ "\$1" = output ] && [ -n "\${2:-}" ] && echo "\$2 \${3:-}" >"$T/output"
exit 0
EOF
for stub in modprobe dbus-uuidgen start-stop-daemon; do
	printf '#!/bin/sh\necho "%s $*" >>"%s/daemons.log"\n' "$stub" "$T" >"$T/bin/$stub"
done
chmod +x "$T/bin/"*
PATH="$T/bin:$PATH"
export PATH

# The script, pointed at the scratch folder, with mounting stood in for.
sed -e "s|^MNT=.*|MNT=$T/boot|" \
	-e "s|/data/catalyst|$T/data/catalyst|g" \
	-e "s|^HCI=.*|HCI=$T/sys/hci0|" \
	-e "s|/run/catalyst-|$T/run-|g" \
	-e 's|mount -t vfat -o rw "$BOOT_DEV" "$MNT" \|\| return 1|:|' \
	-e 's|^\tumount "$MNT"|\t:|' \
	"$HERE/os/board/cardputer-zero/rootfs-overlay/usr/sbin/catalyst-bluetooth" >"$T/catalyst-bluetooth"
# Executable, as on the handheld: pairing runs it again to connect.
chmod +x "$T/catalyst-bluetooth"
P="${CATALYST_TEST_SH:-sh} $T/catalyst-bluetooth"

cat >"$T/boot/config.txt" <<'EOF'
kernel=Image
enable_uart=1
dtoverlay=disable-bt
dtoverlay=cardputerzero-v5-overlay

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
status_of() { $P status | sed -n "s/^$1=//p"; }

echo "== off, as it starts"
check "not enabled" "$(status_of enabled):$(status_of radio)" "no:off"
check "nothing to scan with" "$($P scan 2>&1 || true)" "Bluetooth is off"

echo "== switched on"
check "a restart is asked for" "$($P on)" "restart"
check "the flag is kept" "$([ -e "$T/data/catalyst/bluetooth-enabled" ] && echo kept)" "kept"
check "the overlay and the held clock are in" "$(grep -c '^dtoverlay=catalyst-bt$' "$T/boot/config.txt"):$(grep -c '^core_freq=250$' "$T/boot/config.txt")" "1:1"
bt=$(line_of dtoverlay=catalyst-bt)
check "after disable-bt and the board, before the system block" \
	"$([ "$(line_of dtoverlay=disable-bt)" -lt "$bt" ] && [ "$(line_of dtoverlay=cardputerzero-v5-overlay)" -lt "$bt" ] && [ "$bt" -lt "$(grep -n '^# Which system starts:' "$T/boot/config.txt" | cut -d: -f1)" ] && echo yes)" "yes"
check "the console stays on the PL011" "$(grep -c '^enable_uart=1$' "$T/boot/config.txt")" "1"
check "until the restart, the radio waits for it" "$(status_of radio)" "restart"
$P on >/dev/null
check "on twice, the block is in once" "$(grep -c '^dtoverlay=catalyst-bt$' "$T/boot/config.txt")" "1"

echo "== after the restart"
mkdir -p "$T/sys/hci0"
# bluetoothd's pidfile stands in for it running.
echo $$ >"$T/run-bluetoothd.pid"
check "the radio is there" "$(status_of radio):$(status_of powered):$(status_of name)" "on:yes:atlas"
check "the speaker is connected" "$(status_of connected)" "11:22:33:44:55:66"
devices=$($P devices)
check "audio devices only, paired first, nameless ones left out" "$(echo "$devices" | cut -f1 | tr '\n' ' ')" \
	"11:22:33:44:55:66 98:00:00:00:00:02 0C:A6:94:12:34:56 "
check "each with its state, kind and name" "$(echo "$devices" | head -n1)" "$(printf '11:22:33:44:55:66\tpaired\tconnected\taudio-card\tKitchen Speaker')"
check "a scan lists the same" "$($P scan 3 | wc -l | tr -d ' '):$(grep -c 'timeout 3 scan on' "$T/bluetoothctl.log")" "3:1"

echo "== pairing"
printf 'Attempting to pair with 0C:A6:94:12:34:56\nRequest confirmation\nPairing successful\n' >"$T/pair-answer"
: >"$T/audio.log"
$P pair 0C:A6:94:12:34:56
check "paired with nothing to confirm" "$(grep -c 'agent NoInputNoOutput.*pair 0C:A6:94:12:34:56' "$T/bluetoothctl.log")" "1"
check "trusted, so it reconnects" "$(grep -c '^trust 0C:A6:94:12:34:56$' "$T/bluetoothctl.log")" "1"
check "connected, and sound goes there" "$(head -n1 "$T/audio.log")" "output bluetooth 0C:A6:94:12:34:56"
printf 'Attempting to pair with 0C:A6:94:12:34:56\nFailed to pair: org.bluez.Error.AuthenticationFailed\n' >"$T/pair-answer"
check "a failure says why" "$($P pair 0C:A6:94:12:34:56 2>&1 || true)" "org.bluez.Error.AuthenticationFailed"
check "an address that is not one is refused" "$($P pair nope 2>&1 || true)" "not a Bluetooth address: nope"

echo "== forgetting"
echo "bluetooth 0C:A6:94:12:34:56" >"$T/output"
: >"$T/audio.log"
$P forget 0C:A6:94:12:34:56
check "the speaker plays again" "$(grep -c '^output speaker$' "$T/audio.log")" "1"
echo "bluetooth 11:22:33:44:55:66" >"$T/output"
: >"$T/audio.log"
$P forget 0C:A6:94:12:34:56
check "forgetting another leaves the choice alone" "$(grep -c '^output speaker$' "$T/audio.log")" "0"

echo "== switched off"
$P off
check "the flag is gone" "$([ -e "$T/data/catalyst/bluetooth-enabled" ] && echo kept || echo gone)" "gone"
check "the radio powered down" "$(grep -c '^power off$' "$T/bluetoothctl.log")" "1"
check "config.txt as it was" "$(cmp -s "$T/boot/config.txt" "$T/original.txt" && echo same)" "same"

exit $fail

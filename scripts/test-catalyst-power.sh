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
mkdir -p "$T/boot" "$T/data" "$T/bin"

# The watchdog stood in for: a device file, and a "watchdog" that only waits.
touch "$T/watchdog"
printf '#!/bin/sh\nexec sleep 60\n' >"$T/bin/watchdog"
chmod +x "$T/bin/watchdog"
# OpenRC stood in for: each call noted, Wi-Fi's status kept as the service does.
cat >"$T/bin/rc-service" <<EOF
#!/bin/sh
echo "\$*" >>"$T/rc.log"
case "\$2" in stop) echo off >"$T/run/catalyst-wifi.status" ;; start) echo running >"$T/run/catalyst-wifi.status" ;; esac
EOF
chmod +x "$T/bin/rc-service"
PATH="$T/bin:$PATH"
export PATH

# The script, pointed at the scratch folder, with mount and reboot stood in for.
sed -e "s|^MNT=.*|MNT=$T/boot|" \
	-e "s|/data/catalyst|$T/data|g" \
	-e "s|^WATCHDOG_DEV=.*|WATCHDOG_DEV=$T/watchdog|" \
	-e "s|^WATCHDOG_PID=.*|WATCHDOG_PID=$T/watchdog.pid|" \
	-e "s|^TRYBOOT=.*|TRYBOOT=$T/tryboot|" \
	-e "s|^SYS=.*|SYS=$T/sys|" \
	-e "s|^RUN=.*|RUN=$T/run|" \
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

echo "== lower voltage: tried at once, watched while tried"
out=$($P undervolt try)
check "try sets it and restarts" "$out:$(cat "$T/data/undervolt")" "REBOOT:trying 0"
check "its lines are in" "$(grep -c '^over_voltage=-1$' "$T/boot/config.txt"):$(grep -c '^over_voltage_min=-2$' "$T/boot/config.txt")" "1:1"
$P boot >/dev/null
check "the start it is tried in" "$(cat "$T/data/undervolt")" "trying 1"
dog=$(cat "$T/watchdog.pid" 2>/dev/null || true)
check "the watchdog is fed while it is tried" "$([ -n "$dog" ] && kill -0 "$dog" 2>/dev/null && echo fed)" "fed"
$P undervolt confirm
check "kept after five minutes" "$(cat "$T/data/undervolt")" "on"
check "and the watchdog stood down" "$([ -n "$dog" ] && kill -0 "$dog" 2>/dev/null && echo fed || echo down):$([ -e "$T/watchdog.pid" ] && echo left || echo gone)" "down:gone"
$P undervolt off
check "off: config.txt as it was" "$(cmp -s "$T/boot/config.txt" "$T/original.txt" && echo same)" "same"

echo "== lower voltage that hangs: the next start undoes it"
$P undervolt on
$P boot >/dev/null
out=$($P boot 2>/dev/null)
check "a start finding it tried and never kept restarts" "$out" "REBOOT"
check "without it" "$(cmp -s "$T/boot/config.txt" "$T/original.txt" && echo same):$(cat "$T/data/undervolt")" "same:failed"
[ -s "$T/watchdog.pid" ] && kill "$(cat "$T/watchdog.pid")" 2>/dev/null
rm -f "$T/data/undervolt" "$T/watchdog.pid"

echo "== lower voltage tried on a start that is also an update's try"
$P undervolt on
printf '\000\000\000\001' >"$T/tryboot"
$P boot >/dev/null
check "tried all the same" "$(cat "$T/data/undervolt")" "trying 1"
check "the watchdog is left to catalyst-update" "$([ -e "$T/watchdog.pid" ] && echo taken || echo left)" "left"
printf '\000\000\000\000' >"$T/tryboot"
$P undervolt off
rm -f "$T/tryboot"

echo "== both trials at once"
$P panel on
$P undervolt on
check "both blocks in" "$(grep -c '^# catalyst-power: end$' "$T/boot/config.txt")" "2"
$P panel off
check "the panel's out, the voltage's stays" "$(grep -c '^over_voltage=-1$' "$T/boot/config.txt"):$(grep -c '^dtoverlay=cardputerzero-portrait$' "$T/boot/config.txt")" "1:0"
$P undervolt off
check "both out: config.txt as it was" "$(cmp -s "$T/boot/config.txt" "$T/original.txt" && echo same)" "same"

# Deep standby, against a sysfs laid out as the handheld's: Wi-Fi running,
# a radio unblocked, the USB controller as the host, the Grove output on,
# and the red light following the card.
S=$T/sys
mkdir -p "$T/run" "$S/class/rfkill/rfkill0" "$S/bus/platform/drivers/dwc2/3f980000.usb" "$S/class/udc/3f980000.usb" \
	"$S/firmware/devicetree/base/soc/usb@7e980000" "$S/class/leds/grove_5v_out" "$S/class/leds/ext_5v_out" "$S/class/leds/ACT"
fresh() {
	echo running >"$T/run/catalyst-wifi.status"
	touch "$T/data/wifi-enabled"
	echo 0 >"$S/class/rfkill/rfkill0/soft"
	: >"$S/bus/platform/drivers/dwc2/unbind"
	: >"$S/bus/platform/drivers/dwc2/bind"
	echo "not attached" >"$S/class/udc/3f980000.usb/state"
	printf 'otg\000' >"$S/firmware/devicetree/base/soc/usb@7e980000/dr_mode"
	echo 1 >"$S/class/leds/grove_5v_out/brightness"
	echo 0 >"$S/class/leds/ext_5v_out/brightness"
	echo "none [mmc0] timer heartbeat panic" >"$S/class/leds/ACT/trigger"
	echo 0 >"$S/class/leds/ACT/brightness"
	: >"$T/rc.log"
}
# Wi-Fi comes back in the background: wait for it (two seconds at most).
wifi_back() {
	n=0
	while ! grep -q 'catalyst-wifi start' "$T/rc.log" && [ $n -lt 20 ]; do
		sleep 0.1
		n=$((n + 1))
	done
}

echo "== deep standby: on, and back as it was"
fresh
check "not deep to begin with" "$($P deep status)" "off"
$P deep on
check "deep now" "$($P deep status)" "on"
check "Wi-Fi stopped" "$(cat "$T/rc.log")" "catalyst-wifi stop"
check "the radio blocked" "$(cat "$S/class/rfkill/rfkill0/soft")" "1"
check "the USB host unbound" "$(cat "$S/bus/platform/drivers/dwc2/unbind")" "3f980000.usb"
check "the Grove output off" "$(cat "$S/class/leds/grove_5v_out/brightness")" "0"
check "the red light dark, no trigger" "$(cat "$S/class/leds/ACT/brightness"):$(cat "$S/class/leds/ACT/trigger")" "0:none"
check "in this order: Wi-Fi, radio, USB, 5 V, red light" "$(cut -d' ' -f1 "$T/run/catalyst-deep" | tr '\n' ' ')" "wifi radio usb out red "
check "an output already off is not noted" "$(grep -c ext_5v_out "$T/run/catalyst-deep")" "0"
$P deep on
check "on twice: what to bring back is kept" "$(cut -d' ' -f1 "$T/run/catalyst-deep" | tr '\n' ' ')" "wifi radio usb out red "
$P deep off
wifi_back
check "no longer deep" "$($P deep status)" "off"
check "the red light follows the card again" "$(cat "$S/class/leds/ACT/trigger")" "mmc0"
check "the Grove output on again" "$(cat "$S/class/leds/grove_5v_out/brightness")" "1"
check "the other output left off" "$(cat "$S/class/leds/ext_5v_out/brightness")" "0"
check "the USB host bound again" "$(cat "$S/bus/platform/drivers/dwc2/bind")" "3f980000.usb"
check "the radio unblocked" "$(cat "$S/class/rfkill/rfkill0/soft")" "0"
check "Wi-Fi started again, last" "$(tr '\n' ',' <"$T/rc.log")" "catalyst-wifi stop,catalyst-wifi start,"
$P deep off
check "off twice: nothing more" "$(tr '\n' ',' <"$T/rc.log")" "catalyst-wifi stop,catalyst-wifi start,"

echo "== deep standby leaves alone what was already off, and a computer's link"
fresh
echo off >"$T/run/catalyst-wifi.status"
echo 1 >"$S/class/rfkill/rfkill0/soft"
echo configured >"$S/class/udc/3f980000.usb/state"
echo "none timer heartbeat [panic]" >"$S/class/leds/ACT/trigger"
$P deep on
check "Wi-Fi off already: not touched" "$(cat "$T/rc.log")" ""
check "a computer has it as its device: the controller left bound" "$(cat "$S/bus/platform/drivers/dwc2/unbind")" ""
check "noted: the 5 V output and the red light" "$(cut -d' ' -f1 "$T/run/catalyst-deep" | tr '\n' ' ')" "out red "
$P deep off
check "the quiet red light back as it was" "$(cat "$S/class/leds/ACT/trigger")" "panic"
check "Wi-Fi not started" "$(cat "$T/rc.log")" ""
check "the radio left blocked" "$(cat "$S/class/rfkill/rfkill0/soft")" "1"
fresh
printf 'peripheral\000' >"$S/firmware/devicetree/base/soc/usb@7e980000/dr_mode"
$P deep on
check "a USB mode chosen (the handheld a device): the controller left bound" "$(cat "$S/bus/platform/drivers/dwc2/unbind")" ""
$P deep off
wifi_back
fresh
rm -f "$T/data/wifi-enabled"
$P deep on
$P deep off
sleep 0.3
check "Wi-Fi switched off meanwhile: not started again" "$(cat "$T/rc.log")" "catalyst-wifi stop"
check "the usage is said" "$($P deep sideways 2>&1 >/dev/null | head -n1)" "usage: $T/catalyst-power deep on|off|status"

exit $fail

#!/bin/sh
# catalyst-grove against a scratch boot partition, /proc and /sys: the port
# moved to the smart camera takes the kernel's console and the login off it
# (this system's slot only), keeps the camera unpowered until a start has run
# that way, survives a start whose command line came back with the console,
# and goes back to the console whole. Mounting, OpenRC and dmesg are stood
# in for. Run anywhere with a POSIX shell (WSL is fine):
#   sh scripts/test-catalyst-grove.sh
set -eu
HERE=$(cd "$(dirname "$0")/.." && pwd)
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
mkdir -p "$T/boot/slot-a" "$T/boot/slot-b" "$T/data" "$T/bin" "$T/proc/tty" "$T/sys/leds/grove_5v_out" "$T/run" "$T/procfs"

SHIPPED="root=/dev/mmcblk0p2 rootfstype=ext4 ro rootwait console=ttyAMA0,115200 loglevel=3 vt.global_cursor_default=0 cma=64M panic=10"
echo "$SHIPPED" >"$T/boot/slot-a/cmdline.txt"
echo "$SHIPPED" | sed 's|p2|p3|' >"$T/boot/slot-b/cmdline.txt"
cp "$T/boot/slot-b/cmdline.txt" "$T/slot-b.before"
echo "$SHIPPED" >"$T/proc/cmdline"
echo 1 >"$T/sys/leds/grove_5v_out/brightness"
printf '/dev/tty             /dev/tty        5       0 system:/dev/tty\nserial               /dev/ttyAMA     204 64-77 serial\n' >"$T/proc/tty/drivers"

# OpenRC and dmesg stood in for: each call noted.
printf '#!/bin/sh\necho "rc-service $*" >>"%s/calls"\n' "$T" >"$T/bin/rc-service"
printf '#!/bin/sh\necho "dmesg $*" >>"%s/calls"\n' "$T" >"$T/bin/dmesg"
chmod +x "$T/bin/rc-service" "$T/bin/dmesg"
PATH="$T/bin:$PATH"
export PATH

sed -e "s|^MNT=.*|MNT=$T/boot|" \
	-e "s|^SETTING=.*|SETTING=$T/data/grove-port|" \
	-e "s|^CMDLINE=.*|CMDLINE=$T/proc/cmdline|" \
	-e "s|^TTY_DRIVERS=.*|TTY_DRIVERS=$T/proc/tty/drivers|" \
	-e "s|^LED=.*|LED=$T/sys/leds/grove_5v_out/brightness|" \
	-e "s|^PROC=.*|PROC=$T/procfs|" \
	-e "s|^SERIAL_PID=.*|SERIAL_PID=$T/run/catalyst-serial.pid|" \
	-e 's|mount -t vfat -o rw "$BOOT_DEV" "$MNT" \|\| return 1|:|' \
	-e 's|^\tumount "$MNT"|\t:|' \
	"$HERE/os/board/cardputer-zero/rootfs-overlay/usr/sbin/catalyst-grove" >"$T/catalyst-grove"
G="${CATALYST_TEST_SH:-sh} $T/catalyst-grove"

fails=0
check() {
	if eval "$2"; then
		echo "ok   $1"
	else
		echo "FAIL $1"
		fails=$((fails + 1))
	fi
}
calls() { cat "$T/calls" 2>/dev/null; }
reset_calls() { : >"$T/calls"; }
cmd_a() { cat "$T/boot/slot-a/cmdline.txt"; }

# --- as shipped: the console ---
check "the port is the console by default" '[ "$($G mode)" = console ]'
check "a login may listen" '$G login'
check "the camera may not be powered" '! $G ready && ! $G power on'
check "its 5 V left as it was" '[ "$(cat $T/sys/leds/grove_5v_out/brightness)" = 1 ]'
$G boot
check "a start in console mode writes nothing" '[ "$(cmd_a)" = "$SHIPPED" ]'

# --- a kernel without ttynull refuses ---
reset_calls
if $G camera 2>"$T/err"; then refused=no; else refused=yes; fi
check "no ttynull: camera mode refused" '[ $refused = yes ] && grep -q "cannot take its console" "$T/err"'
check "and nothing changed" '[ "$($G mode)" = console ] && [ "$(cmd_a)" = "$SHIPPED" ] && [ ! -s "$T/calls" ]'

# --- the smart camera ---
echo "ttynull              /dev/ttynull    240 0 console" >>"$T/proc/tty/drivers"
# A login session on the port (a process with it open) that must end.
sleep 30 &
held=$!
mkdir -p "$T/procfs/$held/fd"
ln -s /dev/ttyAMA0 "$T/procfs/$held/fd/0"
reset_calls
said=$($G camera)
check "camera mode taken" '[ "$($G mode)" = camera ]'
check "this system's console is null from its next start" 'cmd_a | grep -q "console=null" && ! cmd_a | grep -q ttyAMA0'
check "the rest of its command line kept" '[ "$(cmd_a)" = "$(echo "$SHIPPED" | sed "s|console=ttyAMA0,115200|console=null|")" ]'
check "the other system's command line untouched" 'cmp -s "$T/boot/slot-b/cmdline.txt" "$T/slot-b.before"'
check "the login stopped" 'calls | grep -q "rc-service catalyst-serial stop"'
check "the kernel's messages held back now" 'calls | grep -q "dmesg -n 1"'
check "the session on the port ended" '! kill -0 $held 2>/dev/null'
check "its 5 V off" '[ "$(cat $T/sys/leds/grove_5v_out/brightness)" = 0 ]'
check "a restart is asked for" '[ "$said" = restart ]'
check "no login may listen" '! $G login'
check "not ready until a start without the console" '! $G ready && $G status | grep -q restart=yes'
check "power on refused meanwhile" '! $G power on && [ "$(cat $T/sys/leds/grove_5v_out/brightness)" = 0 ]'

# --- the start after: the camera's ---
cp "$T/boot/slot-a/cmdline.txt" "$T/proc/cmdline"
echo 1 >"$T/sys/leds/grove_5v_out/brightness"
reset_calls
$G boot
check "early in the start its 5 V goes off" '[ "$(cat $T/sys/leds/grove_5v_out/brightness)" = 0 ]'
check "ready now" '$G ready && $G status | grep -q "ready=yes"'
check "power on and off as the shell asks" '$G power on && [ "$(cat $T/sys/leds/grove_5v_out/brightness)" = 1 ] && $G power off && [ "$(cat $T/sys/leds/grove_5v_out/brightness)" = 0 ]'
check "nothing rewritten, nothing silenced" '[ ! -s "$T/calls" ] && cmd_a | grep -q console=null'
sleep 30 &
getty=$!
echo $getty >"$T/run/catalyst-serial.pid"
check "a getty somehow running: not ready" '! $G ready && ! $G power on'
kill $getty
rm -f "$T/run/catalyst-serial.pid"

# --- a start whose command line came back with the console (an update
# installed by an older updater, a boot partition not written) ---
echo "$SHIPPED" >"$T/boot/slot-a/cmdline.txt"
cp "$T/boot/slot-a/cmdline.txt" "$T/proc/cmdline"
echo 1 >"$T/sys/leds/grove_5v_out/brightness"
reset_calls
$G boot
check "its 5 V off" '[ "$(cat $T/sys/leds/grove_5v_out/brightness)" = 0 ]'
check "the kernel's messages held back" 'calls | grep -q "dmesg -n 1"'
check "the command line right for the next start" 'cmd_a | grep -q console=null'
check "and the camera kept off until then" '! $G ready && ! $G power on'

# --- system B running: its own slot is the one written ---
echo "$SHIPPED" | sed 's|p2|p3|' >"$T/proc/cmdline"
cp "$T/boot/slot-a/cmdline.txt" "$T/slot-a.before"
$G boot
check "system B's slot written, A's left" 'grep -q console=null "$T/boot/slot-b/cmdline.txt" && cmp -s "$T/boot/slot-a/cmdline.txt" "$T/slot-a.before"'
echo "$SHIPPED" >"$T/proc/cmdline"
cp "$T/slot-b.before" "$T/boot/slot-b/cmdline.txt"

# --- back to the console ---
cp "$T/boot/slot-a/cmdline.txt" "$T/proc/cmdline"
reset_calls
$G console
check "console mode again" '[ "$($G mode)" = console ] && [ ! -e "$T/data/grove-port" ]'
check "the shipped command line back" '[ "$(cmd_a)" = "$SHIPPED" ]'
check "the login started" 'calls | grep -q "rc-service catalyst-serial start"'
check "the kernel's loglevel back" 'calls | grep -q "dmesg -n 3"'
check "a login may listen" '$G login'
# A command line with no console at all gets the port's back.
echo "root=/dev/mmcblk0p2 ro console=null" >"$T/boot/slot-a/cmdline.txt"
cp "$T/boot/slot-a/cmdline.txt" "$T/proc/cmdline"
$G boot
check "a start in console mode with console=null puts the port's back" 'cmd_a | grep -q "console=ttyAMA0,115200" && ! cmd_a | grep -q "console=null"'

kill "$held" 2>/dev/null || true
if [ $fails -ne 0 ]; then
	echo "$fails failed"
	exit 1
fi
echo "all passed"

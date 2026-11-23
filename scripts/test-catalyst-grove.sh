#!/bin/sh
# catalyst-grove against a scratch boot partition, /proc and /sys: the port
# moved to the smart camera at once (the login stopped, a session on the
# port hung up, the kernel's console quieted: off the port, emergencies
# only, SysRq off) and ready for the camera without a restart, while this
# system's command line (its slot only) keeps the console off the port from
# the next start; a start whose command line came back with the console;
# a kernel without ttynull; and back to the console at once. Mounting,
# OpenRC and dmesg are stood in for (dmesg writes the scratch printk). Run
# anywhere with a POSIX shell (WSL is fine):
#   sh scripts/test-catalyst-grove.sh
set -eu
HERE=$(cd "$(dirname "$0")/.." && pwd)
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
mkdir -p "$T/boot/slot-a" "$T/boot/slot-b" "$T/data" "$T/bin" "$T/proc/tty" "$T/proc/sys/kernel" "$T/sys/leds/grove_5v_out" "$T/sys/leds/grove_fun" "$T/sys/tty/ttyAMA0" "$T/run" "$T/procfs"

SHIPPED="root=/dev/mmcblk0p2 rootfstype=ext4 ro rootwait console=ttyAMA0,115200 loglevel=3 vt.global_cursor_default=0 cma=64M panic=10"
echo "$SHIPPED" >"$T/boot/slot-a/cmdline.txt"
echo "$SHIPPED" | sed 's|p2|p3|' >"$T/boot/slot-b/cmdline.txt"
cp "$T/boot/slot-b/cmdline.txt" "$T/slot-b.before"
echo "$SHIPPED" >"$T/proc/cmdline"
echo 1 >"$T/sys/leds/grove_5v_out/brightness"
# The port's lines left on I2C by something else: power on puts them on the UART.
echo 1 >"$T/sys/leds/grove_fun/brightness"
printf '/dev/tty             /dev/tty        5       0 system:/dev/tty\nserial               /dev/ttyAMA     204 64-77 serial\n' >"$T/proc/tty/drivers"
# The kernel as it starts: its console on the port, at loglevel 3, SysRq as
# the Pi's kernel has it (0x1f6).
printf '3\t4\t1\t3\n' >"$T/proc/sys/kernel/printk"
echo 502 >"$T/proc/sys/kernel/sysrq"
echo Y >"$T/sys/tty/ttyAMA0/console"

# OpenRC and dmesg stood in for: each call noted; dmesg -n sets the level.
printf '#!/bin/sh\necho "rc-service $*" >>"%s/calls"\n' "$T" >"$T/bin/rc-service"
cat >"$T/bin/dmesg" <<EOF
#!/bin/sh
echo "dmesg \$*" >>"$T/calls"
[ "\$1" = -n ] && printf '%s\t4\t1\t3\n' "\$2" >"$T/proc/sys/kernel/printk"
exit 0
EOF
chmod +x "$T/bin/rc-service" "$T/bin/dmesg"
PATH="$T/bin:$PATH"
export PATH

sed -e "s|^MNT=.*|MNT=$T/boot|" \
	-e "s|^SETTING=.*|SETTING=$T/data/grove-port|" \
	-e "s|^CMDLINE=.*|CMDLINE=$T/proc/cmdline|" \
	-e "s|^TTY_DRIVERS=.*|TTY_DRIVERS=$T/proc/tty/drivers|" \
	-e "s|^LED=.*|LED=$T/sys/leds/grove_5v_out/brightness|" \
	-e "s|^FUN=.*|FUN=$T/sys/leds/grove_fun/brightness|" \
	-e "s|^PROC=.*|PROC=$T/procfs|" \
	-e "s|^SERIAL_PID=.*|SERIAL_PID=$T/run/catalyst-serial.pid|" \
	-e "s|^CONSOLE_ATTR=.*|CONSOLE_ATTR=$T/sys/tty/ttyAMA0/console|" \
	-e "s|^PRINTK=.*|PRINTK=$T/proc/sys/kernel/printk|" \
	-e "s|^SYSRQ=.*|SYSRQ=$T/proc/sys/kernel/sysrq|" \
	-e "s|^SAVED_SYSRQ=.*|SAVED_SYSRQ=$T/run/sysrq|" \
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
level() { cut -f1 "$T/proc/sys/kernel/printk"; }
sysrq() { cat "$T/proc/sys/kernel/sysrq"; }
led() { cat "$T/sys/leds/grove_5v_out/brightness"; }
# A login session on the port: a live process with it open, to be ended.
session() {
	sleep 30 >/dev/null 2>&1 &
	pid=$!
	mkdir -p "$T/procfs/$pid/fd"
	ln -s /dev/ttyAMA0 "$T/procfs/$pid/fd/0"
	echo "$pid"
}

# --- as shipped: the console ---
check "the port is the console by default" '[ "$($G mode)" = console ]'
check "a login may listen" '$G login'
check "the camera may not be powered" '! $G ready && ! $G power on'
check "its 5 V left as it was" '[ "$(led)" = 1 ]'
$G boot
check "a start in console mode writes nothing" '[ "$(cmd_a)" = "$SHIPPED" ]'

# --- a kernel without ttynull: camera mode at once all the same, its
# command line left with the console (each start quiets it instead) ---
held=$(session)
getty_pid=$(sh -c 'sleep 30 >/dev/null 2>&1 & echo $!')
echo "$getty_pid" >"$T/run/catalyst-serial.pid"
# The stand-in rc-service ends the getty as OpenRC would.
printf '#!/bin/sh\necho "rc-service $*" >>"%s/calls"\n[ "$2" = stop ] && kill "$(cat %s/run/catalyst-serial.pid)" 2>/dev/null && rm -f %s/run/catalyst-serial.pid\nexit 0\n' "$T" "$T" "$T" >"$T/bin/rc-service"
reset_calls
said=$($G camera)
check "no ttynull: camera mode taken at once" '[ "$($G mode)" = camera ] && [ "$said" = ready ]'
check "the login stopped" 'calls | grep -q "rc-service catalyst-serial stop" && ! kill -0 $getty_pid 2>/dev/null'
check "the session on the port hung up" '! kill -0 $held 2>/dev/null'
check "the kernel's console taken off the port" '[ "$(cat $T/sys/tty/ttyAMA0/console)" = N ]'
check "its messages held back to emergencies" 'calls | grep -q "dmesg -n 1" && [ "$(level)" = 1 ]'
check "SysRq off, its old setting kept" '[ "$(sysrq)" = 0 ] && [ "$(cat $T/run/sysrq)" = 502 ]'
check "its 5 V off" '[ "$(led)" = 0 ]'
check "no login may listen" '! $G login'
check "ready at once, no restart" '$G ready && $G status | grep -q ready=yes'
check "power on and off as the shell asks" '$G power on && [ "$(led)" = 1 ] && $G power off && [ "$(led)" = 0 ]'
check "powered, the port's lines on the UART" '[ "$(cat $T/sys/leds/grove_fun/brightness)" = 0 ] && $G status | grep -q lines=uart'
check "status: no getty, nobody holding the port" '$G status | grep -q getty=none && $G status | grep -qx holders='
mv "$T/sys/leds/grove_5v_out" "$T/sys/leds/gone"
check "no switch for the 5 V: power on is an error, and says so" '! $G power on 2>$T/err && grep -q grove_5v_out $T/err && $G power off'
mv "$T/sys/leds/gone" "$T/sys/leds/grove_5v_out"
check "without ttynull the command line keeps its console" '[ "$(cmd_a)" = "$SHIPPED" ]'
check "the other system's command line untouched" 'cmp -s "$T/boot/slot-b/cmdline.txt" "$T/slot-b.before"'
# Its next start, console on the port: quieted early, the camera usable.
printf '3\t4\t1\t3\n' >"$T/proc/sys/kernel/printk"
echo 502 >"$T/proc/sys/kernel/sysrq"
echo Y >"$T/sys/tty/ttyAMA0/console"
rm -f "$T/run/sysrq"
echo 1 >"$T/sys/leds/grove_5v_out/brightness"
$G boot
check "a start without ttynull: quieted early, 5 V off, ready" '[ "$(led)" = 0 ] && [ "$(level)" = 1 ] && [ "$(sysrq)" = 0 ] && $G ready'
check "and its command line still has the console" '[ "$(cmd_a)" = "$SHIPPED" ]'
$G console >/dev/null
printf '3\t4\t1\t3\n' >"$T/proc/sys/kernel/printk"
echo Y >"$T/sys/tty/ttyAMA0/console"

# --- the smart camera, with ttynull ---
echo "ttynull              /dev/ttynull    240 0 console" >>"$T/proc/tty/drivers"
held=$(session)
reset_calls
said=$($G camera)
check "camera mode taken" '[ "$($G mode)" = camera ]'
check "and the camera's at once" '[ "$said" = ready ] && $G ready'
check "the login stopped" 'calls | grep -q "rc-service catalyst-serial stop"'
check "the session on the port hung up" '! kill -0 $held 2>/dev/null'
check "the kernel quiet on the port" '[ "$(cat $T/sys/tty/ttyAMA0/console)" = N ] && [ "$(level)" = 1 ] && [ "$(sysrq)" = 0 ] && $G status | grep -q quiet=yes'
check "this system's console is null from its next start" 'cmd_a | grep -q "console=null" && ! cmd_a | grep -q ttyAMA0'
check "the rest of its command line kept" '[ "$(cmd_a)" = "$(echo "$SHIPPED" | sed "s|console=ttyAMA0,115200|console=null|")" ]'
check "the other system's command line untouched" 'cmp -s "$T/boot/slot-b/cmdline.txt" "$T/slot-b.before"'
check "no restart asked for" '! $G status | grep -q restart'
check "power on works now" '$G power on && [ "$(led)" = 1 ] && $G power off'

# What ready refuses, each on its own.
sleep 30 &
getty=$!
echo $getty >"$T/run/catalyst-serial.pid"
check "a getty somehow running: not ready" '! $G ready && ! $G power on'
kill $getty
rm -f "$T/run/catalyst-serial.pid"
held=$(session)
check "someone else holding the port: not ready" '! $G ready && ! $G power on'
kill "$held"
check "the port given up: ready again" '$G ready'
echo 7 >"$T/proc/sys/kernel/printk"
echo Y >"$T/sys/tty/ttyAMA0/console"
check "the kernel's console loud on the port: not ready" '! $G ready'
printf '1\t4\t1\t3\n' >"$T/proc/sys/kernel/printk"
echo 1 >"$T/proc/sys/kernel/sysrq"
check "SysRq back on: not ready" '! $G ready'
echo 0 >"$T/proc/sys/kernel/sysrq"
check "quiet again: ready" '$G ready'

# --- the start after: the camera's, console=null ---
cp "$T/boot/slot-a/cmdline.txt" "$T/proc/cmdline"
echo 1 >"$T/sys/leds/grove_5v_out/brightness"
printf '3\t4\t1\t3\n' >"$T/proc/sys/kernel/printk"
reset_calls
$G boot
check "early in the start its 5 V goes off" '[ "$(led)" = 0 ]'
check "ready, with the console elsewhere" '$G ready && $G status | grep -q console=elsewhere'
check "nothing rewritten, nothing silenced" '[ ! -s "$T/calls" ] && cmd_a | grep -q console=null'

# --- a start whose command line came back with the console (an update
# installed by an older updater, a boot partition not written) ---
echo "$SHIPPED" >"$T/boot/slot-a/cmdline.txt"
cp "$T/boot/slot-a/cmdline.txt" "$T/proc/cmdline"
echo 1 >"$T/sys/leds/grove_5v_out/brightness"
echo Y >"$T/sys/tty/ttyAMA0/console"
echo 502 >"$T/proc/sys/kernel/sysrq"
rm -f "$T/run/sysrq"
reset_calls
$G boot
check "its 5 V off" '[ "$(led)" = 0 ]'
check "the kernel's console quieted at once" 'calls | grep -q "dmesg -n 1" && [ "$(cat $T/sys/tty/ttyAMA0/console)" = N ] && [ "$(sysrq)" = 0 ]'
check "the command line right for the next start" 'cmd_a | grep -q console=null'
check "and the camera usable meanwhile" '$G ready'

# --- system B running: its own slot is the one written ---
echo "$SHIPPED" | sed 's|p2|p3|' >"$T/proc/cmdline"
cp "$T/boot/slot-a/cmdline.txt" "$T/slot-a.before"
$G boot
check "system B's slot written, A's left" 'grep -q console=null "$T/boot/slot-b/cmdline.txt" && cmp -s "$T/boot/slot-a/cmdline.txt" "$T/slot-a.before"'
echo "$SHIPPED" >"$T/proc/cmdline"
cp "$T/slot-b.before" "$T/boot/slot-b/cmdline.txt"

# --- back to the console, at once ---
echo 1 >"$T/sys/leds/grove_5v_out/brightness"
reset_calls
$G console
check "console mode again" '[ "$($G mode)" = console ] && [ ! -e "$T/data/grove-port" ]'
check "its 5 V off" '[ "$(led)" = 0 ]'
check "the shipped command line back" '[ "$(cmd_a)" = "$SHIPPED" ]'
check "the login started" 'calls | grep -q "rc-service catalyst-serial start"'
check "the kernel's loglevel back" 'calls | grep -q "dmesg -n 3" && [ "$(level)" = 3 ]'
check "SysRq as it was" '[ "$(sysrq)" = 502 ] && [ ! -e "$T/run/sysrq" ]'
check "a login may listen" '$G login'
check "the camera may not be powered" '! $G ready && ! $G power on'
# A command line with no console at all gets the port's back.
echo "root=/dev/mmcblk0p2 ro console=null" >"$T/boot/slot-a/cmdline.txt"
cp "$T/boot/slot-a/cmdline.txt" "$T/proc/cmdline"
$G boot
check "a start in console mode with console=null puts the port's back" 'cmd_a | grep -q "console=ttyAMA0,115200" && ! cmd_a | grep -q "console=null"'

if [ $fails -ne 0 ]; then
	echo "$fails failed"
	exit 1
fi
echo "all passed"

#!/bin/sh
# Checks catalyst-time (the clock set from the network) on a computer, with
# stand-ins for ntpd, hwclock and logger: it keeps the time it was set, skips
# a fresh one, gives up on a network with no way out, and the DHCP hook asks
# only with a router.
#
#   sh scripts/test-catalyst-time.sh
set -eu
HERE="$(cd "$(dirname "$0")/.." && pwd)"
TIME="$HERE/os/board/cardputer-zero/rootfs-overlay/usr/sbin/catalyst-time"
HOOK="$HERE/os/board/cardputer-zero/rootfs-overlay/usr/share/udhcpc/default.script.d/50-catalyst-time"
WORK="$(mktemp -d)"
trap 'rm -rf "${WORK:?}"' EXIT
mkdir -p "$WORK/bin" "$WORK/run"
# ntpd answers at once, or never (NTPD_HANGS); hwclock notes that it was asked.
cat >"$WORK/bin/ntpd" <<'EOF'
#!/bin/sh
[ -n "${NTPD_HANGS:-}" ] && exec sleep 60
echo "$@" >"$CATALYST_RUN/ntpd-args"
EOF
cat >"$WORK/bin/hwclock" <<'EOF'
#!/bin/sh
echo "$@" >"$CATALYST_RUN/hwclock-args"
EOF
printf '#!/bin/sh\n' >"$WORK/bin/logger"
chmod +x "$WORK/bin/"*
export PATH="$WORK/bin:$PATH" CATALYST_RUN="$WORK/run" CATALYST_TIME_WAIT=2

fail() {
	echo "FAIL: $*" >&2
	exit 1
}

# Set from the network: the time it was, and the chip written (where there is one).
sh "$TIME" sync now || fail "a server answered, yet it failed"
[ -s "$WORK/run/time-synced" ] || fail "the time it was set is not kept"
grep -q -- "-p pool.ntp.org" "$WORK/run/ntpd-args" || fail "ntpd was not given the servers: $(cat "$WORK/run/ntpd-args")"
[ ! -e /dev/rtc0 ] || [ -s "$WORK/run/hwclock-args" ] || fail "the clock chip was not set"

# Asked again soon after: nothing to do.
rm -f "$WORK/run/ntpd-args"
sh "$TIME" sync || fail "a fresh clock is not a failure"
[ ! -e "$WORK/run/ntpd-args" ] || fail "asked again within six hours"

# A network with no way out: given up after the wait, and nothing kept.
rm -f "$WORK/run/time-synced"
start=$(date +%s)
if NTPD_HANGS=1 sh "$TIME" sync now; then fail "no server answered, yet it succeeded"; fi
[ $(($(date +%s) - start)) -lt 10 ] || fail "it did not give up"
[ ! -e "$WORK/run/time-synced" ] || fail "a time was kept that no server gave"
[ ! -d "$WORK/run/time-sync.lock" ] || fail "the lock was left behind"

# The DHCP hook: only a lease with a router asks.
cp "$HOOK" "$WORK/hook"
sed -i "s|/usr/sbin/catalyst-time|$TIME|" "$WORK/hook"
rm -f "$WORK/run/ntpd-args" "$WORK/run/time-synced"
router="" sh "$WORK/hook" bound
sleep 1
[ ! -e "$WORK/run/ntpd-args" ] || fail "asked with no router"
router=10.58.5.1 sh "$WORK/hook" deconfig
sleep 1
[ ! -e "$WORK/run/ntpd-args" ] || fail "asked on a lease let go"
router=10.58.5.1 sh "$WORK/hook" bound
sleep 1
[ -e "$WORK/run/ntpd-args" ] || fail "a lease with a router did not ask"

sh "$TIME" status | grep -q "set from the network" || fail "status: $(sh "$TIME" status)"
echo "catalyst-time: all checks pass"

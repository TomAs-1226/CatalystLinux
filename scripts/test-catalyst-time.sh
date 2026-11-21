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
# The web: a HEAD answer with a Date line, or none at all (WEB_DOWN).
cat >"$WORK/bin/curl" <<'EOF'
#!/bin/sh
[ -n "${WEB_DOWN:-}" ] && exit 7
printf 'HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nDate: Thu, 20 Nov 2026 20:41:07 GMT\r\n\r\n'
EOF
# Setting the clock is only noted.
cat >"$WORK/bin/set-clock" <<'EOF'
#!/bin/sh
echo "$@" >"$CATALYST_RUN/set-clock-args"
EOF
printf '#!/bin/sh\n' >"$WORK/bin/logger"
chmod +x "$WORK/bin/"*
export PATH="$WORK/bin:$PATH" CATALYST_RUN="$WORK/run" CATALYST_TIME_WAIT=2 CATALYST_TIME_SET="$WORK/bin/set-clock"

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

# The time servers' port blocked: the web's Date line sets it, in UTC.
rm -f "$WORK/run/time-synced"
NTPD_HANGS=1 sh "$TIME" sync now || fail "the web answered, yet it failed"
[ "$(cat "$WORK/run/set-clock-args")" = "2026-11-20 20:41:07" ] || fail "set from the web as: $(cat "$WORK/run/set-clock-args")"
[ -s "$WORK/run/time-synced" ] || fail "the time set from the web is not kept"
sh "$TIME" status | grep -q "set from the web" || fail "status: $(sh "$TIME" status)"

# A network with no way out: given up after the wait, and nothing kept.
rm -f "$WORK/run/time-synced" "$WORK/run/set-clock-args"
start=$(date +%s)
if NTPD_HANGS=1 WEB_DOWN=1 sh "$TIME" sync now; then fail "nothing answered, yet it succeeded"; fi
[ $(($(date +%s) - start)) -lt 15 ] || fail "it did not give up"
[ ! -e "$WORK/run/time-synced" ] || fail "a time was kept that nothing gave"
[ ! -e "$WORK/run/set-clock-args" ] || fail "the clock was set from nothing"
[ ! -d "$WORK/run/time-sync.lock" ] || fail "the lock was left behind"

# Someone already asking is waited for, and their answer is the answer.
mkdir "$WORK/run/time-sync.lock"
(
	sleep 2
	date +%s >"$WORK/run/time-synced"
	rmdir "$WORK/run/time-sync.lock"
) &
sh "$TIME" sync || fail "the asking under way answered, yet waiting for it failed"
wait

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

sh "$TIME" status | grep -q "set from the time servers" || fail "status: $(sh "$TIME" status)"
echo "catalyst-time: all checks pass"

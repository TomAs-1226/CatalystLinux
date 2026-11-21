#!/bin/sh
# Bring a CardputerZero on Catalyst Linux 0.4.0 or 0.4.1 to the newest
# release, which its own updater cannot do: its curl trusts no certificates,
# and it dies with "illegal instruction" unpacking an update. Attached to each
# release; on the handheld, in Terminal:
#
#   curl --cacert /etc/ssl/certs/ca-certificates.crt -fLO https://github.com/TomAs-1226/CatalystLinux/releases/latest/download/upgrade.sh
#   sh upgrade.sh
#
# It fetches the release's own updater to /data and lets it update the usual
# way: check, install into the spare system, then restart into it once to try
# it. Kept if it starts; otherwise the handheld goes back to the system it has
# now. Nothing in /data is touched but the update's own state.
set -eu
CA=/etc/ssl/certs/ca-certificates.crt
BASE=https://github.com/TomAs-1226/CatalystLinux/releases/latest/download
UPDATER=/data/catalyst-update-new

echo "1/4 fetching the new updater"
curl --cacert "$CA" -fsSL -o "$UPDATER.part" "$BASE/catalyst-update-arm64"
mv "$UPDATER.part" "$UPDATER"
chmod +x "$UPDATER"

# A fresh start for the update's state. The shell would start installs with
# the old updater by itself: that stays off until the new system runs
# (Settings > Updates > Download automatically turns it back on).
rm -rf /data/update
mkdir -p /data/update
echo "auto=off" >/data/update/prefs

echo "2/4 looking for the update"
"$UPDATER" check
"$UPDATER" status
echo
echo "3/4 downloading and installing it: a few minutes, leave this running"
"$UPDATER" install
echo "4/4 written: restarting into it"
sleep 2
"$UPDATER" apply

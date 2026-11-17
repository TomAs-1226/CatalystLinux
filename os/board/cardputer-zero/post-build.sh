#!/bin/sh
# Runs on the target tree before the root filesystem image is made.
set -eu

BUILD="${CATALYST_BUILD:-$HOME/catalyst-linux-build}"
SHELL_BIN="$BUILD/out/catalyst-shell"

if [ ! -x "$SHELL_BIN" ]; then
	echo "catalyst-shell has not been built: run scripts/build-shell.sh first" >&2
	exit 1
fi
install -D -m 0755 "$SHELL_BIN" "$TARGET_DIR/usr/bin/catalyst-shell"

for f in etc/init.d/catalyst-shell etc/init.d/catalyst-ssh etc/init.d/catalyst-net etc/init.d/catalyst-grow-data \
	etc/init.d/catalyst-wifi etc/init.d/catalyst-net-wifi \
	usr/sbin/catalyst-ssh-toggle usr/sbin/catalyst-net usr/sbin/catalyst-grow-data usr/sbin/catalyst-wifi-toggle; do
	chmod 0755 "$TARGET_DIR/$f"
done
mkdir -p "$TARGET_DIR/etc/runlevels/default" "$TARGET_DIR/etc/runlevels/boot" "$TARGET_DIR/data"
for service in catalyst-shell catalyst-ssh catalyst-net catalyst-wifi catalyst-net-wifi; do
	ln -sf "/etc/init.d/$service" "$TARGET_DIR/etc/runlevels/default/$service"
done
ln -sf /etc/init.d/catalyst-grow-data "$TARGET_DIR/etc/runlevels/boot/catalyst-grow-data"

# SSH is off unless switched on from the System screen: the Dropbear package's
# own boot script would start it on every boot, so only catalyst-ssh may.
# Nothing here is scheduled either, so no cron daemon.
rm -f "$TARGET_DIR/etc/init.d/S50dropbear" "$TARGET_DIR/etc/init.d/S50crond"

# The SSH keys for root live on the data partition, since / is read-only.
mkdir -p "$TARGET_DIR/root"
rm -rf "$TARGET_DIR/root/.ssh"
ln -s /data/ssh "$TARGET_DIR/root/.ssh"

cat > "$TARGET_DIR/etc/os-release" <<OS
NAME="Catalyst Linux"
ID=catalyst
PRETTY_NAME="Catalyst Linux"
OS

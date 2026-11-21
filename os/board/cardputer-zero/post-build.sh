#!/bin/sh
# Runs on the target tree before the root filesystem image is made.
set -eu

BUILD="${CATALYST_BUILD:-$HOME/catalyst-linux-build}"
SHELL_BIN="$BUILD/out/catalyst-shell"
UPDATE_BIN="$BUILD/out/catalyst-update"
AI_BIN="$BUILD/out/ai"

if [ ! -x "$SHELL_BIN" ] || [ ! -x "$UPDATE_BIN" ] || [ ! -x "$AI_BIN" ]; then
	echo "catalyst-shell has not been built: run scripts/build-image.sh toolchain, then scripts/build-shell.sh" >&2
	exit 1
fi
install -D -m 0755 "$SHELL_BIN" "$TARGET_DIR/usr/bin/catalyst-shell"
install -D -m 0755 "$UPDATE_BIN" "$TARGET_DIR/usr/bin/catalyst-update"
install -D -m 0755 "$AI_BIN" "$TARGET_DIR/usr/bin/ai"

# llama.cpp's tools: only llama-server is used (the on-device model, 7 MB
# with its libraries, libmtmd among them); the rest (cli, bench, quantize,
# perplexity, tts…) would be another 41 MB of the image for nothing.
for tool in "$TARGET_DIR"/usr/bin/llama-*; do
	if [ -e "$tool" ] && [ "${tool##*/}" != llama-server ]; then
		rm -f "$tool"
	fi
done
rm -f "$TARGET_DIR"/usr/bin/convert_hf_to_gguf.py

# Which version this is and when it was built, for updates to compare
# against (scripts/release.sh sets both; a plain build is the base version,
# built now).
VERSION="${CATALYST_VERSION:-$(cat "$BR2_EXTERNAL_CATALYST_PATH/VERSION")}"
BUILT_AT="${CATALYST_BUILD_TIME:-$(date +%s)}"
mkdir -p "$TARGET_DIR/etc/catalyst" "$TARGET_DIR/boot"
# `built` is the same moment as UTC words BusyBox's date -s reads: the clock
# never starts earlier than this (catalyst-early).
printf 'version=%s\nbuild=%s\nbuilt=%s\n' "$VERSION" "$BUILT_AT" "$(date -u -d "@$BUILT_AT" '+%Y-%m-%d %H:%M:%S')" > "$TARGET_DIR/etc/catalyst/version"
# What changed in each version, for Settings › About › Update history and
# the "What's new" shown once after an update: read on the handheld, offline.
install -D -m 0644 "$BR2_EXTERNAL_CATALYST_PATH/CHANGELOG.md" "$TARGET_DIR/usr/share/catalyst/changelog.md"
if [ ! -s "$TARGET_DIR/etc/catalyst/update.pub" ]; then
	echo "post-build: no update key in the image (scripts/release.sh makes one): it will refuse every update" >&2
fi

for f in etc/init.d/catalyst-early etc/init.d/catalyst-shell etc/init.d/catalyst-late etc/init.d/catalyst-net \
	etc/init.d/catalyst-wifi etc/init.d/catalyst-ssh etc/init.d/catalyst-update etc/init.d/catalyst-usb \
	usr/sbin/catalyst-ssh-toggle usr/sbin/catalyst-net usr/sbin/catalyst-grow-data usr/sbin/catalyst-wifi-toggle \
	usr/sbin/catalyst-usb usr/sbin/catalyst-power usr/sbin/catalyst-time usr/share/udhcpc/default.script.d/50-catalyst-time \
	usr/libexec/catalyst/git-credential etc/init.d/catalyst-tailscale usr/sbin/catalyst-tailscale \
	etc/init.d/catalyst-zram; do
	chmod 0755 "$TARGET_DIR/$f"
done
mkdir -p "$TARGET_DIR/data"

# The time zone: /etc/localtime leads to the one chosen in setup or Settings,
# a link on /data to its rules (none chosen: no file there, and the time is
# UTC). The second copy of every zone, with leap seconds ("right"), which
# nothing here reads, goes.
ln -sfn /data/catalyst/localtime "$TARGET_DIR/etc/localtime"
rm -f "$TARGET_DIR/etc/timezone"
rm -rf "$TARGET_DIR/usr/share/zoneinfo/right"

# The CardputerZero's CM0 carries a CYW43439 radio (brcmfmac asks for
# brcm/brcmfmac43439-sdio.raspberrypi,0-compute-module.*, then the plain
# name). Its firmware, clm_blob and board settings are in cypress/ (from
# Raspberry Pi's firmware-nonfree, with their licence); the names the
# driver asks for lead there, as Raspberry Pi OS lays them out.
for ext in bin clm_blob txt; do
	for name in brcmfmac43439-sdio brcmfmac43439-sdio.raspberrypi,0-compute-module; do
		ln -sfn "../cypress/cyfmac43439-sdio.$ext" "$TARGET_DIR/lib/firmware/brcm/$name.$ext"
	done
done

# --- what runs at boot, and in what order ---
#
#   sysinit   /sys and /dev
#   boot      catalyst-early (panel and key drivers, storage), catalyst-shell
#   default   catalyst-late (clock, hotplug, log), network, Wi-Fi, SSH, USB, Tailscale, serial login
#
# The shell is on screen before anything in "default" starts. Buildroot's
# stock set (two dozen services meant for a general machine, several of which
# fail on a read-only root) is replaced outright.
R="$TARGET_DIR/etc/runlevels"
rm -rf "$R"
mkdir -p "$R/sysinit" "$R/boot" "$R/default" "$R/shutdown" "$R/nonetwork"
link() { ln -sf "/etc/init.d/$2" "$R/$1/$2"; }
link sysinit sysfs
link sysinit devfs
link boot catalyst-early
link boot catalyst-shell
link default catalyst-late
link default catalyst-net
link default catalyst-wifi
link default catalyst-ssh
link default catalyst-update
link default catalyst-usb
link default catalyst-tailscale
link default catalyst-zram
link default agetty.ttyAMA0
link shutdown killprocs
link shutdown mount-ro

# Service scripts nothing above uses: OpenRC reads every one of them at each
# boot to work out the order, so the unused ones are pure delay. Among them
# are the Dropbear and cron scripts that would start those at every boot
# (SSH is only ever started by catalyst-ssh, when it is switched on).
for script in "$TARGET_DIR"/etc/init.d/*; do
	case "$(basename "$script")" in
	catalyst-early | catalyst-shell | catalyst-late | catalyst-net | catalyst-wifi | catalyst-ssh | catalyst-update | catalyst-usb | catalyst-tailscale) ;;
	sysfs | devfs | agetty | agetty.ttyAMA0 | killprocs | mount-ro | functions.sh) ;;
	*) rm -f "$script" ;;
	esac
done

# Libraries the toolchain and packages bring along that no program here
# loads (checked with readelf by scripts/inspect-image.sh). libgcc_s stays
# (the shell and plain C programs link it), as do libatomic (curl and libcurl
# link it), the C++ runtime (Mesa's shader compiler and libcamera are C++)
# and ncurses with its form, menu and panel libraries (vim, nano, htop, tmux
# and Python's curses use them). libturbojpeg is libjpeg-turbo's other API
# (catalyst-camera writes photos through libjpeg itself).
for lib in libgomp libkmod libpsx libss libturbojpeg \
	libnl-route-3 libnl-idiag-3 libnl-nf-3 libnl-xfrm-3; do
	rm -f "$TARGET_DIR"/lib/$lib.so* "$TARGET_DIR"/usr/lib/$lib.so*
done
# OpenGL ES 1, which Mesa builds alongside ES 2; nothing here draws with it.
rm -f "$TARGET_DIR"/usr/lib/libGLESv1_CM.so*

# Git as the handheld uses it: the one `git` program (status, log, diff,
# fetch and pull are built into it) and its HTTPS transport. Each tool below
# carries its own 2.3 MB copy of git's library, or needs Perl, which is not
# here: mail, a git server and daemon, the dumb-HTTP walker, the bridges to
# SVN, CVS and Perforce, merge-tool wrappers, gitweb. About 20 MB.
GIT_CORE="$TARGET_DIR/usr/libexec/git-core"
if [ -d "$GIT_CORE" ]; then
	for tool in git-imap-send git-http-fetch git-http-push git-daemon git-http-backend git-shell scalar \
		git-sh-i18n--envsubst git-p4 git-send-email git-svn git-archimport git-cvsimport git-cvsserver \
		git-cvsexportcommit git-instaweb git-web--browse git-mergetool git-mergetool--lib git-difftool--helper; do
		rm -f "$GIT_CORE/$tool"
	done
	rm -rf "$GIT_CORE/mergetools"
	rm -f "$TARGET_DIR/usr/bin/git-cvsserver" "$TARGET_DIR/usr/bin/git-shell" "$TARGET_DIR/usr/bin/scalar"
	rm -rf "$TARGET_DIR/usr/share/perl5" "$TARGET_DIR/usr/share/gitweb"
fi

# The SSH keys for root live on the data partition, since / is read-only.
mkdir -p "$TARGET_DIR/root"
rm -rf "$TARGET_DIR/root/.ssh"
ln -s /data/ssh "$TARGET_DIR/root/.ssh"

cat > "$TARGET_DIR/etc/os-release" <<OS
NAME="Catalyst Linux"
ID=catalyst
PRETTY_NAME="Catalyst Linux"
OS

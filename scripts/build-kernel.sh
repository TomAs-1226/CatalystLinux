#!/bin/bash
# Rebuild the kernel after its configuration (os/board/cardputer-zero/
# linux.fragment) changed, and everything that has to match it. Run inside
# WSL; follow with build-image.sh. A changed kernel takes 15 to 30 minutes.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
BUILD="${CATALYST_BUILD:-$HOME/catalyst-linux-build}"

# Drivers built for the old configuration must not survive into the image:
# one loaded into a kernel laid out differently would crash it.
rm -rf "$BUILD"/output/target/lib/modules

# The kernel, then the board's own drivers against it, then BusyBox (its
# configuration lives beside the kernel's).
bash "$HERE/build-image.sh" reconfigure linux-reconfigure m5stack-cardputer-zero-rebuild busybox-reconfigure

# Did every switch land? A switch can be held on by something that needs it.
CONFIG="$BUILD/output/build/linux-custom/.config"
echo "switches that did not land as asked:"
missed=0
while read -r line; do
	case "$line" in
	"# CONFIG_"*" is not set")
		name=${line#\# }
		name=${name% is not set}
		if grep -q "^$name=" "$CONFIG"; then
			echo "  still on: $(grep "^$name=" "$CONFIG")"
			missed=$((missed + 1))
		fi
		;;
	CONFIG_*=*)
		grep -qx "$line" "$CONFIG" || {
			echo "  wanted $line, got: $(grep -E "^(# )?${line%%=*}[ =]" "$CONFIG" || echo nothing)"
			missed=$((missed + 1))
		}
		;;
	esac
done <"$HERE/../os/board/cardputer-zero/linux.fragment"
echo "  ($missed)"
ls -l "$BUILD/output/images/Image"
size "$BUILD/output/build/linux-custom/vmlinux"

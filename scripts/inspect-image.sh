#!/bin/bash
# Report what the built image contains: boot files, drivers, services, sizes.
# Run inside WSL after build-image.sh.
set -uo pipefail

BUILD="${CATALYST_BUILD:-$HOME/catalyst-linux-build}"
O="$BUILD/output"
T="$O/target"
export PATH="$O/host/bin:$O/host/sbin:$PATH"

echo "== boot partition"
mdir -i "$O/images/boot.vfat" :: | sed -n '4,$p' | grep -v '^$'
echo "-- system A's boot folder: $(mdir -i "$O/images/boot.vfat" ::slot-a | grep -ci -e Image -e dtb) kernel and device tree files, $(mdir -i "$O/images/boot.vfat" ::slot-a/overlays | grep -ci dtbo) overlays"
echo "-- which system starts: $(mtype -i "$O/images/boot.vfat" ::config.txt | grep -A3 '^\[all\]' | tail -3 | tr '\n' ' ')"
echo "-- system A's command line: $(mtype -i "$O/images/boot.vfat" ::slot-a/cmdline.txt)"
echo "-- config.txt names these overlays:"
mtype -i "$O/images/boot.vfat" ::config.txt | grep '^dtoverlay' | while IFS='=,' read -r _ name _; do
	if mdir -i "$O/images/boot.vfat" "::slot-a/overlays/$name.dtbo" >/dev/null 2>&1; then echo "   ok      $name"; else echo "   MISSING $name"; fi
done

echo "== kernel"
KVER=$(ls "$T/lib/modules")
echo "version $KVER, Image $(du -h "$O/images/Image" | cut -f1)"
for opt in DRM_PANEL_MIPI_DBI DRM_FBDEV_EMULATION BACKLIGHT_CLASS_DEVICE PWM I2C_BCM2835 SPI_BCM2835 BRCMFMAC USB_DWC2 EXT4_FS SWAP MEMCG FTRACE DEVTMPFS_MOUNT VIDEO_IMX219 INPUT_EVDEV FB_DEFERRED_IO; do
	printf '   %-26s %s\n' "$opt" "$(grep -E "^CONFIG_$opt=|# CONFIG_$opt is not set" "$O/build/linux-custom/.config" | sed 's/^CONFIG_[A-Z0-9_]*=//; s/^# .* is not set/off/')"
done

echo "== board modules named in modules-load.d (the early list, then the late one)"
LIST=$(mktemp)
cat "$T/etc/modules-load.d/cardputer-zero.conf" "$T/etc/modules-load.d/cardputer-zero.late" > "$LIST"
while read -r mod; do
	case "$mod" in ''|\#*) continue ;; esac
	file=$(find "$T/lib/modules/$KVER" -name "$(echo "$mod" | tr '-' '?').ko*" -o -name "$(echo "$mod" | tr '_' '?').ko*" | head -1)
	if [ -n "$file" ]; then echo "   ok      $mod  (${file#$T/lib/modules/$KVER/})"; else echo "   MISSING $mod"; fi
done < "$LIST"
echo "-- modules.dep: $(wc -l < "$T/lib/modules/$KVER/modules.dep") entries; the panel driver depends on:"
grep -a '^updates/panel-mipi-dbi-m' "$T/lib/modules/$KVER/modules.dep" | cut -d: -f2 | tr ' ' '\n' | grep . | sed 's/^/   /'
echo "-- module loader: $(readlink -f "$T/sbin/modprobe" | sed "s|$T||"); xz support: target $(grep -c 'BR2_PACKAGE_XZ=y' "$O/.config"), host depmod $(grep -c 'BR2_PACKAGE_HOST_KMOD_XZ=y' "$O/.config")"

echo "== root filesystem"
echo "used: $(du -sh "$T" | cut -f1) in a $(du -h --apparent-size "$O/images/rootfs.ext2" | cut -f1) image; modules: $(du -sh "$T/lib/modules" | cut -f1); firmware: $(du -sh "$T/lib/firmware" | cut -f1)"
echo "sdcard.img: $(du -h --apparent-size "$O/images/sdcard.img" | cut -f1)"

echo "== does it run? (user-mode emulation of the target's own programs)"
if command -v qemu-aarch64-static >/dev/null; then
	echo "   busybox:  $(qemu-aarch64-static -L "$T" "$T/bin/busybox" 2>&1 | head -1)"
	echo "   openrc:   $(qemu-aarch64-static -L "$T" "$T/sbin/openrc" --version 2>&1 | head -1)"
	echo "   dropbear: $(qemu-aarch64-static -L "$T" "$T/usr/sbin/dropbear" -V 2>&1 | head -1)"
	echo "   shell:    $(qemu-aarch64-static -L "$T" "$T/usr/bin/catalyst-shell" --demo --bench 2>&1 | head -1)"
	echo "-- each named module resolves to a load order:"
	while read -r mod; do
		case "$mod" in ''|\#*) continue ;; esac
		# kmod acts as modprobe only when called by that name.
		n=$(qemu-aarch64-static -L "$T" "$T/sbin/modprobe" -d "$T" -S "$KVER" --show-depends "$mod" 2>&1 | grep -c '^insmod')
		printf '   %-26s %s modules\n' "$mod" "$n"
	done < "$LIST"
fi
rm -f "$LIST"
ls -l "$T/usr/bin/catalyst-shell" "$T/lib/firmware/cardputerzero,st7789v_lcd.bin" 2>&1 | sed 's/^/   /'
echo "-- wifi firmware for the CM0:"
ls "$T/lib/firmware/brcm" 2>/dev/null | grep -iE "4343|43430|43436|compute" | sed 's/^/   /'
echo "-- runlevels"
for level in sysinit boot default shutdown; do echo "   $level: $(ls "$T/etc/runlevels/$level" 2>/dev/null | tr '\n' ' ')"; done
echo "   service scripts in the image: $(ls "$T/etc/init.d" | tr '\n' ' ')"
for link in "$T"/etc/runlevels/*/*; do [ -e "$T$(readlink "$link")" ] || echo "   MISSING the script behind $link"; done
if ls "$T/etc/init.d" | grep -q dropbear; then echo "   WARNING: a dropbear boot script is present; SSH would start at boot"; else echo "   ssh: started only by catalyst-ssh, when /data/catalyst/ssh-enabled exists"; fi
echo "-- libc: $(ls "$T/lib" | grep -E 'ld-musl|libc\.so' | tr '\n' ' ')"
echo "-- biggest things under /usr and /lib:"
du -s "$T"/usr/bin "$T"/usr/sbin "$T"/usr/lib "$T"/lib/firmware "$T"/lib/modules 2>/dev/null | sort -rn | awk '{printf "   %6.1f MB  %s\n", $1/1024, $2}'
echo "-- shared libraries: every one a program asks for is there, and none is there unasked"
cd "$T" || exit 1
needed=$(find bin sbin usr/bin usr/sbin usr/lib usr/libexec lib -type f ! -name '*.ko*' 2>/dev/null | xargs readelf -d 2>/dev/null | awk -F'[][]' '/NEEDED/ { print $2 }' | sort -u)
# The shell loads Mesa when it starts (dlopen), so nothing names it as needed.
needed=$(printf '%s
libEGL.so.1
libGLESv2.so.2
' "$needed" | sort -u)
for gl in libEGL.so.1 libGLESv2.so.2; do
	[ -e "usr/lib/$gl" ] && echo "   gpu: $gl is there for the shell" || echo "   MISSING $gl: the shell will draw on the processor alone"
done
echo "   the shell: $(readelf -l usr/bin/catalyst-shell 2>/dev/null | awk '/interpreter/ { gsub(/[][]/, ""); print "loaded by " $NF }')"
for soname in $needed; do
	[ -e "lib/$soname" ] || [ -e "usr/lib/$soname" ] || echo "   MISSING $soname"
done
for f in lib/*.so* usr/lib/*.so*; do
	[ -f "$f" ] && [ ! -L "$f" ] || continue
	soname=$(readelf -d "$f" 2>/dev/null | awk -F'[][]' '/SONAME/ { print $2 }')
	[ -z "$soname" ] || echo "$needed" | grep -qx "$soname" || echo "   unused  $f"
done
echo "   $(echo "$needed" | wc -w) libraries asked for"

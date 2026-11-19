#!/bin/bash
# Assembles sdcard.img: boot (FAT), system A and system B (ext4, read-only
# when running), data (ext4). See genimage.cfg.in for the layout.
#
# The boot partition holds the firmware and config.txt at its root, and each
# system's kernel, device trees, overlays and command line in a folder of its
# own: config.txt's os_prefix picks the folder, so the firmware starts system
# A, or, for one boot after `reboot "0 tryboot"`, system B ([tryboot]).
set -e

BOARD_DIR="$(dirname "$0")"
GENIMAGE_CFG="${BINARIES_DIR}/genimage.cfg"
GENIMAGE_TMP="${BUILD_DIR}/genimage.tmp"
FW="${BINARIES_DIR}/rpi-firmware"
STAGE="${BINARIES_DIR}/bootfs"

rm -rf "${STAGE}"
mkdir -p "${STAGE}/slot-a" "${STAGE}/slot-b"

# The firmware itself, at the root.
for f in "${FW}"/bootcode.bin "${FW}"/start*.elf "${FW}"/fixup*.dat; do
	[ -e "$f" ] && cp "$f" "${STAGE}/"
done

# System A's kernel, device trees and overlays.
kernel="$(sed -n 's/^kernel=//p' "${FW}/config.txt")"
cp "${BINARIES_DIR}/${kernel}" "${STAGE}/slot-a/"
cp "${BINARIES_DIR}"/*.dtb "${STAGE}/slot-a/"
cp -r "${FW}/overlays" "${STAGE}/slot-a/"
# Its command line: its own root partition, and a panic restarts the
# handheld (so a new system that panics falls back to the old one).
sed -e 's|root=[^ ]*|root=/dev/mmcblk0p2|' -e 's|$| panic=10|' "${FW}/cmdline.txt" > "${STAGE}/slot-a/cmdline.txt"
# System B's folder stays empty until an update fills it.
echo "System B's kernel goes here when an update is installed." > "${STAGE}/slot-b/README.txt"

# config.txt, ending with which system starts. catalyst-update rewrites only
# this last block: [all] is the system in use, [tryboot] the one tried once.
{
	cat "${FW}/config.txt"
	cat <<'SLOTS'

# Which system starts: written by catalyst-update. [all] is the system in
# use; [tryboot] the one an update is tried in, once, after reboot "0 tryboot".
[all]
os_prefix=slot-a/
[tryboot]
os_prefix=slot-b/
SLOTS
} > "${STAGE}/config.txt"

# The boot partition's files, as genimage lists them (folders are copied whole).
FILES=()
for i in "${STAGE}"/*; do
	FILES+=("${i#${BINARIES_DIR}/}")
done
BOOT_FILES=$(printf '\\t\\t\\t"%s",\\n' "${FILES[@]}")
sed "s|#BOOT_FILES#|${BOOT_FILES}|" "${BOARD_DIR}/genimage.cfg.in" > "${GENIMAGE_CFG}"

trap 'rm -rf "${ROOTPATH_TMP}"' EXIT
ROOTPATH_TMP="$(mktemp -d)"
rm -rf "${GENIMAGE_TMP}"

genimage \
	--rootpath "${ROOTPATH_TMP}" \
	--tmppath "${GENIMAGE_TMP}" \
	--inputpath "${BINARIES_DIR}" \
	--outputpath "${BINARIES_DIR}" \
	--config "${GENIMAGE_CFG}"

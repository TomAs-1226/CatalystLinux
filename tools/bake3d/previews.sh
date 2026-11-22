#!/bin/bash
# The previews of the robot in 3D, from a bake: the shipped pack, then the
# shell's own gallery (`catalyst-shell --objects`) into shots/3d-preview.
#
#   tools/bake3d/previews.sh BAKED     (Git Bash or WSL; the simulator's profile)
#
# Settings and logs go to a folder of their own, so the gallery never
# touches the simulator's.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
baked="$1"
py="${PYTHON:-py -3}"
work="${TMPDIR:-${TEMP:-/tmp}}/catalyst-3d-previews"
mkdir -p "$work"
$py -W ignore "$here/pack.py" "$baked" "$repo/userland/assets/objects/objects.pack"
rm -rf "$work/conf" "$repo/shots/3d-preview"
export CATALYST_CONF="$work/conf/robot" CATALYST_LOGS="$work/conf/logs"
mkdir -p "$work/conf"
cd "$repo/userland"
cargo run --profile sim -p catalyst-shell -- --objects "$repo/shots/3d-preview"

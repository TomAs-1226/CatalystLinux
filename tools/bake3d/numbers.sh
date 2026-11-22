#!/bin/bash
# Numbers, FRC 5805's robot, for the bake: its model and manifest from the
# team website (TomAs-1226/smblyrequired, public/models/), at the commit
# below, checked by their SHA-256, into tools/bake3d/numbers/ (not kept in
# this repository: 5 MB, and the website's to change).
#
#   tools/bake3d/numbers.sh          (Git Bash or WSL, with gh signed in)
#
# Then: blender -b --factory-startup -P tools/bake3d/bake.py -- OUT robot
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="TomAs-1226/smblyrequired"
ref="22d1461bf474ccd308256327ba54ae46adb8323a"
dir="$here/numbers"
mkdir -p "$dir"
fetch() {
	local file="$1" sum="$2"
	if [ -f "$dir/$file" ] && echo "$sum  $dir/$file" | sha256sum -c --status; then
		return
	fi
	gh api "repos/$repo/contents/public/models/$file?ref=$ref" -H "Accept: application/vnd.github.raw" >"$dir/$file.part"
	echo "$sum  $dir/$file.part" | sha256sum -c --status || { echo "numbers.sh: $file is not the one this bake was made from" >&2; rm -f "$dir/$file.part"; exit 1; }
	mv "$dir/$file.part" "$dir/$file"
}
fetch robot.glb 85c86412035a7d8dc9641a34b1e81084bf406e8ab48fd6cb060eaba61f34c69d
fetch robot.json 4491121bf5fbff8acde2331d7fd00805682c9d3d2b81297ae6c5d5e985c9adc5
echo "numbers.sh: $dir has Numbers' model and manifest"

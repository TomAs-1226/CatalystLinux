#!/bin/bash
# A team's own robot for the Robot app, from its CAD, in one command:
#
#   tools/bake3d/robot.sh MODEL [MAP] [OUT]
#
# MODEL is the robot as Onshape (or any CAD) exports it: glTF/GLB (best:
# Onshape keeps every part's name and colour), STEP (.step/.stp, converted
# with gmsh, set up in tools/bake3d/.venv the first time), or OBJ, STL, FBX
# or PLY. MAP says which CAD parts are which part of the robot (robot.map
# is the starting point; copy it beside the model). OUT is the pack to make,
# robot3d.pack by default: copy it to the handheld's /data/catalyst/ and the
# Robot app shows it in place of the shell's own robot (no update needed).
#
# Runs in Git Bash or WSL with Blender 5.2 ($BLENDER, else its usual place)
# and Python 3 with numpy and Pillow ($PYTHON, else `py -3` or python3).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
model="$1"
map="${2:-$here/robot.map}"
out="${3:-robot3d.pack}"
blender="${BLENDER:-}"
if [ -z "$blender" ]; then
	for b in "/c/Program Files/Blender Foundation/Blender 5.2/blender.exe" "/mnt/c/Program Files/Blender Foundation/Blender 5.2/blender.exe" "$(command -v blender || true)"; do
		[ -n "$b" ] && [ -x "$b" ] && blender="$b" && break
	done
fi
[ -n "$blender" ] || { echo "robot.sh: no Blender 5.2 found (set BLENDER)" >&2; exit 1; }
py="${PYTHON:-}"
if [ -z "$py" ]; then
	if command -v py >/dev/null; then py="py -3"; else py="python3"; fi
fi
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
# Paths Blender (a Windows program here) can read.
winpath() { if command -v cygpath >/dev/null; then cygpath -w "$1"; elif command -v wslpath >/dev/null && [[ "$blender" == /mnt/* ]]; then wslpath -w "$1"; else echo "$1"; fi; }

case "${model,,}" in
*.step | *.stp)
	venv="$here/.venv"
	if [ ! -d "$venv" ]; then
		echo "robot.sh: setting up gmsh to read STEP, once" >&2
		$py -m venv "$venv"
		"$venv"/Scripts/python* -m pip install -q gmsh 2>/dev/null || "$venv"/bin/python -m pip install -q gmsh
	fi
	vpy="$(ls "$venv"/Scripts/python.exe "$venv"/bin/python 2>/dev/null | head -1)"
	"$vpy" "$here/step2obj.py" "$model" "$work/robot.obj"
	model="$work/robot.obj"
	;;
esac

"$blender" -b --factory-startup -P "$(winpath "$here/bake.py")" -- "$(winpath "$work/baked")" --robot "$(winpath "$model")" "$(winpath "$map")" 2>&1 | grep -E "robot.sh|bake3d|Error|rror:" || true
[ -f "$work/baked/robot-team/meta.json" ] || { echo "robot.sh: the bake did not finish" >&2; exit 1; }
$py -W ignore "$here/pack.py" "$work/baked" "$out" robot-team
echo "robot.sh: $out is ready: copy it to /data/catalyst/robot3d.pack on the handheld"

#!/bin/bash
# Cut the two fonts down to what the UI draws, from the full files in
# userland/assets/fonts/full into userland/assets/fonts. Run inside WSL
# whenever the character set or the axes the UI uses change; the results are
# committed, so an ordinary build needs none of this.
#
# Kept: Latin (English and Western/Central European), digits, punctuation,
# arrows, the line-drawing characters terminals use, and a few symbols.
# Dropped: other alphabets, and the design axes the UI never moves.
set -euo pipefail
HERE="$(cd "$(dirname "$0")/.." && pwd)"
BUILD="${CATALYST_BUILD:-$HOME/catalyst-linux-build}"
FONTS="$HERE/userland/assets/fonts"
PY="$BUILD/tools/python"

# fontTools, fetched into the build folder (it is pure Python; no pip or root needed).
if [ ! -d "$PY/fontTools" ]; then
	mkdir -p "$PY"
	python3 - "$PY" <<'EOF'
import io, json, sys, urllib.request, zipfile
meta = json.load(urllib.request.urlopen("https://pypi.org/pypi/fonttools/json"))
wheel = next(u for u in meta["urls"] if u["filename"].endswith("py3-none-any.whl"))
print("fetching", wheel["filename"])
zipfile.ZipFile(io.BytesIO(urllib.request.urlopen(wheel["url"]).read())).extractall(sys.argv[1])
EOF
fi
export PYTHONPATH="$PY"

# Basic Latin, Latin-1, Latin Extended-A; punctuation and currency; arrows;
# a few mathematical signs; box drawing and blocks; the replacement character.
RANGES="U+0020-007E,U+00A0-017F,U+2010-2027,U+2030-203A,U+20AC,U+2122,U+2190-2199,U+21B5,U+21E7,U+2318,U+2325,U+2212,U+2248,U+2260,U+2264,U+2265,U+221E,U+2500-257F,U+2580-259F,U+25A0,U+25B2,U+25B6,U+25BC,U+25C0,U+25CF,U+2713,U+2717,U+FFFD"

# $1: font file name, $2: the axes the UI moves and how far (the rest are fixed at their defaults)
subset() {
	local full="$FONTS/full/$1" out="$FONTS/$1" pinned="$BUILD/tools/pinned-$1"
	python3 - "$full" "$pinned" "$2" <<'EOF'
import sys
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer
font = TTFont(sys.argv[1])
# "wght=300:700,ROND=0:50": the axes kept, each with the span the UI may ask for.
keep = {tag: tuple(float(v) for v in span.split(":")) for tag, span in (part.split("=") for part in sys.argv[3].split(","))}
axes = {a.axisTag: (a.minValue, a.defaultValue, a.maxValue) for a in font["fvar"].axes}
print("  axes:", ", ".join(f"{tag} {lo:g}..{hi:g}" for tag, (lo, _, hi) in axes.items()))
limits = {tag: keep.get(tag) for tag in axes}
print("  kept:", ", ".join(f"{tag} {span[0]:g}..{span[1]:g}" if span else f"{tag} fixed" for tag, span in limits.items()))
font = instancer.instantiateVariableFont(font, limits)
font.save(sys.argv[2])
EOF
	python3 -m fontTools.subset "$pinned" --unicodes="$RANGES" --output-file="$out" \
		--layout-features='kern,liga,calt,ccmp,locl,mark,mkmk,tnum,zero' --notdef-outline --no-hinting --desubroutinize \
		--name-IDs='*' --name-languages='*' --ignore-missing-unicodes
	rm -f "$pinned"
	printf '  %s: %s -> %s bytes\n' "$1" "$(stat -c %s "$full")" "$(stat -c %s "$out")"
}

echo "Google Sans Flex"
# Weights light to bold, the widths and roundness the display style uses, sizes up to 72 px.
subset GoogleSansFlex.ttf wght=300:700,wdth=85:125,ROND=0:100,opsz=6:72
echo "Google Sans Code"
subset GoogleSansCode.ttf wght=300:800

#!/bin/bash
# Publish the packed catalogue (catalyst-apps/dist, from pack-catalogue.sh)
# as the `catalogue` release of TomAs-1226/catalyst-apps, where every
# handheld's Catalogue reads it:
#
#   https://github.com/TomAs-1226/catalyst-apps/releases/download/catalogue/index.json
#
# The packages go up first and the index last, so a handheld reading the
# index meanwhile never finds a package missing. Packages no longer in the
# index are taken down afterwards. Needs the GitHub CLI, signed in (WSL's
# own, or Windows' gh.exe).
set -euo pipefail

HERE="$(cd "$(dirname "$0")/.." && pwd)"
DIST="$HERE/catalyst-apps/dist"
REPO="${CATALYST_APPS_REPO:-TomAs-1226/catalyst-apps}"
TAG=catalogue

GH=gh
host_path() { printf '%s\n' "$1"; }
if ! command -v gh >/dev/null 2>&1 && command -v gh.exe >/dev/null 2>&1; then
	GH=gh.exe
	host_path() { wslpath -w "$1"; }
fi
command -v "$GH" >/dev/null || { echo "publishing needs the GitHub CLI (gh)" >&2; exit 1; }
[ -s "$DIST/index.json" ] || { echo "nothing packed: run scripts/pack-catalogue.sh first" >&2; exit 1; }

if ! "$GH" release view "$TAG" --repo "$REPO" >/dev/null 2>&1; then
	"$GH" release create "$TAG" --repo "$REPO" --title "Catalyst apps" \
		--notes "The app catalogue Catalyst Linux handhelds read. Every package is signed; see index.json."
fi
packages=()
for f in "$DIST"/*.tar.gz; do
	packages+=("$(host_path "$f")")
done
"$GH" release upload "$TAG" --repo "$REPO" --clobber "${packages[@]}"
"$GH" release upload "$TAG" --repo "$REPO" --clobber "$(host_path "$DIST/index.json")"

# Packages the index no longer names.
keep=$(cd "$DIST" && ls -- *.tar.gz)
"$GH" release view "$TAG" --repo "$REPO" --json assets --jq '.assets[].name' | while read -r asset; do
	case "$asset" in
	index.json) ;;
	*.tar.gz) printf '%s\n' "$keep" | grep -qxF "$asset" || "$GH" release delete-asset "$TAG" "$asset" --repo "$REPO" --yes ;;
	esac
done
echo "published $(printf '%s\n' "$keep" | wc -l) apps to $REPO's $TAG release"

#!/bin/bash
# Build Catalyst Linux and make a release of it: what handhelds update from
# (over the air), and the card image the flasher and Raspberry Pi Imager
# write. Run inside WSL.
#
#   bash scripts/release.sh nightly            build and make a nightly release here
#   bash scripts/release.sh stable --publish   build, make, and publish on GitHub
#   bash scripts/release.sh nightly --publish --notes "faster glass"
#
# A stable release is version os/VERSION, tagged v<version>. A nightly one is
# that version plus the date and time, published as a pre-release; handhelds
# on the nightly channel take those. Each release carries a patch from the
# last one on its channel, so a handheld that is up to date downloads
# megabytes rather than the whole system.
#
# Releases are signed. The key is made the first time, kept in
# ~/catalyst-linux-build/keys (never in the repository), and its public half
# goes into the image: back the key up, because handhelds only take updates
# signed with it.
#
# Publishing needs the GitHub CLI signed in (gh auth login) and the
# repository (CATALYST_REPO, default TomAs-1226/CatalystLinux) to exist.
set -euo pipefail

HERE="$(cd "$(dirname "$0")/.." && pwd)"
BUILD="${CATALYST_BUILD:-$HOME/catalyst-linux-build}"
REPO="${CATALYST_REPO:-TomAs-1226/CatalystLinux}"
CHANNEL="${1:-nightly}"
shift || true
PUBLISH=0
NOTES=""
while [ $# -gt 0 ]; do
	case "$1" in
	--publish) PUBLISH=1 ;;
	--notes) NOTES="$2"; shift ;;
	*) echo "unknown: $1" >&2; exit 2 ;;
	esac
	shift
done
case "$CHANNEL" in stable | nightly) ;; *) echo "the channel is stable or nightly" >&2; exit 2 ;; esac

export PATH="$HOME/.cargo/bin:$PATH"
BASE="$(tr -d '[:space:]' < "$HERE/os/VERSION")"
STAMP="$(date -u +%Y%m%d.%H%M)"
if [ "$CHANNEL" = stable ]; then
	VERSION="$BASE"
	TAG="v$BASE"
else
	VERSION="$BASE-nightly.$STAMP"
	TAG="nightly-${STAMP/./-}"
fi
BUILD_TIME="$(date +%s)"
OUT="$BUILD/releases/$CHANNEL/$VERSION"
KEY="$BUILD/keys/update.key"
PUB="$HERE/os/board/cardputer-zero/rootfs-overlay/etc/catalyst/update.pub"

# The GitHub CLI: WSL's own, or else Windows' (this is WSL on a PC), which
# is signed in on Windows and takes Windows paths.
GH=gh
host_path() { printf '%s\n' "$1"; }
if ! command -v gh >/dev/null 2>&1 && command -v gh.exe >/dev/null 2>&1; then
	GH=gh.exe
	host_path() { wslpath -w "$1"; }
fi

if [ "$PUBLISH" = 1 ]; then
	command -v "$GH" >/dev/null || { echo "publishing needs the GitHub CLI (gh)" >&2; exit 1; }
	"$GH" auth status >/dev/null 2>&1 || { echo "sign the GitHub CLI in first: gh auth login" >&2; exit 1; }
	if [ "$CHANNEL" = stable ] && "$GH" release view "$TAG" --repo "$REPO" >/dev/null 2>&1; then
		echo "$TAG is out already: raise os/VERSION first" >&2
		exit 1
	fi
fi

# The updater, built for this computer: it makes and signs the release.
echo "== the release tool"
(cd "$HERE/userland" && CARGO_TARGET_DIR="$BUILD/target" cargo build --release -p catalyst-update -q)
TOOL="$BUILD/target/release/catalyst-update"

# The signing key, made once.
if [ ! -f "$KEY" ]; then
	echo "== a new signing key: $KEY (back it up: handhelds only take updates signed with it)"
	"$TOOL" keygen "$KEY" "$PUB"
fi
# The image must carry the key's public half, or it would refuse its own updates.
WANT="$(tr -d '[:space:]' < "$PUB" 2>/dev/null || true)"
if [ -z "$WANT" ]; then
	echo "$PUB is missing: put the key's public half back there" >&2
	exit 1
fi

echo "== building $VERSION ($CHANNEL)"
export CATALYST_VERSION="$VERSION" CATALYST_BUILD_TIME="$BUILD_TIME"
bash "$HERE/scripts/build-shell.sh" >/dev/null
bash "$HERE/scripts/build-image.sh"

# The last release on this channel, for a patch from it.
PREVIOUS="$(ls -1dt "$BUILD/releases/$CHANNEL"/*/ 2>/dev/null | grep -v "/$VERSION/$" | head -1 || true)"
PREV_ARGS=()
if [ -n "$PREVIOUS" ] && [ -f "$PREVIOUS/system.img" ]; then
	PREV_ARGS=(--previous "$PREVIOUS")
fi

echo "== making the release"
rm -rf "$OUT"
"$TOOL" make-release --images "$BUILD/output/images" --out "$OUT" --version "$VERSION" --channel "$CHANNEL" \
	--build "$BUILD_TIME" --key "$KEY" --repo "$REPO" --tag "$TAG" --notes "$NOTES" "${PREV_ARGS[@]}"
ls -l "$OUT"

# The flasher and the simulator for Windows, built by Windows' own Rust (this
# is WSL on a PC): a release carries them, so a new card is one download and
# one click, and so is trying Catalyst Linux on a PC. Both carry their own C
# runtime (userland/.cargo/config.toml): nothing else to install.
if command -v cmd.exe >/dev/null 2>&1 && command -v wslpath >/dev/null 2>&1; then
	WIN_USERLAND="$(wslpath -w "$HERE/userland")"
	echo "== the flasher for Windows"
	if (cd /mnt/c && cmd.exe /c "cd /d $WIN_USERLAND && %USERPROFILE%\\.cargo\\bin\\cargo.exe build --profile sim -p catalyst-flasher") >/dev/null 2>&1; then
		cp "$HERE/userland/target/sim/catalyst-flasher.exe" "$OUT/"
	else
		echo "   (Windows' Rust could not build it: the release goes without it)"
	fi
	# Packed: double-clicked, it opens the simulator with the demo robot. Built
	# apart from the everyday simulator, which stays a console program.
	echo "== the simulator for Windows"
	if (cd /mnt/c && cmd.exe /c "cd /d $WIN_USERLAND && %USERPROFILE%\\.cargo\\bin\\cargo.exe build --profile sim -p catalyst-shell --features packed --target-dir target\\packed") >/dev/null 2>&1; then
		cp "$HERE/userland/target/packed/sim/catalyst-shell.exe" "$OUT/catalyst-sim.exe"
	else
		echo "   (Windows' Rust could not build it: the release goes without it)"
	fi
fi

# Only the last three releases on a channel are kept here (for patches).
ls -1dt "$BUILD/releases/$CHANNEL"/*/ | tail -n +4 | xargs -r rm -rf

if [ "$PUBLISH" = 1 ]; then
	echo "== publishing $TAG to $REPO"
	FILES=("$OUT"/manifest.json "$OUT"/manifest.sig "$OUT"/system.img.gz "$OUT"/boot.pack "$OUT"/catalyst-linux-*.img.gz "$OUT"/os_list_catalyst.json)
	for extra in "$OUT"/system-from-*.cdelta "$OUT"/catalyst-flasher.exe "$OUT"/catalyst-sim.exe; do
		[ -e "$extra" ] && FILES+=("$extra")
	done
	ASSETS=()
	for f in "${FILES[@]}"; do
		ASSETS+=("$(host_path "$f")")
	done
	PRE=()
	[ "$CHANNEL" = nightly ] && PRE=(--prerelease)
	"$GH" release create "$TAG" --repo "$REPO" --title "Catalyst Linux $VERSION" --notes "${NOTES:-Catalyst Linux $VERSION}" "${PRE[@]}" "${ASSETS[@]}"
	echo "published: https://github.com/$REPO/releases/tag/$TAG"
else
	echo "made $OUT (add --publish to put it on GitHub)"
fi

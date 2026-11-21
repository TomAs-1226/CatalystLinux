#!/bin/bash
# Pack and sign the app catalogue: each app under catalyst-apps/src (an app
# folder as /data/apps holds it, with its app file, licence and icon)
# becomes catalyst-apps/dist/<id>-<version>.tar.gz, signed with the
# catalogue's key, and catalyst-apps/dist/index.json lists them. Apps
# written with Catalyst Kit carry their program, built for the handheld by
# scripts/build-shell.sh into ~/catalyst-linux-build/out/apps.
#
# The first run makes the catalogue's signing key (kept with the update
# key, in ~/catalyst-linux-build/keys/apps.key: keep a copy somewhere safe)
# and puts its public half in the image's overlay
# (os/board/cardputer-zero/rootfs-overlay/etc/catalyst/apps.pub): commit it,
# and the next system update teaches every handheld to trust it.
#
# Packing is repeatable: the same files make the same packages. Then
# scripts/publish-catalogue.sh puts dist/ on the catalogue's release.
# Run in WSL (or any Linux).
set -euo pipefail

HERE="$(cd "$(dirname "$0")/.." && pwd)"
BUILD="${CATALYST_BUILD:-$HOME/catalyst-linux-build}"
KEY="${CATALYST_APPS_KEY:-$BUILD/keys/apps.key}"
PUB="$HERE/os/board/cardputer-zero/rootfs-overlay/etc/catalyst/apps.pub"
export PATH="$HOME/.cargo/bin:$PATH"

# catalyst-pack, for this computer, in a folder of its own.
export CARGO_TARGET_DIR="${CATALYST_PACK_TARGET:-$HOME/.cache/catalyst-pack}"
cargo build --quiet --release --manifest-path "$HERE/userland/Cargo.toml" -p catalyst-appkg
PACK="$CARGO_TARGET_DIR/release/catalyst-pack"

if [ ! -s "$KEY" ]; then
	mkdir -p "$(dirname "$KEY")"
	"$PACK" keygen "$KEY" "$PUB"
	echo "commit $PUB: handheld images built from now on trust this key"
fi

"$PACK" build "$HERE/catalyst-apps/src" --out "$HERE/catalyst-apps/dist" --key "$KEY" --bin "$BUILD/out/apps" --name "Catalyst apps"
"$PACK" verify "$HERE/catalyst-apps/dist" --public "$PUB"

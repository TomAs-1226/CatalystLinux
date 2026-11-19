#!/bin/bash
# Pack the app catalogue: each folder under catalyst-apps/src (an app as
# /data/apps holds it, with a `version =` line in its `app` file) becomes
# catalyst-apps/apps/<id>-<version>.tar.gz, and catalyst-apps/index.json
# lists them with their SHA-256, which the handheld checks before installing.
#
# Run it after changing an app, then commit and push catalyst-apps/ as its
# own repository (TomAs-1226/catalyst-apps by default). Packing is
# repeatable: the same sources make the same archives.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)/catalyst-apps"
cd "$ROOT"
mkdir -p apps
rm -f apps/*.tar.gz

field() { sed -n "s/^$1[[:space:]]*=[[:space:]]*//p" "$2" | head -1; }

entries=()
for dir in src/*/; do
	id=$(basename "$dir")
	manifest="$dir/app"
	[ -f "$manifest" ] || { echo "skipped $id: no app file" >&2; continue; }
	version=$(field version "$manifest")
	[ -n "$version" ] || { echo "skipped $id: no version" >&2; continue; }
	archive="apps/$id-$version.tar.gz"
	# Fixed owner, times and order, so packing again changes nothing.
	tar --sort=name --owner=0 --group=0 --numeric-owner --mtime='2026-01-01 00:00:00' \
		-C "$dir" -cf - . | gzip -n -9 > "$archive"
	entries+=("$id	$(field name "$manifest")	$(field about "$manifest")	$(field icon "$manifest")	$version	$archive	$(sha256sum "$archive" | cut -d' ' -f1)	$(stat -c %s "$archive")")
	echo "packed $archive"
done

printf '%s\n' "${entries[@]}" | python3 -c '
import json, sys
apps = []
for line in sys.stdin:
    id, name, about, icon, version, archive, sha256, size = line.rstrip("\n").split("\t")
    apps.append({"id": id, "name": name, "about": about, "icon": icon or "apps", "version": version,
                 "archive": archive, "sha256": sha256, "bytes": int(size)})
json.dump({"catalogue": 1, "apps": apps}, open("index.json", "w"), indent=2, ensure_ascii=False)
print("index.json lists", len(apps), "apps")
'

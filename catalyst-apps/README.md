# Catalyst Linux app catalogue

The apps the handheld's **Catalogue** lists, gets, updates and removes over
Wi-Fi. This folder is meant to be its own repository,
`TomAs-1226/catalyst-apps`; the handheld reads

```
https://raw.githubusercontent.com/TomAs-1226/catalyst-apps/main/index.json
```

## An app

An app is a folder as `/data/apps` holds them: an `app` file and whatever it
runs. Put it under `src/<id>/`, where the id is letters, digits and dashes.

```text
name = Ping the robot
icon = link                 one of the shell's built-in symbols
about = is it there, and how fast
version = 1.0.0             what the catalogue compares to offer updates
run = sh ./run.sh           run from the app's own folder
screen = terminal           or "own" to draw on the panel itself
```

Scripts run on the handheld's BusyBox `sh`. Programs must be built for
aarch64 and statically linked (the image has no C++ runtime).

## Publishing

```bash
bash scripts/pack-catalogue.sh
```

packs each app into `apps/<id>-<version>.tar.gz` and writes `index.json` with
each archive's SHA-256. Commit both and push. Raise `version` to offer an
update.

## What the handheld checks

Before an app is put in place: the archive came over HTTPS; its SHA-256 is
the one in the index; no path in it is absolute or climbs out with `..`; it
unpacks to a folder with an `app` file that names something to run. Any
failure leaves the installed version as it was. Only apps the catalogue
installed (they carry a `.catalogue` file) can be removed from it.

An app runs as root with the whole handheld to itself, so the catalogue is
only as trustworthy as whoever can push to this repository.

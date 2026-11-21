# Catalyst Linux app catalogue

The apps the handheld's **Catalogue** lists, gets, updates and removes
over the air. Apps live on `/data`; nothing here touches the system, and
nothing needs reflashing. This folder is meant to be its own repository,
`TomAs-1226/catalyst-apps`. Every handheld reads the catalogue from that
repository's `catalogue` release:

```
https://github.com/TomAs-1226/catalyst-apps/releases/download/catalogue/index.json
```

`STORE.md` lists every app in M5Stack's CardputerZero app store with its
licence, and which were rebuilt here and why the rest were not.

## An app

An app is a folder as `/data/apps` holds it: an `app` file, a `LICENSE`, and
whatever it runs. Put it under `src/<id>/`; the id is letters, digits and
dashes, and is the folder's name.

```text
id = sudoku
name = Sudoku                 at most 24 characters: the tile's name
icon = grid_on                one of the shell's symbols, or icon.svg
about = one answer, three levels      at most 40: the line under the name
description = A new puzzle each game, …   the app's page in the Catalogue
version = 1.0.0               dotted numbers: raised to offer an update
run = ./catalyst-sudoku       a command line, run from the app's folder
screen = own                  or terminal
permissions = network, files  what it uses (below); leave it out for nothing
binary = catalyst-sudoku      a program built by scripts/build-shell.sh, packed in
credit = Rebuilt for Catalyst from sudokuzero by dbrhspyjkf-code (MIT)
licence = MIT                 an SPDX name; the text is in LICENSE
source = https://github.com/…
needs = 0.6.0                 the oldest Catalyst it runs on (optional)
```

- **`screen = own`** hands the app the panel and keys: write it with
  [Catalyst Kit](../userland/crates/kit/README.md) and it looks like the
  shell's own screens. A script can do the same with `catalyst-ui`.
- **`screen = terminal`** runs it in the handheld's terminal.
- **`icon = icon.svg`** carries the app's own symbol: a Material Symbol
  (Rounded) from `fonts.gstatic.com/s/i/short-term/release/materialsymbolsrounded/<name>/default/20px.svg`,
  in its 960-unit box. The Catalogue shows it before the app is got, and
  Home draws it as it draws its own. `cargo run -p catalyst-kit --example
  icon-sheet -- sheet.png icon.svg` shows it at every size.
- **Permissions** are said on the app's page before it is got, and an
  update that uses more says so and asks again: `network`, `files` (your
  files in `/data`), `camera`, `microphone`, `speaker`, `motion`,
  `infrared`, `usb`, `system`. Catalyst does not fence apps in yet: these
  are what the app says it does, checked when it is added here.
- What an app keeps goes in its own folder, `$CATALYST_APP_DATA`
  (`/data/catalyst/app-data/<id>`): an update replaces the app's folder
  whole, and this one stays. Removing the app leaves it too.

`catalyst-pack check src/<id>` says whether an app is fit for the catalogue.
`catalyst-pack new <id> "<Name>"` starts a script app.

## Packing and signing

```bash
scripts/build-shell.sh        # the shell, catalyst-ui, and the apps' programs (out/apps)
scripts/pack-catalogue.sh     # every app packed and signed into catalyst-apps/dist
scripts/publish-catalogue.sh  # dist/ onto the catalogue release
```

Each app becomes `dist/<id>-<version>.tar.gz`: its folder as a gzipped tar,
the same bytes every time the same files are packed. `dist/index.json`
lists them, each with its size, its SHA-256 and an **Ed25519 signature**
over a short statement (`catalyst-app 1`, the id, the version, the hash).

The signing key is made the first time `pack-catalogue.sh` runs:
`~/catalyst-linux-build/keys/apps.key` (keep it secret, keep a copy). Its
public half goes in the image as `/etc/catalyst/apps.pub`
(`os/board/cardputer-zero/rootfs-overlay/etc/catalyst/apps.pub`): commit
it, and handhelds trust the catalogue from the system update that carries it.

## What the handheld does

1. The Catalogue reads `index.json` over HTTPS (following GitHub's
   redirect, HTTPS only) and keeps a copy, from which Spotlight finds the
   apps not yet on the handheld.
2. **Get** or **Update**: the entry's signature must be by a key the
   handheld trusts (`/etc/catalyst/apps.pub`, or one you added under
   `/data/catalyst/app-keys/*.pub`), or nothing is downloaded. Then the
   package is downloaded (32 MB at most), its SHA-256 checked against the
   index, its listing checked (no path outside its folder, no links), it is
   unpacked beside the apps, and its app file must name this app at this
   version and something to run. Only then does it replace the old folder,
   which is put back if the swap fails. A failure at any step leaves what
   was there as it was.
3. It is on Home at once (and in Spotlight). `u` on the list updates every
   app with an update, one after another.
4. **Remove** (Enter twice on the app's page) takes away an app the
   catalogue installed (it carries a `.catalogue` file); one put there by
   hand is left alone, and what the app kept stays.

To try a catalogue of your own: put its `index.json` address in
`/data/catalyst/catalogue` and its public key in
`/data/catalyst/app-keys/mine.pub`.

## Apps here

| App | What | From |
| --- | --- | --- |
| Sudoku | one answer, three levels, notes, best times | sudokuzero by dbrhspyjkf-code (MIT) |
| Battleship | your fleet against the handheld, three levels | Battleship by yutacar (MIT) |
| Typing | words a minute on this keyboard, English, code or pit words | taM5CZTyping (key-dash) by yuk1-kondo (MIT) |
| Orbits | gravity: the solar system, three-body orbits, a galaxy | AstroBox by eggfly and kur1k0 (MIT) |
| Hello | Catalyst Kit's example: the app to copy | Catalyst |
| Ping the robot, Network, Storage | terminal scripts for the pit | Catalyst |

The four rebuilt apps are new programs written with Catalyst Kit that
follow the originals' rules, levels and algorithms (and Typing's two word
lists); each `LICENSE` carries the original's copyright line beside
Catalyst's.

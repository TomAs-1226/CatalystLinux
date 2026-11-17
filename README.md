# Catalyst Linux

A custom Linux distribution for the M5Stack CardputerZero (Raspberry Pi CM0,
512 MB RAM, 320 × 170 non-touch LCD, 46-key keyboard): a pocket companion for
robots running [FRC Catalyst](https://github.com/TomAs-1226/FrcCatalyst) 2.0 on
Systemcore. Its GUI is Bezel and Detent, drawn natively on the CPU.

## What is here

| Path | What it is |
| --- | --- |
| `userland/crates/detent` | Detent's spring model in Rust: every vocabulary, closed-form, interruptible. |
| `userland/crates/bezel` | Bezel on the CPU: tonal surfaces, the two type voices, Material Symbols, and the full glass pass (refraction, frost, edge light, groups, shadow). |
| `userland/crates/nt4` | A NetworkTables 4.1 client: one thread, read-only, reconnects on its own. |
| `userland/apps/shell` | `catalyst-shell`: the launcher and the built-in apps (Robot, Health, Logs, Tables, Loop, Motors, Console, System). |
| `os/` | The Buildroot external tree: board config, kernel fragment, M5Stack driver package, OpenRC services, image layout. |
| `scripts/` | Build-machine setup, shell cross-build, image build and checks, simulator. |
| `work/FrcCatalyst` | A clone of FrcCatalyst carrying the Agent log routes, uncommitted (not part of this repo). |
| `shots/tour/` | Screenshots of every screen, rendered by `catalyst-shell --shots`. |

## Decisions

Buildroot · 64-bit · musl · OpenRC · read-only root with a writable `/data`
partition · Rust · full Bezel drawn by a CPU renderer · core apps built into one
shell, extra apps as separate full-screen programs · SSH in the image but off
until switched on in System.

## Try the UI on this PC

```powershell
.\scripts\run-sim.ps1            # a window showing the panel at 4x, with a demo robot
.\scripts\run-sim.ps1 -Robot 5805   # a real robot: team number, IP or hostname
```

Arrow keys or `F` `X` `Z` `C` (the keys that carry the arrows on the
Cardputer) move, Enter opens, Esc goes back.

## Build the SD card image

Inside WSL (Ubuntu), nothing needs root:

```bash
bash scripts/wsl-setup.sh     # once: Buildroot, Rust
bash scripts/build-shell.sh   # cross-compile catalyst-shell for the CM0
bash scripts/build-image.sh   # about an hour the first time
```

The image is `~/catalyst-linux-build/output/images/sdcard.img`. Two more
scripts check it without a device, running the image's own programs under
emulation:

```bash
bash scripts/inspect-image.sh        # boot files, drivers, services, sizes
bash scripts/test-image-services.sh  # first-boot partition growth, the network address rule
```

## Match logs

The handheld pulls `.wpilog` files from the robot over the Ethernet cable,
through two read-only routes added to the Catalyst Agent (`/api/logs` and
`/api/logs/<name>`). Those routes are a change to FrcCatalyst, sitting
uncommitted in `work/FrcCatalyst` (`agent/`), and a robot needs that agent
for logs to appear. Pulling happens in the background while the robot is
linked and not running an opmode, resumes where it stopped, and each log is
summarised once (brownouts, battery low point, health checks fired, loop
overruns, hottest mechanisms). Logs are cleared after 30 days, oldest first
if the card fills; `/data` grows to fill the SD card on first boot.

## State

Working and checked on a PC: the spring engine (tests against Apple's
documented values), the renderer, the shell and its screens against a scripted
robot, the NT4 client against a scripted NT4 server, the Catalyst Agent client
and the log puller against the real agent script (with the new log routes),
the log reader against a generated match, the ARM build of the shell under
user-mode emulation, and the image's first-boot scripts under emulation.

Not yet checked: anything on a real CardputerZero (boot, panel, keyboard,
Ethernet, idle memory), the NT4 client against a real Systemcore, the log
reader against a log from a real robot, and the kernel fragment's effect on
memory. Not yet built: the agent's machine pages (CPU, CAN buses, cameras),
the AI log analysis, Wi-Fi setup in the UI, the camera, audio, and the
external-app launcher.

## Third-party pieces

Google Sans Flex and Google Sans Code (SIL OFL 1.1) and Material Symbols
(Apache 2.0) are vendored under `userland/assets` with their licences. The
board drivers come from `m5stack/m5stack-linux-dtoverlays` (GPL-2.0) at build
time.

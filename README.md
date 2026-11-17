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
| `userland/apps/shell` | `catalyst-shell`: the launcher and the built-in apps (Robot, Health, Logs, Tables, Loop, Motors, Console, Terminal, Files, Settings). |
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
.\scripts\run-sim.ps1               # the panel at 4x, a demo robot, at the handheld's estimated speed
.\scripts\run-sim.ps1 -Speed worse  # slower than the handheld, to see whether it still holds up
.\scripts\run-sim.ps1 -Speed pc     # as fast as this PC goes
.\scripts\run-sim.ps1 -Robot 5805   # a real robot: team number, IP or hostname
```

Arrow keys or `F` `X` `Z` `C` (the keys that carry the arrows on the
Cardputer) move, Enter opens, Esc goes back.

The simulator holds itself to the handheld's speed: the processor work is
stretched 20 times (40 at `worse`) and each frame waits 15 ms (25) for the
panel, and the window title shows frames per second, the cost of a frame and
how busy the handheld's processor would be. Those factors are estimates until
a CardputerZero has been measured with `catalyst-shell --bench`. At them, an
idle screen costs nothing, a screen showing live robot data keeps the
processor about 6% busy (and is only sent to the panel when the picture
actually changed), and transitions run at about 40 frames a second.

## Motion

Motion is Detent's, by Bezel's roles, and every piece of it is a spring that
starts from what is on screen:

- An app grows out of the tile it was opened from and shrinks back into it
  (`release`). Press Esc while it opens and it turns round where it is,
  keeping its speed; press Enter while it closes and the same window returns.
- A page inside an app slides in from the right and back out (`page`), and
  turns round the same way.
- The focus ring glides to whatever is focused (`hold`). Bezel itself lands
  changes made from keys at once; this bends that for the sake of flow, and
  the calm version in Settings restores it.
- Home's tiles cascade into place when it first appears (`release`, spread
  over 0.28 s), and the island grows to carry a message, its words fading in
  once it is past halfway.

Springs are solved in closed form, so motion is the same at 20 frames a
second as at 60, and nothing moves while the screen is idle.

Even frames matter more than fast ones, so while anything moves, frames start
on a steady beat: 25 ms (40 a second) if the handheld holds it, else 33, 50 or
100 ms. One late frame is let go; a second soon after slows the beat, and it
only quickens again between animations. `catalyst-shell --smooth` reports,
for each kind of transition, the first, typical and worst gap between frames
at the simulator's two speeds. At the estimated device speed apps open and
close on an even 33 ms beat and pages turn on an even 25 ms one; at the
worse-than-device speed everything holds an even 50 ms.

On the device, frames are written to the framebuffer file a changed band of
rows at a time, which the kernel sends to the panel at once. (Writing through
a memory map would be gathered and sent on a timer about twenty times a
second.) The tests in
`shell.rs` hold all of this: turning round mid-flight without a jump,
reopening from where a window is, frame-rate independence, and that an
animation begun after an idle spell starts from its beginning.

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

## Terminal, Files and the editor

Terminal is a real shell (`/bin/sh` on a pseudo-terminal) on the handheld's
own screen, 46 columns by 10 rows at the standard text size. The emulator
behind it handles what shells, `vi` and the usual tools print: cursor
movement, colours, scrolling regions, the alternate screen. The session
outlives the screen, so going Home and back finds it as it was. Esc belongs
to the programs in it; Ctrl-Q returns Home. Page Up and Page Down scroll back.

Files browses the storage (starting in `/data`), makes a file with `n` and
deletes one with `d` pressed twice. Enter opens a text file in the editor:
arrows move, Ctrl-S saves (through a temporary file, so a power cut cannot
leave half a file), Ctrl-Q closes, Ctrl-K removes a line.

On a PC the terminal is a small stand-in shell, since there is no
pseudo-terminal to use the same way. The real one is exercised by
`scripts/test-arm.sh`, which runs the tests as the ARM build under emulation
and talks to an actual shell through the emulator.

## Wi-Fi

Settings › Network › Wi-Fi networks switches Wi-Fi on, lists what is in range,
joins a network (the password is typed on the keyboard) and forgets one. It
drives the system's `wpa_supplicant`; networks are kept in
`/data/wifi/wpa_supplicant.conf`, and the radio stays off until it is switched
on. On a PC a scripted set of networks stands in.

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

Not yet checked: anything on a real CardputerZero (boot, panel and how fast
it takes frames, keyboard, Ethernet, Wi-Fi, idle memory), the NT4 client against a real Systemcore, the log
reader against a log from a real robot, and the kernel fragment's effect on
memory. Not yet built: the agent's machine pages (CPU, CAN buses, cameras),
the AI log analysis, Wi-Fi setup in the UI, the camera, audio, and the
external-app launcher.

## Roadmap

Catalyst Linux is meant to be a pocket developer machine as well as a Catalyst
tool. Chosen so far, not yet built: scripting with a kit for writing Bezel
apps; an SSH client and serial
console; a network toolkit; USB gadget modes (serial, network and a drive of
logs when plugged into a PC); a system monitor; standby and power saving;
image updates with rollback; a status bar; an AprilTag reader on the built-in
camera, calibrated so it can measure for the robot; a pit checklist and match
timer; a robot finder; the AI log analysis; memory and boot-time tuning.

## Third-party pieces

Google Sans Flex and Google Sans Code (SIL OFL 1.1) and Material Symbols
(Apache 2.0) are vendored under `userland/assets` with their licences. The
board drivers come from `m5stack/m5stack-linux-dtoverlays` (GPL-2.0) at build
time.

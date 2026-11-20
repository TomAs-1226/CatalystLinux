# Catalyst Linux

A custom Linux distribution for the M5Stack CardputerZero (Raspberry Pi CM0,
512 MB RAM, 320 × 170 non-touch LCD, 46-key keyboard): a pocket companion for
robots running [FRC Catalyst](https://github.com/TomAs-1226/FrcCatalyst) 2.0 on
Systemcore, and a pocket developer machine. Its GUI is Bezel and Detent, drawn
natively: by the processor, with the VideoCore GPU taking the glass and the
windows that grow out of tiles whenever it is there.

## What is here

| Path | What it is |
| --- | --- |
| `userland/crates/detent` | Detent's spring model in Rust: every vocabulary, closed-form, interruptible. |
| `userland/crates/bezel` | Bezel on the CPU: tonal surfaces, the two type voices, Material Symbols, and the full glass pass (refraction, frost, edge light, groups, shadow). |
| `userland/crates/nt4` | A NetworkTables 4.1 client: one thread, read-only, reconnects on its own. |
| `userland/apps/shell` | `catalyst-shell`: the start animation, first-boot setup, Home, search, the glance page, the control centre, the built-in apps (Robot, Health, Logs with its trends, Schedule, Tables, Loop, Motors, Console, Terminal, Files, Git, Catalogue, Settings, Graphs, Pit) and the launcher for external ones. |
| `userland/apps/update` | `catalyst-update`: system updates on the handheld (A/B, tried once, rolled back if it does not come up), and the release tool that makes and signs them. |
| `userland/apps/flasher` | Catalyst Flasher: the newest release onto an SD card in one go. |
| `userland/assets/app-icon` | Catalyst's icon (the FrcCatalyst bolt) at each size Windows uses, built into the simulator; `assets/` holds it for Raspberry Pi Imager's listing. |
| `os/` | The Buildroot external tree: board config, kernel and BusyBox fragments, M5Stack driver package, OpenRC services, image layout. |
| `scripts/` | Build-machine setup, shell cross-build, kernel and image builds, image checks, the emulator harness, the simulator, the catalogue packer. |
| `catalyst-apps/` | The app catalogue, meant to become its own repository (`TomAs-1226/catalyst-apps`): three example apps, their packed archives and the index. |
| `work/FrcCatalyst` | A clone of FrcCatalyst carrying the Agent log routes, uncommitted (not part of this repo). |
| `shots/start/` | The start animation as an animated PNG, and stills of it. |
| `shots/tour/` | Screenshots of every screen, rendered by `catalyst-shell --shots`. |
| `shots/looks/` | Home, Robot and Settings › Appearance in eight looks (`--looks`). |
| `shots/flasher/` | Catalyst Flasher's screens (`catalyst-flasher --shot`). |
| `shots/motion/` | Each kind of motion as a strip of frames 50 ms apart and an animated PNG, and the Home settings (`--motion`). |
| `shots/vm/` | The same shell as the ARM build on the image's own kernel, in the emulator. |

## Decisions

Buildroot · 64-bit · musl · OpenRC · read-only root with a writable `/data`
partition · two systems on the card, updated over the air as Android does
(A/B, tried once, kept only if it comes up) · Rust · full Bezel drawn by its
own renderer on the processor (NEON where it counts), with
the GPU (Mesa's vc4 driver, OpenGL ES 2) taking the glass and growing windows
when it draws them as the processor does · core apps built into one
shell, extra apps as separate full-screen programs · SSH in the image but off
until switched on · the kernel trimmed of what costs memory, with every USB
driver kept as a loadable file · Latin-only fonts · HTTPS through curl on
mbedTLS, shared by git, the schedule and the catalogue · keys for online
services set in first-boot setup (from the handheld or a computer), in
Settings, or read from a file.

## Try it on any Windows PC

Download **[catalyst-sim.exe](https://github.com/TomAs-1226/CatalystLinux/releases/latest/download/catalyst-sim.exe)**
from the newest release and double-click it. The simulator opens on the demo
robot every time (a team number typed in setup is saved but the demo stays),
starting with first-boot setup as a new handheld does, and runs at the PC's
full speed in step with its screen. It is one file with nothing to install:
its fonts, icons and C runtime are inside it. What it saves (settings, the
robot's address, keys) goes in `%LOCALAPPDATA%\CatalystLinux`. It is not
signed, so Windows may stop it the first time: *More info*, then *Run
anyway*. From a terminal it takes the shell's own options: `--no-intro`,
`--setup`, `--light`, `--speed device` (paced as the handheld, see below),
`--robot 5805` (a real robot instead of the demo), `--arrive 5`.

## Try the UI on this PC

```powershell
.\scripts\run-sim.ps1               # the panel at 4x, a demo robot, at the handheld's estimated speed
.\scripts\run-sim.ps1 -Speed worse  # slower than the handheld, to see whether it still holds up
.\scripts\run-sim.ps1 -Speed pc     # as fast as this PC goes, in step with its screen
.\scripts\run-sim.ps1 -Robot 5805   # a real robot: team number, IP or hostname
.\scripts\run-sim.ps1 -Setup        # first-boot setup, as on a new handheld
.\scripts\run-sim.ps1 -Arrive 5     # the demo robot plugged in, back from a match, after 5 s
.\scripts\run-sim.ps1 -NoIntro      # straight to Home, without the start animation
```

Arrow keys or `F` `X` `Z` `C` (the keys that carry the arrows on the
Cardputer) move, Enter opens, Esc goes back. On Home, Left and Right go
through the apps in order (on to the next row, and the next page), Up and
Down within a page. Space on Home searches.
Ctrl-Space brings the control centre down over any screen; Left from Home's
first tile is the glance page. `?` held for five seconds on any screen that
is not taking text brings up the list of shortcuts (the island counts down
from the first second; a `?` typed in passing does nothing). The window
shows what the panel shows: 16-bit colour, dithered as on the device.

A held key repeats on the shell's own clock, the same on the handheld and in
the simulator: once as it lands, then after 0.4 s about 13 times a second,
so a list can be run through without racing to its end. Only keys that move
or type repeat; Enter, Esc and the shortcuts count once a press. (The
kernel's repeats on the handheld, 30 a second, are let go.)

At `-Speed pc` (and in the packed simulator) frames keep time with the PC's
screen, 60 a second on most. Otherwise the simulator holds itself to the
handheld's speed, which is at most 40 frames a second: the processor work is
stretched 20 times (40 at `worse`) and each frame waits 15 ms (25) for the
panel, and the window title shows frames per second, the cost of a frame and
how busy the handheld's processor would be. Those factors are estimates until
a CardputerZero has been measured with `catalyst-shell --bench`. At them, an
idle screen costs nothing, and a screen showing live robot data keeps the
processor about 6% busy (and is only sent to the panel when the picture
actually changed).

The simulator is built with its own profile (`cargo run --profile sim`): the
same optimisation where it counts, compiled in pieces. The device build makes
one large unit, which Rust 1.92 on Windows fails to compile.

## Help, About and developer options

- **The tour.** Once first-boot setup is done, a tour of the keys follows as
  Home arrives (and Settings › Help › Take the tour runs it again): the
  screen dims, a spotlight glides to each thing in turn (the apps, the
  island, the page dots) and a card says what it is and what the keys do.
  Enter goes on, Left goes back, Esc skips it.
- **Shortcuts.** `?` held for five seconds over any screen not taking text,
  or Settings › Help › Keyboard shortcuts: every key by where it works,
  drawn as keycaps. Where text is typed (search, a text field, the
  terminal), `?` is only ever typed.
- **About**, laid out as Android's "About phone": the handheld's name large
  at the top with Catalyst's icon (the header folds away as the list goes
  down), its model, the Catalyst Linux version (a page of its own: channel,
  build date, kernel, shell, graphics), its status, the open-source
  licences, and the build number at the bottom.
- **Developer options.** As on Android, pressing Enter on Build number seven
  times makes you a developer (the island counts down the last presses), and
  Settings gains Developer options: an animation scale (0.5× to 10× slower,
  to study motion frame by frame; Android's animator duration scale) and
  Show key presses (each key on a keycap at the bottom of the screen, for
  testing the keyboard and for recordings). Switched off, they go again.
- As on Android, the version hides an Easter egg.

## Starting up, and setting up

Every start opens with the Catalyst mark (the bolt of FrcCatalyst's own
logo, drawn as vector artwork on its dark ground): a trail of light sweeps in
from the lower left and strikes the middle, the bolt ignites white-hot and
cools to crimson with a flash and a ring going out, then rises to make room
as "Catalyst" (bold) and "Linux" (thin) set themselves under it and the
crimson rule draws across. It gives way to Home as the tiles cascade in.
Every part arrives on Detent's springs by Bezel's roles; the trail alone
accelerates, since it is thrown rather than placed. Any key skips it.
`catalyst-shell --start-movie file.png` renders it as an animated PNG
(`shots/start/`).

The first start continues into **setup**, which owns the whole panel the way
a computer's own setup does: no island, no status, the brand's ground, each
page's title, words and rows rising in one after another from the side the
page came from. "Hi." offers three ways on:

- **Set up here**: the team number (the robot's address follows from it),
  Wi-Fi, keys for online services, a page of choices (open Robot on link,
  the match report, SSH, the calm version), and the look (accent, mode, text
  size, Home's style, with a live picture of Home), then "Just a moment…",
  "You're all set." and Home.
- **Set up from a computer**: the handheld shows its address, as text and a
  QR code, and a six-digit code. Any browser on the same network (the
  Ethernet cable, or Wi-Fi) gets one page asking for all of the above plus an
  SSH public key, typed on a real keyboard. Nothing listens on the network
  except while that page is offered, for ten minutes at most; five wrong
  codes close it. It is plain HTTP on the local network, so keys cross it
  unencrypted: use it on the robot's cable or a network you trust. Settings ›
  Set up from a computer offers it again later.
- **Set up later**: straight to Home; everything is in Settings.

`/data/catalyst/setup-done` marks setup as done.

## Home, the glance page and the control centre

Home comes in two styles, chosen in Settings › Home › Home screen:

- **Grid**: tiles, six to a page, each with its app's status line.
- **Carousel**: one large card in the middle with its neighbours peeking in at
  either side. Left and Right turn it one app at a time, and it lands like a
  dial on its detent.

Settings › Home also arranges it:

- **Arrange apps**: Space makes an app a favourite (pinned first on Home,
  with a star), Left and Right move it earlier or later among the favourites
  or among the rest, Enter hides it or shows it again. Settings is never
  hidden: it is the way back. Home's focus stays on the same app however
  the tiles move.
- **Tiles per row**: three, or two larger ones (two always at the large text size).
- **Clock**: 24-hour or 12-hour, with or without seconds.
- **Island at rest**: what the glass island at the top shows when it has
  nothing to say: the robot (its name, or why there is none), the clock, or
  the battery. What it shows leaves the status line, so nothing is there twice.
- **Greeting**: "good morning · team 5805" in the island as Home appears.

The start animation is personal too: once the team is known (from the robot's
address) and a robot has linked, the line under the name reads
`team 5805 · atlas` instead of the version.

**Search**: Space (or /) on Home, then type. One list finds apps, everything
in Settings, any value the robot publishes, and match logs, ranked by where
the words are found (the start of a name, then the start of a word in it,
then anywhere). Enter opens it; Esc clears, then leaves. The bare letters
stay arrows on Home, so search has its own key.

One step left of Home's first app is the **glance page** (负一屏): cards to be
read without opening anything, two to a page, the pages running further left.
The cards and their order are chosen in Settings › Home › Glance page:
this handheld (time, battery, memory, disk, address), the robot (battery
volts, health, loop time, motors, the last match), the next match (its
alliance, when to queue, partners and opponents), the last match (battery low
point and what stood out), a pinned value (`p` on any value in Tables pins
it), and a note (`/data/note.txt`, written in the editor). Enter on a card
opens what it is about, out of the card.

**A robot arriving.** When a robot links (the cable goes in and its tables
answer) while Home or the glance page is showing, the Robot app opens by
itself, out of its tile; if Home was turned to another page it turns back to
the tile first. Anywhere else (another app, the terminal, the editor, under
the control centre) the screen in front is left alone and the island says
`<name> linked`. A key pressed in that moment keeps the screen too. A link
that drops for under three seconds and returns is the same visit and opens
nothing; a robot restarting, or another one plugged in, counts as arriving
again.

**After a match**, the log of the match the robot has just played is pulled
within seconds of plugging in; once it has been read, its summary opens over
the Robot page (brownouts, battery low point, faults, hot motors). If a key
has been pressed meanwhile, the island says where to find it instead. Both
switch off in Settings › Robot. `.\scripts\run-sim.ps1 -Arrive 5` shows the
two together: no robot at first, then the demo one, back from a match.

The **control centre** is a sheet of glass that drops from the top over
whatever is showing: Wi-Fi, SSH, light or dark, the calm version, and
brightness. Ctrl-Space brings it down from any screen, a terminal included
(so does the Opt key, if the keyboard reports one), and Up from the top of
Home pulls it down too. Esc, Ctrl-Space again, or Up from its top row puts it
away. The screen behind is held still while it is out, so the glass is worked
out once and moved as a picture.

## Looks

Settings › Appearance and Settings › Text, every choice remembered:

- **Theme**: *Catalyst*, the look of first-boot setup (the brand's warm
  ground, crimson by default, capsule rows, a white focus ring, names in Sans
  and values in mono), or *Bezel*, the look of Bezel's wall panels: cool
  neutral greys with no brand glow, every name lowercase in the mono voice,
  each tile's reading large in the display voice with its name small above
  it, squarer tonal rows with their values in Sans, lowercase headings, and
  one orange signal. Choosing a theme brings its own wallpaper and colour.
- **Mode**: dark or light; light mode is its own design, not an inversion.
- **Accent**: five named colours, or **your own colour**: a hue slider all
  round the wheel, every 5° (72 hues). Each is a full Bezel palette, grown
  from its seed by Detent's generator as the named ones are
  (`scripts/grow-hues.mjs`), and every pair of colours in it passes Bezel's
  contrast checks. A fault always reads red: under a blue or green accent
  it is a fixed red (Material's own in light mode), never the accent.
- **Wallpaper**: Catalyst (the brand's glow), Glow, Aurora, Deep or Plain,
  drawn once per palette and larger than the panel.
- **Corners**: round, standard or square, kept concentric throughout.
- **Letters** and **letter width**: crisp, soft or round, and narrow, normal
  or wide, through Google Sans Flex's own axes (roundness and width), so the
  type changes shape rather than being stretched.
- **Text size** and the **calm version** (no transparency, no motion).

`catalyst-shell --looks shots/looks` renders eight of these side by side.

## Motion

Motion is Detent's, by Bezel's roles, and every piece of it is a spring that
starts from what is on screen:

- An app grows out of the tile it was opened from and shrinks back into it
  (`release`). Small, the window is the tile itself (its icon and name,
  scaled with it); the app takes it over by two thirds of the way, and
  closing, the app melts back into the tile's own picture, so the tile is
  what lands: nothing is swapped at the end. Press Esc while it opens and it
  turns round where it is, keeping its speed; press Enter while it closes
  and the same window returns.
- A page inside an app slides in from the right and back out (`page`), and
  turns round the same way. Home's pages and the glance page turn on the same
  spring; the carousel turns on `detent`.
- Lists glide to where they are scrolled rather than jumping a row at a time
  (`hold`), with the focus ring heading for where the focused row will rest.
  A key pressed against the end of a list, the last app or the glance page
  makes it give a little and spring back: "no further", without a word.
- The focus ring glides to whatever is focused (`hold`). Bezel itself lands
  changes made from keys at once; this bends that for the sake of flow, and
  the calm version restores it.
- The control centre comes down on `release` and leaves on `smooth`; its
  switches' knobs slide (`tick`) and the brightness bar follows the keys.
- Home's tiles cascade into place when it first appears (`release`, spread
  over 0.28 s), and the island grows to carry a message, its words fading in
  once it is past halfway.
- **Pages rise in.** As any page arrives (sliding in, or inside an app's
  window as it grows), its title and then each row come up into place one
  after another, 35 ms apart, on `release`. The page is cut into blocks at
  rows where only wallpaper shows, so a block is a title, a row or a line of
  tiles, never half of one; only what the page drew moves, and the wallpaper
  stays put.
- **Focus lift.** The focused tile on Home lifts (`detent`), and so does the
  row under the focus ring in every list, following the ring as it glides:
  a touch larger and lighter, with a soft shadow, the row it leaves settling
  as the next one rises.
- **Press.** Enter answers as it lands: the focused row or tile takes a state
  layer and gives a pixel or two, and springs back (`settle`). On Home the
  tile is pressed in for 70 ms before its app grows out of it; any other key
  in that moment changes its mind.
- **Tiles that react.** A tile pulses once in the colour of what it now says
  when its news changes: a new warning on Health, a log arriving on Logs, a
  motor dropping out, the robot linking, a fault coming or going. Never for a
  live reading (volts, loop time, a countdown), which changes all the time.
- **Colour changes blend.** A new accent, mode, theme or wallpaper fades in
  over the old one rather than snapping.
- **Parallax and a living wallpaper.** The wallpaper moves a little as Home's
  pages turn, and drifts after a key the way the key pointed, and after a
  screen has come or gone the way it went, settling back still. It holds
  still while a screen is moving, so the transition's pictures stay good.

The calm version takes all of these out.

Springs are solved in closed form, so motion is the same at 20 frames a
second as at 60, and nothing moves while the screen is idle.

Even frames matter more than fast ones, so while anything moves, frames start
on a steady beat: 25 ms (40 a second) if the handheld holds it, else 33, 50 or
100 ms. One late frame is let go; a second soon after slows the beat, and it
only quickens again between animations. `catalyst-shell --smooth` reports,
for each kind of motion (apps opening and closing, pages, the control centre,
a list scrolling, Home's pages, the glance page, the carousel), the first,
typical and worst gap between frames at the simulator's two speeds. At the
estimated device speed every one of them runs on the 25 ms beat, with at most
one late frame as a page slides; at the worse-than-device speed everything
holds an even 50 ms, but for one late frame as an app opens. A page rising in
is drawn afresh each frame until its rows have arrived (0.8 s), where before
it moved as a still picture; an app opening now comes to rest in 35 frames.
`catalyst-shell --motion shots/motion` renders every kind of motion as strips
of frames.

What a frame costs is mostly glass: `--bench` splits it, and before this was
measured the glass over a screen cost fifteen times the screen itself. The
glass pass now frosts only the patch around each group of glass, tabulates
the bezel's bend instead of working it out per pixel, and keeps a group whose
shapes and whose patch of the scene are unchanged, which is most frames. The
frost is worked at half the panel's resolution (frost is a wide blur, so a
quarter of the pixels carry all of it), with the three colour channels summed
in one integer and the vertical passes run along rows. A full frame fell from
0.70 ms to 0.31 ms, then to 0.21 ms, on the development PC (an estimated
14 ms, then 4 ms, of the handheld's processor); the glass over the island
alone, when what is under it changes, from 147 µs to 100 µs.

On the device, frames are written to the framebuffer file a changed band of
rows at a time, which the kernel sends to the panel at once. (Writing through
a memory map would be gathered and sent on a timer about twenty times a
second.) The panel takes 16-bit colour, so each frame is dithered on its way
there with a fixed 4 × 4 pattern: dark gradients (the ground, glows, frosted
glass) keep their smoothness instead of breaking into bands, and a still
picture does not shimmer. The kernel's text console is put in graphics mode
while the shell runs, so a kernel message cannot draw over it, and its own
keyboard handling is off, so keys meant for the shell cannot switch consoles.
The tests in `shell.rs`, `gfx.rs` and `control.rs` hold all of this:
turning round mid-flight without a jump, reopening from where a window is,
frame-rate independence, that an animation begun after an idle spell starts
from its beginning, a list gliding and giving at its end, the control centre
coming down over any screen, and a robot's arrival opening the Robot app from
Home and nowhere else.

## The processor's share: NEON

The per-pixel loops every frame runs through use the Cortex-A53's NEON, four
or eight pixels to an instruction (`crates/bezel/src/simd.rs`): the 16-bit
dithered pixels each changed row becomes on its way to the panel, a frame
blended into another or faded toward black (a look changing, a finished app
fading out, the screen dimming behind the control centre), and each row of a
screen scaled into a growing window. Each gives exactly the pixels the plain
code does, bit for bit, which the ARM build's tests check.

## Graphics: the GPU

The CM0's VideoCore IV draws the two things that cost the processor most per
pixel, every frame they move: the **glass** (refraction through the bezel,
frost, dispersion, the edge light and the shadow, for the island, the pager
and the control centre's sheet) and an **app's window growing** out of its
tile (the whole screen scaled into it with its shadow and rim). Everything
else (text, tiles, icons, the wallpaper) stays the processor's: there is little
of it per frame, and its glyphs are cached.

The panel hangs off SPI, which no GPU can scan out to, so what the GPU draws
is read back into the frame and sent like the rest. The shaders are the
processor's own glass and window code, line for line (`apps/shell/src/gpu.rs`),
in OpenGL ES 2 through Mesa's EGL with no window system.

It is automatic, and never a risk:

- Mesa is loaded when the shell starts (`dlopen`), not linked, so the shell
  starts and draws on the processor alone without it.
- At start the GPU draws a test frame of glass and a growing window, and the
  shell compares them with the processor's. Only if they agree (within a
  level or two of rounding) does the GPU draw; under emulation, through the
  image's own Mesa, they differ by 1.45 levels in 255 on average for glass
  (3 at most) and 0.57 for windows.
- Then it takes only the jobs it does faster. Both are timed on the same
  test frame (best of three): reading a frame back from a GPU has a fixed
  cost, so glass as small as the island may still be quicker on the
  processor, while a whole screen scaled into a window is the GPU's kind of
  work. The shell's log and Settings › About say what was chosen and why.
  Under emulation the software renderer is about 40 times slower, and the
  processor keeps both.
- A software renderer is never used in its place, and if the GPU's driver has
  not loaded yet when the shell starts, the shell looks again when its device
  (`/dev/dri/renderD128`) appears.
- Any error while drawing hands everything back to the processor for good.
- Memory: Mesa is loaded only once a GPU device is found, so without one, or
  with Graphics set to the processor, it costs nothing (the emulator's idle
  figures are unchanged). With it, the shell's share grows by the driver's.
  Under emulation the software renderer took the shell to 80 MB, which says
  little about the VideoCore's driver: that is to be measured on the device.
- Settings › Display › Graphics switches it off (*processor*); Settings ›
  About says which is drawing. `CATALYST_GPU=off` does the same from outside;
  `CATALYST_GPU=force` gives the GPU every job, quicker or not, and
  `CATALYST_GPU=software` allows Mesa's software renderer too, for testing.

The boot configuration turns on the VideoCore's kernel driver
(`vc4-kms-v3d`, without HDMI or audio) with 32 MB of contiguous memory for it.
The panel is found by its size, so the second graphics device this brings does
not confuse it. `scripts/test-arm.sh` runs the GPU test through Mesa's
software renderer under emulation, which stands in for the VideoCore there.

## External apps

Apps that are not part of the shell cost no memory until they are opened and
none after they are left. Their tiles follow the built-in ones on Home.

An app is a folder under `/data/apps` (your own) or `/usr/share/catalyst/apps`
(the image's) holding a file named `app`:

```text
name = Processes
icon = memory            one of the built-in symbols; optional
about = what is running  the line under the name; optional
run = top                a shell command line, run from the app's folder
screen = terminal        or "own"
```

- `screen = terminal` runs the program in the handheld's terminal, which is
  all a script or a text program needs.
- `screen = own` hands it the panel and keys outright. It draws to the
  framebuffer named in `CATALYST_FB` and reads the keyboard itself, and is
  told the look in use (`CATALYST_LIGHT`, `CATALYST_ACCENT`, `CATALYST_TEXT`,
  `CATALYST_CALM`). The shell draws nothing until it ends.

A script dropped straight into `/data/apps` (`check.sh`) is an app too, with
no folder needed: write one in the editor and it has a tile.

Ctrl-Q, or Esc held for a second, always comes back Home, from an app that
has hung as well: the program and everything it started are stopped. (A short
Esc still goes to the app.) The image brings one external app, Processes
(`top`), as a working example.

Programs brought to the handheld can be statically linked, as Rust and Go
programs are by default, or linked against the image's musl, as the shell
itself now is (it needs `dlopen`). The C++ runtime is in the image, since
Mesa's shader compiler is C++. `CATALYST_ACCENT` is an accent's name, or
`hue-<degrees>` (0 to 355, every 5) for your own colour.

## Terminal, Files and the editor

Terminal is a real shell (`/bin/sh` on a pseudo-terminal) on the handheld's
own screen, 46 columns by 10 rows at the standard text size. The emulator
behind it handles what shells, `vi` and the usual tools print: cursor
movement, colours, scrolling regions, the alternate screen. The session
outlives the screen, so going Home and back finds it as it was. Esc belongs
to the programs in it; Ctrl-Q (or Esc held) returns Home. Page Up and Page
Down scroll back.

Files browses the storage (starting in `/data`), makes a file with `n` and
deletes one with `d` pressed twice. A link to a folder opens the folder;
devices, pipes and sockets are listed but never opened. Enter opens a text
file in the editor: arrows move, Ctrl-S saves (through a temporary file put
on the card before it replaces the old one, so a power cut cannot leave half
a file), Ctrl-Q closes, Ctrl-K removes a line. A file comes back out as it
went in apart from what was changed: tabs stay tabs (shown as →, and Tab
types one in a file indented with them), Windows line endings stay, a missing
final newline is not added, a script keeps its permissions, and saving
through a link changes the file it points at.

On a PC the terminal is a small stand-in shell, since there is no
pseudo-terminal to use the same way. The real one is exercised by
`scripts/test-arm.sh`, which runs the tests as the ARM build under emulation
and talks to an actual shell through the emulator, and by `scripts/vm.py`,
which types into it on the image's own kernel.

## Wi-Fi

Settings › Network › Wi-Fi networks switches Wi-Fi on, lists what is in range,
joins a network (the password is typed on the keyboard) and forgets one. It
drives the system's `wpa_supplicant`; networks are kept in
`/data/wifi/wpa_supplicant.conf`. While Wi-Fi is off the radio's driver is
not loaded at all, and switching it off unloads it. On a PC a scripted set of
networks stands in.

## Match logs

The handheld pulls `.wpilog` files from the robot over the Ethernet cable,
through two read-only routes added to the Catalyst Agent (`/api/logs` and
`/api/logs/<name>`). Those routes are a change to FrcCatalyst, sitting
uncommitted in `work/FrcCatalyst` (`agent/`), and a robot needs that agent
for logs to appear. Pulling happens in the background while the robot is
linked and not running an opmode, resumes where it stopped, and each log is
summarised once (brownouts, battery low point, health checks fired, loop
overruns, hottest mechanisms, the busiest CAN bus). A summary an older shell
wrote is made again the next time its log is looked at, so trends have
everything. Logs are cleared after 30 days, oldest first
if the card fills; `/data` grows to fill the SD card on first boot. Space is
made for a log before it is pulled, and one the card could never hold is not
started; the space kept free is 256 MB, or a quarter of the partition if
that is less. A cleared log is remembered (`/data/logs/.cleared`) and not
pulled again. A damaged log is summarised up to the damage, never read whole
into memory, and a resumed pull keeps only the missing part even from a
server that ignores the request to resume.

## Match trends

Once two matches have been read, the top of Logs is a Trends row saying what
is getting worse ("battery sagging lower · 4 more", or "nothing getting
worse"); the focus still starts on the newest match below it. Trends shows,
match by match across the last sixteen read, the battery's low point,
brownouts, loop overruns, the hottest motor and the busiest CAN bus, as bars
with the newest match at the right. A measure that is worse in the newest
match than in the five before it (the battery more than 0.4 v lower or under
9 v, a brownout, three more overruns, a motor 6 °c hotter or at 80, CAN 8%
busier or at 90%) is marked, its bar and value in amber with ▲ or ▼. Left and
Right go back through the matches (the match chosen is named at the bottom),
Enter opens it. The demo robot's logs are a day and a half of qualifying in
which the robot slowly gets worse until its newest match browns out.

## Live graphs

In Tables, Enter on a value graphs it (Enter again takes it off; a flag
graphs as 1 and 0), up to four at once; `g` opens Graphs, which is also on
Home. Each value has a lane: its name and reading on the left (the unit read
from the key's name, so `BatteryVolts` is "battery" at "12.4 v"), and its
last minute on its own scale on the right, since graphed values are rarely
in the same units. The fewer graphed, the taller the lanes. Up and Down
choose one, whose range over the minute is at the bottom; Space pauses; `d`
takes one off; Enter opens it in Tables. The robot's recorder samples the
graphed values ten times a second whatever is on screen, so a graph opened
late already has its minute; which values are graphed is remembered. A
graphed value wears its line's colour in Tables. The colours are the signal,
ice (deeper in light mode), amber and ink, each at least 3:1 against the
plot in both tones for every accent.

## The pit: checklist and match timer

The Pit app opens on two cards, the checklist and the match timer.

- **Checklist.** What a team checks before queueing (battery charged and
  strapped in, bumpers, radio, code, mechanisms zeroed, bolts and chains,
  breaker and CAN, driver station, spares), the team's own to change: Enter
  ticks an item, `n` adds one, `d` twice deletes one, `r` clears every tick
  for the next match. Where the robot can vouch for an item, the row says
  what it reads (the battery's volts, whether the radio links, the deployed
  code's version, the busiest CAN bus), amber when it looks wrong. Kept in
  `pit-checklist` beside the settings.
- **Match timer.** A match counted as the field counts it, from the 2026
  game manual (REBUILT): AUTO's 20 seconds, then TELEOP's 2:20 as a 10 s
  transition shift, four 25 s alliance shifts and END GAME's last 30
  seconds. The clock large, the phase under it, what comes next and when,
  and the match as a bar of its phases. Enter starts and pauses, Left and
  Right go to the phase before or after, `r` resets; the island says each new
  phase. It keeps running whatever is on screen, and Home's Pit tile shows it
  ("1:45 shift 2", in amber during END GAME). While the robot is linked and
  publishes its own match clock (`/Catalyst/Match/TimeLeft`, which FrcCatalyst
  2.0 fills from the FMS), that is shown instead, its period told from what
  it counted before (the clock alone cannot tell AUTO's 0:15 from END
  GAME's).

## Event schedule

The Schedule app lists the team's matches at the event under way (or the
next one): alliance colour, the time or the result, and while the event runs,
live queueing from FRC Nexus ("now queuing Q27", "queuing soon"). Enter shows
both alliances. The Schedule tile and the glance page's next-match card show
the next match and when to queue.

The schedule comes from The Blue Alliance (API v3: the team's events this
year, then its matches at the one under way), fetched every five minutes, and
from FRC Nexus every 30 seconds while the event is on, both with `curl` over
HTTPS and the keys from setup or Settings › Keys. What was fetched is kept
(`/data/catalyst/schedule.json`), so in the stands with no network it still
shows, saying how old it is. The team is the robot setting's. The Nexus side
follows its published v1 docs; The Blue Alliance's docs could not be fetched
while this was written, so its side is written from its long-standing v3 API
and tested against sample replies, not yet against the live service.

## Git and the robot's code

The Git app lists the repositories under `/data/code` and clones another by
its address. A repository's page says its branch and how far it is from its
upstream, pulls (fast-forward only) with one key, and shows what has changed
as a coloured diff and its history. Git runs in the background, so a slow
card never holds a frame up. Credentials are not stored: public repositories
clone and pull as they are.

One repository can be marked as the robot's code. FrcCatalyst publishes the
commit a robot was built from (`Robot/Software/RobotCodeVersion`, from the
team's build stamp through `RobotIdentity`), and the Robot page's `code` row
then says how it stands against the repository here: the latest, so many
commits behind, built from uncommitted changes, or not in this repository.

## App catalogue

The Catalogue app lists the apps in the index of a GitHub repository
(`TomAs-1226/catalyst-apps`; this project's `catalyst-apps/` folder is meant
to become it), and gets, updates and removes them over Wi-Fi. Each app is a
gzipped tar of an app folder with its SHA-256 in the index; before one goes
in, its hash is checked, every path in it must stay inside its folder, and it
must unpack to an `app` file that names something to run. A failure at any
step leaves what was installed as it was, and only apps the catalogue
installed can be removed from it. `scripts/pack-catalogue.sh` packs the apps
under `catalyst-apps/src/` and writes the index; packing is repeatable. The
three example apps ping the robot and its radio, show what is using the card,
and show the handheld's addresses and routes.

## Memory, size and boot

Measured by booting the image itself in QEMU's Raspberry Pi 3A+ (the same
processor family and 512 MB; `scripts/vm.py`), before and after the trim:

| | Before | After |
| --- | --- | --- |
| Memory in use at rest (total less available) | 36.4 MB | 26 MB (25.3 and 26.3 in two runs) |
| Memory the kernel keeps for itself before that count | 27.4 MB | 21.1 MB |
| The shell (its share of memory) | 5.6 MB | 4.5 MB |
| Kernel caches and tables (slab) | 28 MB | 9 to 11 MB |
| Kernel image | 16.8 MB | 10.1 MB |
| `catalyst-shell` | 6.2 MB | 3.3 MB |
| Root filesystem used | 48 MB | 37 MB |
| Image compressed for download | 38 MB | 29 MB |
| Boot to a login prompt, emulator time | 38 s | 24 s |

Counted the strictest way (what the kernel keeps for itself plus everything
in use), an idle system is 47 MB, down from 64. On the handheld the boot
firmware takes another 16 MB for the video processor, which no setting
returns. The emulator has no panel, keyboard matrix, fuel gauge or radio, so
the board's own drivers are loaded but idle in these figures, and emulator
seconds are not handheld seconds (about six of the 24 are spent waiting for a
panel that is not there): the table compares one build with another.

Since then, HTTPS and git came in: curl, mbedTLS and the certificates add
about 2 MB, and git about 8 MB once its server, mail, Perl and SVN/CVS tools
(which need Perl, or carry their own 2.3 MB copy of git's library each) are
left out. The root filesystem then used 47 MB, and the download was 33.6 MB.
None of it is in memory until something runs it.

Then the GPU's driver: Mesa (EGL, OpenGL ES 2 and the gallium driver for the
VideoCore, 17 MB of it) and the C++ runtime its shader compiler needs add
about 20 MB. The root filesystem now uses 67 MB of its 96 MB, the download is
39.0 MB, and `catalyst-shell` is 4.1 MB (it was 3.3). Mesa is loaded only once
the shell finds the GPU's device, so under emulation, which has none, the
memory figures above are unchanged by it.

With A/B updates the card image is 513 MB: the boot partition (128 MB, room
for two kernels), two 160 MB systems (68 MB used of each), and /data. System B
is empty in the image, so the download is still 40.8 MB (gzip); an update
from one version to the next is a patch of a few megabytes.

What changed:

- **Kernel.** On top of the Raspberry Pi configuration,
  `os/board/cardputer-zero/linux.fragment` takes out what is built in and
  costs memory whether or not it is used: virtual machines, PCI and NVMe, the
  Pi 4's and Pi 5's own chips, server networking, containers and accounting,
  32-bit program support, debugging aids. Every USB driver stays, as a module
  loaded when its device is plugged in: serial adapters, drives, keyboards
  and controllers, network adapters. USB storage and the network drivers that
  were built in became modules too. `scripts/build-kernel.sh` rebuilds it and
  lists any switch that did not land.
- **Boot.** Two dozen stock services became five. `catalyst-early` loads the
  panel, key and clock drivers and mounts storage; the shell starts straight
  after; `catalyst-late` (hotplug, the rest of the board's drivers, the
  system log) runs behind it. The services that failed on a read-only root at
  every boot are gone.
- **Idle.** The Wi-Fi driver loads only while Wi-Fi is on. The cable is
  watched by `ifplugd` instead of two shell scripts waking every two seconds.
  The system log is a 64 KB ring in memory (`logread`). The file names the
  kernel remembers from scanning for hardware are handed back once boot is
  done.
- **Size.** The fonts keep Latin, digits, punctuation, arrows and line-drawing
  characters, and only the design axes the UI moves
  (`scripts/subset-fonts.sh`; the full files are in
  `userland/assets/fonts/full`): 4.0 MB became 0.9 MB. Libraries no program
  loads are left out of the image.

## Updates, over the air

`catalyst-update` updates the handheld the way Android does, with two
systems on the card:

| Partition | |
| --- | --- |
| 1 boot (FAT, 128 MB) | the firmware, `config.txt`, and each system's kernel, device trees and overlays in its own folder (`slot-a/`, `slot-b/`) |
| 2 system A, 3 system B (ext4, 160 MB each) | one runs (read-only); the other takes the next update |
| 4 data (ext4) | settings, logs, keys, downloads; grows to fill the card |

- **Found.** Once a day when online (and from Settings › System update ›
  Check now) it looks for the newest release on its channel, *stable* or
  *nightly*, and checks the release's manifest against the Ed25519 key built
  into the image. Nothing unsigned, or signed by another key, is installed.
- **Installed in the background** while the handheld is used: a patch from
  the version running (only the 64 KB blocks that changed, typically a few
  megabytes) or the whole system, written into the system not running, read
  back and checked against the release's hash, and its kernel put in that
  system's folder on the boot partition. Downloads resume where they
  stopped. It waits for a charger below 25% battery.
- **Tried once.** "Restart to update" (or overnight, at three in the morning
  when nobody is using it, if chosen) restarts with `reboot "0 tryboot"`: the
  firmware starts the new system for that one boot only (`config.txt`'s
  `[tryboot]` section). Once the shell has been up for 15 seconds the new
  system is made the one that starts. If it panics (`panic=10`), hangs (the
  watchdog), or its shell never comes up (three minutes), the next start is
  the old system, which says the update did not take.
- **Seen.** Settings › System update shows where it stands with a progress
  bar, the next step, what is new, the channel, and the two choices
  (download automatically, restart overnight); the island says when an
  update is available, ready, done, or did not take. Apps update apart from
  the system, through the catalogue.

Releases are made on this PC by `scripts/release.sh` (stable, tagged
`v<os/VERSION>`; or nightly, a pre-release) and published to the GitHub
repository's releases with `--publish`: the system and its patches, the
signed manifest, the card image, its Raspberry Pi Imager listing, the
Windows flasher and the packed simulator. The signing key is made on the first run and kept in
`~/catalyst-linux-build/keys` (back it up: handhelds only take updates signed
with it). `scripts/schedule-nightly.ps1` makes Windows run it every night.

## Flashing a card

**Catalyst Flasher** (`catalyst-flasher.exe`, attached to every release):
three steps, left to right. The system (the newest stable or nightly
release, or an image file dropped on the flasher's icon), the card (only
removable SD and USB drives are offered, never a disk the PC starts from),
and Flash, pressed twice. It downloads the image and checks it against the
release's hash, asks for Administrator, writes the card with the partition
table last (so Windows cannot mount half a card), reads every byte back, and
says when the card is ready. `catalyst-flasher --shot DIR` renders its screens.

**Raspberry Pi Imager** lists Catalyst Linux too, from the release's listing:

```powershell
rpi-imager --repo https://github.com/TomAs-1226/CatalystLinux/releases/latest/download/os_list_catalyst.json
```

## Build the SD card image

Inside WSL (Ubuntu), nothing needs root:

```bash
bash scripts/wsl-setup.sh               # once: Buildroot, Rust
bash scripts/build-image.sh toolchain   # once: Buildroot's cross compiler, which the shell links with
bash scripts/build-shell.sh             # cross-compile catalyst-shell and catalyst-update for the CM0
bash scripts/build-image.sh             # about an hour the first time
bash scripts/build-kernel.sh            # only after changing the kernel configuration
```

The image is `~/catalyst-linux-build/output/images/sdcard.img`. These check
it without a device:

```bash
bash scripts/inspect-image.sh        # boot files, drivers, services, libraries, sizes
bash scripts/test-image-services.sh  # first-boot partition growth and its retries, the network address rule
bash scripts/test-arm.sh             # the shell's, renderer's and updater's tests as the ARM build
bash scripts/wsl-qemu.sh             # once: fetch the machine emulator
python3 scripts/vm.py --tour shots/vm  # boot the image, measure it, drive the UI
```

`vm.py` boots the image's own kernel, device tree and root filesystem, goes
through the first boot's restart (which grows `/data`), reports memory and
what is running, and then drives the real ARM shell from an emulated USB
keyboard: the start animation skipped with a key, setup put off (and that
remembered), an app in the terminal, an app that takes the screen, Esc held
to leave it, the control centre, keys typed into a real shell, and at the end
that no program is left behind as a zombie. Then a system update through the
real partitions: a release made from the image itself and served from the
PC is found, its signature checked, written into system B, read back, and
the boot partition set to try it. The firmware's framebuffer, set to
320 × 170, stands in for the panel.

The shell's own tests include one that drives every screen with thousands of
random keys at random frame times, with robots coming and going and settings
changing, as a debug build so that an integer overflow fails it too.

## State

Working and checked on a PC: the spring engine (tests against Apple's
documented values), the renderer, the shell and its screens against a scripted
robot, the NT4 client against a scripted NT4 server, the Catalyst Agent client
and the log puller against the real agent script (with the new log routes),
the log reader against a generated match, the ARM build of the shell under
user-mode emulation, the image's first-boot scripts under emulation, and the
image booted whole in a machine emulator: its services, its first-boot
restart, its memory, and the shell drawing and taking keys on its kernel.
The GPU path is checked through the image's own Mesa under user-mode
emulation, with the software renderer standing in for the VideoCore. System
updates are checked end to end on files (found, patched, tried, kept, rolled
back, a wrong key refused) and installed through the real partitions in the
machine emulator; NEON against the plain code, bit for bit, on the ARM build.

Not yet checked: anything on a real CardputerZero (the panel and how fast it
takes frames, the GPU on a real VideoCore and whether reading its frames back
costs less than drawing them, the firmware's one-time try of an updated
system (the emulator starts kernels itself), the flasher writing a real card
(its writing and reading back are tested on a file), the keyboard, Ethernet, Wi-Fi, the fuel gauge and clock chip,
idle memory with the board's drivers at work), the NT4 client against a real
Systemcore, the log reader against a log from a real robot, the schedule
against the live Blue Alliance and Nexus services, setup from a computer over
a real network, and the catalogue against its repository (which does not
exist yet; its install path is tested with a stand-in download). Not yet
built: the agent's machine pages (CPU, CAN buses, cameras), the AI log
analysis, the camera, audio.

## Roadmap

Chosen so far, not yet built: scripting with a kit for writing Bezel apps (so
an external app can draw like a built-in one); an SSH client and serial
console; a network toolkit; USB gadget modes (serial, network and a drive of
logs when plugged into a PC); a system monitor; standby and power saving;
image updates with rollback; an AprilTag reader on the built-in camera,
calibrated so it can measure for the robot; a robot finder; the AI log
analysis.

Added since, also not yet built:

- For the robot: alerts (rules that watch the robot and flag it on the
  island with a tone). The link stays read-only: a tuner that writes to the
  robot was offered and not chosen.
- For the pit: QR codes for a match summary or an address.
- For development: an HTTP tester; a hex viewer and programmer's calculator;
  saved commands run in the terminal with one key; a serial plotter.
- For the system: an app switcher; screenshots; sounds on the speaker, with a
  mute switch in the control centre; a lock screen with a PIN.

Built since: the post-match report, the code version check, the event
schedule, search, the app catalogue, git, the glance page's cards, first-boot
setup (on the handheld or from a computer) and the start animation; then the
themes and looks, Home's arrangement, clock, island and greeting, the
personal start, the new motion, and the GPU; then live graphs, the pit
checklist and match timer, and match trends.

## Third-party pieces

Google Sans Flex and Google Sans Code (SIL OFL 1.1) and Material Symbols
(Apache 2.0) are vendored under `userland/assets` with their licences; the
fonts built into the shell are subsets of them. The board drivers come from
`m5stack/m5stack-linux-dtoverlays` (GPL-2.0) at build time.

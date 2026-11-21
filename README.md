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
| `userland/crates/apriltag` | AprilTag detection: AprilRobotics' C library (3.4.5, BSD 2-Clause, vendored with the 36h11 family FRC uses) behind a small flat shim, with each tag's pose. |
| `userland/apps/shell` | `catalyst-shell`: the start animation, first-boot setup, Home, search, the glance page, the control centre, recent apps, the built-in apps (Robot, Health, Logs with its trends, Schedule, Tables, Loop, Motors, Console, Terminal, Files, Git, Catalogue, Monitor, Net tools, Connect, Actions, Camera, Photos, AprilTag, Graphs, Pit, Remote, Level, Compass, Weather, Settings) and the launcher for external ones. |
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
OpenSSL (from 0.4.2; mbedTLS before, which left curl with no certificates
to trust), shared by updates, git, the schedule, Weather, the assistant and
the catalogue · keys for online
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
Ctrl-Space brings the control centre down over any screen, and Ctrl-Tab the
recent apps; Left from Home's first tile is the glance page. `?` held for five seconds on any screen that
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
  drawn as keycaps, ending with the keyboard itself (the modifiers, and
  Fn's keys). Where text is typed (search, a text field, the terminal), `?`
  is only ever typed.
- **The keys of the handheld.** There are no arrow keys: F, X, Z and C are
  up, down, left and right wherever text is not being typed, and Backspace
  is Back. In a text field (and the terminal) they type, and Fn with them
  gives the arrows. Sym's layer types the symbols printed on the keys: its
  key codes are ones the keyboard's driver borrows from keys the CardputerZero
  does not have, so the shell maps them back, as M5Stack's own system does with
  a console keymap (only for the built-in keyboard: a USB keyboard's keys
  are their own). Fn + Backspace is Delete, Fn + K and N are Home and End
  (the first and last row of a list), Fn + L and M are Page Up and Down, Fn
  with the number row is F1 to F10 (O and P are F11 and F12: htop's F10
  quits it), and Fn + U and I set the brightness over any screen.
- **About**, laid out as Android's "About phone": the handheld's name large
  at the top with Catalyst's icon (the header folds away as the list goes
  down), its model, the Catalyst Linux version (a page of its own: channel,
  build date, kernel, shell, graphics), the update history (every version's
  notes; see *Update logs*), its status, the open-source licences, and the
  build number at the bottom.
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
Before it, the panel stays dark. The backlight is not lit until the first
frame is on the panel, and then it rises to the brightness kept in the
settings. The kernel's console is not drawn on the panel; it goes to the
serial port only. So nothing the panel held from power-on, and no boot
text, ever shows. Every part arrives on Detent's springs by Bezel's roles; the trail alone
accelerates, since it is thrown rather than placed. Any key skips it.
`catalyst-shell --start-movie file.png` renders it as an animated PNG
(`shots/start/`).

The first start continues into **setup**, which owns the whole panel the way
a computer's own setup does: no island, no status, the brand's ground, each
page's title, words and rows rising in one after another from the side the
page came from. "Hi." offers three ways on:

- **Set up here**: Wi-Fi first (its radio is switched on as the page
  arrives, so the networks are listed by the time they are asked for, and
  off again if the page is passed by without joining one), the hardware
  check (below), the keyboard lesson (below), the team number (the robot's
  address follows from it), the handheld's name and time zone, keys for
  online services, a page of choices (open
  Robot on link, the match report, SSH, the calm version), battery and power
  (the screen-off time, the battery saver at 20%, Wi-Fi in standby, and lift
  to wake where there is a motion sensor), and the look (accent, mode, text size,
  Home's style, with a live picture of Home), then "Just a moment…", "You're
  all set." and Home. The pages are one list in `setup.rs` (`FLOW`): Next goes
  one along, Esc one back, Tab skips one, and the progress marks count them.
- **Set up from a computer**: the handheld shows its address, as text and a
  QR code, and a six-digit code. Any browser on the same network (the
  Ethernet cable, or Wi-Fi) gets one page asking for all of the above plus an
  SSH public key, typed on a real keyboard. Nothing listens on the network
  except while that page is offered, for ten minutes at most; five wrong
  codes close it. It is plain HTTP on the local network, so keys cross it
  unencrypted: use it on the robot's cable or a network you trust. Settings ›
  Set up from a computer offers it again later. With no network yet, the
  page offers to join Wi-Fi there (or take a cable), and opens by itself
  once there is an address. When the browser sends the form, the handheld
  shows each item arriving (team, networks, keys shown only by their last
  four characters, look, choices), then "Just a moment…". Each browser
  connection gets its own thread, so a spare connection the browser opened
  early and left idle cannot hold up the one carrying the form.
- **Set up later**: straight to Home; everything is in Settings.

`/data/catalyst/setup-done` marks setup as done.

**The hardware check** comes first, and is in Settings › About too. Each part
of the handheld is looked for and shown on a chip:

- the battery's fuel gauge;
- storage;
- the clock chip;
- the Wi-Fi radio;
- the Ethernet adapter (behind the USB switch, which must be to the right);
- the camera;
- the motion sensor, and the compass where the unit has one;
- infrared.

A found part is ticked. A missing one is amber, and the line under the title
says what was found or what to try. Enter on Keys opens a line to type that
uses every letter and digit, and Keys is ticked once the line is typed whole.
Parts that come late are looked for again every few seconds: the motion
sensor starts a moment after the shell, and the adapter appears when the
switch moves.

**The keyboard lesson** is five short cards, each with a little practice
that ticks off as it is done (Enter always goes on, so nobody is held up, and
Tab skips the lesson): Shift, Fn and Sym, and their three ways (tapped: for
the next key only, its light blinking slowly; tapped twice: locked on until
tapped again, its light flickering; held: while held, its light on), as the
keyboard's driver does them; F X Z C as the arrows outside typing, with a
ring to move; a capital with Shift and a symbol with Sym; in a text field
the letters type and Fn turns F X Z C into arrows that move the cursor; and
Fn's other keys (Delete, Home and End, the pages, F1 to F10, brightness). On
a computer the PC's own arrows and Shift do the same. Settings › Help ›
Keyboard lesson has it again.

**Name and time zone.** The name is offered with the team in it
(`catalyst-5805`); it is typed as a network takes one (lower case, a hyphen
for a space) and becomes the handheld's hostname, sent with every DHCP
request so a router lists it by name, and shown in About. The time zone is
picked from a list by part of the world (Left and Right jump a region; Space
or / searches every zone the handheld has rules for, by city, zone or
region). Both are kept on `/data` (`/data/catalyst/hostname`, `timezone`, and
`localtime`, a link to the zone's rules that `/etc/localtime` points at) and
come back after a restart or an update; both are in Settings › Name & time,
with where the clock's time came from. The clock sets itself from the
network whenever DHCP gives an address with a router (`catalyst-time`, BusyBox's
`ntpd`, giving up after 25 s on a network with no way out, and asking at most
every six hours), then writes the RX8130 clock chip, which keeps the time
while the handheld is off and gives it back at the next start.

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
whatever is showing: Wi-Fi, SSH, light or dark, the battery saver, and
brightness, with two actions along its foot, Recents and Screenshot (R and S
while it is down). Ctrl-Space brings it down from any screen, a terminal
included (so does the Opt key, if the keyboard reports one), and Up from the
top of Home pulls it down too. Esc, Ctrl-Space again, or Up from its top row
puts it away. The screen behind is held still while it is out, so the glass
is worked out once and moved as a picture.

## Recent apps

An app left is kept alive just as it was: its pages, its place in a list, a
terminal's session. The last six are kept, one card an app; an older one is
let go as a new one comes.

- **Ctrl-Tab** (or Recents in the control centre) brings up the **recent
  apps** over any screen, a terminal included: no program in a terminal can
  be sent Ctrl-Tab, so it takes nothing from one. The screen in front fades
  into the wallpaper as the cards rise in: a tonal card for each app with the
  picture it last showed, the chosen one in the middle in the focus ring, the
  others sitting back beside it.
- The app before the one in front is offered first, so **Ctrl-Tab, Enter**
  goes back and forth between two. Left and Right (or Tab, or Ctrl-Tab again)
  choose; **Enter** brings an app back growing out of its card (the card's
  picture melting into the app as it grows), in place of the one in front,
  which joins the recent apps; **Up** (or Backspace, the handheld's Del) lets
  one go, rising away as the others close up; Esc puts the cards away.
- An app **opened again from Home** comes back as it was too, as Android
  brings a task back, rather than as a new copy.
- Memory stays small: a card's picture is the panel at 0.425 (39 kB).

## Screenshots

Screenshot in the control centre (or S while it is down) saves the screen
beneath the sheet, not the sheet: a flash, and the island says when it is
saved. Each is saved twice, as PNG: at the panel's own size, pixel for pixel,
and at twice that, for looking at on a phone or putting in a document.
They go in `/data/screenshots` on the handheld (Files opens onto `/data`), or
`screenshots` beside the settings on a computer. Saving is done on a thread
of its own, so no frame waits for it.

## Standby and power saving

After a while with no key, the screen **dims** (eight seconds before it would
go off), then goes **off**: the backlight fades down and out, and nothing more
is drawn or sent to the panel. **Any key wakes it**, and is spent doing so (it
does not also open or move something); a robot plugged in wakes it too.
Settings › Display:

- **Screen off after**: 30 seconds, 1 minute (the default), 2, 5, or never.
- **Processor**: *balanced* (fast when there is work, slow when not:
  `schedutil` or `ondemand`), *power saving* (`conservative`, or
  `powersave`), or *performance*. While the screen is off the processor runs
  at its slowest (`powersave`), whatever is chosen. The kernel's own
  governors, so only on the handheld.

A screen with something to watch keeps it awake while it shows (the hook is
`Screen::keeps_awake`): an app's program running in a terminal, and the login
shell while what runs in it is printing (a build, a log being followed), for
a minute after it last printed. On a computer, with no backlight to dim, the
picture itself dims. Where there is a motion sensor, picking the handheld up
wakes it for a look and laying it face down puts it out (see *Level, Compass
and the motion sensor*).

**Deep standby.** The CM0's chip has no suspend-to-RAM, so standby means
switching off everything that can be. Once the screen is off and dark:

- the panel itself sleeps: the framebuffer is blanked, and a patch to
  M5Stack's panel driver sends the ST7789 display-off and sleep-in (enabling
  it again runs its whole init sequence, as at boot);
- the shell stops keeping a frame beat and looks in four times a second;
- after ten minutes Wi-Fi goes off (the choice to have it is kept, and it
  comes back with the screen), unless an update is downloading or someone is
  logged in over SSH. Settings › Battery › Wi-Fi in standby turns this off.

Some savings run all the time. The Ethernet chip (an SR9900A, driven by
r8152) and the USB hub suspend whenever they are idle. M5Stack's community
measured that as the largest draw after the screen: about 90 mA. The driver
wakes on a cable or a packet. ifplugd reads the link from the interface's
own flags (`-m iff`), so it never wakes the chip to ask. The speaker
amplifier is held off from the first instant (`gpio=24=op,dl`). Wi-Fi's
own power save is on, which is brcmfmac's default.

## Battery

Settings › Battery has:

- **The charge**, large, beside a chart of the last 24 hours (a point every
  five minutes, a week kept in `battery-history` on /data).
- **How it stands**: "on battery · 4 h 38 m left", or "charging · full in
  40 m".
- **The draw**, in mA and W, from the BQ27220 fuel gauge.
- **The battery itself**: its health (what it holds now as a share of new),
  its charge cycles, temperature and voltage.

Its choices:

- **Battery saver**: 30 frames a second instead of 40, the wallpaper held
  still, and the processor capped at 800 MHz and slow to climb (the kernel's
  `conservative`). It is a switch in the control centre too, and comes on by
  itself at 20% (**On by itself at 20%**), going off again on the charger
  unless you switched it on yourself.
- **Turn off at 5%**: warnings at 15% and at 10%, once each time on battery.
  The island says it and the keyboard's lights blink for a few seconds. At
  5%, held for three readings, a card counts ten seconds down and the
  handheld turns off cleanly, so the card is never cut off half-written.
  Plugging in keeps it on, and Esc puts it off until 2%. Slide the switch off
  afterwards: a software power-off leaves the switch to cut the power.
- **Wi-Fi in standby**: off after ten minutes with the screen off, or left on.
- **Red light**: quiet (dark, blinking only if the kernel panics) or card
  activity, as the CM0 does by default.

**Lower voltage** (Developer options) runs the processor's core a step
lower: `over_voltage=-1` (−25 mV at full speed) and `over_voltage_min=-2`
(−50 mV when idle, where there is the most to spare). The lines go into
config.txt above the system block, so they hold whichever system starts and
updates keep them. Enter on it says what it does and offers *Restart and try
it*: `catalyst-power undervolt try` sets the lines and restarts cleanly.

It is tried before it is kept. The first start with it counts. Once the shell
has run on it for five minutes, `catalyst-power` confirms it. While it is
tried, the hardware watchdog is fed, so a processor that hangs on the lower
voltage is restarted within 15 seconds. (Not on a start that is an update
being tried, where catalyst-update holds the watchdog.) If a start finds it
tried but never confirmed (that start crashed, hung or was switched off), it
takes the lines out again and restarts, and the island says it did not
hold.

The row then says what the firmware actually gave the core, as read from
its mailbox (`/dev/vcio`, as vcgencmd does): *on · 1.175 V*, or *on · not
taken* if the ceiling did not come down. It is judged against the ceiling
read on a start without it, or 1.2 V. Kept, the island says the same.

## The keyboard's lights

The Shift, Fn and Sym lights hang off the M5IOE1, which blinks them by
itself, so a blink costs the processor nothing. The keyboard driver uses them
to show a modifier's state: on while held, a slow blink for one key, fast
while locked. The shell borrows them for two things and gives them back. If
a modifier changes meanwhile, the keyboard's state stands.

- **A notice**: something was said while the screen was off (an update
  ready, a message). Fn blinks slowly until the screen wakes.
- **A warning**: a battery warning, the pit timer moving to its next phase,
  queueing for the next match (five minutes ahead, then when it is time,
  from FRC Nexus). All three blink fast for eight seconds, then stop.

## The panel, without tearing

Anything moving on the panel used to tear on a diagonal. The ST7789 is a
170 × 320 panel, upright. M5Stack's set-up turns it on its side in the
controller (MADCTL `0xA0`: rows and columns exchanged), so the panel
refreshes its gate lines across the picture's columns while frames arrive
over SPI row by row. Every frame meets the refresh part-way and splits along
a slope. No sync can fix that on its own: the frame and the scan run
crosswise.

Developer options › **Tear-free panel** drives it the panel's own way:

- **Upright.** The `cardputerzero-portrait` overlay, applied after M5Stack's,
  gives the panel its own init sequence. That is
  `cardputerzero,st7789v_portrait.bin`, made from M5Stack's by
  `scripts/panel-portrait-blob.py`, with MADCTL `0x00` and nothing else
  changed. It also gives a framebuffer of 170 × 320 on the 170 of the
  controller's 240 columns that the glass shows: the same pixels as
  M5Stack's landscape mode, one for one. The shell turns its 320 × 170
  picture a quarter as it sends it, and sends only the columns that changed
  (now a run of the panel's rows).
- **Timed from the scan.** The panel's tearing-effect line (GPIO5, wired by
  M5Stack, its code in their driver but switched off) is switched on by
  patch 0003. Its interrupt stays on while the panel is on, and each rising
  edge (the scan leaving the last row) only notes the time: that gives the
  scan's phase, and between two edges its speed (16.2 ms a refresh, 49 µs a
  row over 320 rows and 10 of porch).
- **Never crossed.** A frame is 108,800 bytes (320 × 170 at 16 bits). The SPI
  clock is the core clock over an even divisor, so the 60 MHz asked for is
  about 42 MHz (core at 250 MHz) to 50 MHz (400): 17 to 21 ms a frame, 54 to
  65 µs a row. That is slower than the scan, so a band of rows follows it:
  the driver starts it just after the scan has passed its first row. That
  refresh shows the old rows, the next shows the new ones whole, provided
  the band is done before the scan comes round behind it. The gap closes at
  5 to 16 µs a row, at most 5.2 ms over the whole height against a 16.2 ms
  period, so a flush can usually start at once and otherwise waits for the
  scan. Were writing ever faster than the scan, the driver would lead it
  instead (start while the scan is above the band, or once it is past). The
  time a row takes to write is measured on the flushes themselves.

The first version of this mode tore all the same and looked worse. Each
flush enabled the interrupt, waited for one edge and disabled it again. The
BCM2835's GPIO interrupts have no disable of their own, so the kernel
disabled them lazily: an edge that came while "disabled" was kept, and
replayed the moment the interrupt was enabled. The wait returned at once,
anywhere in the scan. To lead the scan from the blank, it had also slowed
the panel to 40 refreshes a second, which made motion look rougher. Now the
interrupt is disabled for real (`IRQ_DISABLE_UNLAZY`) and the panel keeps
its own 60.

`/sys/bus/spi/devices/spi0.0/tearing_effect` says how it is doing:
`state=in-step period_ns=… write_ps=… followed=… led=… unsynced=…`. The
Tear-free panel row reads it: *on · in step*, *on · timing* while it times
the first edges, or *on · no sync* if the line never moves. After three
flushes with no edge, flushes go out as before until the panel is next
switched on.

Without the overlay, nothing changes: the driver uses the tearing-effect
line only where the Device Tree gives `te-gpios`. Everything this mode needs
comes with an update (the overlay in the system's slot folder, the driver
and the init sequence in its root filesystem). Only the trial's one
`dtoverlay=` line goes into config.txt, written on the handheld above the
system block, which updates keep as it is.

It is tried before it is kept (`catalyst-power panel`). Switched on, the
handheld restarts into it, and that start asks: *Keep the tear-free panel?*
Enter keeps it. With no answer in 15 seconds (or Esc), the panel goes back
as it was and the handheld restarts. A picture that came up turned,
mirrored or not at all needs nobody to read it: waiting is enough. A start
that finds it tried and never answered (a crash, the power cut) undoes it
too. `scripts/test-catalyst-power.sh` checks both trials.

Not yet checked on a CardputerZero: that the picture comes up upright. The
quarter turn is worked out from M5Stack's `0xA0`: picture pixel (x, y) on
the controller's column 35 + y and gate line 319 − x. Under `0x00`, row
`py` of the framebuffer must be the picture's column 319 − py, which is a
turn counter-clockwise. If the handheld shows it upside down or mirrored,
`CATALYST_PANEL_TURN` (`counter`, `clockwise`, `transpose`,
`antitranspose`) tries the others without a rebuild: every one still sends
in the panel's own order. `Turn::FOR_THE_PANEL` in `platform.rs` then takes
the one that is right. Also unchecked: that TE pulses on GPIO5 (the kernel
says "Timeout waiting for TE signal" once if not), and how a frame now
looks in motion. Apps that draw for themselves get the upright 170 × 320
framebuffer in this mode.

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
- With keys, nothing carries the motion but the spring, so what moves under
  them runs a little past and comes back, on Material 3 Expressive's spatial
  springs (as Detent carries them): `m3-expressive-fast-spatial` (stiffness
  800, damping ratio 0.6) for small moves, `-default-spatial` (380, 0.8) for
  everyday ones, `-slow-spatial` (200, 0.8) for a whole page. The calm
  version damps every one critically, as Bezel's does.
- A page inside an app slides in from the right and back out (slow spatial:
  as long as Bezel's page turn, but it runs about 5 px past and settles back),
  and turns round the same way. Pushed while the page before it is still
  sliding in, that slide plays on beneath the new page rather than landing at
  once. Home's pages and the glance page turn on default spatial; the
  carousel turns on `detent`.
- Lists glide to where they are scrolled rather than jumping a row at a time
  (`hold`), with the focus ring heading for where the focused row will rest.
  A key pressed against the end of a list, the last app or the glance page
  makes it give a little and spring back: "no further", without a word.
- **The focus ring** glides to whatever is focused on fast spatial: about 2 px
  past a row, 10 px past a tile, settled within 0.4 s. Moving, it stretches
  along its way with its tail behind it, as Bezel's dock droplet does (0.04 ×
  speed, at most 13 px, never more than a third of its length), thinning a
  little across, and draws itself together as it lands; its size and corners
  follow on default spatial. Keys pressed quickly or held retarget it from
  where it is at the speed it has, so it never starts over. On Home it rides
  the strip of pages as they turn instead of chasing its tile, and it is
  drawn over every tile it crosses. Bezel itself lands changes made from keys
  at once; this bends that for the sake of flow, and the calm version
  restores it (no glide, no stretch).
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

**Why the processor draws most of the time.** Settings › Display ›
Graphics says what draws and why in a few words: *gpu · glass, windows*, or
*processor* with the reason (*faster here*, *no gpu found*, *gpu differs*,
*your choice*). Settings › About › version › Graphics has the timings that
decided it. The reasons it is usually the processor on this handheld:

- **The picture has to come back to the processor anyway.** The panel is on
  SPI, so every frame is sent by the processor's SPI controller from
  ordinary memory: 108,800 bytes at 16 bits, 17 to 21 ms of DMA per whole
  frame, during which the processor is free. A GPU frame means handing the
  GPU the 217,600-byte scene under the glass, waiting for it, and reading
  the result back out of uncached memory, every frame. That copying is
  processor time too. At this panel's size it is of the same order as the
  glass itself (about 4 ms of the handheld's processor for a whole frame of
  glass, estimated from the PC's timings), so the shell times both on a
  test frame and uses the GPU only for what it wins.
- **Nothing is drawn while nothing moves.** A still screen costs neither:
  no frame is made, and the panel keeps its own picture. When something
  moves, only the rows that changed are converted and sent.
- **The GPU costs power to wake** (its core and clock) and memory (Mesa's
  driver), so it is worth it only where it is clearly faster.

What only the handheld can say is which way its timing goes, and the
shell's memory with the VideoCore's driver loaded. `catalyst-shell --about`
prints the choice and the timings.

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

## The developer's tools: Monitor, Net tools, Connect, Actions

Four apps for using the handheld as a pocket Linux machine. Each reads or
works only while it is open (a thread of its own at most, stopped when the
page goes), so none of them costs anything once closed.

- **Monitor**: how the handheld itself is doing, read once a second from
  `/proc` and `/sys`. Four cards: the processor (all of it, each core as a
  bar, the load), memory in use and in all, the processor's temperature
  (amber from 70 °C, red from 80) with the time since start, and network
  traffic in and out; each with the last half minute as a graph. Right (or
  Enter) turns to the processes using the most of the processor, Enter
  sorting them by memory instead. On a computer there is no `/proc`, so a
  simulated handheld stands in and says so.
- **Net tools**: the robot finder looks everywhere a team's robot can be at
  once (its address 10.TE.AM.2, the roboRIO by name and over USB at
  172.22.11.2, the radio) and says what answered: NetworkTables (5810), the
  Catalyst Agent (9010), SSH, the web; Enter on one that is a robot makes it
  the robot the shell links to. Ping pings once a second with the replies as
  bars (lost ones in red) and the loss and times; Ports checks a host's ports
  (by default those a robot uses; `p` changes the list, as `22 80
  5800-5810`); Name lookup turns a name into its addresses; the Wi-Fi survey
  lists what is in range by signal, in dBm and bars. They work in the
  simulator too, against the computer's own network.
- **Connect**: saved SSH hosts (`user@host`, `user@host:port`; `e` edits, `d`
  twice forgets), each opened in a terminal through Dropbear's client, which
  asks for a password or a new server's say-so there as `ssh` does. Key for
  servers makes the handheld's own key (ed25519, kept in `/data/ssh`) once
  and shows its public half to put in a server's `authorized_keys`. Below,
  the serial ports there are (USB adapters and boards, by their own names
  from `/dev/serial/by-id`), opened raw at the speed chosen (9600 to 921600
  baud) in the terminal. Ctrl-Q (or Esc held) leaves a session.
- **Actions**: shell commands kept under a name and run with Enter, each in
  a terminal of its own with its output left on screen. Some come ready:
  ping the robot, ask its agent how it is, disk space, memory, addresses,
  the busiest processes, the kernel's messages, the system log. `n` adds
  one, `e` edits, `d` twice removes; `{robot}` and `{team}` in a command are
  filled in when it runs. They are kept in `commands` beside the settings.

## Wi-Fi

Settings › Network › Wi-Fi networks switches Wi-Fi on, lists what is in range,
joins a network (the password is typed on the keyboard) and forgets one. It
drives the system's `wpa_supplicant`; networks are kept in
`/data/wifi/wpa_supplicant.conf`. While Wi-Fi is off the radio's driver is
not loaded at all, and switching it off unloads it. On a PC a scripted set of
networks stands in.

## Tailscale

Settings › Network › Tailscale puts the handheld on your tailnet: it reaches
your other machines from anywhere (SSH to a laptop from Connect, a
dashboard at home) and they reach it. It is off until switched on there,
and while it is off nothing of it runs.

- **Switching on** starts `tailscaled` (the `catalyst-tailscale` service)
  and asks it to connect. The first time, the page shows a QR code: scan it
  with a phone's camera and sign in to Tailscale there (or open the link
  under it on any device). The sign-in and the machine's key are kept in
  `/data/tailscale`, so it stays signed in across restarts and updates.
- **Signed in**, the page shows this handheld's address on the tailnet, the
  tailnet's name, its other machines (online ones with their addresses, the
  others with when they were last seen), how much memory `tailscaled` is
  using, and Sign out (pressed twice). Switching off keeps the sign-in.
- It changes nothing else: no firewall rules (`--netfilter-mode=off`) and no
  DNS (`--accept-dns=false`), and the handheld keeps its own name. Nothing
  goes to Tailscale's log service.

It is our own slim build (`os/package/catalyst-tailscale`): Tailscale
1.102.3, the newest Buildroot's Go 1.27 builds, with only what a client on
the kernel's TUN needs (the OS router, the CLI inside the daemon, the
notification bus, health, port mapping, raw-socket NAT traversal, baked-in
root certificates; Tailscale's own `featuretags --min`, as its
`build_dist.sh --extra-small` does). One static, stripped 13.3 MB program is
both `tailscaled` and `tailscale`. Buildroot's own `tailscale` package is not
used: it switches on nftables, NAT and connection tracking in the kernel,
which this client does without. The kernel's TUN is a module already, loaded
when Tailscale starts.

Memory: run under qemu on four cores, signed out and idle, it holds 5 to 8
MB of its own plus about 13 MB of its program's pages (read from the card,
shared with the `tailscale` command, given back under pressure), so about
20 MB; signed in with a handful of machines, an estimated 25 to 30 MB. The
page's Memory row reads the real figure on the handheld. On a PC, a made-up
tailnet stands in and signs in by itself a few seconds after its code shows.

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

The Git app starts with the GitHub account, then your repositories on
GitHub, then the ones under `/data/code`, and last a field that clones any
repository by its HTTPS address. Git runs in the background, so a slow card
never holds a frame up.

- **Account.** Signing in needs a GitHub token with access to your
  repositories (github.com › Settings › Developer settings; a fine-grained
  token with Contents read and write is enough). It is typed in Settings ›
  Keys, read from a file there, or sent from a computer on the "set up from
  a computer" page, like the other keys. Git › Account then asks GitHub
  whose it is (`/user`) and keeps the login, name and the address commits
  carry (GitHub's private `…@users.noreply.github.com` one when the email is
  hidden) in `/data/catalyst/github`. Sign out forgets that; the key itself
  stays in Keys until it is cleared there.
- **Your repositories** come from GitHub's `/user/repos` (yours, your
  collaborators' and your organisations', newest first), each with whether
  it is private and when it last changed. Enter clones one into
  `/data/code`, with git's own progress on screen, or opens it when it is
  already there; `r` asks GitHub again.
- **A repository's page**: pull (fast-forward only), push (a new branch gets
  one on `origin` to push to), commit everything with a message, the
  changes as a coloured diff, the history and the branches.
- **History** draws the branch graph beside the commits, each lane its own
  colour, merges joining back in, with the branch and tag names on their
  commits. Enter opens a commit: who, when, the message, the files it
  touched and its diff.
- **Branches**: the one checked out first, then the others, then the ones
  only on the remote. Enter switches, tracking a remote one; uncommitted
  changes come along.
- **Deploy as an Action**, for a repository with `gradlew`: saves
  `./gradlew deploy` in it as an Action (critical, so it asks before
  running), for the robot on the Wi-Fi.

The token never goes on a command line or into a repository's
`.git/config`. Calls to GitHub's API hand it to curl in a header file only
the shell can read, removed as soon as the call ends. Git reaches it
through a credential helper (`/usr/libexec/catalyst/git-credential`, set in
`/etc/gitconfig`) that answers only for `https://github.com`; any other
server, or plain HTTP, gets nothing, and git's requests to store or erase
are ignored. `scripts/test-git-credential.sh` checks that on a computer.
Public repositories anywhere clone and pull over HTTPS with no account.

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
firmware takes another 64 MB for the video processor, which no setting
returns: the camera's image processing runs in the full firmware and needs it
(with the cut-down firmware and 16 MB, which earlier images used, the camera
does not appear). What the system uses is unchanged; there is 48 MB less to
use, about 400 MB still free. The emulator has no panel, keyboard matrix, fuel gauge or radio, so
the board's own drivers are loaded but idle in these figures, and emulator
seconds are not handheld seconds (about six of the 24 are spent waiting for a
panel that is not there): the table compares one build with another.

Since then, HTTPS and git came in: curl, mbedTLS and the certificates added
about 2 MB (in 0.4.2 curl moved to the OpenSSL that Python already brings),
and git about 8 MB once its server, mail, Perl and SVN/CVS tools
(which need Perl, or carry their own 2.3 MB copy of git's library each) are
left out. The root filesystem then used 47 MB, and the download was 33.6 MB.
None of it is in memory until something runs it.

Then the GPU's driver: Mesa (EGL, OpenGL ES 2 and the gallium driver for the
VideoCore, 17 MB of it) and the C++ runtime its shader compiler needs add
about 20 MB. The root filesystem now uses 67 MB of its 96 MB, the download is
39.0 MB, and `catalyst-shell` is 4.1 MB (it was 3.3). Mesa is loaded only once
the shell finds the GPU's device, so under emulation, which has none, the
memory figures above are unchanged by it.

With A/B updates the card image was 513 MB: the boot partition (128 MB, room
for two kernels), two 160 MB systems (68 MB used of each), and /data. System
B is empty in the image, so the download stayed small (40.8 MB, gzip), and
an update from one version to the next is a patch of a few megabytes. From
0.4.0 the systems are 1 GB each and the image 2.2 GB. Most of that is empty,
so the file holds 191 MB of real data, and the download is about what the
system holds.

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

## The AprilTag reader

The camera's picture (the IMX219, through libcamera) with every 36h11 tag in
it outlined and numbered, and beside it each tag, nearest first: how far away
it is, how far to the left or right, and how far it is turned (its yaw).
Space holds the picture. It checks what the robot's vision should be seeing.

- The picture is 640 × 480 at up to 30 frames a second (down to 15 when the
  light is low and the exposure long) from the sensor's full-view binned
  mode, so the lens is known: nominally 530 pixels of focal length,
  or a calibration in `apriltag/lens.json` beside the settings
  (`{"fx": …, "fy": …, "cx": …, "cy": …}`, as PhotonVision's calibration gives
  them). Tags are taken to be FRC's, 6.5 inches across their black square.
- **Which way up.** On the CardputerZero the IMX219 sits the other way round
  from what the kernel's `imx219` overlay declares (the Raspberry Pi camera
  board's 180°, which libcamera undoes), so left as it is the picture is
  upside down, and a tag to the right is reported to the left. The camera is
  asked for half a turn (`--orientation 180`): the sensor reads out flipped,
  at no cost, or `catalyst-camera` turns each frame as it copies it where a
  pipeline cannot. M5Stack's own camera app for the CardputerZero asks for the
  same, and `config.txt` keeps M5Stack's plain `dtoverlay=imx219`, so apps
  written for their image see what they expect.
- `catalyst-camera` (`os/package/catalyst-camera`), a small libcamera program
  of the image's own, streams the brightness of each frame to the reader,
  only while it is open; rpicam-apps needs kernel headers newer than this
  toolchain's, and Boost. Each frame is copied out as it arrives and the
  camera's buffer goes straight back; the newest copy is what is sent when
  the reader is ready, so a busy reader skips frames rather than falling
  behind.
- The picture and the reading go at their own paces. Every picture is shown
  as it comes; the detector (AprilRobotics', as PhotonVision uses, on two of
  the four cores, looking at the picture halved) runs on a thread of its own,
  a step lower in priority, and reads whichever picture is newest when it is
  free. When the two went in step, the picture moved only as fast as tags
  could be read, which on the CM0 is slower than the camera.
- libcamera's Raspberry Pi pipeline needs the firmware's image processor (the
  ISP) as well as the sensor. The ISP is reached through `vchiq`, which the
  image does not load at start, so the reader loads `vchiq`, `vc-sm-cma` and
  `bcm2835-isp` the first time it opens. Without them the sensor is found
  (the hardware check's tick) but libcamera sees no camera.
- In the simulator a simulated camera shows tags 7 and 13 in a grey room,
  tag 7 drifting and turning, drawn as the nominal lens would see them and
  read by the same detector; its distance and turn come back within a few
  centimetres and degrees of where it was drawn (tested).

Not yet checked on a CardputerZero: the picture the right way up after the
half turn, the frame rate the CM0 reaches at 30 a second, how often it reads
tags, and the lens figures.

## Camera and Photos

The Camera is the IMX219 as a camera: its picture in colour, photos kept in
an album, and QR codes read. The AprilTag reader is still there beside it.

- **The viewfinder** fills the panel's height at the photo's own shape (4:3,
  227 × 170), with a rail of controls beside it: photo or scan (← → or Tab),
  the shutter, and the newest photo (↓ opens the album on it). Enter or Space
  takes a photo: the shutter presses in, the picture flashes, the photo is
  saved and its thumbnail drops into the corner. `t` sets a timer (3 s, then
  10 s, then off), counted down over the picture; Esc stops it. `g` lays a
  rule-of-thirds grid over the picture. `n` is for low light: the exposure may
  run to a fifth of a second, so the preview slows down too.
- **Photos are full size.** The preview is 320 × 240 YUV 4:2:0 at up to 30
  frames a second from the sensor's binned full-view mode. A photo stops the
  preview and asks the sensor for all 3280 × 2464 pixels. `catalyst-camera`
  waits a few frames so the exposure settles, then writes the JPEG
  (libjpeg-turbo, quality 92) straight from the camera's buffer, with no RGB
  copy in between. The preview carries on as soon as the photo is written.
  The camera's memory (CMA) is 32 MB, so where a full-size frame cannot be had
  it takes the binned 1640 × 1232 instead, and the note says which size it
  saved. Photos are written to a temporary file and renamed into place, so a
  photo is never half there.
- **Where they go:** `/data/photos`, named for when they were taken
  (`IMG_20261121_140312.jpg`, with `_2` added if two land in the same
  second). Files opens a photo in the viewer too.
- **Scan mode** reads QR codes from the preview's brightness, about four
  times a second, on a thread of its own (rqrr, pure Rust), so the preview
  never waits for it. A code read is outlined and a card rises with what it
  holds:
  - a Wi-Fi code (`WIFI:T:WPA;S:…;P:…;;`, escapes and all) offers to join
    that network, through the same Wi-Fi page as Settings, which says when it
    has joined;
  - a web address opens in the Web app (lynx). Only `http` and `https` are
    opened, and the address is passed quoted, so a code cannot run a command;
  - anything else is shown as text, on a page of its own.
- **Photos** (the album) is a grid of thumbnails, newest first, four across.
  The thumbnails are made on a thread and kept in `/data/photos/.thumbs`, so
  each is decoded only once. Enter opens a photo on the whole panel:
  - ← → go to the previous or next photo;
  - `+` (or Enter) zooms to 2× and then 4×, and `-` zooms back out. Zoomed,
    the arrows pan, and Esc goes back to the whole photo first;
  - `i` shows the details: name, size in pixels and megapixels, the file's
    size, and when it was taken;
  - `d` asks, and `d` again deletes it (its thumbnail goes too).
- **Memory.** A photo is decoded at an eighth, a quarter or half of its size,
  whichever is the smallest that still covers the panel at the zoom asked
  for. Even at 4× a full-size photo is decoded at half its size (6 MB), never
  its whole 24 MB. Only one photo is decoded at a time, even when you turn
  through them quickly; until it is ready, its thumbnail stands in.
- **In the simulator** a simulated camera shows a field at the end of an
  afternoon, drifting a little as a hand would, with a Wi-Fi code held up in
  front of it in scan mode. Its photos are written by a JPEG encoder in the
  shell (`jpeg-encoder`, built only for computers), and the album starts with
  six of them. The QR reader reads the simulated code at the preview's size
  (tested).

Not yet checked on a CardputerZero: the colours and which way up the colour
preview is, whether a full-size photo fits in the camera's 32 MB or falls back
to 1640 × 1232, the preview's frame rate and how long a photo takes to write,
and how far away a QR code can be read at 320 × 240.

## Remote: a universal remote

The CardputerZero has an infrared LED (GPIO12) and an infrared receiver
(GPIO13). The Remote app learns any remote's buttons and sends them back:

- **Remotes** are kept by name ("TV", "Projector"), each with named buttons:
  `n` names a new one, `e` renames, `d` twice removes.
- **Learning**: `l` (or the last row) listens; point the remote at the
  handheld and press its button. What was heard is drawn as it arrives, and
  named if its protocol is a common one: NEC (and NEC with a 16-bit address),
  Samsung, Sony SIRC (12, 15 and 20 bits), Philips RC5 and RC6, e.g.
  `NEC · 0x04 · 0x08`. Then it is given a name and kept.
- **Sending**: Enter sends a button; held down, it keeps sending it, as the
  remote would (a NEC remote's repeat code, RC5's and RC6's toggle bit kept;
  a new press flips the toggle). A recognised button is sent cleanly at its
  protocol's own timing and carrier (38 kHz NEC and Samsung, 40 kHz Sony,
  36 kHz RC5 and RC6; Sony three times over, as its receivers expect). One
  in a protocol it does not know (an air conditioner's) is sent back exactly
  as it was heard, at 38 kHz.

Remotes are kept in `remotes.json` beside the settings. Underneath, the
shell talks to the kernel's LIRC devices directly (`ir.rs`): the
transmitter is the kernel's `gpio-ir-tx` and the receiver `gpio-ir-recv`,
from M5Stack's own overlay lines in `config.txt` (the receiver with the
empty key map, so a remote in the room is never taken for a keyboard).
Neither driver loads at start: the transmitter is loaded while the app is
open and the receiver only while a button is being learned, since every
edge of infrared it sees is an interrupt; both are unloaded after. In the
simulator there is no infrared: it has two remotes to try, sends are only
timed, and learning hears a simulated TV remote.

Not yet checked on a CardputerZero: the LED's range, and the receiver's
timing (the decoders allow for a receiver lengthening light by a hundred
microseconds or so, tested).

## Level, Compass and the motion sensor

The CardputerZero has a Bosch BMI270 (accelerometer and gyroscope) at 0x68 on
the board's I2C bus and, on the C154, a BMM150 magnetometer behind it on the
BMI270's own auxiliary bus (the C154-V1 has none; the Lite has no motion
sensor at all). What answers is what is offered: the **Level** needs the
BMI270, the **Compass** the BMM150 too; the apps, their search entries and
their Settings rows are left out where the chip is not there, and Home lays
itself out again once the sensor is found (what the last start found is kept,
`motion-sensor` beside the settings, so the tiles are there from the start).

- **Level**: pitch and roll to a tenth of a degree, and a bubble that drifts
  to the high side, turning ice when level (within 0.2°). Stood on an edge it
  reads that edge's angle to level instead (a line across the page stays
  level with the world; "right end low"), as a spirit level laid along a
  frame or an arm does. **Space** zeroes it against whatever it rests on (and
  clears the zero), **Enter** holds the reading.
- **Compass**: the top edge's magnetic heading, held level against the way up
  so it stays true as the handheld tips (and face down). The metal around a
  magnetometer pulls it off true: **Enter** calibrates by turning it round
  once, flat (the rim fills as the gyroscope sees each eighth of the turn);
  the offset it finds is taken away from every reading after (kept in
  `compass.json` beside the settings). It says when a magnet is near
  (over 100 µT).
- **Lift to wake** and **face down to sleep** (Settings › Display, both on):
  while the screen is off, picking the handheld up turns it on for a look
  (off again after ten seconds unless a key follows); while it is on, laid
  face down for two seconds it goes out, whatever is showing, until a key or
  a lift. A lift only counts from lying still, so carried about in a pocket
  or a bag it never wakes itself; a knock on the bench shakes it without
  turning it.

The shell drives the chip itself through `/dev/i2c-1` (`i2c-dev`, loaded first
thing in boot), not through a kernel driver, because standby wants the chip's
low-power mode: watching for a lift, the accelerometer alone runs at 12.5
samples a second in its least-power setting, read four times a second; with
nothing wanting it everything on the chip is off; the Level and Compass run
it at full rate only while open. The BMI270 needs Bosch's 8 KB program loaded
at every power-up (`userland/assets/bmi270`, BSD-3-Clause, the same bytes
M5Stack's image loads), sent in short pieces so the keyboard on the same bus
is never kept waiting. The chips sit turned on the board; the turn is
M5Stack's own (from their Compass app). In the simulator a simulated sensor
lies on a bench, nudged now and then, with a magnetometer whose own offset a
calibration takes away (tested); it never lifts or lies face down.

Not yet checked on a CardputerZero: the sensor answering over the real bus
(its start-up, the magnetometer behind it), the axes as M5Stack gives them
(a heading that runs the wrong way, or a pitch with its sign turned, would
show it), how a lift and face down feel in the hand, and the calibration on
the real board's magnetic field.

## Weather

The Weather app shows the weather where the handheld is: **now** (the
temperature large, what it is like in a few words and an icon for day or
night, what it feels like, the wind, the humidity, and where), the next 24
**hours** along a strip, and the **week** as rows (each day's weather, its
chance of rain, and its low and high on a bar against the whole week's,
today's with a dot at now). Up and Down (F and X) go between the three, and
now folds away under the title as the week comes up; Left and Right (Z and C)
go along the hours. Enter on a day, or on an hour, opens that day: its
temperature hour by hour over its daylight, the chance of rain beneath, its
warmest and coolest hours, and now on today's; Left and Right go through the
week from there. Its Home tile says how warm it is and what it is like in a
word ("27° sunny").

**No key, on purpose.** The forecasts come from
[Open-Meteo](https://open-meteo.com), which serves the national weather
services' models free for non-commercial use with no key and no account.
So there is nothing to sign up for and nothing to type into Settings › Keys,
and no key is built into the firmware, where anyone could lift it from an
image and where one revoked or over its quota would stop every handheld at
once. What goes out is the place's latitude and longitude (and, when a place
is searched for, its name), nothing about the handheld. Open-Meteo's data is
CC BY 4.0; the Place page credits it.

It is fetched with `curl` over HTTPS on a thread of its own: when the app
opens on a forecast older than half an hour, every half hour while it stays
open, and on **r**. The last forecast is kept (`weather.json` beside the
settings), so the app opens on it at once, and offline it still shows, its
bottom line saying so ("offline · as of 13:15"; a failed fetch is tried again
after five minutes). An hour and a half on, the forecast's own reading of now
gives way to that hour's forecast; a kept forecast that has run out (after a
week) says so rather than show old weather.

**Where** is the time zone's city (Settings › Name & time), looked up once
by name with Open-Meteo's place search (the one in that zone, where several
places share the name) and kept in `weather-place.json`; a new time zone
looks again. **l** (or Enter on now) chooses another place: type a city,
Enter searches, Enter on a result chooses it; the first row goes back to the
time zone's city. With the zone UTC and nothing chosen, the app asks for a
place. **Units** are °F and mph in the United States' zones and °C and km/h
everywhere else; **u** switches them, and the choice is kept.

In the simulator the forecast is one captured from Open-Meteo (Sydney on a
November afternoon, `weather-demo.json`, which the tests read too) and the
place search knows a few cities, so nothing needs the network;
`CATALYST_WEATHER_LIVE=1` asks Open-Meteo instead. The fetch, the search and
the kept file are tested against the live services from a PC (an ignored
test, `live_open_meteo`); not yet from a CardputerZero over its own Wi-Fi.

## Assistant

The Assistant app answers questions about the robot, typed on the
keyboard, with the team's own key for **OpenAI** (GPT-6.2 Sol) or **Claude**
(Claude Opus 5.5): Settings › Keys, the handheld's setup, or the page for
setting up from a computer. A third service, **on this handheld**, is the
on-device model once it is downloaded in Settings; until then Tab › Answers
with says where to get it. Tab › Answers with goes round the services that
can answer; a change of service starts a new conversation. **Effort** is
light (`low`) or medium (`medium`, the default).

**Lightweight on purpose.** With OpenAI or Claude nothing of the model runs
on the handheld: each turn is one HTTPS request made with `curl`, as the
schedule and Weather are, on a thread of its own. The key reaches curl
through a file only root can read (`-H @file`, removed straight after),
never on its command line where any process could read it. Claude is used
through its API with a key from the Claude Console, not a Claude Code
sign-in token, which Anthropic allows only in its own apps.

The on-device model is asked the same way, over plain HTTP to its server on
the handheld itself (`127.0.0.1` only; curl is held to `http` there), in
OpenAI's chat-completions shape (`POST /chat/completions`, at most 400
tokens back). It answers from what it knows, with no tools and no web, and
its server is started when it is first asked and stopped when the service
is changed away from it.

- OpenAI: the Responses API, `store: false`, so nothing is kept on OpenAI's
  side between turns; every turn sends the conversation back, reasoning
  items (encrypted) included, so the model keeps its train of thought across
  tool calls.
- Claude: the Messages API (version `2026-10-21`). Thinking blocks go back
  unchanged; the conversation is cached on Anthropic's side
  (`cache_control`), and a request declined on safety grounds is answered by
  another Claude model on the server (`fallbacks: "default"`).

**The web.** It can search the web and read pages: Claude with its server
tools `web_search` and `web_fetch` (`web_search_20260209`,
`web_fetch_20260209`, at most five of each a turn, a fetched page held to
20,000 tokens), OpenAI with its hosted `web_search`. Each shows as a step,
*searched the web: …* or *read thebluealliance.com*, failed ones dimmed with
why. The search results go back to the service exactly as they came
(encrypted content and all), and a turn the server paused (`pause_turn`) is
carried on by itself, up to five times. Under the answer a **sources** line
names the pages it rests on (the ones cited, then the ones read); Enter on
it, with nothing typed, lists them, and Enter on one opens it in the Web app
(lynx).

**Its tools:** what the robot publishes (Catalyst's telemetry by group, and
the health checks firing), the Catalyst Agent's report from the Systemcore,
the match logs pulled to the handheld, the event's schedule (the team's next
match, and the rest), the live graphs' latest values, the handheld itself
(battery, Wi-Fi, storage, memory, temperature, uptime, version), a text file
under `/data` (at most 64 KB, a folder listed; never keys, Wi-Fi, SSH,
Tailscale, update state, kept conversations or anything named like a secret
or token), and the Actions. It can **run
Actions** by itself, except a **critical** one, which could change something:
its card waits in the conversation, Enter opens a page showing it in full
(what, why, the command exactly as it will run) that starts on *Don't run
it*, and Esc on the card skips it. A no goes back to the model as a no.
Every command it runs is kept as a file in `/data/assistant` (what, when,
why, which model, whether you said yes, and what it printed; the newest 200
stay), where Files opens.

**Conversations are kept**, the newest 20, in `/data/assistant/chats`: each
as the service's own items (so it carries on exactly where it was) with what
the page showed, readable only by root. Tab › Conversations lists them,
newest first; Enter carries one on with the service it was with, **d** twice
forgets it. Tab › New conversation starts afresh.

Up and Down scroll the conversation; Esc clears the line, stops an answer
under way, or leaves; Ctrl N starts afresh. Answers are kept short and plain
for the panel (no Markdown); its text is set at the caption's size so more
fits. At most twelve rounds of tools go into one question.

In the simulator a script answers in each service's own shape (so the real
parsing runs) with the demo robot, the web included: a question about the
rules, the manual or autonomous is searched for, with a source.
`CATALYST_AI_LIVE=1` asks the real services, and the ignored test
`live_services_answer` does so with keys from `OPENAI_API_KEY` and
`ANTHROPIC_API_KEY`. Not yet tried against the live services from this
repository's tests or from a CardputerZero: the web tools' real replies,
whether the Claude Console organization has web search turned on (web fetch
needs nothing), OpenAI taking its `web_search_call` items back with
`store: false`, and the on-device model's server are all still to be seen.

## Browse

Browse (Home, just before the Assistant) is the web read for you, with the
Assistant's key. Type an **address** and the service reads the page and gives
it back as clean text: its title, its sections, and the links worth
following, along the bottom. Type a **question** and it searches and
answers, with the pages it used as the links. An empty page offers five
places to start (The Blue Alliance, the WPILib documentation, Chief Delphi,
the FRC Q&A and FIRST's news).

Keyboard first: typing goes to the address bar, Enter goes; Up and Down
scroll the page with the same glide as every list; Left and Right choose a
link, Enter follows it; Backspace or Esc goes back a page (the last twelve
are kept), and Tab types afresh. Each page is a conversation of its own, so
following links does not pile up what each page cost, and the address goes
in the question itself, which is what lets Claude's `web_fetch` read it.
The on-device model has no web, so Browse needs an OpenAI or Claude key.

## On-device model

A small language model that runs on the handheld itself, with no network
and no key: for the Assistant app when it is chosen there, and for `ai` in
the Terminal. Settings › On-device model chooses one, downloads it, tries it
with one question, and removes it.

| Model | File | In use while answering | Like |
|---|---|---|---|
| SmolLM2 135M (default) | 145 MB | about 190 MB | light; simple answers |
| SmolLM2 360M | 271 MB | about 305 MB | better; slower |

Both are Hugging Face's SmolLM2 Instruct (Apache-2.0) as GGUF files
quantised by bartowski (135M at 8 bits, 360M at 4), each pinned in
`userland/crates/localmodel` by its size and SHA-256. Neither is in the image:
one is downloaded when asked for, over HTTPS with curl, to `/data/models`,
carrying on after a dropped connection, and kept only if its sum checks out.
Be honest about them: they are quick and offline, fine for short answers and
wording, often wrong on facts, and they have none of the assistant's tools
(they cannot see the robot or run Actions).

It runs as llama.cpp's `llama-server` (Buildroot's `llama-cpp`, b8117), only
while something asks: on 127.0.0.1:8088, one conversation, three threads
(the fourth core stays the shell's), at a lower priority than the shell, and
stopped after three minutes unasked, so it costs nothing at rest. Memory is
what limits it on 512 MB, so it runs with an 8-bit attention cache (flash
attention), batches of 128 and the model mapped from the card; measured
under emulation, llama's own buffers come to 185 MB for the 135M (model 136,
cache 24, working space 25) and 302 MB for the 360M, and a model is not
started unless the kernel says that much and 40 MB more is free (the 360M
then asks for other apps to be closed). Every other llama.cpp tool is taken
out of the image (41 MB).

**Built for this processor.** ggml chooses its kernels when it is compiled,
so `os/package/llama-cpp-catalyst` pins it: no native guessing,
`-march=armv8-a+crc`, one CPU build, no OpenMP runtime, no downloads of its
own. The CM0's A53 has no dot product, int8 matrix, fp16 arithmetic, crypto
or LSE instructions (the emulator's does, so a wrong build would pass there);
a scan of llama-server and its libraries finds none of the first four, and
the few LSE atomics are GCC's outline helpers, each behind its run-time
check.

## ai in the Terminal

`ai` is the assistant for a shell, using the same keys and choices as the
Assistant app (Settings › Keys; Tab › Effort there), or the on-device model
with `--local` (and when no key is set):

```
ai what does a brownout do to the robot
ai cmd find the ten biggest files in /data
dmesg | tail -n 30 | ai explain
```

- `ai <question>` answers in plain text, wrapped to the terminal (53 columns
  on the handheld).
- `ai cmd <what you want>` proposes one BusyBox command and a line on what it
  does, and runs it only after a `y`; one that could delete or change things
  (rm, dd, mkfs, a redirect into a file, kill, reboot, git push, `sed -i`,
  `find -delete`…) needs `yes` typed out. The answer is read from the
  terminal itself, so piping into `ai` cannot answer for you. It needs a key:
  held to a strict answer shape, the on-device model still made commands up
  (`print_current_date`), so it is not asked for them.
- `… | ai explain [question]` explains output or an error (its last 12,000
  characters).

Keys reach curl through a file only root can read, removed straight after,
never on its command line; requests are the Assistant's (OpenAI's Responses
API with `store: false`, Claude's Messages API with server-side fallback).
`--cloud` uses a key even with the model downloaded.

## Actions

Actions keeps shell commands under a name, run with one key in a terminal of
their own. The team's own come first, then **26 built-in** ones for the robot
(ping, its agent, the logs on it, a trace, its name), the network (Wi-Fi link
and networks, Ethernet, addresses, routes, name servers, the internet) and
the handheld (disk, memory, processes, load, clock, temperature, speed,
services, USB and I2C devices, the kernel's messages and the system log),
plus restarting Wi-Fi or the handheld. Built-in ones stay as they are; **e**
on one keeps a copy to change. `{robot}` and `{team}` are filled in when one
runs.

Whether the assistant must ask before running one is worked out from the
command: anything not known to only look (an unknown program or script,
ssh, git, gradle, a redirect into a file, curl sending data) is critical.
Each command's editor can say otherwise (auto, asks first, runs it); that
choice is kept in `commands-ask`, apart from the list, so an older shell
still reads the list after an update is rolled back.

## USB to a computer

Settings › USB chooses what the handheld is to a computer its USB-C port is
plugged into, kept across restarts (`catalyst-usb`, the kernel's USB gadget
framework on the dwc2 controller):

- **Charging only**, the default.
- **Serial console**: a serial port with a login on it (`/dev/ttyGS0` on the
  handheld; `screen`, PuTTY or `picocom` on the computer). The login is root's,
  which has no password, as on the Grove port's serial console.
- **Network**: a network adapter (CDC NCM, which Windows 10 and 11, macOS and
  Linux all drive themselves). The handheld is 192.168.7.1 and gives the
  computer 192.168.7.2, with no route or DNS, so the computer's own internet
  stays as it was; SSH and the setup page are reachable there.
- **Drive of logs**: a read-only drive holding the match logs as they were
  when it was made; choosing it again brings it up to date.

The page says whether a computer has it. The CardputerZero's USB switch
chooses where the CM0's one USB port goes, according to M5Stack's guide. To
the left, it goes straight to USB-C2, for these modes. To the right, it goes
to the internal hub, which carries the USB-A port, USB-C1 and the Ethernet
chip, so the robot's cable needs the switch to the right. Not yet checked on
a CardputerZero: whether its USB-C port takes the device role when plugged
into a computer.

## Command-line tools

From the Terminal (and over SSH): Python 3 (with zlib, ssl, readline,
curses, sqlite3, xml and the xz and bzip2 archives), vim (with its syntax
files) and nano, htop, tmux, rsync, jq, strace, i2c-tools, picocom for serial
ports, and Dropbear's SSH client (`dbclient`, with `dropbearkey`), beside
BusyBox's own small tools and git. Nothing of them runs until called;
together they take about 50 MB. Since 0.4.0 each system partition is 1 GB
(it was 160 MB, 18 MB of it spare): about 830 MB is free for more.

## Updates, over the air

`catalyst-update` updates the handheld the way Android does, with two
systems on the card:

| Partition | |
| --- | --- |
| 1 boot (FAT, 128 MB) | the firmware, `config.txt`, and each system's kernel, device trees and overlays in its own folder (`slot-a/`, `slot-b/`) |
| 2 system A, 3 system B (ext4, 1 GB each) | one runs (read-only); the other takes the next update |
| 4 data (ext4) | settings, logs, keys, downloads; grows to fill the card (4 KB blocks, an inode per 64 KB, room in its tables to grow online to 256 GB) |

A card flashed before 0.4.0 has 160 MB system partitions, too small for
0.4.0's systems: its updater says so before downloading anything, and the
card is flashed afresh.

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

**Update logs.** What changed is written once, in `os/CHANGELOG.md`, and
read in three places:

- **Before installing.** Settings › System update › *What's new in 0.4.1*
  (with how many changes) opens the update's own notes, carried in its
  signed manifest: the version large, then its changes on a card for each
  heading. F and X scroll, Pg Up and Pg Dn a page at a time; Enter downloads
  and installs it (or restarts into it, once it is written).
- **Once, after it takes.** When an update has been kept (the new system's
  shell came up), its notes slide in over Home, once, the next time Home is
  at rest. Never on a first start, a card flashed afresh, or an update that
  did not take. (`whats-new=` in `/data/update/prefs` remembers which
  version's were shown.)
- **Any time.** About › Update history lists every version, newest first,
  from the copy of the changelog the image carries
  (`/usr/share/catalyst/changelog.md`), so it works offline; Z and C turn
  from one version's notes to the next.

To write a release's notes, add a section at the top of `os/CHANGELOG.md`
before running `scripts/release.sh`:

```markdown
## 0.4.2 · 28 Nov 2026

### New
- One line for each change, in plain words.

### Fixed
- What works now that did not.
```

`## <version>` starts a section (the date after `·` is optional); `### `
headings group its lines; each change is a `- ` line, wrapped on the panel,
so keep it to a sentence or two. The script puts the section for the version
it makes (a nightly takes its base version's) into the manifest and onto the
release's GitHub page, and stops if there is none; `--notes` replaces it.

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
sh scripts/test-catalyst-time.sh     # the clock set from the network, with stand-ins for ntpd and hwclock
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
(its writing and reading back are tested on a file), the keyboard (Sym's
symbols and Fn's keys are mapped from the driver's keymap, not yet pressed),
the clock set from the network and kept in the clock chip, Ethernet, Wi-Fi, the fuel gauge,
idle memory with the board's drivers at work, what the power savings measure
in milliamps, the panel waking from its sleep, the keyboard's lights borrowed,
the infrared LED and receiver, the motion sensor and compass and their axes),
the NT4 client against a real
Systemcore, the log reader against a log from a real robot, the schedule
against the live Blue Alliance and Nexus services, setup from a computer over
a real network, and the catalogue against its repository (which does not
exist yet; its install path is tested with a stand-in download). Not yet
built: the agent's machine pages (CPU, CAN buses, cameras), audio. (The
camera and the assistant, which reads the match logs, came later.)

## Roadmap

Chosen so far, not yet built: scripting with a kit for writing Bezel apps (so
an external app can draw like a built-in one).

Added since, also not yet built:

- For the robot: alerts (rules that watch the robot and flag it on the
  island with a tone). The link stays read-only: a tuner that writes to the
  robot was offered and not chosen.
- For the pit: QR codes for a match summary or an address.
- For development: an HTTP tester; a hex viewer and programmer's calculator;
  a serial plotter.
- For the system: sounds on the speaker, with a mute switch in the control
  centre; a lock screen with a PIN.

Built since: the post-match report, the code version check, the event
schedule, search, the app catalogue, git, the glance page's cards, first-boot
setup (on the handheld or from a computer) and the start animation; then the
themes and looks, Home's arrangement, clock, island and greeting, the
personal start, the new motion, and the GPU; then the recent apps (an app
switcher), screenshots, standby and power saving, the system monitor, the
network toolkit with the robot finder, the SSH client and serial console,
saved commands, the AprilTag reader, USB to a computer, the command-line
tools, live graphs, the pit checklist and match timer, and match trends; then
(0.4.0) deep standby, the battery saver, low-battery care and the Battery page,
the lower-voltage option, the keyboard's lights for notices and warnings, 1 GB
system slots, the universal remote, the Level, the Compass and lift to wake,
the keyboard lesson, the handheld's name and time zone, and the hardware check.

## Third-party pieces

Google Sans Flex and Google Sans Code (SIL OFL 1.1) and Material Symbols
(Apache 2.0) are vendored under `userland/assets` with their licences; the
fonts built into the shell are subsets of them. The board drivers come from
`m5stack/m5stack-linux-dtoverlays` (GPL-2.0) at build time. The BMI270's
program is Bosch's (`bmi270_config_file` from the BMI270 Sensor API,
BSD-3-Clause), vendored under `userland/assets/bmi270` with its licence.
Weather's forecasts and place names come from Open-Meteo at run time (CC BY
4.0), as does the simulator's captured forecast. The Camera's photos are
written by libjpeg-turbo (IJG, BSD-3-Clause and zlib licences) in
`catalyst-camera`. In the shell, QR codes are read by rqrr ((MIT or Apache
2.0) and ISC) and photos are decoded by jpeg-decoder (MIT or Apache 2.0). The
simulator alone writes its photos with jpeg-encoder ((MIT or Apache 2.0) and
IJG).

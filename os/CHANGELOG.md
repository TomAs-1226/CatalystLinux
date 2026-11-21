# What's new in Catalyst Linux

Each release's notes, newest first. `scripts/release.sh` puts the section
for the version it makes into the release (its manifest and its GitHub
page). The handheld shows it before an update installs and once after it
restarts into one, and the image carries this whole file for
Settings › About › Update history.

A section starts `## <version>`, optionally `· <date>`. `### ` groups its
lines; each line is a `- ` bullet, short and in plain words. Anything before
the first section (this introduction) is not shown.

## 0.5.4

### Spotlight
- Search is now Spotlight: press Ctrl-Enter on any screen, even in the Terminal, and a glass search bar grows out of the island. Space and / on Home open it too.
- Results appear from the first key: apps, any setting, Actions, notes, reminders, files, photos, repositories, SSH hosts, robot values, match logs and the schedule. Type a match or team number to jump to it.
- Type a sum ("12*7") or a conversion ("3 in to mm") and the answer shows at once; the last rows search the web or ask the Assistant.
- Enter opens a result where you are, and Esc brings you back; Tab shows a preview. What you pick often comes first.
- Actions found in Spotlight still ask first, showing the exact command.

### Reminders
- Reminders: a new app with Today, Upcoming and Done. Type it the way you'd say it: "swap battery in 10 min", "tomorrow 8am check bumpers", "before Q42 charge spare".
- Pit reminders follow the event schedule: before a match, at queueing, after each match, or between matches. When the event runs late, they move with it.
- The pit routine (p) puts your usual after-match jobs on the list in one go.
- Snooze with s (press again for longer), repeat with r, switch pit or personal with l.
- In Notes, press r to be reminded of a note. A bell shows which notes have one, and the reminder opens the note.
- A reminder that comes due wakes the screen, shows in the island and blinks the keyboard lights, in deep standby too.
- The time-to-queue alert now arrives in deep standby too, and wakes the screen.

## 0.5.3

### Fixed
- Opening and closing apps quickly no longer flashes an old screen, a blank frame or the wrong app: a window keeps moving from where it was, even when the next key comes first.
- Apps that draw for themselves, like Paint, start every time they're opened, and close from their own last picture.
- Recent apps (Ctrl Tab) opened during a zoom shows the app as it is, not jumped to full size.

## 0.5.2

### Battery
- Deep standby: after 30 minutes with no key, Wi-Fi, USB, the 5 V outputs and every light go off, and the handheld stops checking for anything. Any key wakes it, and the island says how long it slept and what it drew.
- Settings › Battery › Deep standby: after 10, 30 or 60 minutes, or never, with what it does and what it draws. Last sleep shows the last one's length and average draw.
- After 12 hours in deep standby, or at 10%, on battery, the red light blinks once a minute to say the handheld is still on.
- Deep standby waits while a robot is linked, a computer is connected, someone is signed in over SSH, an update is running or a timer is counting.

### Fixed
- The red light set to stay dark still lights up if the system crashes.

## 0.5.1

### Fixed
- Updates start the new system: the handheld's firmware reads tryboot.txt for a try, which is now written. Before, an update was downloaded and written, but the try came straight back to the old system.

## 0.5.0

### Camera and Photos
- Camera: take photos in colour; they are saved to Photos. A 3 or 10 second timer, a grid, and a night setting for low light.
- Camera's scan mode reads QR codes: a Wi-Fi code offers to join the network, a web address opens in Web, anything else is shown as text.
- Photos: everything the camera takes, newest first. Zoom in up to 4×, see a photo's details, or delete it. Files opens photos there too.

### Assistant
- The Assistant searches and reads the web, shows each step, and lists its sources, which open in Web.
- Browse: type an address and get the page back as clean text with links to follow, or ask a question and get an answer with sources.
- It can look at the event schedule, the live graphs, text files under /data and the handheld's own status.
- Conversations are kept (the newest 20) and can be reopened from Tab › Chats.
- On-device model: SmolLM2 answers on the handheld with no network and no key. Settings › On-device model downloads, tries and removes it.
- ai in the Terminal: ask a question, have a command proposed and run only on your yes, or pipe output in with `| ai explain`.

### Git and Tailscale
- Git signs in to GitHub with a token: your repositories, cloning with progress, the history with its branch graph, a commit's diff, branches, pull, commit and push.
- Tailscale in Settings › Network, off until you switch it on. Sign in by scanning a QR code, then see your tailnet's machines.

### Tools
- Calculator, with a programmer mode: 8 to 64 bits in two's complement, hex, decimal, octal and every bit at once.
- Clock: a stopwatch with laps, timers that keep running and announce themselves, and a world clock.
- Notes: quick pit notes, stamped with the coming match, pinned and searched.
- Units: every unit at once, for what robots are measured in (in·lb, N·m, rpm, psi, mAh…).
- HTTP: try a request to the robot or anywhere; status, timing, headers and readable JSON.
- Share: this Wi-Fi, a link, text, SSH or the team page as a QR code for a phone.
- Monitor: open a process to see its details, end it, or change its priority.
- Files: h shows any file as hex, with go-to and find.

## 0.4.2

### Fixed
- Installing an update no longer stops with "illegal instruction". The programs had been built to use crypto instructions this processor does not have; connecting to a robot could have hit the same.
- HTTPS works: updates, Weather, the event schedule, the assistant, the app catalogue and git all reach their servers. curl had been built without a list of certificates to trust, so every secure connection failed.
- The clock is set from the web when a network blocks the time servers, and never starts earlier than the system was built.
- Settings › Name & time › Clock: Enter asks the network for the time now.
- When an update check cannot be made, Settings › Updates says why.

## 0.4.1

### New
- Assistant: ask about the robot, answered by GPT-6.2 Sol or Claude Opus 5.5 with your own key. It reads what the robot publishes, its agent and the match logs, and runs Actions.
- Before the assistant runs anything that could change something, a page shows it in full and asks you to confirm.
- Everything the assistant runs is kept in /data/assistant, with its output.
- Actions has 26 built-in commands for the robot, the network and the handheld.
- Web: pages as text with lynx, from a start page of searches and FRC sites. links is in the Terminal too.
- Weather: now, the next 24 hours and the week, from Open-Meteo. No key needed.
- Update logs: what's new before an update installs, once after it does, and every version's notes in About › Update history.
- OpenAI and Claude keys in Settings › Keys, on the handheld's setup and on the page for setting up from a computer.

### Better
- The selector glides with a spring and stretches as it moves; pages settle with a little give.
- Esc on Home goes back to its first page.
- The camera turns its picture the right way up and runs at up to 30 frames a second, with tags read alongside.

### Fixed
- Setup from a computer: sending the form works, and the handheld shows each item arriving.
- Setup from a computer goes on to the keyboard lesson and the handheld's own pages.
- Graphs has its icon again.
- The tear-free panel (a developer option) truly waits for the panel's refresh now, and runs at 60 Hz again.
- Lower voltage restarts to try it, says what the processor was given, and backs off by itself if it hangs.
- Display › Graphics says what draws, and why.

## 0.4.0 · 20 Nov 2026

### Battery
- Deep standby: the panel sleeps with the screen, and Wi-Fi rests after ten minutes.
- Battery saver, from the control centre or by itself at 20%.
- Warnings at 15% and 10%, and a clean power-off at 5%.
- The Battery page: the charge, time left, the draw, and the last day as a chart.
- Less power at rest: the Ethernet chip and USB hub sleep when idle.
- A lower processor voltage, as a developer option, tried before it is kept.

### Lights
- Fn blinks for anything said while the screen was off.
- Battery warnings, the pit timer and queueing blink all three lights, then stop.
- The red light can stay dark.

### New
- Remote: a universal remote on the infrared LED. It learns buttons too.
- Level and Compass, from the motion sensor.
- Lift to wake, and face down to sleep.

### Setup
- Wi-Fi comes first, and setup carries on by itself once joined.
- A hardware check, a keyboard lesson, the handheld's name and time zone, and power choices.

### System
- 1 GB system partitions, with room for the updates to come. A card from before 0.4.0 is flashed afresh.
- Wi-Fi on the CardputerZero's own radio.
- The panel draws without tearing, in step with its refresh.
- A dark start: no console text before the start animation.
- The AprilTag reader loads the camera's image processor when it opens.
- USB to a computer takes the device role after a restart.

## 0.3.0 · 19 Nov 2026

### New
- Recent apps (Ctrl Tab) and screenshots from the control centre.
- Standby and power saving: the screen dims, then goes off.
- Monitor, Net tools with the robot finder, Connect (SSH and serial) and Actions.
- The AprilTag reader, USB to a computer, and command-line tools (Python, vim, htop, tmux and more).
- Live graphs, the pit checklist and match timer, and match trends in Logs.
- About, Help and developer options.

### Better
- Steady key repeat, and apps that open out of their tiles.
- Hold ? for five seconds for every shortcut.

## 0.2.0 · 19 Nov 2026

### New
- The first release: setup on the handheld or from a computer, and the start animation.
- Themes, wallpapers, your own colour, and Home's arrangement.
- The robot's tools: post-match report, code version check, event schedule and match logs.
- Search, the app catalogue, git, and the glance page's cards.
- Terminal, Files and the editor, and Wi-Fi setup.
- System updates over the air, the flasher, and the GPU.

# What's new in Catalyst Linux

Each release's notes, newest first. `scripts/release.sh` puts the section
for the version it makes into the release (its manifest and its GitHub
page). The handheld shows it before an update installs and once after it
restarts into one, and the image carries this whole file for
Settings › About › Update history.

A section starts `## <version>`, optionally `· <date>`. `### ` groups its
lines; each line is a `- ` bullet, short and in plain words. Anything before
the first section (this introduction) is not shown.

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

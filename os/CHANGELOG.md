# What's new in Catalyst Linux

Each release's notes, newest first. `scripts/release.sh` puts the section
for the version it makes into the release (its manifest and its GitHub
page). The handheld shows it before an update installs and once after it
restarts into one, and the image carries this whole file for
Settings › About › Update history.

A section starts `## <version>`, optionally `· <date>`. `### ` groups its
lines; each line is a `- ` bullet, short and in plain words. Anything before
the first section (this introduction) is not shown.

## 0.6.2

### Fixed
- 60 frames a second, held: the first frames of a scroll or a page turning, drawn while the processor speeds up, no longer drop everything after them to 45.
- The Assistant answers with the on-device model again: with no room for the model you chose (the 350M needs more memory than is free while Music plays), the biggest one downloaded that fits answers, and says which. If none can, it says why instead of the offline help.
- Questions the match logs can't answer go to the model, not only ones that look like chat.
- The graphics chip has the 64 MB of memory 0.6.1 meant to give it.

## 0.6.1

### Smoother
- Everything moves at up to 60 frames a second, up from 30: a whole picture reaches the screen in 15 ms instead of 33. The screen's link runs at its full 62.5 MHz now, and the picture no longer waits 11 ms in slow memory before it is sent. Full speed from the second start after updating.
- The processor speeds up the moment you press a key or something starts to move, and slows again as soon as it stops; downloads, indexing, updates and Music's helpers make way for the screen.
- A change on screen only one line tall (a thin progress bar) always shows now; it could be missed before.
- If the graphics chip runs out of memory, the picture is finished on the processor rather than shown half-drawn, and the chip is tried again later.
- Secure connections use the cipher this processor is fastest at.

### Notifications
- Reminders, timers, downloads, installs, updates, the robot (linked, match summary, brownout), queueing, long Assistant answers and battery warnings are kept until you've seen them.
- The notification centre: Ctrl+Backspace, or Notifications on the control centre. Enter goes where one leads, Fn+Backspace removes one, Clear all removes everything.
- The island shows how many are new, and the lock screen the newest ones; their words only if you choose. Fn+F or Fn+X picks one, and it opens once unlocked.
- Quiet on the control centre: notifications collect silently, while reminders, timers and "time to queue" still come through.

### Fixed
- On the lock screen, the charging chip no longer runs into the music card: the chips sit under the date and the cards start below them, at every text size.
- The screen stays dark no more after a restart: 0.6.0's new brightness curve read a dark backlight as lit, so the first picture never turned it on.
- Installing an update no longer slows the handheld down: it runs at the lowest priority and writes to the card in small steps, so the screen never waits on it.
- Music in the background no longer makes everything lag: saving songs waits for the card behind everything else, the card writes a little at a time, and the graphics chip has twice the memory it ran out of.
- Music: Esc on Now playing goes back to the search, list, library or downloads you played from.
- With no internet, the Assistant answers any question with the on-device model when one is downloaded, not only questions about match logs.
- Logs copied onto the handheld (over USB, SSH or Files) show at once.
- catalyst-shell --demo-logs in the Terminal writes a demo event's match logs, to try Logs, Trends and the offline Assistant without a robot.
- Everything new in 0.6.0 comes with this update too: Flow, the hold-Tab lock, live activities, the new Music, offline answers, Numbers in 3D and more.

## 0.6.0

### Home and the lock screen
- Flow, a new Home style: your apps as album covers. Hold left or right to fly through them, Enter turns one over to show what it is and says right now, Enter again opens it. Settings › Home › Home screen.
- Lock the handheld by holding Tab for a moment, from anywhere; a ring fills around the island as you hold. Without a passcode, any key takes you back.
- Live activities in the island, on the lock screen and in StandBy: the song playing, timers, your next match and when to queue, a reminder due soon, downloads, the Assistant thinking, and the robot while it is enabled.
- Music's keys (Fn + Q, W, E) play, pause and skip on the lock screen without unlocking; on the charger StandBy turns to Now playing.
- Screens use the whole height: lists run to the bottom and fade out under it. The Terminal shows one more line.

### Music
- Music finds a song in about a third of a second, and a new, lighter player plays smoothly even when the handheld is busy or Wi-Fi stalls.
- Every song you play is kept, so it plays again with no internet. Music opens on your Library when offline.
- A Library on the card (/data/music/library): artists, albums, songs, played and playlists, with your own music too, and album covers.
- Download whole albums, playlists, an artist's top songs, your likes or everything you've played, with covers; Wi-Fi only or while charging if you choose.
- Japanese, Chinese and Korean show everywhere: song titles, file names, notes, web pages, Spotlight and the Assistant.

### Assistant
- Works offline: ask about your matches with no internet and no key, answered from the logs on the handheld. Every number is checked against the log.
- Ctrl R shows the newest match's report, worst first, with times.
- Settings › On-device model offers Fastest, Balanced and Smartest, measured on this handheld. Balanced (LFM2.5 230M) is the new default.
- The model gives its memory back two minutes after its last answer, and at once when the screen goes off; it never pushes the rest of the handheld into swap.

### Robot
- The Robot app shows Numbers in 3D: its intake slides out, its hood rises and its flywheel spins as the robot reports them; a hot part glows amber, a fault red, and the bumpers take your alliance colour.

### Battery
- Music standby: playing with the screen off, the USB host, 5 V outputs, lights and motion sensor switch off, and Wi-Fi too once what plays next is downloaded.
- Far fewer wakeups while idle, and the backlight follows a perceptual curve: about 40% less of its power at the usual brightness. You may want to raise brightness a step once.
- The screen never stalls because something else filled the memory: the shell is kept in RAM.

### Fixed
- The speaker plays both channels from the moment the handheld starts.
- Setup's choice pages start on Next, and setup carries on by itself once Wi-Fi joins.

## 0.5.6

### Fixed
- 0.5.5 could leave the screen dark after it installed: its panel driver did not match its kernel. Fixed, and an update that leaves the panel dark now goes back to the system before it by itself.
- This update also brings everything 0.5.5 had: apps from the Catalogue, Catalyst Kit, and compressed swap in memory.

### Sound
- The speaker and headphones work. The speaker plays both channels, and its amplifier is on only while something plays.
- Volume on Fn+S and Fn+D, mute on Fn+A, a volume bar in the control centre, and Settings › Sound.
- The microphone works, and any program can record from it. Settings › Sound shows its level.
- Bluetooth headphones and speakers, off until switched on in Settings. The first time, the handheld restarts once; the serial console stays where it was.
- Reminders chime through the speaker, with a brighter sound for pit reminders.

### Music
- Music: YouTube Music, signed out unless you choose to sign in. Search, quick picks, a song's radio and the queue, with the cover on the now-playing page.
- It plays on behind other apps and with the screen off. Fn+Q, W and E play, pause and skip from anywhere, and Spotlight finds music.

### Voice
- Talk to the Assistant: hold Alt anywhere and speak; let go and it writes down what you said and asks it. While it listens, the island shows a microphone and a live level.
- What you said sits in the question line for a moment before it sends: press any key to edit it first, or Enter to send now.
- Dictate into Notes: hold Alt in a note to type what you say at the cursor; on the Notes list it starts a new note.
- Spoken answers, off unless you turn them on in the Assistant's choices: for questions asked aloud, or for every answer. Long answers are said in two sentences, and any key stops it.
- ai --listen in the Terminal: ask out loud and press Enter when you're done.
- Titles and the status give way under a wide island, fading out and back instead of being cut short.

## 0.5.5

### Apps
- Get apps over the air from the Catalogue: Sudoku, Battleship, Typing and Orbits, rebuilt from M5Stack's CardputerZero store, plus Hello.
- Every catalogue app is signed, and the handheld installs only what the catalogue's key signed.
- Each app has its own page: what it uses, its credit and licence, Get, Update or Remove. u updates them all.
- Spotlight finds apps in the catalogue you haven't got yet.
- Apps keep what they save across updates, and can bring their own icon.

### For developers
- Catalyst Kit: write apps that look and move like the handheld's own.
- catalyst-ui gives shell scripts the handheld's own screens: menus, input, confirm and messages.
- ai app in the Terminal writes an app for you, shows it whole, and puts it on Home only on a yes.

### System
- Compressed swap in memory (zram): idle memory is squeezed to about a third and kept in RAM, so the on-device model and big pages have more room. Nothing is written to the card.

## 0.5.4

### StandBy and the lock screen
- StandBy: on the charger and left alone, the handheld shows a clock, the charge, the next match, the weather, photos or a little of everything instead of going dark. ↑↓ change the face, ←→ its style. At night it turns red and dims.
- Set it down on the charger with the screen off and StandBy comes up at once. Deep standby never starts on the charger.
- Lock screen: set a passcode in Settings › Lock screen and the handheld asks for it once the screen has been off. No passcode is needed; without one, a key wakes it straight to where it was.
- Customise the lock screen (its clock, its colour, and what shows with the time) with the lock screen itself as the preview.
- A wrong passcode shakes the field; after five in a row, each try waits longer.
- Notices and reminders that arrive while it's locked show as a count, and are said once it opens.

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

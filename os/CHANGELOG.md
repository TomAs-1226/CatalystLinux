# What's new in Catalyst Linux

Each release's notes, newest first. `scripts/release.sh` puts the section
for the version it makes into the release (its manifest and its GitHub
page). The handheld shows it before an update installs and once after it
restarts into one, and the image carries this whole file for
Settings › About › Update history.

A section starts `## <version>`, optionally `· <date>`. `### ` groups its
lines; each line is a `- ` bullet, short and in plain words. Anything before
the first section (this introduction) is not shown.

## 1.5.2

Truer colour, and every photo made from several frames.

### Camera
- Colour photos no longer come out yellow under lamps or tinted in other light. A smart white balance checks each viewfinder frame for a leftover cast and corrects it, keeping just a little warmth under lamps; photos match the viewfinder.
- Every photo is made from three quick frames merged together: cleaner detail and colour, no grain. Saving takes about a second longer; the shutter stays instant.
- Shadows and faces in shade are lifted gently, without the flat HDR look, and sharpening crisps edges while leaving sky and skin smooth. Night and HDR get the same finish.
- Detail now sets how many frames each photo uses: auto (3), on (6) or off (1, the quickest).
- Pro mode's own white balance and black-and-white looks are left as they were.

## 1.5.1

Three new camera modes, and photos that look like photos again.

### Camera
- Burst takes about ten frames at a fast shutter and keeps only the sharpest, for a robot crossing the field. s picks the shutter; Esc stops early and keeps the best so far.
- Light trails keeps the brightest light from each frame, so moving LEDs, car lights and sparklers become streaks, built live on the screen. s sets how long; Enter or Esc finishes and keeps it.
- Time-lapse takes a picture every few seconds (s: 2 s to a minute) and saves a video that loops in Photos, shows in the build log and plays on a phone once saved. Enter starts and stops it.
- Daylight photos are no longer over-sharpened or gritty: the camera's own sharpening, with a light noise filter.
- The bright-sky guard and HDR only step in for a really blown sky or a backlit scene, and more gently, so ordinary photos aren't flat or grey.
- With a look on, Night and HDR no longer add their own shadow lift underneath it: the look alone sets the tone.

### Photos
- Time-lapses show in the phone gallery too, saved to play.

## 1.5.0

### Home
- Hold c on Home to open the Camera straight away. A tap still moves right as before.

### Photos
- Photos on your phone: in Photos press p (or Share › Photos) and scan the code. It works on the same Wi-Fi or on your phone's own hotspot, with no cable and no app.
- Your phone's browser shows the album by day, with captions, places and matches. Open a photo full screen, swipe between them, save one, or download them all (or the ones you pick) as one ZIP.
- Sharing lasts only while that screen is open and stops ten minutes after the last look; Esc stops it at once. Nobody gets in without the code on the screen.
- iPhone hotspots: the handheld's Wi-Fi is 2.4 GHz, so turn on Maximize Compatibility in Personal Hotspot for the handheld to see it.

## 1.4.0

Night photos, HDR, sharper pixel pictures, and photos that document
themselves: captions by Claude, the places they were taken, and a build
log.

### Camera
- Night: in the dark the Camera takes a night photo by itself, short frames merged into one: about a second in the hand, up to four set down. n steps auto, off, 1, 2, 4 s; its chip shows how long. A ring counts down; Esc cancels.
- Night's preview is brighter, so you can frame in the dark.
- HDR: backlit and high-contrast scenes are merged from four short frames, keeping the sky and lifting the shadows. b: auto or off.
- Bright skies and windows are no longer blown when the rest of the picture can spare it.
- e locks the exposure on the middle of the picture; e again lets it go.
- Pixel pictures are made from the camera's full-size still, so they're sharper and cleaner; if the still doesn't come, the viewfinder's picture is kept.
- Pixel mode's colours are truer (hue counts more than brightness when picking a colour), edges are crisper with no halo, and darker skin and green fields no longer go too dark.
- Pixel mode has an auto palette of eight colours taken from the scene, and a two-ink print palette.
- g in pixel mode records a two-second loop, saved as an animated GIF that Photos plays; y adds a date stamp in a pixel font (off by default).

### Photos
- Captions by Claude if you turn them on (Settings › Photos). New photos are sent shrunk to Anthropic in the background and come back with a sentence or two for the build log, about 0.4¢ a photo, never past your monthly limit ($3 to start).
- ↓ on a photo shows its caption, place and match under it; t writes or edits the caption, a asks Claude now.
- n names where a photo was taken (Shop, Pit, Home). Photos there before and after take the name, from the Wi-Fi around them, kept on the handheld.
- Photos at an event say which and when, such as "Finger Lakes Regional · during Q31".
- Find the town online (off unless turned on) sends the nearby Wi-Fi to BeaconDB and names the town on the handheld.
- The build log, l in the album: days, then places, each photo with its caption. e exports a day, the event, a week or everything as Markdown with its photos into photos/Build log; s twice sends the day to Team.
- Loops play and are marked in the album; a pixel picture's details fit at every text size.

### Fixed
- Camera: night photos were never longer than 1/15 s, whatever n said.
- Camera: turning Night on or off no longer blacks out the picture.
- Camera: a photo taken just after opening the Camera, or right after the scene changed, is exposed where the camera was heading.

## 1.3.1

### Fixed
- Photos: taken at last, for real this time. 1.1.1 gave the camera's sensor two frames but let its pipeline keep none of them, and still no photo came; it keeps at least one now, and a full 8 MP photo takes under a second.

## 1.3.0

A pixel camera, film looks, and a laptop that sets itself up for Claude on
the handheld.

### Camera
- Pixel mode, between pro and doc: the picture made small, in few colours and dithered, live, like a toy camera. Size s (56, 75, 112), palette k (six), dither d (Bayer, Floyd–Steinberg, Atkinson, none), brightness ↑ ↓, contrast - =. Each is in the bar and shown on the picture as you choose. The album is on a.
- A pixel picture is kept at its true size and as a sharp copy about 450 px wide for sharing; Photos shows its pixels sharp, zoomed or not.
- Film looks, a group of their own: Sunlit Slide, Soft Negative, Faded Slide, Tungsten, Pushed Mono, Pearl Mono and Cine, each with a film's tone and colour.
- Film grain and glow, off, light or strong (↑ ↓ in the looks), laid on the photo just after it is saved.
- Photos editor: the film looks too, their grain on ← →.

### Claude on the handheld
- Catalyst Companion sets the laptop up by itself: one Set up installs Claude Code and Tailscale if needed, turns on Funnel, starts the bridge with Windows and keeps the laptop awake when plugged in. You only sign in, and approve in the browser where Tailscale asks.
- Each step shows done, working, waiting for you, or what to do if it failed. Running it again only does what is missing.
- Keeping awake changes mains power only (sleep and hibernate Never, lid Does nothing); battery is untouched, and Restore my power settings puts the old values back.
- Each time the bridge starts it checks everything again: it fixes what changed (Funnel off after an update, for one) and says in the tray what needs you, such as signing in again.

## 1.2.0

Your laptop's Claude Code sessions, from the handheld, anywhere you have
internet.

### Claude on the handheld
- Claude: your laptop's Claude Code sessions from the handheld, over any internet, a phone's hotspot included. Pair once with the QR code Catalyst Companion shows (or its address and a six-digit code).
- The sessions with their project and last activity; a conversation as Claude writes it, each tool call one line (Enter opens it); type or hold Alt to talk; Tab asks for everything or accepts edits; Ctrl-X stops Claude.
- Every command or file outside the project waits for you: allow (y) or deny (n) on a sheet. Unanswered for five minutes, it is denied.
- A reply finished or a request waiting shows in the island and as a live activity while you are elsewhere.
- Everything is sealed end to end; the handheld never signs in to Claude, and the connection is held only while you need it.
- Catalyst Companion: Claude on the handheld, a bridge in the tray, with the pairing code, what is left to set up, and Start with Windows.

## 1.1.1

### Fixed
- Photos: taken at last, at the full 8 MP in under a second. The camera's pipeline kept one frame for the sensor, which had nowhere to write while the picture was being made, so no photo ever came; it keeps two now.

## 1.1.0

Photos are taken again, every control says its key, and the smart camera
gets its own firmware, with voice commands and dictation that work without
the internet.

### Fixed
- Photos: no photo was ever saved. The memory set aside for the camera was too small for a full 8 MP picture; it is 128 MB now, which the rest of the handheld borrows while the camera is not taking one.
- Camera: the shell waits for the camera's last try at a photo instead of giving up just before it.
- Camera keys that never worked now do: looks k, focus check +, timer t, zebras q, timing readout j (the bare f, x, z and c are the arrows outside typing).
- Photos editor exports with s; Scout "do not pick" is p; Batteries charger is o; Rulebook Ask is Tab; Mech and Gamepad forget with del twice.

### Keys in sight
- ? on any screen shows that screen's keys on a sheet, then the keys that work everywhere; Fn 1 where you are typing; hold ? for every shortcut. Hint lines end "? keys", and each app says so the first time it opens.
- Camera: a bar of controls along the viewfinder (mode, look, timer, grid, detail, pro, and lens with a smart camera), each showing its key. Tab to use it, Enter for its choices, Esc back. Space always takes the photo.
- Camera: looks are larger, named and live; Enter keeps one, Esc goes back. Pro's controls get a row of their own.

### Smart camera: Catalyst Coprocessor
- The UnitV can run Catalyst's own firmware (installed over USB-A, once): it always starts its program, idles at about a third of the power when nothing is asked, and wakes in a few hundredths of a second.
- Each screen asks the camera only for what it shows, at its own pace; results carry when they were taken; Settings › Camera › Smart camera shows its state, clock, memory and wake time.
- AprilTag distances calibrated: t on the smart camera's page, a tag at half a metre and a metre.

### Voice
- Voice commands on the smart camera's NPU: hold Alt and say yes, no, up, down, left, right, on, off, stop, go or a number; each does what Settings › Voice commands says (Team's calls on one to six), and you can teach it four words of your own.
- Anything else you say is dictated, only the words: silence is never sent. Tap Alt to dictate hands-free; it stops when you do.
- Dictation without the internet: Settings › Offline dictation downloads Whisper (44 MB, English) and writes down what you say on the handheld itself.
- It loads when you first dictate and gives its memory back a minute later or as soon as the screen goes off.

## 1.0.0

Catalyst Linux 1.0: the handheld for an FRC team at an event. With 0.9's
Scout, Strategy, Batteries, Mech, Gamepad and Rulebook, the field view and
match replays, the pro camera and the smart camera, 1.0 adds a way for the
team's handhelds to talk to each other, a photo editor and a document
scanner, and makes the handheld safe to recover when something goes wrong.

### Team
- Team: messages between the team's handhelds at an event, over the pit's Wi-Fi or a phone's hotspot, with no server.
- Make a team space and show its QR code; others join by scanning it in the Camera or typing a six-digit code. Everything in a team is sealed with its own key, made on the handheld that started it.
- Six calls on the number keys (robot to queue, battery needed, bring the cart, need a driver, match in 5, all good), yours to change. Urgent calls break through Quiet and show on the island in the signal colour.
- The Team page shows the next match, who is here and has read the newest, and the thread. Messages are resent until each handheld there has them, and one that was away gets what it missed.
- Send a Strategy board or a match's scouting straight to the team: t in their share menus.

### Photos
- Edit a photo with e: exposure, contrast, highlights, shadows, saturation, warmth, tint, vibrance, sharpness and a vignette; any of the Camera's looks laid on after the fact.
- Crop to free, 4:3, 3:2, 16:9 or 1:1, turn a quarter, straighten by up to 15° over a grid, and flip. Mark up a photo for the robot's notes: arrows, boxes, circles and short labels.
- Edits are kept beside the photo and never change it; x exports a new full-size IMG_…_edit.jpg.

### Document scanner
- The Camera's new doc mode: the page is found and outlined, and taken by itself once held still for a second.
- Pages are straightened to their true shape (A4 or Letter found), in colour, grey or black and white. Gather several, reorder or delete them, and save one PDF with a PNG for each page in Scans; Catalyst Companion downloads them.

### Fixed
- A device on the Grove port (the smart camera, powered or not) no longer keeps the screen dark at start. The port's two lines were switched to the I2C bus the board's I/O expander is on until the system set them, and a device there hid the expander, which the screen needs. They are on the serial port from the first instant now.
- The smart camera answers: MaixPy as M5Stack ships it for the UnitV starts boot.py and not main.py, so the camera's program never ran. boot.py starts it now, and a camera with the old one is offered the update.
- Photos no longer end with "the camera gave no picture": a photo's first frame is waited for up to 4 seconds (8 on the last try), not 1.
- The screen's brightness is set from Settings at every start. The backlight keeps whatever it was left at across a restart, so an update installed overnight with the screen off left the next start all but dark.

### Recovery
- A way back in from any computer: put a file named catalyst-recovery.txt on the card's boot drive (the small one any computer can open) and start the handheld. It turns the Grove port's console and the USB serial console back on, copies its logs to catalyst-logs on the boot drive, and removes the file.
- The shell's log is kept on the card, with the start before's beside it, so a start that went wrong leaves its story behind.

## 0.9.4

### Camera
- Photos are quick again. "Hold still" could last ten seconds and more: a failed attempt waited six seconds before the next. Now a failed attempt gives up at once and every wait is bounded.
- "Hold still" shows only while the sensor is exposing a long shot, a detail burst or a bracketing set, with the seconds left. Detail mode's auto burst only in real low light (under 25 lux); it no longer fires in every evening room.
- Every photo's timing is in the log; in pro mode t shows "last shot: 0.42 s capture · 0.7 s save".
- Self-timer 2 or 10 s (c), grids in thirds or squares (g), a horizon level from the motion sensor (h), and 3:2, 16:9 and 1:1 framing with the photo cropped to match (o).
- Pro mode: bracketing of 3 or 5 frames at ±1 or ±2 EV (b); shutter priority, ISO priority and manual (m); long exposures up to 11.7 s; grey-card white balance (y); an RGB histogram (u); false colour (x); presets C1–C3 (1–3, Shift to save); an intervalometer that dims the screen and keeps the handheld awake (v).

### Smart camera
- Settings › Camera › Smart camera: live status in plain words (power, bytes, speed, what the camera said), and c for a report in /data/logs/smartcam-report.txt.
- Installing on the camera ends by giving the Grove port to it, so it powers up; a microSD card's main.py is handled.
- The camera's program can no longer stop before it listens, and its light says how it stands: amber powered, green listening, blue linked, violet updating, red sensor failed.
- 12 s to answer at first; the speed climbs one confirmed step at a time; a quiet link is found again.
- The camera's program and face model update over the Grove port from each new Catalyst (resumed if cut off, rolled back if the new one never starts). MaixPy itself still over USB-A, and only when needed.
- Ctrl-Enter opens the island's notice ("install on the camera") while it shows.
- The Grove 5 V is rated 0.5 A, the UnitV's own figure: if the camera restarts by itself, give it power on its USB-C (a power bank) as well.

### Scout
- Scout: match scouting for the stands. Each scout is given a place on the field (s changes it) and Scout opens on the next match and the robot there.
- The 2026 REBUILT form, kept as a file so a new season needs no update: a key adds (q fuel, w missed, e passed, r and t the tower, y and u fouls), Shift or the key under it takes one away, del undoes. It follows the match clock from AUTO to TELEOP to END GAME, then ratings and notes.
- Share with no internet: a match or the whole event as QR codes, read in with the Camera's scan; nothing comes in twice, and the newer edit wins. And a CSV file for Catalyst Companion.
- Teams sorted by any metric with a line over their matches, a picklist with notes and do-not-pick, and a practice cycle timer with each driver's average and best.

### Strategy
- Strategy: a board for each match on the field map, filled from the schedule with all six robots. Draw with the keys: robots, paths with arrowheads in each robot's colour, drop zones and notes; auto, teleop and endgame on layers of their own (a, t, e); Space plays the paths.
- Keep a board as a template and use it on any match, on either alliance. Share it as QR codes, or as a picture for Catalyst Companion. When it is nearly time to queue, the island offers the match's board.

### Batteries
- Batteries: each robot battery's state, health and trend, and which to put in next. When the next match's queue time nears, the island asks which battery goes in.
- After each match its log tells the battery's volts, sag, brownouts and, where the robot logs its current, its resistance; a battery wearing out is flagged to retire soon. On the charger and off it, with a reminder and rest before it counts as charged. QR labels to print; the Camera's scan opens a battery from its label.

### Pit tools
- Mech: work out a gear train, drivetrain, arm, elevator or flywheel from its motor, with a graph, from ReCalc's vendor data for the Kraken, NEO, Vortex, Falcon, Minion, Pulsar and the rest.
- Gamepad: plug a driver's controller into USB-A to see its sticks, triggers and buttons live and rumble it; its check measures drift, shake, full throw and report rate, suggests a deadzone, and keeps each controller's result, best first.

### Rulebook
- Rulebook: FIRST's 2026 game manual on the handheld, downloaded from FIRST the first time, for events where the Wi-Fi is bad. Read it at any text size, jump to a rule by its ID, bookmark, and search it offline.
- Ask in plain words: Claude online, or the on-device model offline, answers from the rules the search finds, naming them, with those rules listed under the answer. When online it checks for a revised manual and lists what changed.

## 0.9.3

### Fixed
- Settings › Camera › UnitV installs again: the smart camera's program, its face model and MaixPy were missing from 0.9.1 and 0.9.2, so it said main.py was missing.

## 0.9.2

### Camera
- Photos are quicker: the shot appears in the corner the moment you press the shutter, the next photo can be taken while the last is still being saved, and a photo now takes about half the time to save.
- Crisper photos: in auto the shutter stays at 1/60 or faster while it can, and in good light the camera's noise filter no longer smears fine detail.
- Pro mode: shutter (s), ISO (i), exposure compensation (e) and white balance (w), each turned with ← →, with a live histogram, zebras on bright highlights (x) and the shutter speed always in sight. The lens is a fixed f/2.0.
- RAW (r in pro mode): a DNG of the sensor's own picture beside each JPEG.
- Looks (f): Vivid, Dramatic, Mono, Silvertone, Noir, Classic, Contemporary, BW Natural, BW High Contrast, a Tint in any colour and more, shown live and kept on the photo. Photos shows the original with o.
- The focus check (z): the picture at its real pixels, with sharp edges marked, to see what is in focus.
- Detail mode's auto now merges a burst only in real low light.

## 0.9.1

### Smart camera
- Settings › Camera › Grove port changes at once, both ways: no restart. Moving to the smart camera stops the login on the port and keeps the system's messages off it; back to the console, the login is there again at once and the system's messages return from the next start.
- The smart camera's program installs from the handheld: plug the UnitV's USB-C into the USB-A port and the island offers "install on the camera". No microSD card needed. It puts MaixPy on it if it needs it, then the face model and the program, and checks it answers. Takes about a minute.
- Settings › Camera › UnitV says whether the camera's program is up to date, and "update on the camera" when a newer Catalyst has a newer one: plug it into USB-A once to update.
- USB serial adapters that use CDC-ACM now work when plugged in.

### Panel
- The tear-free panel (Developer options) no longer tears at the full-speed link: each frame goes out where the panel's refresh cannot cross it, usually at once and at most a few milliseconds later. Try it: it is to become the default once it is seen upright on a handheld.
- In the tear-free mode the panel refreshes 59.9 times a second, just under the 60 frames drawn, so motion no longer hitches about twice a second.

## 0.9.0

### Lock screen
- The depth effect: Numbers, 5805's robot, stands in front of the time on the lock screen, covering no more than the lower part of the digits, with the date and the status above it. Settings › Lock screen › Depth effect turns it off.
- Its pose follows the handheld: it turns slowly on its stand, rests docked over a soft light on the charger, slumps and dims when the battery is low, and stands ready facing you before a match, in your alliance's colour. Settings › Lock screen › Pose follows status keeps it idle.
- StandBy has a robot clock too: ←→ on the clock to find it.
- The turning robot draws a few frames a second and stops still in the battery saver, on StandBy and with reduced motion.

### Field
- The Robot app has a field: press → on the Robot page. This season's field, with the robot where its pose says, pointing the way it faces, and the last ten seconds of its path fading behind it. + and − zoom, the arrows move about, Enter follows the robot again.
- The pose comes from AdvantageKit's odometry or WPILib's Field2d, as often as the robot sends it. Without one, the field says there is no pose on the link.
- The field is drawn only while the robot moves, and not at all while it stands still.

### Match replays
- Replay a match from its log: Logs › a match › replay the match. The field with the robot's path, a timeline with auto, teleop and the endgame marked, and the battery, CAN and current under it. Space plays and pauses; 1, 2 and 4 set the speed; ←→ skip 5 seconds.
- Brownouts and CAN spikes are marked, and ↑↓ jump between them. Each finding in a match's summary shows when it happened, and Enter replays the match from there: Q14's brownout opens at 1:18.

### Camera
- Every photo is the sensor's full 3280 × 2464. Until now they all came out at 1640 × 1232: the camera's memory could not hold a full-size frame. Photos are saved at quality 95 with their details (exposure, lens, time), and the preview comes back while the photo is written.
- Detail mode (Settings › Camera, or d in the Camera): in dim light, or always, a quick burst merged into one clearer photo in the background. The photo is there at once; its thumbnail says how many seconds until the merged one replaces it.

### Smart camera
- An M5Stack UnitV on the Grove port can see for the handheld: it finds faces, QR codes, AprilTags and coloured game pieces on its own processor. Faces steer the exposure, scan mode reads its codes, and ↑ in the Camera shows what it sees.
- Settings › Camera › Grove port: Console (as before) or Smart camera. As the camera's, the port carries no login and no boot messages, and its power is on only while the camera is in use. Setting it up once: tools/unitv in the README.

## 0.8.1

### Fixed
- Music no longer lags as it opens, nor does the rest of the handheld while a song plays. The island's bars draw 15 times a second on their own instead of holding every page to 60 (which also kept the frame rate from ever coming back up after a heavy moment), and the library's lists of songs, albums, artists and playlists are made once instead of on every frame.

## 0.8.0

### Lyrics
- Lyrics in Music: press Y on the song playing. The line being sung is large, the lines round it dimmed, and they move in time with the song, gliding when you skip ahead or back. Lyrics without timings scroll with ↑↓.
- Lyrics come from LRCLIB, or from YouTube Music when you are signed in and LRCLIB has none. They are kept with each song, so they show offline after the first time, and songs you download get theirs as they are saved.
- The island's card shows the line being sung. Settings › Music › Lyrics on the island turns it off.

### Music on the island
- While a song plays, the island shows its cover and bars that move with the music, and lie flat when it is paused. They stand still on StandBy and with the battery saver.
- The island's card shows the song's progress in its cover's colour, and the next song slides in as the last one goes.

### Sleep timer
- Press T in Music to stop the music after 15, 30, 45 or 60 minutes, or at the end of the song. The last half minute fades out, then it pauses.
- The island and Music count it down. It stays set when you leave Music, until the handheld restarts.

### Sound
- Spatial sound for headphones: Settings › Sound puts a song's two channels on a pair of speakers in front of you, in a small room, instead of inside your head. Off unless switched on, and never through the speaker.
- With AirPods that track the head, spatial sound stays where it is as you turn, and comes round slowly if you stay turned. Head tracking runs only while spatial sound plays.
- AirPods: each bud's battery and the case's in Settings › Bluetooth, and in the island as they connect; noise control (off, noise cancelling, transparency, adaptive) in Settings › Sound; take a bud out and Music pauses, put it back and it plays on.
- Bluetooth headphones that take AAC (AirPods and most others) get it, at 256 kb/s, instead of SBC. Settings › Bluetooth shows the codec.
- USB headphones with a volume of their own play songs bit for bit, at the song's own rate and depth. Settings › Sound › Quality says how a song plays: "44.1 kHz · 16-bit · bit-perfect".
- An equalizer: flat, bass, vocal, treble, loudness (more bass and treble as the volume comes down), or five bands of your own.
- Loudness match: every song as loud as the last, from its ReplayGain or R128 tags, or measured the first time it plays.
- Gapless: songs meant to run on into each other (a live album, a DJ mix) play without a gap. A crossfade of up to 8 seconds, never within an album, in Settings › Sound.

### Fixed
- Your likes hold only the songs you liked on YouTube Music, not every song of every album you saved. The first look after the update takes the others out of Your likes (they stay in the library), and Liked music is no longer listed twice.
- On the charger (a computer's USB counts), StandBy goes dark five minutes after it comes up; a key or a lift brings it back. Deep standby still waits until it is unplugged.
- Music paused for ten minutes leaves the island and stops fetching the songs after it; play carries on where it was.
- Deep standby, and music with the screen off, no longer take Wi-Fi away while songs are downloading.

## 0.7.0

### Computer companion
- Catalyst Companion for Windows (catalyst-companion.exe, in each release) reaches the handheld from a computer. It finds it by itself on Wi-Fi or over the USB cable, and pairs once with the code Settings › Computer companion shows.
- YouTube Music signs in with one click: a sign-in window opens on the computer and closes once you are signed in, and the handheld checks and keeps the sign-in. The computer keeps it only in that window's own profile, which Sign out clears.
- GitHub signs in with the computer's own sign-in (GitHub CLI or Git Credential Manager, asked first), a pasted token, or a code on github.com when the app has an OAuth client ID.
- Songs dropped on the window go into the library, and Music indexes them as its own. Match logs, photos, screenshots and notes download to the computer.
- The team, Wi-Fi, keys and the look change from the computer, as Set up from a computer changes them. System updates check, install and restart from there too.
- Back up the handheld to a zip on the computer, and restore it. Caches, downloaded updates and YouTube Music's sign-in are left out.
- Safe on a shared network: TLS to the handheld's own certificate, pinned when pairing; the code never crosses the network, and five wrong tries end it; each computer's token is kept on the handheld only as a hash. Unpair a computer in Settings › Computer companion, or switch it off there and nothing listens.

### Fixed
- After a heavy moment, such as Music opening, pages come back to 60 frames a second on the next key, not after several presses at 30.
- An update looked for while the handheld was offline no longer installs the system it already runs and restarts into it.

## 0.6.9

### Fixed
- Music, and any page after a heavy opening, runs at 60 frames a second again: the moment a page opens no longer holds everything after it at 30.
- Settings › Sound › USB headphones takes the next step on Enter: USB on charging only while a computer's mode has the port, then a restart.

## 0.6.8

### Music
- Sign in to YouTube Music from a computer: Settings › Set up from a computer takes the cookies.txt a browser extension exports from music.youtube.com, chosen as a file or pasted. It checks the cookies sign in, keeps only YouTube's, and never shows them again; left empty, the sign-in stays as it is.
- Signed in, Your library in Music has your playlists (Liked music first), the albums you saved and your artists, as YouTube Music lists them. Each opens to play, shuffle or download all for offline.
- Your library shows at once, offline too: it is kept on the card and brought up to date in the background whenever Music opens online. Offline, a playlist you downloaded opens from the card.

### USB headphones
- USB-C headphones, or a USB headphone adapter, play: plug them into USB-C1 or the USB-A port and sound moves to them, the song playing too, with the speaker off. The island says so.
- Pulled out, sound goes back to the speaker and music pauses, as a phone does.
- Their volume is their own and kept for them: the volume keys move it while they play.
- With Bluetooth headphones too, the ones connected last play; Enter on Settings › Sound › USB headphones swaps.
- Settings › Sound shows where sound plays, and what USB headphones need while USB is set for a computer: charging only, the USB switch on the hub's side, and a restart.
- Music with the screen off keeps playing through them: music standby leaves the USB host on for them.

## 0.6.7

### Fixed
- The island shrinks to music's cover and level while a song plays, like the Dynamic Island, so the time and battery stay in view; the song's name shows on the card it opens to when the song changes.
- Live activities never push the time and battery away: the island keeps the part of its words that matters ("queue 8:12", then "8:12"). Only something urgent, like queueing now, takes the room while it lasts.

## 0.6.6

### Fixed
- Updates checked for in standby, after Wi-Fi has gone to sleep, wake Wi-Fi rather than failing to reach GitHub; it goes back to sleep once the update is done with it.
- A download never carries on from a file another release left with the same name, which could make an update fail its check.

## 0.6.5

### Fixed
- The Assistant answers with Claude again: 0.6.3's new tasks took it past the 20 tools Claude checks strictly in one question, and every question was refused.
- An update cut off part way (the handheld switched off, the installer stopped) no longer keeps the handheld from ever checking for updates again.

## 0.6.4

### Fixed
- Set up from a computer shows what the handheld has now (the look, the choices, which keys are set) and changes only what you change: sent to add one key, it no longer turns SSH off or puts the look back to grid, dark and standard text.
- Updates install much faster from the next one on: only the parts of the system that changed are written to the card, not all 1 GB.

## 0.6.3

### Home widgets
- Choose what the glance page and Home's grid show: next match, battery, robot, music, weather, reminders, timers and more, each small, wide or large. Press E on Home to arrange them from the keyboard.
- A new glance page to start with: next match, battery and robot, then music, reminders and timers.
- The island's battery and clock now match the widgets exactly, and the clock turns with the minute.

### Clipboard and quick capture
- Ctrl+C and Ctrl+V copy and paste everywhere (Ctrl+Shift+C and Ctrl+Shift+V in the Terminal); scripts can use catalyst-clip.
- Ctrl+Shift+H from anywhere shows your last 20 copies and your pins; Enter pastes one into what you're typing.
- Ctrl+Shift+N from anywhere jots a note or a reminder ("swap battery in 10 min") without leaving what you're doing; Ctrl+P adds a photo, and holding Alt dictates.

### Vault
- Passwords, Wi-Fi keys, card PINs, notes and 2FA codes, sealed on the card and opened with your passcode. It locks as you leave, as the screen goes off, or after two idle minutes.
- Bring passwords in from Bitwarden, 1Password, KeePass or an authenticator's otpauth list, and back up to a .cvault file under its own password.
- Copied passwords paste anywhere for 30 seconds, never go in the clipboard's history, and clear when the handheld locks. No screenshots while the vault is open.

### Assistant
- Does what you ask, several steps at once and offline too: timers, reminders, notes, music, settings, downloads. Anything that could change something asks first.
- "Brief me" (or Ctrl+B): the next match or the day, the robot, the battery, reminders and the last match's log, at a glance.
- Remembers what you tell it, in /data/assistant/memory.md, which you can read and forget item by item. It leaves the handheld only with a question to OpenAI or Claude.

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

# The CardputerZero app store, and what Catalyst took from it

M5Stack's app store for the CardputerZero is the **CardputerZero** organisation on GitHub
(`CardputerZero/Store`, `CardputerZero/packages`), which docs.m5stack.com links as the official app
repository. Its registry listed 136 apps on 2026-11-21. Each app's licence below was read from its own
LICENSE, its headers, or its package's copyright file; "store says" means only the store's listing
says it. Four were rebuilt for Catalyst with Catalyst Kit; their credits and licences are in their
folders under `src/` and on each app's page in the Catalogue.

| # | Package | App | What it does | Licence | Catalyst |
| --- | --- | --- | --- | --- | --- |
| 1 | `advancewebui` | AdvanceWebUI | web file manager and .deb installer over Wi-Fi or its own AP | store MIT; NO licence file; Template files carry MIT SPDX, main.cpp none | Catalyst has it, near enough: Files; and no licence file. |
| 2 | `ai-wars` | AI Wars | Drug Wars-style trading game | MIT (LICENSE: Template + randombits0110); art: "no separate asset license found" (docs/ASSET-LICENSING.md); intro.mp3 MIT (intro-LICENSE.txt) | Open (MIT code) and worth having; its 117 pictures have no licence, so a rebuild needs art of its own. Next in line. |
| 3 | `airfiles` | Airfiles | browser file drop/share by QR+PIN, AirDrop beta | MIT (LICENSE, JackyC) | Catalyst has it, near enough: Share and setup from a computer. |
| 4 | `airsim-cardputer` | AirSIM Cardputer | calls/SMS client for the AirSIM phone app | PolyForm-Noncommercial-1.0.0 (LICENSE + NOTICE); icon reused from AirSIM iOS | A client for an outside service or server; not first for a pit tool. |
| 5 | `amiibo-sim` | amiibo_sim | NTAG215 emulation on the Cap ST25R3916 | MIT (amiibo_sim/LICENSE in monorepo folder; repo root has none) | Needs hardware a CardputerZero does not have built in (a Cap, a radio, GPS, USB gadget or a second screen). |
| 6 | `aptool` | APTool | diagnostic Wi-Fi AP (NAT/bridge/VLAN) + LLDP | MIT (LICENSE, FPVZaphod) | Catalyst is not a router; its Net tools cover the checks. |
| 7 | `aria2-remote` | Aria2 Remote | aria2 JSON-RPC remote | store MIT; copyright file is only the Template MIT notice; no repo | A client for an outside service or server; not first for a pit tool. |
| 8 | `auracam` | AuraCam | camera + MLX90640 thermal overlay | MIT (LICENSE); vendor/melexis Apache-2.0 | Needs hardware a CardputerZero does not have built in (a Cap, a radio, GPS, USB gadget or a second screen). |
| 9 | `battleship` | Battleship | 10×10 vs AI, BLE or Wi-Fi | MIT (LICENSE has a preamble, so GitHub says NOASSERTION; holders M5Stack + yutacar) | **Rebuilt: Battleship** (against the handheld; its Bluetooth and Wi-Fi play left out). |
| 10 | `bitmap16dx` | BitMap16 DX | indexed-colour pixel-art sketchbook | MIT (LICENSE, Brad McNally); picojson BSD-2; Lospec palettes uncredited | Open (MIT); a pixel sketchbook. A candidate later. |
| 11 | `brucecardputerzero` | Bruce CardputerZero | Bruce-style multi-tool | AGPL-3.0-only (LICENSE + SPDX); Sub-GHz protocol files GPL-3.0-or-later; ArduinoJson MIT | An attack toolkit: not for a team's pit handheld. |
| 12 | `camillia` | Camillia for Meshtastic | Meshtastic node on Cap LoRa | GPL-3.0-or-later (LICENSE, README, SPDX) | Needs hardware a CardputerZero does not have built in (a Cap, a radio, GPS, USB gadget or a second screen). |
| 13 | `cardbili` | CardBili | Bilibili client | code MIT (LICENSE); shipped binary GPL-3.0-only because Slint is under its GPL option (THIRD_PARTY_NOTICES); ffmpeg LGPL-3.0 | A client for an outside service or server; not first for a pit tool. |
| 14 | `cardflash` | CardFlash | download OS images, write and verify USB sticks | MIT (GitLab ScarletPachyderm/cardflash LICENSE); star.png CC BY-SA 3.0 | Needs hardware a CardputerZero does not have built in (a Cap, a radio, GPS, USB gadget or a second screen). |
| 15 | `cardmesh` | CardMesh | MeshMonitor client + Meshtastic/MeshCore chat | MIT (LICENSE + appendix, hence NOASSERTION); Twemoji CC-BY 4.0 compiled in | Needs hardware a CardputerZero does not have built in (a Cap, a radio, GPS, USB gadget or a second screen). |
| 16 | `cardputer-player` | Cardplayer | Winamp-style player + NL radio | store MIT; repo 404; no licence in .deb | Sound: Catalyst's audio is being built separately. A candidate once it lands. |
| 17 | `cardputer-remote` | Remote | browser remote desktop | Apache-2.0 only in store metadata; no text | A client for an outside service or server; not first for a pit tool. |
| 18 | `cardputer-zero-geiger` | Geiger Counter | Geiger pulse counter | MIT (LICENSE) | Needs hardware a CardputerZero does not have built in (a Cap, a radio, GPS, USB gadget or a second screen). |
| 19 | `cardputer-zero-iptv` | Cardputer TV | IPTV/HLS/local video | NOASSERTION; no grant; WenQuanYi font Apache-2.0 | A client for an outside service or server; not first for a pit tool. |
| 20 | `cardputer-zero-radio` | WebRadio | Radio Browser internet radio | Proprietary (store); no licence | Not open source. |
| 21 | `cardputersdr` | CardputerSDR | Airspy spectrum/waterfall | MIT (cardputersdr/LICENSE) | Needs hardware a CardputerZero does not have built in (a Cap, a radio, GPS, USB gadget or a second screen). |
| 22 | `cardputerzero-astro-box` | AstroBox | 2D N-body sandbox, QR sharing | MIT (LICENSE "Copyright (c) 2026 eggfly"; store author is kur1k0) | **Rebuilt: Orbits** (its QR sharing left out). |
| 23 | `cardputerzero-pwnagotchi` | Pwnagotchi | dashboard for an installed pwnagotchi/bettercap | MIT (LICENSE, JdaieLin) | An attack toolkit: not for a team's pit handheld. |
| 24 | `cardputerzero-xiaozhi` | XiaoZhi | xiaozhi.me voice assistant | NO LICENSE file (README "MIT"); bundled _watercolor_rust.so GPL-3.0 | Catalyst has it: Assistant; and no licence file. |
| 25 | `cardrouter` | CardRouter | pocket router (NAT, forwards, QoS, WireGuard) | store MIT; no repo, no licence | No licence; Catalyst is not a router. |
| 26 | `chainled` | ChainLED | 8×8 LED animation editor, M5 Chain UART, BLE | MIT (LICENSE names only the Template holder; SPDX headers name yutacar) | Needs hardware a CardputerZero does not have built in (a Cap, a radio, GPS, USB gadget or a second screen). |
| 27 | `coinflip` | Coinflip | 3D coin toss | store MIT; no LICENSE, no repo | No licence file and no source. |
| 28 | `controller` | Controller | Bluetooth gamepad pair/tester | store MIT; no text | Needs hardware a CardputerZero does not have built in (a Cap, a radio, GPS, USB gadget or a second screen). |
| 29 | `craps` | Craps | Vegas craps | Proprietary ("All rights reserved", Software Amusements) | Not open source. |
| 30 | `crockogoldslots` | Crock O'Gold Slots | 6-reel slots | Proprietary (copyright file forbids redistribution) | Not open source. |
| 31 | `cz-battery-saver` | Battery Saver | power-saving service | MIT (LICENSE, InstantComet) | Catalyst has it: the battery saver. |
| 32 | `cz-desk-mode` | Desk Mode | HDMI desktop + keyboard handover | MIT | Needs hardware a CardputerZero does not have built in (a Cap, a radio, GPS, USB gadget or a second screen). |
| 33 | `cz-peers` | Peers | Tailscale companion TUI | MIT | Catalyst has it: Tailscale in Settings. |
| 34 | `czmail` | Email | IMAP/SMTP client | MIT (H4W9) | A client for an outside service or server; not first for a pit tool. |
| 35 | `czproxy` | CZProxy | mihomo (Clash Meta) panel | store MIT; no licence/repo; bundles mihomo v1.19.34 (GPL-3.0 at that tag) with no licence text or source offer | A client for an outside service or server; not first for a pit tool. |
| 36 | `doodycalls` | Doody Calls | 3D squad shooter | Proprietary | Not open source. |
| 37 | `dweebs-pets` | Dweebs Pets | virtual pet | Proprietary | Proprietary. |
| 38 | `esp32c5-spectrum` | ESP32-C5 Spectrum | 2.4/5 GHz spectrum via a USB ESP32-C5 | GPL-3.0-or-later (SPDX + LICENSE); fonts OFL | Needs hardware a CardputerZero does not have built in (a Cap, a radio, GPS, USB gadget or a second screen). |
| 39 | `five` | Five | Gomoku vs Rapfi or LAN | own code proprietary (README); Rapfi GPL-3.0-or-later (source in .deb); weights CC0 | Its own code is proprietary. |
| 40 | `flagpi` | Semaphore旗语 | flag semaphore learn/quiz | store MIT; no text | No licence the source backs up. |
| 41 | `flight-club` | Flight Club | flight arcade | MIT (LICENSE, Cargo.toml) | Open (MIT, Rust); a flight arcade. A candidate later. |
| 42 | `flightzero` | flightZero | internet ADS-B radar | Proprietary (store) | Not open source. |
| 43 | `flint` | Flint | ~100-tool offensive suite | CONFLICT: store MIT vs about.py `_LICENSE = "GPL-3.0"`; repo 404; no licence file | An attack toolkit: not for a team's pit handheld. |
| 44 | `flipper-one-remote` | FlipCTL | Flipper One mirror/control | GPL-3.0-only (LICENSE, copyright); icon MIT (Flipper) | Needs hardware a CardputerZero does not have built in (a Cap, a radio, GPS, USB gadget or a second screen). |
| 45 | `fontpreview` | FontPreview | preview installed fonts | MIT; SFX CC0-1.0 (uisfx) | Open (MIT), but Catalyst carries two fonts only. |
| 46 | `frame` | Frame | YouTube client | store MIT; repo 404; no LICENSE | A client for an outside service or server; not first for a pit tool. |
| 47 | `fxcam` | FXCam | camera with 106 filters | MIT (imliubo + M5Stack); Inter OFL; shutter.wav not stated | Catalyst has it: Camera (filters aside). |
| 48 | `geminilive` | Gemini Live | Gemini Live voice assistant | store MIT; repo 404; no licence | Catalyst has it: Assistant; and no licence. |
| 49 | `hamlog` | HamLog | ham QSO logger | NO licence (only app-builder.json "MIT"); fonts and flags provenance not stated | No licence the source backs up. |
| 50 | `herdr-zero` | herdr-zero | Herdr remote terminal/agent client | app MIT (Template text); repo 404; Rime data LGPL-3.0+ | A client for an outside service or server; not first for a pit tool. |
| 51 | `icomzero` | ICOMzero | Icom/Kenwood LAN client + FT8 | GPL-3.0-only (LICENSE, README); ft8_lib MIT | Needs hardware a CardputerZero does not have built in (a Cap, a radio, GPS, USB gadget or a second screen). |
| 52 | `jiggler` | Jiggler | USB HID jiggler | MIT (intGus) | Needs hardware a CardputerZero does not have built in (a Cap, a radio, GPS, USB gadget or a second screen). |
| 53 | `kanstl` | KanSTL | STL viewer | GPL-3.0 (only vs or-later not determined); embedded Terminus font has no notice | Open (GPL-3.0); an STL viewer. A candidate later, kept under the GPL. |
| 54 | `kesha-llm` | Kesha Code LLM | llama.cpp coding/chat + YOLOX | MIT; YOLOX Apache-2.0 | Catalyst has it: Assistant and the on-device model. |
| 55 | `key-dash` | taM5CZTyping | typing game | MIT (Template-holder LICENSE); StackChan images: no licence stated | **Rebuilt: Typing** (its online leaderboard and mascot art left out; the art has no licence). |
| 56 | `kimi-remote` | KimiRC | Kimi Code remote client | MIT | A client for an outside service or server; not first for a pit tool. |
| 57 | `klipper-deck-zero` | KlipperDeckZero | Klipper/Moonraker control | MIT (denchi-45) | A client for an outside service or server; not first for a pit tool. |
| 58 | `kvxputerzero` | kvxputerzero | kvxputer (Bruce-derived) port | store AGPL-3.0; Zero-port source not public (repo is the ESP32 firmware) | An attack toolkit: not for a team's pit handheld. |
| 59 | `lan-proxy` | LAN Proxy | route TCP/DNS through a LAN proxy | NO licence (copyright line only) | No licence; not a pit tool. |
| 60 | `launcherespflasher` | Launcher ESP Flasher | flash ESP32 firmware from LauncherHub | MIT (LICENSE; the repo holds no code); bootloader .bin provenance not stated | Needs hardware a CardputerZero does not have built in (a Cap, a radio, GPS, USB gadget or a second screen). |
| 61 | `lofibox-cardputer-zero` | LoFiBox | full music player | GPL-3.0-or-later (LICENSE, COPYING, SPDX) | Sound: Catalyst's audio is being built separately. A candidate once it lands. |
| 62 | `m-pocket` | M-Pocket | Mastodon client | MIT | A client for an outside service or server; not first for a pit tool. |
| 63 | `meshcore-zero` | MeshCore | standalone MeshCore node | MIT; openHop MIT | Needs hardware a CardputerZero does not have built in (a Cap, a radio, GPS, USB gadget or a second screen). |
| 64 | `meshterm` | MeshTerm | MeshCore TUI | Apache-2.0 (+ NOTICE: name/logo not licensed) | Needs hardware a CardputerZero does not have built in (a Cap, a radio, GPS, USB gadget or a second screen). |
| 65 | `meshzero` | MeshZero | Meshtastic UI over meshtasticd | GPL-3.0-only | Needs hardware a CardputerZero does not have built in (a Cap, a radio, GPS, USB gadget or a second screen). |
| 66 | `mimo-talk` | MiMo Talk | Xiaomi MiMo voice chat | MIT (mimo_talk/LICENSE) | Catalyst has it: Assistant (voice is being built separately). |
| 67 | `mod-player` | Mod Player | tracker-module player | MIT | Sound: Catalyst's audio is being built separately. A candidate once it lands. |
| 68 | `monster-cyberspace` | Monster for Cyberspace | cyberspace.online client | GPL-3.0-or-later | A client for an outside service or server; not first for a pit tool. |
| 69 | `mountable` | Mountable | present ISO/disk image as a USB drive | GPL-3.0-only (copyright file) | Needs hardware a CardputerZero does not have built in (a Cap, a radio, GPS, USB gadget or a second screen). |
| 70 | `mouse-jiggler` | Mouse Jiggler | Bluetooth HID mouse jiggler | MIT (sibling repo kkazakov/cardputer-zero-mouse-jiggler; listed repo 404) | Needs hardware a CardputerZero does not have built in (a Cap, a radio, GPS, USB gadget or a second screen). |
| 71 | `muser` | Muser | Meta Muse voice companion | Apache-2.0; Jollybot avatar EXCLUDED (Meta artwork) | Catalyst has it: Assistant; its avatar is Meta's. |
| 72 | `musictagger` | MusicTagger | cover/lyrics tagging | store MIT; no licence; bundled mutagen GPL-2.0+ | Sound: Catalyst's audio is being built separately. A candidate once it lands. |
| 73 | `nettoolkit` | NET Toolkit | network toolbox | store MIT; no licence | Catalyst has it: Net tools; and no licence. |
| 74 | `nfcqsl` | NFCQSL | ham QSL NFC cards | MIT (LICENSE only inside .deb; the repo lacks the code) | Needs hardware a CardputerZero does not have built in (a Cap, a radio, GPS, USB gadget or a second screen). |
| 75 | `ns-con` | NS-Con | wired Switch controller (USB gadget) | store MIT; no licence; listed repo does not contain it | Needs hardware a CardputerZero does not have built in (a Cap, a radio, GPS, USB gadget or a second screen). |
| 76 | `octet` | Octet Desktop | windowed desktop, ~48 apps | Proprietary (personal non-commercial) | Not open source. |
| 77 | `opencartazero` | OpenCartaZero | BLE vaporiser controller | GPL-3.0-or-later | Needs hardware a CardputerZero does not have built in (a Cap, a radio, GPS, USB gadget or a second screen). |
| 78 | `pixelaces` | Pixel Aces | pixel-3D dogfight | store MIT; no licence/repo; port of @creativedash's game | No licence the source backs up. |
| 79 | `plai` | Plai | Meshtastic node | GPL-3.0 text in .deb (store says or-later); no source for this build | Needs hardware a CardputerZero does not have built in (a Cap, a radio, GPS, USB gadget or a second screen). |
| 80 | `pocket-lorascan` | LoRa Scanner | RX-only LoRa survey | MIT (Duane Sweazy) | Needs hardware a CardputerZero does not have built in (a Cap, a radio, GPS, USB gadget or a second screen). |
| 81 | `pocket-sattrack` | Sat Tracker | SGP4 pass predictor | MIT | Needs hardware a CardputerZero does not have built in (a Cap, a radio, GPS, USB gadget or a second screen). |
| 82 | `pocket-sysmon` | SysMon | system monitor | MIT | Catalyst has it: Monitor. |
| 83 | `pocket-trail` | Trail Recorder | GPS to GPX | MIT | Needs hardware a CardputerZero does not have built in (a Cap, a radio, GPS, USB gadget or a second screen). |
| 84 | `pocket-zero-tools` | Tools | 13 hardware tools | MIT | Brings a runtime or emulator of its own, many megabytes; not a native app. |
| 85 | `r86` | r86 | x86 PC emulator | Apache-2.0; SeaBIOS LGPL-3.0; FreeDOS mostly GPL-2.0 | Brings a runtime or emulator of its own, many megabytes; not a native app. |
| 86 | `raspyjack` | RaspyJack | offensive toolkit | MIT root; Responder GPL-3.0; prebuilt ELFs with no source; Arial fonts, copyrighted GIFs and cloned brand pages shipped | An attack toolkit: not for a team's pit handheld. |
| 87 | `refract-pocket` | REFRACT Pocket | ZX Spectrum Next emulator | Proprietary | Not open source. |
| 88 | `retro-zero` | Retro-Zero | libretro frontend | NO licence (its copyright file says the author must still choose one); store MIT | Brings a runtime or emulator of its own, many megabytes; not a native app. |
| 89 | `rf-heatmap` | RF Heatmap | CC1101 sweep/decode | MIT | Needs hardware a CardputerZero does not have built in (a Cap, a radio, GPS, USB gadget or a second screen). |
| 90 | `rfid` | RFID | NFC/UHF tool + mfkey | NO licence file; bundled crapto1 GPL-3.0-or-later | Needs hardware a CardputerZero does not have built in (a Cap, a radio, GPS, USB gadget or a second screen). |
| 91 | `rubiks2x2x2-autosolver` | Rubiks 2x2x2 | 2×2×2 cube + solver | MIT (Template text); legacy solver files from the author's unlicensed repo | Open (MIT) and worth having: a 3D cube on the processor. Next in line. |
| 92 | `sondechase` | SondeChase | RS41 radiosonde chaser | GPL-3.0-or-later | Needs hardware a CardputerZero does not have built in (a Cap, a radio, GPS, USB gadget or a second screen). |
| 93 | `sparrow` | Sparrow | offline almanac | MIT text in .deb, no source; HYG stars CC BY-SA 4.0; voice has non-commercial lineage | No source published. |
| 94 | `subghz` | SubGHz | Flipper-style Sub-GHz | MIT | Needs hardware a CardputerZero does not have built in (a Cap, a radio, GPS, USB gadget or a second screen). |
| 95 | `sudokuzero` | Sudoku | Sudoku | MIT (code); font is Alibaba PuHuiTi under Alibaba's own free-commercial licence | **Rebuilt: Sudoku.** |
| 96 | `tailputerzero` | Tailputer Zero | Tailscale up/down + QR login | MIT | Catalyst has it: Tailscale in Settings. |
| 97 | `tailzero` | TailZero | Tailscale manager | MIT | Catalyst has it: Tailscale in Settings. |
| 98 | `talktome` | TalkToMe | offline TTS (EN/UK) | MIT (+ third-party note, hence NOASSERTION) | Sound: Catalyst's audio is being built separately. A candidate once it lands. |
| 99 | `telega` | Telega | Telegram (TDLib) | MIT text in .deb; repo 404 | A client for an outside service or server; not first for a pit tool. |
| 100 | `tic80-c0` | TIC-80 | TIC-80 port | MIT (Vadim Grigoruk) | Brings a runtime or emulator of its own, many megabytes; not a native app. |
| 101 | `tyrell-video` | Tyrell Video | video player LCD/HDMI | MIT text in .deb; no repo | A client for an outside service or server; not first for a pit tool. |
| 102 | `ultimate-flasher-cardputerzero` | Ultimate Flasher | ESP32/Flipper flasher | MIT (LAB5); vendored esptool GPL-2.0+ | Needs hardware a CardputerZero does not have built in (a Cap, a radio, GPS, USB gadget or a second screen). |
| 103 | `ultimate-remote` | Ultimate Remote | universal IR remote | MIT + MakeHex notice + irdb "used under permission" (irdb terms are restrictive) | Catalyst has it: Remote (IR). |
| 104 | `vibeoffice` | VibeOffice | docx/xlsx/pptx suite | Apache-2.0 | Brings a runtime or emulator of its own, many megabytes; not a native app. |
| 105 | `wallpaper` | Wallpaper | change boot splash/sound/lockscreen | NO licence; build_splash.py not in repo | Catalyst has its own looks; and no licence. |
| 106 | `wanwu-desktop` | Desktop | mirror labwc desktop to LCD | store MIT; no text | Needs hardware a CardputerZero does not have built in (a Cap, a radio, GPS, USB gadget or a second screen). |
| 107 | `wanwu-usbscreen` | USB Screen | drive an MSU2 160×80 USB screen | store MIT; no text | Needs hardware a CardputerZero does not have built in (a Cap, a radio, GPS, USB gadget or a second screen). |
| 108 | `wanwu-wireless` | Wireless Toolbox | net/BLE/Wi-Fi audit tools | store MIT; no text | An attack toolkit: not for a team's pit handheld. |
| 109 | `wasteland` | Wasteland | co-op hex-crawl for phone browsers | MIT (LICENSE in .deb); FreeFont GPL+font exception; upstream repo has no licence | A client for an outside service or server; not first for a pit tool. |
| 110 | `wifi-manager` | WiFi USB | Wi-Fi adapter picker | NO LICENSE file; README + SConstruct SPDX say MIT | Catalyst has it: Settings > Network. |
| 111 | `wirezero` | WireZero | WireGuard via NetworkManager | MIT | Needs NetworkManager, which Catalyst does not use; Tailscale covers it. |
| 112 | `xianjian1-zero` | 仙剑1·Zero | SDLPAL port (no game data) | wrapper MIT; SDLPAL GPL-3.0-only (files inside the source zips) | Brings a runtime or emulator of its own, many megabytes; not a native app. |
| 113 | `zero-browser` | Zero Browser | web browser | MIT; bundled Qt LGPL | Catalyst has it: Web and Browse. |
| 114 | `zero-music` | Zero Music | player + MyGo plugins | AGPL-3.0-only | Sound: Catalyst's audio is being built separately. A candidate once it lands. |
| 115 | `zero-music163` | ZeroMusic163 | NetEase Cloud Music | MIT | Sound: Catalyst's audio is being built separately. A candidate once it lands. |
| 116 | `zero-news` | News | RSS headlines | MIT | A client for an outside service or server; not first for a pit tool. |
| 117 | `zero-ssh` | Zero SSH | SSH + web console | MIT | Catalyst has it: Connect (SSH) and Terminal. |
| 118 | `zero-stocks` | Stocks | Yahoo Finance watchlist | MIT | A client for an outside service or server; not first for a pit tool. |
| 119 | `zero-usbimager` | USB Imager | bzt USBImager port | MIT (upstream, DEP-5); repo 404 | Needs hardware a CardputerZero does not have built in (a Cap, a radio, GPS, USB gadget or a second screen). |
| 120 | `zero-watt` | Zero Watt | battery power monitor | MIT | Catalyst has it: Settings > Battery. |
| 121 | `zero-weather` | Weather | Open-Meteo + GPS | MIT | Catalyst has it: Weather. |
| 122 | `zeroamp` | ZeroAmp | Winamp-like player | "Unlicense" only in store metadata | Sound: Catalyst's audio is being built separately. A candidate once it lands. |
| 123 | `zerodaw` | Zero DAW | pattern DAW | Proprietary | Not open source. |
| 124 | `zerodesk` | ZeroDesk | HTTPS remote control | MIT; fonts OFL | A client for an outside service or server; not first for a pit tool. |
| 125 | `zerogallery` | ZeroGallery | photo/video viewer | MIT | Catalyst has it: Photos. |
| 126 | `zerogotchi` | Zerogotchi | Wi-Fi capture/wardrive/mesh mapping | MIT (README says the theme art too) | An attack toolkit: not for a team's pit handheld. |
| 127 | `zeroradio` | ZeroRadio | RTL-SDR suite | MIT; bundled readsb/AIS-catcher GPL-3.0 | Needs hardware a CardputerZero does not have built in (a Cap, a radio, GPS, USB gadget or a second screen). |
| 128 | `zerosdr` | ZeroSDR | RTL-SDR + ADS-B | GPL-3.0; readsb GPL-3.0+ | Needs hardware a CardputerZero does not have built in (a Cap, a radio, GPS, USB gadget or a second screen). |
| 129 | `zerosend` | ZeroSend | LocalSend v2 client | MIT | Catalyst has Share; LocalSend needs a TLS server of its own. A candidate later. |
| 130 | `zerosky` | ZeroSky | AR planetarium | MIT code; stars/DSO CC BY-SA 4.0; constellations BSD-3 + CC BY 4.0; cities CC BY 4.0; WMM public domain | Open (MIT code; star data CC BY-SA 4.0); needs the location and the compass. A candidate later. |
| 131 | `zerosub` | ZeroSub | CC1101/IR/NFC/motion kit | MIT | Needs hardware a CardputerZero does not have built in (a Cap, a radio, GPS, USB gadget or a second screen). |
| 132 | `zerotag` | ZeroTag | amiibo library/emulator | MIT code; 954 Nintendo thumbnails, no licence | Needs hardware a CardputerZero does not have built in (a Cap, a radio, GPS, USB gadget or a second screen). |
| 133 | `zerotracker` | ZeroTracker | M8-style tracker | GPL-3.0-or-later; demo content CC BY-NC-SA 4.0, samples CC0 | Sound: Catalyst's audio is being built separately. A candidate once it lands. |
| 134 | `zerotvbgone` | ZeroTVBGone | TV-B-Gone | MIT AND CC-BY-SA-3.0 (code table) | Catalyst has it: Remote (IR). |
| 135 | `ziiro` | Ziiro | amiibo catalogue + NFC | NO licence (RELEASE.md: "MIT proposed ... pending maintainer approval") | Needs hardware a CardputerZero does not have built in (a Cap, a radio, GPS, USB gadget or a second screen). |
| 136 | `zsynth` | ZSynth | 4-track groovebox | MIT; samples CC0 (Versilian) | Sound: Catalyst's audio is being built separately. A candidate once it lands. |

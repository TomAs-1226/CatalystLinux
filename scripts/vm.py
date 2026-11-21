#!/usr/bin/env python3
"""Boot the SD card image in a whole-machine emulator and look inside it.

QEMU's Raspberry Pi 3A+ stands in for the CardputerZero: the same processor
family and 512 MB, running the image's own kernel, device tree and root
filesystem. The panel, keyboard and radio are not there, so the firmware's
framebuffer (set to 320 x 170) stands in for the panel and a USB keyboard for
the keys. Run inside WSL after build-image.sh and wsl-qemu.sh:

    python3 scripts/vm.py              # boot, measure memory and boot time
    python3 scripts/vm.py --tour DIR   # also drive the UI, check it, save what it drew
    python3 scripts/vm.py --verbose    # show the serial console as it boots

Emulated time is not device time: the boot figures compare one build with
another, and say nothing certain about seconds on the handheld.
"""
import argparse
import os
import re
import select
import shutil
import socket
import subprocess
import sys
import time

BUILD = os.environ.get("CATALYST_BUILD", os.path.expanduser("~/catalyst-linux-build"))
IMAGES = f"{BUILD}/output/images"
QEMU = f"{BUILD}/tools/qemu"
WORK = f"{BUILD}/vm"
PROMPT = "vm# "


def device_tree():
    """The image's device tree as the boot firmware would hand it over.

    On the handheld the firmware applies `dtoverlay=disable-bt` and then names
    the freed serial port serial0 (ttyAMA0). The emulator has no firmware, so
    the same two steps are done here with the kernel's own tools."""
    tools = f"{BUILD}/output/build/linux-custom/scripts/dtc"
    freed, text, out = f"{WORK}/freed.dtb", f"{WORK}/vm.dts", f"{WORK}/vm.dtb"
    subprocess.run([f"{tools}/fdtoverlay", "-i", f"{IMAGES}/bcm2710-rpi-cm0.dtb", "-o", freed, f"{IMAGES}/rpi-firmware/overlays/disable-bt.dtbo"], check=True)
    dts = subprocess.run([f"{tools}/dtc", "-q", "-I", "dtb", "-O", "dts", freed], check=True, capture_output=True, text=True).stdout
    dts = dts.replace('serial0 = "/soc/serial@7e215040";', 'serial0 = "/soc/serial@7e201000";', 1)
    dts = dts.replace('serial1 = "/soc/serial@7e201000";', 'serial1 = "/soc/serial@7e215040";', 1)
    # The emulator does not model the power-domain registers (the "asb" block)
    # and the kernel faults reading them; without them only the watchdog is set up.
    dts, n = re.subn(r"reg = <0x7e100000 0x114 0x7e00a000 0x24>;", "reg = <0x7e100000 0x114>;", dts)
    dts = dts.replace('reg-names = "pm", "asb";', 'reg-names = "pm";')
    if n != 1:
        print("warning: the power block was not where the emulator's device tree expects it", file=sys.stderr)
    open(text, "w").write(dts)
    subprocess.run([f"{tools}/dtc", "-q", "-I", "dts", "-O", "dtb", "-o", out, text], check=True)
    return out


class Machine:
    def __init__(self, fresh=True, extra_args=(), quiet=True):
        os.makedirs(WORK, exist_ok=True)
        self.card = f"{WORK}/card.img"
        if fresh or not os.path.exists(self.card):
            # Sparse: system B and most of the rest are empty.
            subprocess.run(["cp", "--sparse=always", f"{IMAGES}/sdcard.img", self.card], check=True)
            # The emulated card reader wants a power-of-two size; the spare
            # room is what the first boot grows /data into. CATALYST_CARD_GB
            # makes it a card of that size (sparse), e.g. 128 like Thomas's.
            image = os.path.getsize(self.card)
            size = 1 << (image + (512 << 20) - 1).bit_length()
            if os.environ.get("CATALYST_CARD_GB"):
                size = max(size, int(os.environ["CATALYST_CARD_GB"]) << 30)
            with open(self.card, "r+b") as card:
                card.truncate(size)
        self.monitor_path = f"{WORK}/monitor"
        self.extra_args = extra_args
        self.quiet = quiet
        self.log = open(f"{WORK}/serial.log", "wb")
        self.boots = 0
        self.lines = []
        self.power_on()

    def timeline(self):
        """The last boot's service messages with how long each step took."""
        steps = [(t, line) for boot, t, line in self.lines if boot == self.boots and line.startswith("* ") and "login" not in line]
        out = []
        for (t, line), (after, _) in zip(steps, steps[1:] + [(steps[-1][0], "")] if steps else []):
            out.append(f"  {t:6.1f} s  +{after - t:4.1f}  {line[2:70]}")
        return "\n".join(out)

    def power_on(self):
        """Start the machine on the card as it now is."""
        quiet, extra_args = self.quiet, self.extra_args
        if os.path.exists(self.monitor_path):
            os.remove(self.monitor_path)
        dtb = device_tree()
        cmdline = open(f"{IMAGES}/rpi-firmware/cmdline.txt").read().strip()
        if not quiet:
            cmdline = cmdline.replace("loglevel=3", "loglevel=7")
        # The emulator has no boot firmware to size its framebuffer.
        cmdline += " bcm2708_fb.fbwidth=320 bcm2708_fb.fbheight=170 bcm2708_fb.fbdepth=16"
        env = dict(os.environ, LD_LIBRARY_PATH=f"{QEMU}/usr/lib/x86_64-linux-gnu")
        args = [
            f"{QEMU}/usr/bin/qemu-system-aarch64", "-L", f"{QEMU}/usr/share/qemu",
            "-M", "raspi3ap", "-kernel", f"{IMAGES}/Image", "-dtb", dtb,
            "-drive", f"file={self.card},if=sd,format=raw", "-append", cmdline,
            "-display", "none", "-serial", "stdio", "-monitor", f"unix:{self.monitor_path},server,nowait",
            "-device", "usb-kbd", "-device", "usb-net,netdev=n0", "-netdev", "user,id=n0",
            # The emulator does not come back from a guest restart: stop
            # instead, and power_on() again, which is what a restart is.
            "-no-reboot",
            *extra_args,
        ]
        self.boots += 1
        self.started = time.time()
        self.qemu = subprocess.Popen(args, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, env=env)
        self.buffer = b""

    def read_until(self, pattern, timeout):
        """Everything the serial console says until `pattern` appears."""
        deadline = time.time() + timeout
        wanted = re.compile(pattern.encode())
        while True:
            found = wanted.search(self.buffer)
            if found:
                out, self.buffer = self.buffer[: found.end()], self.buffer[found.end():]
                return out.decode(errors="replace")
            left = deadline - time.time()
            if self.qemu.poll() is not None and self.boots < 3:
                # The guest restarted itself (the first boot does, to grow /data).
                self.buffer = b""
                self.power_on()
                deadline = time.time() + timeout
                continue
            if left <= 0 or self.qemu.poll() is not None:
                raise TimeoutError(f"waited {timeout}s for {pattern!r}; last output:\n{self.buffer[-2000:].decode(errors='replace')}")
            if select.select([self.qemu.stdout], [], [], min(left, 1.0))[0]:
                chunk = os.read(self.qemu.stdout.fileno(), 65536)
                self.log.write(chunk)
                self.log.flush()
                if not self.quiet:
                    sys.stdout.write(chunk.decode(errors="replace"))
                    sys.stdout.flush()
                self.buffer += chunk
                # When each line arrived, for the boot timeline.
                now = time.time() - self.started
                for line in chunk.decode(errors="replace").split("\n"):
                    line = re.sub(r"\x1b\[[0-9;]*[A-Za-z]", "", line).strip()
                    if line:
                        self.lines.append((self.boots, now, line))

    def send(self, text):
        self.qemu.stdin.write(text.encode())
        self.qemu.stdin.flush()

    def login(self, timeout=420):
        """Wait for the login prompt (through the first boot's reboot, if it
        happens) and sign in. Returns seconds of wall time to the prompt."""
        while True:
            self.read_until(r"login: ", timeout)
            took = time.time() - self.started
            self.send("root\n")
            time.sleep(1.0)
            self.send(f"PS1='{PROMPT}'; stty -echo cols 250\n")
            self.read_until(re.escape(PROMPT), 30)
            time.sleep(0.3)
            self.drain()
            # A first boot that is about to restart to grow /data: wait for the real one.
            if "pending" in self.run("[ -e /data/.grow-pending ] && echo pending"):
                continue
            # The panel is not there. The firmware's framebuffer, sized like
            # it, stands in: the shell (which keeps retrying) finds it.
            self.run("modprobe bcm2708_fb; sleep 1; rc-service catalyst-shell start >/dev/null 2>&1; pidof catalyst-shell >/dev/null || echo 'the shell did not start'", 180)
            return took

    def drain(self):
        while select.select([self.qemu.stdout], [], [], 0.3)[0]:
            chunk = os.read(self.qemu.stdout.fileno(), 65536)
            self.log.write(chunk)
        self.buffer = b""

    def run(self, command, timeout=60):
        """Run a command in the machine and return what it printed."""
        self.send(command + "\n")
        out = self.read_until(re.escape(PROMPT), timeout)
        return out[: -len(PROMPT)].replace("\r", "").strip("\n")

    def monitor(self, command):
        with socket.socket(socket.AF_UNIX) as s:
            s.connect(self.monitor_path)
            s.settimeout(5)
            time.sleep(0.2)
            s.sendall((command + "\n").encode())
            time.sleep(0.5)
            try:
                return s.recv(65536).decode(errors="replace")
            except socket.timeout:
                return ""

    def key(self, name, pause=0.9):
        """Press a key on the emulated USB keyboard (QEMU key names)."""
        self.monitor(f"sendkey {name}")
        time.sleep(pause)

    def screenshot(self, path):
        """Save the panel as a PNG; returns how many different colours are on it."""
        ppm = f"{WORK}/screen.ppm"
        self.monitor(f"screendump {ppm}")
        time.sleep(0.5)
        ppm_to_png(ppm, path)
        _, _, pixels = read_ppm(open(ppm, "rb").read())
        return len({pixels[i:i + 3] for i in range(0, len(pixels), 3)})

    def stop(self):
        try:
            self.monitor("quit")
            self.qemu.wait(10)
        except Exception:
            self.qemu.kill()


def read_ppm(data):
    """A binary PPM's width, height and pixels. Its header is four fields
    (P6, width, height, the largest value), then exactly one byte of white
    space; the pixels after it may begin with bytes that look like white space
    (a dark picture), which a plain split would eat."""
    fields, i = [], 0
    while len(fields) < 4:
        while data[i:i + 1].isspace():
            i += 1
        j = i
        while j < len(data) and not data[j:j + 1].isspace():
            j += 1
        fields.append(data[i:j])
        i = j
    w, h = int(fields[1]), int(fields[2])
    return w, h, data[i + 1:i + 1 + w * h * 3]


def ppm_to_png(ppm, png, scale=3):
    """Write a binary PPM as a PNG, enlarged, with nothing but the standard library."""
    import struct
    import zlib
    w, h, pixels = read_ppm(open(ppm, "rb").read())
    rows = []
    for y in range(h):
        row = pixels[y * w * 3:(y + 1) * w * 3]
        wide = b"".join(row[x * 3:x * 3 + 3] * scale for x in range(w))
        rows.extend([b"\0" + wide] * scale)

    def chunk(kind, body):
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body))

    with open(png, "wb") as out:
        out.write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w * scale, h * scale, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(b"".join(rows), 6)) + chunk(b"IEND", b""))


def meminfo(text):
    return {m.group(1): int(m.group(2)) for m in re.finditer(r"^(\w+(?:\(\w+\))?):\s+(\d+)", text, re.M)}


def measure(vm):
    """Memory at rest, where it goes, and how the boot was spent."""
    out = {}
    mem = meminfo(vm.run("cat /proc/meminfo"))
    out["mem"] = mem
    out["dmesg_memory"] = vm.run("dmesg | grep -E 'Memory:|Kernel command|Freeing unused' | cut -c1-200")
    out["processes"] = vm.run("ps -o pid,vsz,rss,args | grep -v ' \\[' | sort -k3 -n -r | head -14")
    out["shell"] = vm.run("p=$(pidof catalyst-shell); echo pid $p; grep -E 'Rss|Pss|Anonymous|Private' /proc/$p/smaps_rollup 2>/dev/null")
    out["modules"] = vm.run("cat /proc/modules | awk '{ s += $2; n++ } END { print n \" loaded, \" int(s / 1024) \" kB\" }'; cut -d' ' -f1,2 /proc/modules | sort -k2 -n -r | head -12 | tr '\\n' ';'")
    out["slab"] = vm.run("sort -k3 -n -r /proc/slabinfo 2>/dev/null | head -0; awk '/^Slab|^SReclaimable|^SUnreclaim|^KernelStack|^PageTables|^Percpu|^Shmem:|^AnonPages|^Mapped|^Cached|^Buffers/' /proc/meminfo | tr -s ' ' | tr '\\n' ';'")
    out["services"] = vm.run("rc-status -a 2>/dev/null | grep -vE '^ *$' | tr -s ' ' | head -60")
    out["boot"] = vm.run("dmesg | grep -E 'Run /sbin/init|Freeing unused kernel|EXT4-fs \\(mmcblk0p2\\): mounted|random: crng init' | cut -c1-120; cat /proc/uptime")
    out["mounts"] = vm.run("grep -E ' / | /data | /boot ' /proc/mounts; df -k / /data | tail -2")
    out["net"] = vm.run("ip -o -4 addr | awk '{ print $2, $4 }' | tr '\\n' ';'")
    out["failed"] = vm.run("dmesg | grep -iE 'error|failed|oops|abort|cannot' | cut -c1-150 | head -20")
    out["slab_top"] = vm.run("awk 'NR > 2 { kb = $3 * $4 / 1024; if (kb > 150) printf \"%6d kB  %s\\n\", kb, $1 }' /proc/slabinfo | sort -rn | head -22")
    out["timeline"] = vm.timeline()
    return out


def report(m, to_login):
    mem = m["mem"]
    total, free, avail = mem["MemTotal"], mem["MemFree"], mem["MemAvailable"]
    used = total - avail
    print(f"boot to login prompt: {to_login:.0f} s of emulator time (first boot includes growing /data and a restart)")
    print(f"memory the kernel manages: {total / 1024:.1f} MB; available {avail / 1024:.1f} MB; in use at rest {used / 1024:.1f} MB")
    print(f"  (free {free / 1024:.1f} MB, file cache {mem['Cached'] / 1024:.1f} MB, kernel slab {mem['Slab'] / 1024:.1f} MB, programs' own {mem['AnonPages'] / 1024:.1f} MB)")
    for key in ("dmesg_memory", "slab", "slab_top", "modules", "shell", "processes", "boot", "timeline", "mounts", "net", "services", "failed"):
        print(f"-- {key}\n{m[key]}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--tour", metavar="DIR", help="drive the UI from the keyboard, check it, and save screenshots here")
    ap.add_argument("--keep", action="store_true", help="reuse the card from the last run (a second boot)")
    ap.add_argument("--verbose", action="store_true")
    ap.add_argument("--run", action="append", default=[], help="a command to run in the machine after the measurements")
    opts = ap.parse_args()
    vm = Machine(fresh=not opts.keep, quiet=not opts.verbose)
    failed = 0
    try:
        to_login = vm.login()
        # Let the services finish and the shell settle before measuring.
        time.sleep(20)
        vm.drain()
        report(measure(vm), to_login)
        for command in opts.run:
            print(f"-- {command}\n{vm.run(command, 120)}")
        if opts.tour:
            os.makedirs(opts.tour, exist_ok=True)
            failed = tour(vm, opts.tour)
            failed += tools(vm)
            failed += updates(vm)
    finally:
        vm.stop()
    sys.exit(1 if failed else 0)


# Home's built-in apps, and Terminal's place among them (screens.rs, APPS).
BUILT_IN = 33
TERMINAL = 8


def tools(vm):
    """What this image adds for a developer and for the AprilTag reader, USB
    and the Remote app: each there and answering. Returns how many checks
    failed."""
    failed = 0

    def check(what, ok, detail=""):
        nonlocal failed
        print(f"   {'ok   ' if ok else 'FAIL '} {what}{': ' + str(detail) if detail else ''}")
        failed += 0 if ok else 1

    print("== the image's tools")
    out = vm.run("python3 -c 'import sys, json, curses, ssl, sqlite3, zlib, readline, xml.parsers.expat, lzma, bz2; print(sys.version.split()[0])'", 60)
    check("Python 3 runs, with its common modules (ssl, sqlite3, curses, zlib...)", out[:1] == "3", out.splitlines()[-1] if out else "")
    for command, want in [
        ("vim --version | head -n1", "VIM"),
        ("nano --version | head -n1", "nano"),
        ("htop --version", "htop"),
        ("tmux -V", "tmux"),
        ("rsync --version | head -n1", "rsync"),
        ("jq --version", "jq"),
        ("strace -V | head -n1", "strace"),
        ("i2cdetect -V 2>&1", "i2cdetect"),
        ("picocom --help 2>&1 | head -n1", "picocom"),
        ("dbclient -h 2>&1 | head -n2", "Dropbear"),
        ("dropbearkey -h 2>&1 | head -n2", "dropbearkey"),
    ]:
        out = vm.run(command, 60)
        check(f"{command.split()[0]} is there", want.lower() in out.lower(), out.splitlines()[0] if out else "")
    out = vm.run("catalyst-camera; echo exit $?", 60)
    check("the camera program runs and says there is no camera", "exit 1" in out and "camera" in out, " | ".join(out.splitlines()[-2:]))
    out = vm.run("rc-status default | grep catalyst-usb | tr -s ' '")
    check("the USB service starts (charging only by default)", "started" in out, out)
    out = vm.run("catalyst-usb status")
    check("catalyst-usb says how it stands", out.startswith("off"), out)
    out = vm.run("busybox --list | grep -x udhcpd")
    check("BusyBox has its DHCP server for USB networking", out.strip() == "udhcpd", out)
    # HTTPS with curl's own defaults, as Weather, the schedule and the
    # assistant ask (the emulator's network reaches the internet): a curl
    # with no certificate list fails every request (60), as 0.4.1's did.
    out = vm.run("curl -sS -o /dev/null -w '%{http_code}' --max-time 20 https://api.github.com/ 2>&1; echo \" exit $?\"", 40)
    online = not any(f" exit {code}" in out for code in (6, 7, 28))
    if online:
        check("curl checks certificates with its own defaults (HTTPS to GitHub)", out.startswith("200"), out)
        out = vm.run("lynx -dump -nolist https://example.com/ 2>&1 | head -n 12", 40)
        check("lynx reads a page over HTTPS", "Example Domain" in out or "documentation examples" in out, out.strip()[:120])
    else:
        print(f"   skip  HTTPS (no internet here): {out}")
    # The emulator has no IR parts (no overlays), but the drivers can still be
    # loaded and let go by name, as the Remote app does; at start, neither is in.
    out = vm.run("lsmod | grep -c '^gpio_ir'; modprobe gpio_ir_tx; modprobe gpio_ir_recv; lsmod | grep -c '^gpio_ir'; "
                 "modprobe -r gpio_ir_recv; modprobe -r gpio_ir_tx; lsmod | grep -c '^gpio_ir'", 60)
    check("the IR drivers stay out until the Remote app loads them, and go again", out.split() == ["0", "2", "0"], " ".join(out.split()))
    # The update history the handheld reads offline: the changelog, with a
    # section for the version this image is.
    version = vm.run("sed -n 's/^version=//p' /etc/catalyst/version").strip().split("-")[0]
    sections = vm.run("grep '^## ' /usr/share/catalyst/changelog.md").splitlines()
    check("the image carries the changelog, with this version's notes", any(s.split()[1:2] == [version] for s in sections), f"{version}: {len(sections)} versions")

    print("== power")
    out = vm.run("catalyst-power undervolt status")
    check("catalyst-power answers; the lower voltage is off unless chosen", out.strip() == "off", out)
    out = vm.run("ps | grep '[i]fplugd -n -i eth0' | grep -c -- '-m iff'")
    check("the cable is watched from the interface's flags (the adapter can sleep)", out.strip() == "1", out)
    out = vm.run("grep -c '^gpio=24=op,dl' /boot/config.txt 2>/dev/null || (mkdir -p /run/b && mount -o ro /dev/mmcblk0p1 /run/b && grep -c '^gpio=24=op,dl' /run/b/config.txt; umount /run/b)")
    check("the speaker's amplifier is held off", out.strip().endswith("1"), out)
    out = vm.run("df -k /data | tail -n1 | tr -s ' ' | cut -d' ' -f2")
    want = int(os.environ.get("CATALYST_CARD_GB", "0") or 0)
    grown = out.strip().isdigit() and (want == 0 or int(out) > (want - 6) * 1_000_000)
    check(f"/data grew to the card ({want or 'emulator'} GB card)", grown, f"{int(out) // 1024 if out.strip().isdigit() else out} MB")
    out = vm.run("tune2fs -l /dev/mmcblk0p4 2>/dev/null | grep -E '^(Block size|Block count|Inode count):' | tr -s ' ' | cut -d' ' -f3 | tr '\\n' ' '")
    f = out.split()
    # Inode count, block count, block size, as tune2fs lists them.
    per_inode = int(f[1]) * int(f[2]) // max(1, int(f[0])) if len(f) == 3 and all(x.isdigit() for x in f) else 0
    check("/data has 4 KB blocks and an inode per 64 KB", f[2:] == ["4096"] and per_inode >= 60_000, f"{out.strip()} ({per_inode} bytes an inode)")
    return failed


def tour(vm, folder):
    """Drive the UI from the emulated keyboard: the real ARM shell on the real
    kernel, drawing to a framebuffer and reading an evdev keyboard. Checks the
    external apps end to end; returns how many checks failed."""
    failed = 0

    def check(what, ok, detail=""):
        nonlocal failed
        print(f"   {'ok   ' if ok else 'FAIL '} {what}{': ' + str(detail) if detail else ''}")
        failed += 0 if ok else 1

    def shot(name):
        colours = vm.screenshot(f"{folder}/{name}.png")
        print(f"   saved {name}.png ({colours} colours)")
        return colours

    def until(command, want, timeout=150):
        """Ask the machine again and again until the answer is the one wanted.
        The emulator is many times slower than the handheld, and motion there
        runs in slow motion rather than skipping, so nothing here is timed."""
        deadline = time.time() + timeout
        while True:
            out = vm.run(command)
            if want(out) or time.time() > deadline:
                return out
            time.sleep(1.5)

    running = lambda what: f"ps | grep -v grep | grep '{what}'"

    print("== the UI, driven from the keyboard")
    # Two apps of the user's own: one that takes the screen and paints it a
    # single colour, and a loose script. (The image brings Processes.)
    vm.run("mkdir -p /data/apps/paint; cd /data/apps/paint; echo 'name = Paint' > app; echo 'icon = lightbulb' >> app; echo 'run = ./paint.sh' >> app; echo 'screen = own' >> app")
    vm.run("echo '#!/bin/sh' > paint.sh; echo 'cat /dev/zero > \"$CATALYST_FB\" 2>/dev/null' >> paint.sh; echo 'echo $CATALYST_ACCENT $CATALYST_LIGHT > /tmp/paint-saw' >> paint.sh; echo 'sleep 600' >> paint.sh; chmod +x paint.sh; cd /")
    vm.run("echo 'echo hello from a script; uname -m' > /data/apps/hello.sh")
    vm.run("rc-service catalyst-shell restart >/dev/null 2>&1")
    check("the shell is running", until("pidof catalyst-shell", lambda out: out != "") != "")
    # A first start: the start animation (which any key skips), then setup.
    time.sleep(3)
    shot("0-start")
    vm.key("esc")
    time.sleep(10)
    fresh = vm.run("[ -e /data/catalyst/setup-done ] || echo fresh")
    check("a first start opens setup", shot("0-setup") > 20 and fresh == "fresh", fresh)
    # Set up later: down twice, Enter. Home follows, and setup is not shown again.
    for key in ["down", "down", "ret"]:
        vm.key(key)
    done = until("[ -e /data/catalyst/setup-done ] && echo done", lambda out: out == "done", 90)
    check("setup can be put off, and that is remembered", done == "done", done)
    time.sleep(10)
    # The tour of the keys follows setup; Esc skips it.
    shot("1-tour")
    vm.key("esc")
    time.sleep(8)
    check("Home draws", shot("1-home") > 50)

    # Left and Right go through the apps in order: thirty-three built in,
    # Settings last; then the image's Processes (33) and the user's own by
    # name: hello (34) and Paint (35).
    for key in ["right"] * BUILT_IN:
        vm.key(key)
    time.sleep(6)
    shot("2-home-apps")
    vm.key("ret")
    check("an app in the terminal runs its program", until(running(" top$"), lambda out: out != "") != "")
    time.sleep(8)
    shot("3-processes-in-terminal")
    vm.key("ctrl-q")
    check("leaving it with ctrl-q ends the program", until(running(" top$"), lambda out: out == "") == "")
    time.sleep(8)

    # A loose script: it runs, prints, and ends; any key returns.
    vm.key("right")
    vm.key("ret")
    time.sleep(14)
    check("a loose script runs in the terminal", shot("4-script-in-terminal") > 20)
    vm.key("ret")
    time.sleep(10)

    vm.key("right")
    vm.key("ret")
    check("an app with its own screen is started", until(running("sleep 600"), lambda out: out != "") != "")
    saw = vm.run("cat /tmp/paint-saw")
    check("it is told the look in use", saw == "rose 0", saw)
    check("it has the whole panel (one colour)", shot("5-paint-own-screen") == 1)
    # Esc held for a second and a half: the way out of an app that has every key.
    vm.monitor("sendkey esc 1500")
    left = until(running("sleep 600\\|paint.sh"), lambda out: out == "")
    check("holding esc leaves it and stops everything it started", left == "", left)
    time.sleep(12)
    check("Home is drawn again, whole", shot("6-home-again") > 50)

    # The control centre, over Home: Ctrl-Space, and Esc to put it away.
    vm.key("ctrl-spc")
    time.sleep(12)
    check("the control centre comes down", shot("7-control-centre") > 50)
    vm.key("esc")
    time.sleep(10)

    # The built-in terminal has a real shell behind it: from Paint's tile,
    # Left back through the apps to Terminal's.
    for key in ["left"] * (BUILT_IN + 2 - TERMINAL) + ["ret"]:
        vm.key(key)
    time.sleep(14)
    for key in ["e", "c", "h", "o", "spc", "o", "k", "spc", "shift-dot", "slash", "t", "m", "p", "slash", "t", "y", "p", "e", "d", "ret"]:
        vm.key(key, 0.3)
    typed = until("cat /tmp/typed 2>/dev/null", lambda out: out == "ok", 60)
    check("keys typed in Terminal reach a real shell", typed == "ok", typed)
    time.sleep(4)
    shot("8-terminal")
    crashes = vm.run("rc-status boot | grep catalyst-shell | tr -s ' '")
    check("the shell never crashed", "(0)" in crashes, crashes)
    # Programs left behind are waited for: none lingers as a zombie, and
    # nothing an app started is still running.
    zombies = vm.run("ps -o stat,comm | awk '$1 ~ /^Z/' | wc -l")
    check("no program is left as a zombie", zombies.strip() == "0", zombies)

    # Graphics. The emulator has no VideoCore, so the shell draws on the
    # processor and says why. With Mesa's software renderer allowed, the
    # image's own Mesa loads into the dynamically linked shell, and its test
    # frame of glass and a growing window comes out as the processor's.
    log = vm.run("grep -m1 'drawing with' /tmp/catalyst-shell.log")
    check("with no gpu device, the processor draws", "drawing with the processor" in log, log)
    soft = vm.run("CATALYST_GPU=software catalyst-shell --about 2>&1 | grep -E 'drawing with|graphics|this shell'")
    check("the image's Mesa opens in the shell and draws as the processor does", "drawing with the gpu (softpipe" in soft, " | ".join(soft.splitlines()))
    return failed


def updates(vm):
    """A system update through the real partitions: a release made from this
    very image (with a later version), signed with the release key and served
    from this computer, is found, its signature checked, written into system B,
    read back, its kernel put in slot-b on the boot partition, and config.txt
    set to try system B once. (Restarting into it needs the firmware's
    tryboot, which the emulator does not have: that part is tested on files,
    in catalyst-update's own tests.)"""
    failed = 0

    def check(what, ok, detail=""):
        nonlocal failed
        print(f"   {'ok   ' if ok else 'FAIL '} {what}{': ' + str(detail) if detail else ''}")
        failed += 0 if ok else 1

    print("== a system update, end to end")
    tool = f"{BUILD}/target/release/catalyst-update"
    key = f"{BUILD}/keys/update.key"
    if not (os.path.exists(tool) and os.path.exists(key)):
        print("   (no release tool or key: run scripts/release.sh once)")
        return 1
    release = f"{WORK}/release"
    shutil.rmtree(release, ignore_errors=True)
    # Notes as release.sh hands them over: a changelog section, in a file.
    notes = f"{WORK}/vmtest-notes.md"
    with open(notes, "w") as f:
        f.write("### New\n- made by vm.py\n- a second line\n")
    made = subprocess.run([tool, "make-release", "--images", IMAGES, "--out", release, "--version", "0.2.1-vmtest", "--channel", "stable",
                           "--build", str(int(time.time()) + 100000), "--key", key, "--repo", "test/test", "--tag", "vmtest", "--notes-file", notes],
                          capture_output=True, text=True)
    check("a release is made from this image and signed", made.returncode == 0, (made.stdout + made.stderr).strip()[-200:])
    server = subprocess.Popen([sys.executable, "-m", "http.server", "8765", "--bind", "127.0.0.1", "--directory", release],
                              stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        time.sleep(1)
        vm.run("mkdir -p /data/update && echo 'source=url:http://10.0.2.2:8765' > /data/update/prefs")
        found = vm.run("catalyst-update check 2>&1; catalyst-update status", 180)
        check("it is found, and its signature checked against the key in the image", '"state":"available"' in found, found[-160:])
        check("its notes arrive whole, line for line", '"notes":"### New\\n- made by vm.py\\n- a second line"' in found, found[-160:])
        vm.run("(catalyst-update install >/tmp/install.log 2>&1 &)")
        deadline = time.time() + 1500
        while time.time() < deadline:
            status = vm.run("catalyst-update status", 60)
            if '"state":"ready"' in status or '"state":"failed"' in status:
                break
            time.sleep(10)
        check("it is downloaded, written into system B and read back", '"state":"ready"' in status, status[-200:] + " " + vm.run("tail -2 /tmp/install.log"))
        boot = vm.run("mkdir -p /tmp/b && mount -t vfat -o ro /dev/mmcblk0p1 /tmp/b && tail -4 /tmp/b/config.txt | tr '\n' ' '; "
                      "echo '|'; cat /tmp/b/slot-b/cmdline.txt; echo '|'; ls /tmp/b/slot-b | wc -l; umount /tmp/b")
        parts = [p.strip() for p in boot.split("|")]
        check("config.txt starts system A and tries system B", len(parts) == 3 and "[all] os_prefix=slot-a/ [tryboot] os_prefix=slot-b/" in parts[0], parts[0] if parts else boot)
        check("system B's kernel is in slot-b, rooted on system B", len(parts) == 3 and "root=/dev/mmcblk0p3" in parts[1] and int(parts[2] or 0) > 3, boot)
        verified = vm.run("rc-status default | grep catalyst-update | tr -s ' '")
        check("the start-up check of the last update ran", "started" in verified, verified)
    finally:
        server.terminate()
    return failed


if __name__ == "__main__":
    main()

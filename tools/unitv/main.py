# Catalyst Linux's smart camera: an M5Stack UnitV (K210, M12 lens, U078-V-M12)
# on the CardputerZero's Grove port, seeing for the handheld.
#
# Install: plug the UnitV's USB-C into the handheld's USB-A port; it offers
# "Install on the camera" (also in Settings > Camera > UnitV) and
# puts MaixPy (if it needs it), the face model, boot.py and this file on the
# camera by itself. Then Settings > Camera > Grove port > Smart camera on the
# handheld (at once) and plug the UnitV into the Grove port. Later versions
# come over the Grove port itself (see "files" below). The README's "The
# smart camera on the Grove port" has the rest.
#
# What it does: nothing until the handheld speaks to it (it never sends a
# byte unasked, and never drives a Grove pin before it knows which is its
# own). Then, as asked: faces (YOLOv2 on the KPU), QR codes, AprilTags
# (36h11, with their pose) and colour blobs, as small JSON results, and on
# request a QVGA JPEG of what it sees for the handheld's AI view.
#
# Its light (the UnitV's RGB LED, dim): amber from its first line (it has
# power and MaixPy started it), green while it listens for the handheld,
# blue once the handheld has spoken to it, violet while an update arrives,
# red if its image sensor did not start.
#
# The link (the handheld's smartcam.rs has the same):
#   C5 7A | kind | seq | len (2, LE) | header check | payload | CRC-32 (LE)
# The header check is (kind + seq + len_lo + len_hi + 0x5A) & 0xFF; the CRC
# (IEEE, as zlib's) covers kind through payload. Junk between frames is
# skipped to the next sync word. It starts at 115200 baud; the handheld asks
# for faster one step at a time, confirmed by a ping at the new rate; if no
# frame arrives at the new rate within a second this goes back to the rate
# before, and after five seconds with no frame at all to 115200.

import gc
import os
import sys
import time

VERSION = 2
# Which way up the picture is: check against the handheld's AI view.
FLIP = False
MIRROR = False
# The UnitV's RGB LED (a WS2812 on G8) says how it stands; False leaves it dark.
LED = True
# The M12 lens: 80 degrees, taken as the diagonal. Its focal length in pixels
# at 160 x 120 (where tags are looked for) is 100 / tan(40 deg).
FX_SMALL = 119.2
# FRC's AprilTags (36h11) are 6.5 in (165.1 mm) across their black square.
# MaixPy's pose comes in tag widths with the right focal length: check the
# distance against a tape measure once and correct this if it is off.
TAG_MM = 165.1
# Colour blobs in LAB: orange (a game piece), yellow, red and blue (bumpers).
# Tune for the field's light.
COLOURS = [
    (30, 80, 15, 70, 30, 80),
    (55, 95, -20, 15, 40, 90),
    (25, 70, 30, 85, 10, 65),
    (15, 60, -10, 30, -80, -25),
]

START_BAUD = 115200
SYNC = b"\xc5\x7a"
HELLO, BAUD, WANT, PING, PUT, CHUNK, DONE, COMMIT = 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08
HELLO_BACK, BAUD_BACK, WANT_BACK, PONG, RESULTS, PREVIEW, LOG = 0x81, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87
PUT_BACK, CHUNK_BACK, DONE_BACK, COMMIT_BACK = 0x88, 0x89, 0x8A, 0x8B
MAX = 32 * 1024

# ---------------------------------------------------------------- CRC-32 ---
crc_kind = "native"
try:
    from ubinascii import crc32
except ImportError:
    try:
        from binascii import crc32
    except ImportError:
        crc_kind = "python"
        _TABLE = []
        for _i in range(256):
            _c = _i
            for _ in range(8):
                _c = (0xEDB88320 ^ (_c >> 1)) if (_c & 1) else (_c >> 1)
            _TABLE.append(_c)

        def crc32(data, crc=0):
            crc ^= 0xFFFFFFFF
            t = _TABLE
            for b in data:
                crc = t[(crc ^ b) & 0xFF] ^ (crc >> 8)
            return crc ^ 0xFFFFFFFF


def frame(kind, seq, payload):
    n = len(payload)
    head = bytes([kind, seq & 0xFF, n & 0xFF, n >> 8, (kind + seq + (n & 0xFF) + (n >> 8) + 0x5A) & 0xFF])
    crc = crc32(head + payload) & 0xFFFFFFFF
    return SYNC + head + payload + bytes([crc & 0xFF, (crc >> 8) & 0xFF, (crc >> 16) & 0xFF, crc >> 24])


class Decoder:
    """Bytes in, frames (kind, seq, payload) out, whatever the cuts."""

    def __init__(self):
        self.held = b""
        self.bad = 0

    def push(self, data):
        self.held += data
        out = []
        h = self.held
        at = 0
        while True:
            start = h.find(SYNC, at)
            if start < 0:
                at = len(h) - 1 if h[-1:] == b"\xc5" else len(h)
                break
            at = start
            if len(h) - at < 7:
                break
            kind, seq, n = h[at + 2], h[at + 3], h[at + 4] | (h[at + 5] << 8)
            if h[at + 6] != (kind + seq + h[at + 4] + h[at + 5] + 0x5A) & 0xFF or n > MAX:
                self.bad += 1
                at += 1
                continue
            whole = 7 + n + 4
            if len(h) - at < whole:
                break
            body = h[at + 2 : at + 7 + n]
            c = h[at + 7 + n : at + whole]
            if crc32(body) & 0xFFFFFFFF != c[0] | (c[1] << 8) | (c[2] << 16) | (c[3] << 24):
                self.bad += 1
                at += 1
                continue
            out.append((kind, seq, h[at + 7 : at + 7 + n]))
            at += whole
        self.held = h[at:]
        return out


def u32(b, at=0):
    return b[at] | (b[at + 1] << 8) | (b[at + 2] << 16) | (b[at + 3] << 24)


def le32(v):
    return bytes([v & 0xFF, (v >> 8) & 0xFF, (v >> 16) & 0xFF, (v >> 24) & 0xFF])


# ----------------------------------------------------------------- files ---
# A newer program (and face model) from the handheld, over the Grove link,
# into /flash: each file into name.new beside a note of what it is to be
# (name.part: its size, CRC and id, so a transfer cut off resumes where it
# stopped), its CRC checked when it is whole, then all moved into place at
# once (COMMIT). The files they replace are kept as name.old, and boot.py
# counts the new program's starts: two that never heard the handheld, and
# the old files come back. A commit cut off part way is finished at the next
# start (boot.py), from its journal.

FILES = ("main.py", "boot.py", "face.kmodel")
CHUNK_MAX = 4096


def exists(path):
    try:
        os.stat(path)
        return True
    except OSError:
        return False


def remove(path):
    try:
        os.remove(path)
    except OSError:
        pass


def size_of(path):
    try:
        return os.stat(path)[6]
    except OSError:
        return -1


def file_crc(path):
    crc, n = 0, 0
    with open(path, "rb") as f:
        while True:
            b = f.read(4096)
            if not b:
                break
            crc = crc32(b, crc)
            n += len(b)
    return crc & 0xFFFFFFFF, n


def free_bytes(root):
    try:
        s = os.statvfs(root)
        return s[0] * s[3]
    except Exception:
        return -1


def settle(root):
    """A commit's journal carried out (again, if it was cut off): each file's
    .new moved into place, what was there kept as .old, then the trial count
    begun for boot.py. True if there was one."""
    try:
        with open(root + "/catalyst.commit") as f:
            lines = f.read().split("\n")
    except OSError:
        return False
    names = []
    for line in lines:
        parts = line.split()
        if len(parts) != 2:
            continue
        name, had = parts[0], parts[1] == "1"
        names.append(line.strip())
        cur = root + "/" + name
        new = cur + ".new"
        if exists(new):
            if had:
                if exists(cur):
                    remove(cur + ".old")
                    os.rename(cur, cur + ".old")
            else:
                remove(cur + ".old")
                remove(cur)
            os.rename(new, cur)
        remove(cur + ".part")
    with open(root + "/catalyst.trial", "w") as f:
        f.write("0\n" + "\n".join(names) + "\n")
    remove(root + "/catalyst.rolledback")
    remove(root + "/catalyst.commit")
    return True


class Files:
    def __init__(self, root):
        self.root = root
        self.cur = None
        self.out = None

    def path(self, name, ext=""):
        return self.root + "/" + name + ext

    def close(self):
        if self.out is not None:
            try:
                self.out.close()
            except Exception:
                pass
        self.out = None

    def begin(self, name, size, crc, ident):
        """Ready for `name`: how much of it is already here (to resume from)."""
        self.close()
        self.cur = None
        if name not in FILES:
            return {"name": name, "err": "name"}
        new, part = self.path(name, ".new"), self.path(name, ".part")
        note = "%d %d %s" % (size, crc, ident)
        try:
            with open(part) as f:
                same = f.read().strip() == note
        except OSError:
            same = False
        have = size_of(new) if same else -1
        if have < 0 or have > size:
            remove(new)
            free = free_bytes(self.root)
            if free >= 0 and size + 16384 > free:
                return {"name": name, "err": "room", "free": free}
            with open(part, "w") as f:
                f.write(note)
            open(new, "wb").close()
            have = 0
        try:
            self.out = open(new, "ab")
        except OSError:
            # A filesystem that will not append: from the start, no resuming.
            have = 0
            self.out = open(new, "wb")
        self.cur = [name, size, crc, have]
        return {"name": name, "have": have}

    def chunk(self, offset, data):
        """Written if it follows on; how much is here now (0xFFFFFFFF: no
        file under way)."""
        if self.cur is None:
            return 0xFFFFFFFF
        have = self.cur[3]
        if offset == have and have + len(data) <= self.cur[1]:
            self.out.write(data)
            have += len(data)
            self.cur[3] = have
        return have

    def end(self, name):
        cur = self.cur
        if cur is None or cur[0] != name:
            return {"name": name, "ok": 0, "err": "none"}
        self.close()
        self.cur = None
        new, part = self.path(name, ".new"), self.path(name, ".part")
        crc, n = file_crc(new)
        if n != cur[1] or crc != cur[2]:
            remove(new)
            remove(part)
            return {"name": name, "ok": 0, "err": "size" if n != cur[1] else "crc"}
        return {"name": name, "ok": 1}

    def commit(self, names, stamp):
        """Every file named, checked whole, moved into place with the stamp."""
        self.close()
        for name in names:
            if name not in FILES or not exists(self.path(name, ".new")):
                return {"ok": 0, "err": "missing " + name}
        with open(self.path("catalyst.json", ".new"), "w") as f:
            f.write(stamp)
        lines = ["%s %d" % (n, 1 if exists(self.path(n)) else 0) for n in list(names) + ["catalyst.json"]]
        with open(self.path("catalyst.commit"), "w") as f:
            f.write("\n".join(lines) + "\n")
        settle(self.root)
        return {"ok": 1}


def confirm(root):
    """The program started and the handheld heard it: no trial any more."""
    remove(root + "/catalyst.trial")


# ---------------------------------------------------------------- sensor ---
# Everything from here on is the camera's own (MaixPy). Nothing in it may stop
# the program before it listens on the link: what fails is said in its hello.
import image
import sensor
import ujson
from fpioa_manager import fm
from machine import UART

booted = time.ticks_ms()
ROOT = "/flash"
light = None


def lamp(r, g, b):
    global light
    if not LED:
        return
    try:
        if light is None:
            from modules import ws2812

            light = ws2812(8, 1)
        light.set_led(0, (r, g, b))
        light.display()
    except Exception:
        pass


lamp(10, 5, 0)

# The sensor before the UART (MaixPy issue 363: a UART set up first can stop
# sending). Tried three times: a K210 short of power, or a sensor slow to
# wake, can fail the first.
sensor_ok = False
sensor_said = "not started"
for _try in range(3):
    try:
        sensor.reset()
        sensor.set_pixformat(sensor.RGB565)
        sensor.set_framesize(sensor.QVGA)
        sensor.set_vflip(FLIP)
        sensor.set_hmirror(MIRROR)
        sensor.skip_frames(time=300)
        sensor_ok, sensor_said = True, "ok"
        break
    except Exception as e:
        sensor_said = "failed: %s" % e
        time.sleep_ms(300)

# The face detector on the KPU: Sipeed's YOLOv2 face model (MaixPy_scripts,
# MIT): from the card, else the one the handheld sent over Grove (in /flash),
# else from flash where the installer put it (0x300000).
task = None
kpu = None
kpu_name = "none"
try:
    import KPU as kpu

    for where in ("/sd/face.kmodel", "/flash/face.kmodel", "/flash/face.kmodel.old", 0x300000):
        if isinstance(where, str) and not exists(where):
            continue
        try:
            task = kpu.load(where)
            kpu_name = "face yolo2 320x240 (%s)" % (where if isinstance(where, str) else "flash 0x300000")
            break
        except Exception:
            task = None
    if task is not None:
        anchors = (1.889, 2.5245, 2.9465, 3.94056, 3.99987, 5.3658, 5.155437, 6.92275, 6.718375, 9.01025)
        kpu.init_yolo2(task, 0.5, 0.3, 5, anchors)
except Exception as e:
    task = None
    kpu_name = "failed: %s" % e

# Whether this MaixPy has what the program needs (else: a full one over USB-A).
full = 1
try:
    for _n in ("find_apriltags", "find_qrcodes", "find_blobs"):
        if not hasattr(image.Image, _n):
            full = 0
    if kpu is None or not hasattr(kpu, "init_yolo2"):
        full = 0
except Exception:
    full = 0
try:
    maixpy = ".".join([str(x) for x in sys.implementation.version])
except Exception:
    maixpy = "?"
try:
    cwd = os.getcwd()
except Exception:
    cwd = "?"

# What the handheld installed (its version stamp), for its hello: the handheld
# sends a newer program when its own is newer.
try:
    with open(ROOT + "/catalyst.json") as f:
        stamp = ujson.loads(f.read())
except Exception:
    stamp = None
trial = None
try:
    with open(ROOT + "/catalyst.trial") as f:
        trial = int(f.read().split()[0])
except Exception:
    trial = None
rolled_back = exists(ROOT + "/catalyst.rolledback")

# ------------------------------------------------------------------ link ---
# The Grove port's two pins are G34 and G35. Which is which depends on the
# cable, so both only listen (UART1 on 35, UART2 on 34) until the handheld's
# hello arrives on one; that one is this end's RX, the other its TX. The USB
# serial line (MaixPy's REPL) listens too, so the handheld can check this
# answers right after installing it over USB-A; a Ctrl-C there still stops it.
fm.register(35, fm.fpioa.UART1_RX, force=True)
fm.register(34, fm.fpioa.UART2_RX, force=True)
listen = [(UART(UART.UART1, START_BAUD, 8, None, 1, timeout=0, read_buf_len=8192), 35, 34), (UART(UART.UART2, START_BAUD, 8, None, 1, timeout=0, read_buf_len=8192), 34, 35)]
try:
    usb = UART.repl_uart()
except Exception:
    usb = None
uart = None
via_usb = False
decoder = Decoder()
heard = [Decoder(), Decoder()]
usb_heard = Decoder()
listening_ms = time.ticks_diff(time.ticks_ms(), booted)
lamp(0, 8, 0) if sensor_ok else lamp(12, 0, 0)
while uart is None:
    for i, (port, rx, tx) in enumerate(listen):
        if port.any():
            data = port.read()
            if data and any(f[0] == HELLO for f in heard[i].push(data)):
                uart, rx_pin, tx_pin = port, rx, tx
                break
    if uart is None and usb is not None:
        try:
            data = usb.read() if usb.any() else None
        except Exception:
            data = None
        if data and any(f[0] == HELLO for f in usb_heard.push(data)):
            uart, via_usb = usb, True
    time.sleep_ms(5)
for port, rx, tx in listen:
    if port is not uart:
        port.deinit()
if not via_usb:
    # Older MaixPy wants unregister(pin, function); force=True replaces the
    # listening function either way.
    try:
        fm.unregister(tx_pin)
    except Exception:
        pass
    fm.register(tx_pin, fm.fpioa.UART1_TX if rx_pin == 35 else fm.fpioa.UART2_TX, force=True)
lamp(0, 0, 10) if sensor_ok else lamp(12, 0, 0)
confirm(ROOT)

seq = 0
files = Files(ROOT)


def send(kind, payload):
    global seq
    seq = (seq + 1) & 0xFF
    uart.write(frame(kind, seq, payload))


def hello():
    send(
        HELLO_BACK,
        ujson.dumps(
            {
                "v": VERSION,
                "fw": "catalyst-unitv %d" % VERSION,
                "upd": 1,
                "mp": maixpy,
                "full": full,
                "sensor": sensor_said,
                "kpu": kpu_name,
                "crc": crc_kind,
                "fov": 80,
                "w": 320,
                "h": 240,
                "up_ms": time.ticks_diff(time.ticks_ms(), booted),
                "listen_ms": listening_ms,
                "usb": via_usb,
                "cwd": cwd,
                "free": free_bytes(ROOT),
                "trial": trial,
                "rolled_back": rolled_back,
                "stamp": stamp,
            }
        ).encode(),
    )


hello()  # the hello that found the pins is answered at once

baud = START_BAUD
before = START_BAUD
moved_at = None
heard_at = time.ticks_ms()
want = {"faces": 0, "codes": 0, "tags": 0, "blobs": 0, "preview": 0}
n = 0
clock = time.clock()


def set_baud(rate):
    global baud
    baud = rate
    uart.init(rate, 8, None, 1, timeout=0, read_buf_len=8192)


def quiet():
    for k in want:
        want[k] = 0


def handle(kind, payload):
    global moved_at, want, before
    if kind == HELLO:
        hello()
    elif kind == BAUD and len(payload) >= 4 and not via_usb:
        rate = u32(payload)
        send(BAUD_BACK, payload)
        # The answer out at the old rate before the change (about 15 bytes).
        time.sleep_ms(2 + 160000 // baud)
        before = baud
        set_baud(rate)
        moved_at = time.ticks_ms()
    elif kind == PING:
        moved_at = None
        send(PONG, payload)
    elif kind == WANT:
        try:
            asked = ujson.loads(payload)
            for k in want:
                want[k] = 1 if asked.get(k) else 0
        except Exception:
            pass
        send(WANT_BACK, payload)
    elif kind == PUT:
        quiet()
        lamp(8, 0, 10)
        try:
            a = ujson.loads(payload)
            if cwd == "/sd":
                said = {"name": a.get("name"), "err": "card"}
            else:
                said = files.begin(a["name"], int(a["size"]), int(a["crc"]), str(a.get("id", "")))
        except Exception as e:
            said = {"err": str(e)[:80]}
        send(PUT_BACK, ujson.dumps(said).encode())
    elif kind == CHUNK and len(payload) >= 4:
        try:
            have = files.chunk(u32(payload), payload[4:])
        except Exception:
            have = 0xFFFFFFFF
        send(CHUNK_BACK, le32(have))
    elif kind == DONE:
        try:
            said = files.end(ujson.loads(payload)["name"])
        except Exception as e:
            said = {"ok": 0, "err": str(e)[:80]}
        send(DONE_BACK, ujson.dumps(said).encode())
    elif kind == COMMIT:
        try:
            a = ujson.loads(payload)
            said = files.commit(a["files"], ujson.dumps(a["stamp"]))
        except Exception as e:
            said = {"ok": 0, "err": str(e)[:80]}
        send(COMMIT_BACK, ujson.dumps(said).encode())
        if said.get("ok"):
            # Out on the wire, then the new program from its first line.
            time.sleep_ms(50 + 200000 // baud)
            import machine

            machine.reset()
        lamp(0, 0, 10)


def first_bit(code):
    i = 0
    while code > 1:
        code >>= 1
        i += 1
    return i


def results(img):
    r = {"n": n, "w": 320, "h": 240, "fps": round(clock.fps(), 1)}
    if want["faces"] and task is not None:
        found = kpu.run_yolo2(task, img) or []
        r["f"] = [[o.x(), o.y(), o.w(), o.h(), int(o.value() * 100)] for o in found]
    if want["codes"]:
        r["q"] = [[c.x(), c.y(), c.w(), c.h(), c.payload()] for c in img.find_qrcodes()]
    if want["tags"]:
        # At 160 x 120 (memory, and pace); found places scaled back to 320.
        small = img.resize(160, 120)
        tags = small.find_apriltags(families=image.TAG36H11, fx=FX_SMALL, fy=FX_SMALL, cx=80, cy=60)
        r["a"] = [[t.id(), t.cx() * 2, t.cy() * 2, int(t.rotation() * 1000), int(t.x_translation() * TAG_MM), int(t.y_translation() * TAG_MM), int(-t.z_translation() * TAG_MM)] for t in tags]
        del small
    if want["blobs"]:
        r["b"] = [[first_bit(b.code()), b.x(), b.y(), b.w(), b.h(), b.pixels()] for b in img.find_blobs(COLOURS, pixels_threshold=120, area_threshold=120, merge=False)]
    return r


def turn():
    global moved_at, heard_at, n
    # What the handheld sent.
    if uart.any():
        data = uart.read()
        if data:
            for kind, s, payload in decoder.push(data):
                heard_at = time.ticks_ms()
                handle(kind, payload)
    # No frame at a new rate for a second: back to the rate before it.
    if moved_at is not None and time.ticks_diff(time.ticks_ms(), moved_at) > 1000:
        moved_at = None
        set_baud(before)
    # Nothing at all for five seconds at a raised rate (the handheld lost
    # it, or started again): back to where both start.
    if baud != START_BAUD and moved_at is None and time.ticks_diff(time.ticks_ms(), heard_at) > 5000:
        set_baud(START_BAUD)
    if not sensor_ok or not any(want.values()):
        time.sleep_ms(5)
        return
    clock.tick()
    img = sensor.snapshot()
    n += 1
    send(RESULTS, ujson.dumps(results(img)).encode())
    if want["preview"]:
        # Smaller at slow rates, so a frame still arrives often.
        quality = 55 if baud >= 1000000 else 35
        jpeg = img.compress(quality=quality)
        send(PREVIEW, jpeg.to_bytes())


while True:
    try:
        turn()
    except MemoryError:
        gc.collect()
    except Exception as e:
        try:
            send(LOG, str(e).encode()[:200])
        except Exception:
            pass
        time.sleep_ms(20)

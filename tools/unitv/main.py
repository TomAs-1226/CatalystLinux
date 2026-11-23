# Catalyst Linux's smart camera: an M5Stack UnitV (K210, M12 lens, U078-V-M12)
# on the CardputerZero's Grove port, seeing for the handheld.
#
# Install: plug the UnitV's USB-C into the handheld's USB-A port; it offers
# "Install on the camera" (also in Settings > Camera > UnitV) and
# puts MaixPy (if it needs it), the face model and this file on the camera
# by itself. Or copy this file to the root of a FAT32 microSD card as main.py
# (and face.kmodel beside it for faces) and put the card in the UnitV. Then
# Settings > Camera > Grove port > Smart camera on the handheld (at once) and
# plug the UnitV into the Grove port. The README's "The smart camera on the
# Grove port" has the rest.
#
# What it does: nothing until the handheld speaks to it (it never sends a
# byte unasked, and never drives a pin before it knows which is its own).
# Then, as asked: faces (YOLOv2 on the KPU), QR codes, AprilTags (36h11,
# with their pose) and colour blobs, as small JSON results, and on request a
# QVGA JPEG of what it sees for the handheld's AI view.
#
# The link (the handheld's smartcam.rs has the same):
#   C5 7A | kind | seq | len (2, LE) | header check | payload | CRC-32 (LE)
# The header check is (kind + seq + len_lo + len_hi + 0x5A) & 0xFF; the CRC
# (IEEE, as zlib's) covers kind through payload. Junk between frames is
# skipped to the next sync word. It starts at 115200 baud; the handheld asks
# for faster (up to 3 Mbaud), and if no frame arrives at the new rate within
# a second this goes back to 115200 by itself.

import gc
import time

import image
import sensor
import ujson
from fpioa_manager import fm
from machine import UART

VERSION = 1
# Which way up the picture is: check against the handheld's AI view.
FLIP = False
MIRROR = False
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
HELLO, BAUD, WANT, PING = 0x01, 0x02, 0x03, 0x04
HELLO_BACK, BAUD_BACK, WANT_BACK, PONG, RESULTS, PREVIEW, LOG = 0x81, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87
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
                at += 1
                continue
            whole = 7 + n + 4
            if len(h) - at < whole:
                break
            body = h[at + 2 : at + 7 + n]
            c = h[at + 7 + n : at + whole]
            if crc32(body) & 0xFFFFFFFF != c[0] | (c[1] << 8) | (c[2] << 16) | (c[3] << 24):
                at += 1
                continue
            out.append((kind, seq, h[at + 7 : at + 7 + n]))
            at += whole
        self.held = h[at:]
        return out


# ---------------------------------------------------------------- sensor ---
# The sensor before the UART (MaixPy issue 363: a UART set up first can stop
# sending).
booted = time.ticks_ms()
sensor.reset()
sensor.set_pixformat(sensor.RGB565)
sensor.set_framesize(sensor.QVGA)
sensor.set_vflip(FLIP)
sensor.set_hmirror(MIRROR)
sensor.skip_frames(time=300)

# The face detector on the KPU: Sipeed's YOLOv2 face model (MaixPy_scripts,
# MIT), from the card, else from flash where kflash put it.
task = None
kpu_name = "none"
try:
    import KPU as kpu

    for where in ("/sd/face.kmodel", 0x300000):
        try:
            task = kpu.load(where)
            break
        except Exception:
            task = None
    if task is not None:
        anchors = (1.889, 2.5245, 2.9465, 3.94056, 3.99987, 5.3658, 5.155437, 6.92275, 6.718375, 9.01025)
        kpu.init_yolo2(task, 0.5, 0.3, 5, anchors)
        kpu_name = "face yolo2 320x240"
except Exception:
    task = None

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
    fm.unregister(tx_pin)
    fm.register(tx_pin, fm.fpioa.UART1_TX if rx_pin == 35 else fm.fpioa.UART2_TX, force=True)

# What the handheld installed (its version stamp), for its hello: Settings
# says when a newer Catalyst has a newer program for the camera.
try:
    with open("/flash/catalyst.json") as f:
        stamp = ujson.loads(f.read())
except Exception:
    stamp = None

seq = 0


def send(kind, payload):
    global seq
    seq = (seq + 1) & 0xFF
    uart.write(frame(kind, seq, payload))


def hello():
    send(HELLO_BACK, ujson.dumps({"v": VERSION, "fw": "catalyst-unitv %d" % VERSION, "kpu": kpu_name, "crc": crc_kind, "fov": 80, "w": 320, "h": 240, "up_ms": time.ticks_diff(time.ticks_ms(), booted), "usb": via_usb, "stamp": stamp}).encode())


hello()  # the hello that found the pins is answered at once

baud = START_BAUD
moved_at = None
want = {"faces": 0, "codes": 0, "tags": 0, "blobs": 0, "preview": 0}
n = 0
clock = time.clock()


def set_baud(rate):
    global baud
    baud = rate
    uart.init(rate, 8, None, 1, timeout=0, read_buf_len=8192)


def handle(kind, payload):
    global moved_at, want
    if kind == HELLO:
        hello()
    elif kind == BAUD and len(payload) >= 4 and not via_usb:
        rate = payload[0] | (payload[1] << 8) | (payload[2] << 16) | (payload[3] << 24)
        send(BAUD_BACK, payload)
        # The answer out at the old rate before the change (about 15 bytes).
        time.sleep_ms(2 + 160000 // baud)
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


while True:
    # What the handheld sent.
    if uart.any():
        data = uart.read()
        if data:
            for kind, s, payload in decoder.push(data):
                handle(kind, payload)
    # No frame at a new rate for a second: back to where both started.
    if moved_at is not None and time.ticks_diff(time.ticks_ms(), moved_at) > 1000:
        moved_at = None
        set_baud(START_BAUD)
    if not any(want.values()):
        time.sleep_ms(5)
        continue
    clock.tick()
    img = sensor.snapshot()
    n += 1
    try:
        send(RESULTS, ujson.dumps(results(img)).encode())
        if want["preview"]:
            # Smaller at slow rates, so a frame still arrives often.
            quality = 55 if baud >= 1000000 else 35
            jpeg = img.compress(quality=quality)
            send(PREVIEW, jpeg.to_bytes())
    except MemoryError:
        gc.collect()
    except Exception as e:
        send(LOG, str(e).encode()[:200])

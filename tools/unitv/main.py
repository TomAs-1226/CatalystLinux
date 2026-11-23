# Catalyst Coprocessor: the program on the smart camera, an M5Stack UnitV
# (K210, M12 lens, U078-V-M12) on the CardputerZero's Grove port, seeing and
# listening for the handheld at the least power it can.
#
# It runs on Catalyst's own MaixPy firmware (tools/unitv/firmware: the
# `catalyst` module, frozen helpers, a boot that always starts this file).
# Install: the UnitV's USB-C into the handheld's USB-A port ("Install on the
# camera", also Settings > Camera > UnitV) flashes the firmware and puts
# this file and the models on /flash; later versions of this file and the
# models come over the Grove port itself. README: "The smart camera on the
# Grove port".
#
# Idle by default: the image sensor powered down (PWDN, its clock stopped),
# the KPU's clock and PLL1 off, the CPU at 100 MHz waiting for an interrupt
# between looks at the link, the light off. A frame from the handheld wakes
# it; it runs only the pipelines asked for (faces on the KPU, QR codes,
# AprilTags 36h11 with their pose, colour blobs, a QVGA preview), at the
# frame rate asked for, and goes back to idle when nothing has been asked
# for a while (IDLE_AFTER_MS, or what the handheld set). Speech: the
# handheld streams 16 kHz PCM only while its talk key is held; the front
# end (log-mels, voice activity) runs as it arrives, and at the end the
# keyword spotter runs on the KPU.
#
# The link (frames: catalyst_link; the handheld's smartcam.rs has the same):
#   HELLO  -> HELLO_BACK  what it is (JSON)
#   BAUD   -> BAUD_BACK   a faster rate, confirmed by a PING at it
#   PING   -> PONG
#   PIPE   -> PIPE_BACK   {"p":[pipelines],"fps":n,"idle_ms":n}; [] = idle
#   WANT   -> WANT_BACK   the older form of PIPE (version 2's)
#   IDLE   -> IDLE_BACK   idle now
#   SYNC   -> SYNC_BACK   8 bytes of the handheld's clock (us) echoed, then
#                         8 of the camera's (us): the handheld maps the
#                         camera's times onto its own
#   HEALTH -> HEALTH_BACK state, pace, errors, memory, clock
#   AUDIO                 u32 offset (samples) + int16 PCM; offset 0 starts
#                         an utterance; HEARD when the voice starts or ends
#   AUDIO_END -> COMMAND  {"mode":"command"|"dictation"}: the keyword
#                         spotter's top three and embedding, the speech's span
#   CALIB  -> CALIB_BACK  the AprilTag pose calibration (measure, save, get)
#   PUT, CHUNK, DONE, COMMIT: a new program or model (catalyst_link)
# and unasked, once asked to: RESULTS (JSON, with "t", the camera's ms at
# capture), PREVIEW (JPEG), HEARD, LOG.

import gc
import time

try:
    import ujson as json
except ImportError:
    import json

import catalyst_link as L

VERSION = 3
# Which way up the picture is: check against the handheld's AI view.
FLIP = False
MIRROR = False
# The UnitV's RGB LED (a WS2812 on G8) says how it stands; False leaves it dark.
LED = True
ROOT = "/flash"
W, H = 320, 240
# The M12 lens, until calibrated: 80 degrees taken as the diagonal, so the
# focal length is 200 / tan(40 deg) pixels at 320 x 240.
FX_DEFAULT = 238.4
# FRC's AprilTags (36h11) are 6.5 in (165.1 mm) across their black square.
TAG_MM = 165.1
# Colour blobs in LAB: orange (a game piece), yellow, red and blue (bumpers).
COLOURS = [
    (30, 80, 15, 70, 30, 80),
    (55, 95, -20, 15, 40, 90),
    (25, 70, 30, 85, 10, 65),
    (15, 60, -10, 30, -80, -25),
]
PIPES = ("faces", "codes", "tags", "blobs", "preview")
# The CPU's clock: full while it works, low while idle (the link's top rate,
# 3 Mbaud, still divides from the UART clock at 100 MHz).
CLOCK_FULL = 400000000
CLOCK_IDLE = 100000000
# Back to idle when nothing has come from the handheld for this long (it
# pings every two seconds while it wants anything).
IDLE_AFTER_MS = 5000
MAX_FPS = 30
# An utterance with no audio for this long is let go.
AUDIO_STALE_MS = 3000
RATE = 16000
# Speech longer than this is not a command (the keyword spotter still runs
# on its loudest second; the handheld decides).
COMMAND_MAX_MS = 1600

# The keyword spotter's input normalisation (tools/unitv/speech/kws.json for
# kws.kmodel sha256 4d603785ce7d...; test_protocol.py checks they agree).
KWS_MEAN = [-1.472912, -1.207986, -1.422817, -1.596645, -1.801277, -1.807432, -1.668044, -1.706724, -1.786358, -1.914321, -2.02782, -2.097692, -2.175764, -2.226368, -2.27734, -2.405553, -2.391168, -2.394937, -2.466646, -2.520516, -2.558886, -2.607702, -2.593508, -2.574128, -2.576035, -2.652733, -2.645905, -2.610902, -2.537985, -2.496236, -2.432925, -2.403436, -2.454382, -2.534835, -2.603509, -2.68257, -2.735904, -2.722924, -2.758487, -2.888197]
KWS_INV_STD = [0.24209, 0.240588, 0.237355, 0.238504, 0.240118, 0.236563, 0.233421, 0.232998, 0.233876, 0.236431, 0.241397, 0.246263, 0.249434, 0.251215, 0.253205, 0.256236, 0.25691, 0.258301, 0.26115, 0.263093, 0.264772, 0.266032, 0.265029, 0.264817, 0.264828, 0.265976, 0.266769, 0.266553, 0.264574, 0.262333, 0.258156, 0.255596, 0.25666, 0.257877, 0.259186, 0.260586, 0.262342, 0.262134, 0.261965, 0.258744]
FRAME_MS = 20  # the front end's hop


def since(now, then):
    """Milliseconds from `then` to `now` on the camera's clock (30-bit, as
    catalyst.time_ms() wraps)."""
    return (now - then) & 0x3FFFFFFF


def reached(now, at):
    """Whether `now` is at or after `at` (within half the clock's wrap)."""
    return since(now, at) < 0x20000000


def first_bit(code):
    i = 0
    while code > 1:
        code >>= 1
        i += 1
    return i


def side_px(corners):
    """The mean length of a tag's four edges, in pixels."""
    n = len(corners)
    total = 0.0
    for i in range(n):
        x0, y0 = corners[i]
        x1, y1 = corners[(i + 1) % n]
        total += ((x1 - x0) ** 2 + (y1 - y0) ** 2) ** 0.5
    return total / n


class Intrinsics:
    """The lens, at 320 x 240: focal lengths and principal point in pixels,
    and how many millimetres one unit of MaixPy's tag pose is. Defaults until
    the calibration (a 36h11 tag held square-on at known distances) stores
    its own in /flash/intrinsics.json."""

    def __init__(self, path):
        self.path = path
        self.reset()
        try:
            with open(path) as f:
                v = json.loads(f.read())
            self.fx, self.fy = float(v["fx"]), float(v["fy"])
            self.cx, self.cy = float(v["cx"]), float(v["cy"])
            self.unit_mm = float(v["unit_mm"])
            self.tag_mm = float(v.get("tag_mm", TAG_MM))
            self.calibrated = True
        except Exception:
            self.reset()

    def reset(self):
        self.fx = self.fy = FX_DEFAULT
        self.cx, self.cy = W / 2, H / 2
        # MaixPy's pose is in units of half the tag's side (its tag spans
        # -1..1); a calibration measures it rather than trusting it.
        self.unit_mm = TAG_MM / 2
        self.tag_mm = TAG_MM
        self.calibrated = False

    def describe(self):
        return {"fx": round(self.fx, 2), "fy": round(self.fy, 2), "cx": self.cx, "cy": self.cy, "unit_mm": round(self.unit_mm, 3), "tag_mm": self.tag_mm, "calibrated": 1 if self.calibrated else 0}

    def save(self, measures, tag_mm):
        """From measurements at known distances (each: d_mm, side_px at 320
        x 240, z_raw as the pose said it, the fx it was found with): fx from
        the tag's apparent size, the pose's unit from its distance, both
        averaged over the measurements."""
        good = [m for m in measures if m["side_px"] > 0 and m["z_raw"] > 0]
        if not good:
            return False
        fx = sum(m["side_px"] * m["d_mm"] / tag_mm for m in good) / len(good)
        # The pose's z scales with the fx it was found with.
        unit = sum(m["d_mm"] / (m["z_raw"] * fx / m["fx_used"]) for m in good) / len(good)
        self.fx = self.fy = fx
        self.cx, self.cy = W / 2, H / 2
        self.unit_mm = unit
        self.tag_mm = tag_mm
        self.calibrated = True
        with open(self.path, "w") as f:
            f.write(json.dumps({"fx": fx, "fy": fx, "cx": self.cx, "cy": self.cy, "unit_mm": unit, "tag_mm": tag_mm, "w": W, "h": H, "family": "36h11", "measures": good}))
        return True

    def forget(self):
        L.remove(self.path)
        self.reset()


class Program:
    """The camera's state and what it answers, apart from the hardware
    (`hw`, the K210's below or the tests' stand-in): idle, active (the
    pipelines asked for, at their pace), listening (an utterance arriving).
    handle() takes one frame and returns the frames to send; tick() is
    called between frames and returns what is due."""

    def __init__(self, hw, hello_info):
        self.hw = hw
        self.info = hello_info
        now = hw.now_ms()
        self.started = now
        self.heard_at = now
        self.state = "idle"
        self.pipes = []
        self.fps = 0
        self.idle_after = IDLE_AFTER_MS
        self.next_frame = now
        self.frames = 0
        self.errors = 0
        self.last_error = None
        self.wake_ms = None
        self.paced = []  # recent capture times, for the measured pace
        self.audio = None
        self.calib = []
        self.intr = hw.intrinsics()

    # ------------------------------------------------------------ states ---
    def wake(self, pipes, fps):
        pipes = [p for p in PIPES if p in pipes]
        fps = max(0, min(MAX_FPS, fps))
        if not pipes:
            self.sleep()
            return
        if self.state != "active":
            self.wake_ms = self.hw.wake(pipes)
            self.next_frame = self.hw.now_ms()
            self.paced = []
        else:
            self.hw.need(pipes)
        self.pipes, self.fps, self.state = pipes, fps, "active"

    def sleep(self):
        if self.state != "idle" or self.pipes:
            self.hw.sleep()
        self.pipes, self.fps, self.state = [], 0, "idle"

    # ------------------------------------------------------------ frames ---
    def handle(self, kind, payload):
        now = self.hw.now_ms()
        self.heard_at = now
        out = []
        if kind == L.HELLO:
            out.append((L.HELLO_BACK, json.dumps(self.hello()).encode()))
        elif kind == L.PING:
            out.append((L.PONG, payload))
        elif kind == L.PIPE:
            try:
                a = json.loads(payload)
                if "idle_ms" in a:
                    self.idle_after = max(1000, min(600000, int(a["idle_ms"])))
                self.wake(a.get("p") or [], float(a.get("fps", 0) or 0))
            except Exception as e:
                self.fault("pipe: %s" % e)
            out.append((L.PIPE_BACK, json.dumps(self.pipe_said()).encode()))
        elif kind == L.WANT:
            try:
                a = json.loads(payload)
                self.wake([p for p in PIPES if a.get(p)], 0)
            except Exception as e:
                self.fault("want: %s" % e)
            out.append((L.WANT_BACK, payload))
        elif kind == L.IDLE:
            self.sleep()
            out.append((L.IDLE_BACK, json.dumps({"state": "idle", "t": now}).encode()))
        elif kind == L.SYNCH:
            # Answered at once, with the camera's time as late as it can be.
            echo = bytes(payload[:8]) if len(payload) >= 8 else bytes(8)
            out.append((L.SYNC_BACK, echo + L.le64(self.hw.now_us())))
        elif kind == L.HEALTH:
            out.append((L.HEALTH_BACK, json.dumps(self.health()).encode()))
        elif kind == L.AUDIO:
            out.extend(self.audio_in(payload, now))
        elif kind == L.AUDIO_END:
            out.append((L.COMMAND, json.dumps(self.audio_end(payload)).encode()))
        elif kind == L.CALIB:
            out.append((L.CALIB_BACK, json.dumps(self.calibrate(payload)).encode()))
        return out

    def tick(self):
        """What is due now: a frame's results at the pace asked for; idle
        after a quiet while; an utterance gone stale let go."""
        now = self.hw.now_ms()
        out = []
        if self.audio is not None and since(now, self.audio["at"]) > AUDIO_STALE_MS:
            self.audio = None
        if self.state == "active" and since(now, self.heard_at) > self.idle_after:
            self.sleep()
            return out
        if self.state != "active":
            return out
        if self.fps > 0:
            if not reached(now, self.next_frame):
                return out
            step = int(1000 / self.fps)
            self.next_frame = (self.next_frame + step) & 0x3FFFFFFF
            if reached(now, self.next_frame):
                # Fell behind (a slow frame): the pace again from now, no burst.
                self.next_frame = (now + step) & 0x3FFFFFFF
        try:
            got = self.hw.capture(self.pipes, self.frames + 1, self.intr)
        except MemoryError:
            gc.collect()
            self.fault("memory")
            return out
        except Exception as e:
            self.fault("capture: %s" % e)
            return out
        self.frames += 1
        results, jpeg = got
        self.paced.append(results["t"])
        if len(self.paced) > 10:
            self.paced.pop(0)
        results["fps"] = self.pace()
        out.append((L.RESULTS, json.dumps(results).encode()))
        if jpeg is not None:
            out.append((L.PREVIEW, jpeg))
        return out

    def pace(self):
        if len(self.paced) < 2:
            return 0.0
        span = since(self.paced[-1], self.paced[0])
        return round(1000.0 * (len(self.paced) - 1) / span, 1) if span else 0.0

    def due_in(self):
        """How long the loop may sleep (ms) before tick() has work."""
        if self.state != "active":
            return 20
        now = self.hw.now_ms()
        if self.fps <= 0 or reached(now, self.next_frame):
            return 0
        return min(since(self.next_frame, now), 20)

    # ------------------------------------------------------------ speech ---
    def audio_in(self, payload, now):
        if len(payload) < 4:
            return []
        offset = L.u32(payload)
        pcm = memoryview(payload)[4:]
        if offset == 0 or self.audio is None:
            self.hw.audio_reset()
            self.audio = {"n": 0, "lost": 0, "vad": 0, "at": now, "since": now}
            if offset != 0:
                self.audio["lost"] += offset
                self.hw.audio_push(bytes(2 * min(offset, RATE)))
                self.audio["n"] = offset
        a = self.audio
        if offset > a["n"]:
            # A chunk lost on the way: its time kept, as silence.
            gap = offset - a["n"]
            a["lost"] += gap
            self.hw.audio_push(bytes(2 * min(gap, RATE)))
            a["n"] = offset
        elif offset < a["n"]:
            return []  # sent again: already here
        self.hw.audio_push(pcm)
        a["n"] += len(pcm) // 2
        a["at"] = now
        vad = self.hw.vad()
        if vad[0] != a["vad"]:
            a["vad"] = vad[0]
            return [(L.HEARD, json.dumps(self.vad_said(vad)).encode())]
        return []

    def vad_said(self, vad):
        state, start, last, speech = vad
        n = self.audio["n"] if self.audio else 0
        return {"vad": state, "start_ms": start * FRAME_MS, "end_ms": (last + 1) * FRAME_MS + 12 if last >= 0 else 0, "speech_ms": speech * FRAME_MS, "n": n}

    def audio_end(self, payload):
        try:
            mode = json.loads(payload).get("mode", "command")
        except Exception:
            mode = "command"
        if self.audio is None:
            return {"mode": mode, "n": 0, "vad": 0, "top": [], "err": "no audio"}
        vad = self.hw.vad()
        said = self.vad_said(vad)
        said["mode"] = mode
        said["lost"] = self.audio["lost"]
        if mode == "command":
            try:
                said.update(self.hw.kws(vad))
            except MemoryError:
                gc.collect()
                said["err"] = "memory"
            except Exception as e:
                said["err"] = str(e)[:80]
                self.fault("kws: %s" % e)
        self.audio = None
        return said

    # ------------------------------------------------------- calibration ---
    def calibrate(self, payload):
        try:
            a = json.loads(payload)
        except Exception:
            return {"ok": 0, "err": "json"}
        do = a.get("do", "get")
        if do == "measure":
            was = self.state
            if was != "active":
                self.wake(["tags"], 0)
            try:
                m = self.hw.measure_tag(int(a.get("frames", 15)), self.intr)
            finally:
                if was != "active":
                    self.sleep()
            if m is None:
                return {"do": do, "ok": 0, "err": "no tag"}
            m["d_mm"] = float(a["d_mm"])
            tag_mm = float(a.get("tag_mm", self.intr.tag_mm))
            m["fx"] = round(m["side_px"] * m["d_mm"] / tag_mm, 2)
            self.calib = [c for c in self.calib if abs(c["d_mm"] - m["d_mm"]) > 1] + [m]
            m["do"], m["ok"], m["kept"] = do, 1, len(self.calib)
            return m
        if do == "save":
            tag_mm = float(a.get("tag_mm", TAG_MM))
            if len(self.calib) < 1 or not self.intr.save(self.calib, tag_mm):
                return {"do": do, "ok": 0, "err": "no measurements"}
            self.calib = []
            said = self.intr.describe()
            said.update({"do": do, "ok": 1})
            return said
        if do == "reset":
            self.intr.forget()
            self.calib = []
        said = self.intr.describe()
        said.update({"do": do, "ok": 1, "kept": len(self.calib)})
        return said

    # ------------------------------------------------------------ health ---
    def fault(self, words):
        self.errors += 1
        self.last_error = words[:120]

    def pipe_said(self):
        return {"p": self.pipes, "fps": self.fps, "state": self.state, "wake_ms": self.wake_ms, "idle_ms": self.idle_after, "t": self.hw.now_ms()}

    def health(self):
        h = {
            "t": self.hw.now_ms(),
            "up_ms": since(self.hw.now_ms(), self.started),
            "state": "listening" if self.audio is not None else self.state,
            "p": self.pipes,
            "fps_set": self.fps,
            "fps": self.pace(),
            "frames": self.frames,
            "errors": self.errors,
            "last_error": self.last_error,
            "wake_ms": self.wake_ms,
            "idle_ms": self.idle_after,
            "calibrated": 1 if self.intr.calibrated else 0,
            # The K210 has no temperature sensor, nor has the OV7740.
            "temp": None,
        }
        h.update(self.hw.health())
        return h

    def hello(self):
        h = dict(self.info)
        h["up_ms"] = since(self.hw.now_ms(), self.started)
        h["calibrated"] = 1 if self.intr.calibrated else 0
        h["state"] = self.state
        return h


# ===================================================================== K210 ===
# Everything below is the camera's own (MaixPy); nothing in it may stop the
# program before it listens on the link: what fails is said in its hello.


class K210:
    """The UnitV's hardware for Program: the sensor, the KPU, the clock, the
    front end, the light."""

    def __init__(self):
        import sensor
        import image

        self.sensor_mod, self.image = sensor, image
        try:
            import catalyst

            self.cat = catalyst
        except ImportError:
            self.cat = None
        try:
            import KPU

            self.kpu = KPU
        except ImportError:
            self.kpu = None
        self.uart = None
        self.light = None
        self.sensor_ok = False
        self.sensor_said = "not started"
        self.sensor_on = False
        self.face = None
        self.face_said = "not loaded"
        self.kws_task = None
        self.kws_said = "not loaded"
        self.kws_in = None
        self.lamp(10, 5, 0)

    # ------------------------------------------------------------- time ---
    def now_ms(self):
        if self.cat is not None:
            return self.cat.time_ms()
        return time.ticks_ms() & 0x3FFFFFFF

    def now_us(self):
        if self.cat is not None:
            return self.cat.time_us()
        return time.ticks_us()

    def intrinsics(self):
        return Intrinsics(ROOT + "/intrinsics.json")

    # ------------------------------------------------------------ light ---
    def lamp(self, r, g, b):
        if not LED:
            return
        try:
            if self.light is None:
                from modules import ws2812

                self.light = ws2812(8, 1)
            self.light.set_led(0, (r, g, b))
            self.light.display()
        except Exception:
            pass

    # ----------------------------------------------------------- sensor ---
    def start_sensor(self):
        """Set up once at start, before the UART (MaixPy issue 363: a UART
        set up first can stop sending), then powered down until a pipeline
        wants it. Tried three times: a sensor slow to wake can fail the
        first."""
        s = self.sensor_mod
        for _ in range(3):
            try:
                s.reset()
                s.set_pixformat(s.RGB565)
                s.set_framesize(s.QVGA)
                s.set_vflip(FLIP)
                s.set_hmirror(MIRROR)
                s.skip_frames(time=200)
                self.sensor_ok, self.sensor_said = True, "ok"
                break
            except Exception as e:
                self.sensor_said = "failed: %s" % e
                time.sleep_ms(300)
        self.sensor_on = self.sensor_ok
        self.sensor_power(False)

    def sensor_power(self, on):
        if not self.sensor_ok or on == self.sensor_on:
            return
        s = self.sensor_mod
        try:
            if on:
                if self.cat is not None:
                    self.cat.sensor(True)
                else:
                    s.shutdown(False)
                s.run(1)
                try:
                    # Its exposure settles: about a fifth of a second.
                    s.skip_frames(time=200)
                except Exception:
                    # Not back as it was: set up afresh.
                    s.reset()
                    s.set_pixformat(s.RGB565)
                    s.set_framesize(s.QVGA)
                    s.set_vflip(FLIP)
                    s.set_hmirror(MIRROR)
                    s.skip_frames(time=200)
            else:
                s.run(0)
                if self.cat is not None:
                    self.cat.sensor(False)
                else:
                    s.shutdown(True)
            self.sensor_on = on
        except Exception as e:
            self.sensor_said = "power: %s" % e

    # -------------------------------------------------------------- KPU ---
    def kpu_power(self, on):
        if self.cat is not None:
            self.cat.kpu(on)

    def load_face(self):
        """Sipeed's YOLOv2 face detector (MaixPy_scripts, MIT): the one the
        handheld sent over Grove (in /flash), else where the installer wrote
        it (0x300000)."""
        if self.face is not None or self.kpu is None:
            return self.face
        for where in ("/flash/face.kmodel", 0x300000):
            if isinstance(where, str) and not L.exists(where):
                continue
            try:
                self.face = self.kpu.load(where)
                self.kpu.init_yolo2(self.face, 0.5, 0.3, 5, (1.889, 2.5245, 2.9465, 3.94056, 3.99987, 5.3658, 5.155437, 6.92275, 6.718375, 9.01025))
                self.face_said = "face yolo2 320x240 (%s)" % (where if isinstance(where, str) else "flash 0x300000")
                break
            except Exception as e:
                self.face = None
                self.face_said = "failed: %s" % e
        return self.face

    def load_kws(self):
        if self.kws_task is not None or self.kpu is None or self.cat is None:
            return self.kws_task
        try:
            import ustruct

            self.kws_task = self.kpu.load(ROOT + "/kws.kmodel")
            self.cat.fe_norm(ustruct.pack("40f", *KWS_MEAN), ustruct.pack("40f", *KWS_INV_STD))
            # 48 x 40 float32, as fe_window writes it and the KPU reads it.
            self.kws_in = bytearray(4 * 48 * 40)
            self.kws_said = "kws 48x40 (/flash/kws.kmodel)"
        except Exception as e:
            self.kws_task = None
            self.kws_said = "failed: %s" % e
        return self.kws_task

    # ----------------------------------------------------------- states ---
    def wake(self, pipes):
        """Full clock, the sensor and (for faces) the KPU on: how long it
        took, in ms."""
        t0 = self.now_ms()
        self.clock(CLOCK_FULL)
        self.need(pipes)
        self.lamp(0, 0, 6)
        return since(self.now_ms(), t0)

    def need(self, pipes):
        self.sensor_power(True)
        if "faces" in pipes and self.load_face() is not None:
            self.kpu_power(True)
        else:
            self.kpu_power(False)

    def sleep(self):
        self.sensor_power(False)
        self.kpu_power(False)
        self.lamp(0, 0, 0)
        gc.collect()
        self.clock(CLOCK_IDLE)

    def clock(self, hz):
        if self.cat is None or self.uart is None:
            return
        # What was written goes out at the old clock first.
        time.sleep_ms(3)
        self.cat.power(clock=hz, uarts=(self.uart,))

    # ---------------------------------------------------------- capture ---
    def capture(self, pipes, n, intr):
        img = self.sensor_mod.snapshot()
        t = self.now_ms()
        r = {"n": n, "t": t, "w": W, "h": H}
        if "faces" in pipes and self.face is not None:
            found = self.kpu.run_yolo2(self.face, img) or []
            r["f"] = [[o.x(), o.y(), o.w(), o.h(), int(o.value() * 100)] for o in found]
        if "codes" in pipes:
            r["q"] = [[c.x(), c.y(), c.w(), c.h(), c.payload()] for c in img.find_qrcodes()]
        if "tags" in pipes:
            r["a"] = self.tags(img, intr)
        if "blobs" in pipes:
            r["b"] = [[first_bit(b.code()), b.x(), b.y(), b.w(), b.h(), b.pixels()] for b in img.find_blobs(COLOURS, pixels_threshold=120, area_threshold=120, merge=False)]
        jpeg = None
        if "preview" in pipes:
            jpeg = img.compress(quality=50).to_bytes()
        return r, jpeg

    def find_tags(self, img, intr):
        # At 160 x 120 (memory, and pace), the lens figures halved.
        small = img.resize(160, 120)
        tags = small.find_apriltags(families=self.image.TAG36H11, fx=intr.fx / 2, fy=intr.fy / 2, cx=intr.cx / 2, cy=intr.cy / 2)
        del small
        return tags

    def tags(self, img, intr):
        u = intr.unit_mm
        return [[t.id(), t.cx() * 2, t.cy() * 2, int(t.rotation() * 1000), int(t.x_translation() * u), int(t.y_translation() * u), int(-t.z_translation() * u)] for t in self.find_tags(img, intr)]

    def measure_tag(self, frames, intr):
        """A tag held square-on: its side in pixels at 320 x 240 and the
        pose's raw distance, the medians over the frames it was seen in."""
        sides, zs, ids = [], [], []
        for _ in range(max(1, frames)):
            img = self.sensor_mod.snapshot()
            tags = self.find_tags(img, intr)
            if tags:
                t = max(tags, key=lambda t: t.w() * t.h())
                sides.append(side_px(t.corners()) * 2)
                zs.append(-t.z_translation())
                ids.append(t.id())
        if not sides:
            return None
        sides.sort()
        zs.sort()
        return {"side_px": round(sides[len(sides) // 2], 2), "z_raw": round(zs[len(zs) // 2], 4), "fx_used": intr.fx, "seen": len(sides), "id": ids[-1]}

    # ----------------------------------------------------------- speech ---
    def audio_reset(self):
        if self.cat is not None:
            self.cat.fe_reset()
            self.lamp(0, 4, 4)

    def audio_push(self, pcm):
        if self.cat is not None:
            self.cat.fe_push(pcm)

    def vad(self):
        if self.cat is None:
            return (0, 0, -1, 0)
        return self.cat.fe_vad()

    def kws(self, vad):
        """The keyword spotter on the loudest 0.96 s of the speech: the top
        three (label index, probability), the 64-d embedding (for words the
        user taught the handheld), and how long it took."""
        if self.load_kws() is None:
            return {"err": self.kws_said}
        c = self.cat
        t0 = self.now_us()
        state, start = vad[0], vad[1]
        w = c.fe_best(start if state else 0, c.fe_count() - 1)
        c.fe_window(w, self.kws_in)
        self.clock(CLOCK_FULL)
        self.kpu_power(True)
        try:
            probs = self.kpu.forward(self.kws_task, self.kws_in)[:]
            emb = self.kpu.get_output(self.kws_task, 1)[:]
        finally:
            self.kpu_power(False)
        order = sorted(range(len(probs)), key=lambda i: -probs[i])[:3]
        self.lamp(0, 0, 0)
        return {"top": [[i, round(probs[i], 3)] for i in order], "emb": [round(v, 4) for v in emb], "win_ms": w * FRAME_MS, "ms": (self.now_us() - t0) // 1000}

    # ----------------------------------------------------------- health ---
    def health(self):
        h = {"gc_free": gc.mem_free(), "gc_used": gc.mem_alloc(), "sensor": self.sensor_on, "kpu_face": self.face is not None, "kpu_kws": self.kws_task is not None}
        if self.cat is not None:
            h["heap"] = self.cat.heap()
            h["cpu_hz"] = self.cat.power()
            h["kpu"] = self.cat.kpu()
            h["reset"] = self.cat.reset_cause()
        return h


class Link:
    """The UART the handheld was found on: frames in and out, the baud
    rate's steps (back to the rate before if no frame comes at a new one
    within a second, to 115 200 after five quiet seconds)."""

    def __init__(self, uart, via_usb):
        self.uart, self.via_usb = uart, via_usb
        self.decoder = L.Decoder()
        self.seq = 0
        self.baud = self.before = 115200
        self.moved_at = None

    def send(self, kind, payload):
        self.seq = (self.seq + 1) & 0xFF
        self.uart.write(L.frame(kind, self.seq, payload))

    def set_baud(self, rate):
        self.baud = rate
        self.uart.init(rate, 8, None, 1, timeout=0, read_buf_len=16384)


def main():
    import os
    import sys

    booted = time.ticks_ms()
    hw = K210()
    hw.start_sensor()

    full = 1
    try:
        for n in ("find_apriltags", "find_qrcodes", "find_blobs"):
            if not hasattr(hw.image.Image, n):
                full = 0
        if hw.kpu is None or not hasattr(hw.kpu, "init_yolo2") or hw.cat is None:
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
    try:
        with open(ROOT + "/catalyst.json") as f:
            stamp = json.loads(f.read())
    except Exception:
        stamp = None
    trial = None
    try:
        with open(ROOT + "/catalyst.trial") as f:
            trial = int(f.read().split()[0])
    except Exception:
        trial = None

    # Idle until the handheld speaks: the CPU slow, the sensor down.
    hw.lamp(0, 2, 0) if hw.sensor_ok else hw.lamp(12, 0, 0)
    if hw.cat is not None:
        hw.cat.kpu(False)
        hw.cat.power(clock=CLOCK_IDLE)
    listening_ms = time.ticks_diff(time.ticks_ms(), booted)

    import catalyst_port

    uart, via_usb, _rx, _tx, first = catalyst_port.find(poll_ms=20)
    hw.uart = uart
    link = Link(uart, via_usb)
    hw.lamp(0, 0, 0)
    L.confirm(ROOT)

    info = {
        "v": VERSION,
        "fw": hw.cat.version() if hw.cat is not None else "maixpy",
        "fwv": hw.cat.FIRMWARE if hw.cat is not None else 0,
        "upd": 1,
        "mp": maixpy,
        "full": full,
        "sensor": hw.sensor_said,
        "kpu": hw.face_said,
        "kws": "present" if L.exists(ROOT + "/kws.kmodel") else "missing",
        "crc": L.crc_kind,
        "fov": 80,
        "w": W,
        "h": H,
        "up_ms": 0,
        "listen_ms": listening_ms,
        "usb": via_usb,
        "cwd": cwd,
        "free": L.free_bytes(ROOT),
        "trial": trial,
        "rolled_back": L.exists(ROOT + "/catalyst.rolledback"),
        "boot": globals().get("BOOT_SAID", "maixpy"),
        "stamp": stamp,
        "caps": ["idle", "pipes", "sync", "health", "audio", "calib"] if hw.cat is not None else ["pipes"],
    }
    prog = Program(hw, info)
    prog.started = (prog.started - time.ticks_diff(time.ticks_ms(), booted)) & 0x3FFFFFFF
    files = L.Files(ROOT)
    heard_at = time.ticks_ms()

    def take(kind, payload):
        nonlocal heard_at
        heard_at = time.ticks_ms()
        if kind == L.BAUD and len(payload) >= 4 and not link.via_usb:
            rate = L.u32(payload)
            link.send(L.BAUD_BACK, payload)
            # The answer out at the old rate before the change (about 15 bytes).
            time.sleep_ms(2 + 160000 // link.baud)
            link.before = link.baud
            link.set_baud(rate)
            link.moved_at = time.ticks_ms()
            return
        if kind == L.PING:
            link.moved_at = None
        if kind in (L.PUT, L.CHUNK, L.DONE, L.COMMIT):
            if kind == L.PUT:
                prog.sleep()
                hw.clock(CLOCK_FULL)
                hw.lamp(8, 0, 10)
                if cwd == "/sd":
                    link.send(L.PUT_BACK, json.dumps({"err": "card"}).encode())
                    return
            back, said, committed = L.handle_file(files, kind, payload, json.loads, json.dumps)
            link.send(back, said)
            if committed:
                # Out on the wire, then the new program from its first line.
                time.sleep_ms(50 + 200000 // link.baud)
                import machine

                machine.reset()
            return
        for k, p in prog.handle(kind, payload):
            link.send(k, p)

    for kind, _s, payload in first:
        take(kind, payload)

    while True:
        try:
            if uart.any():
                data = uart.read()
                if data:
                    for kind, _s, payload in link.decoder.push(data):
                        take(kind, payload)
            # No frame at a new rate for a second: back to the rate before it.
            if link.moved_at is not None and time.ticks_diff(time.ticks_ms(), link.moved_at) > 1000:
                link.moved_at = None
                link.set_baud(link.before)
            # Nothing at all for five seconds at a raised rate (the handheld
            # lost it, or started again): back to where both start.
            if link.baud != 115200 and link.moved_at is None and time.ticks_diff(time.ticks_ms(), heard_at) > 5000:
                link.set_baud(115200)
            for k, p in prog.tick():
                link.send(k, p)
            rest = prog.due_in()
            if rest > 0 and not uart.any():
                time.sleep_ms(rest)
        except MemoryError:
            gc.collect()
        except Exception as e:
            prog.fault(str(e))
            try:
                link.send(L.LOG, str(e).encode()[:200])
            except Exception:
                pass
            time.sleep_ms(20)


if __name__ == "__main__":
    main()

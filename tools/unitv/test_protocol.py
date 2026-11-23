"""The camera's side of the Grove link, checked on a computer (CPython):

- the frames (firmware/frozen/catalyst_link.py) are the handheld's
  (smartcam.rs checks the same bytes), the CRC is zlib's, the decoder skips
  junk and finds frames however the bytes are cut;
- files arrive in chunks, resume after a cut, are refused on a bad CRC, move
  into place together with the old ones kept; the frozen boot
  (catalyst_boot) finishes a commit cut off part way and rolls a new program
  back after two starts that never reached the handheld;
- the program (main.py's Program, with a stand-in for the K210): idle until
  asked, only the pipelines asked for at the pace asked for, idle again after
  a quiet while or when asked, the clock sync, health, audio chunks (lost
  ones kept as silence, repeated ones dropped) with the voice's start and
  end, the keyword spotter's answer, and the tag pose calibration.

    python tools/unitv/test_protocol.py
"""

import json
import os
import pathlib
import struct
import sys
import tempfile
import zlib

here = pathlib.Path(__file__).parent
frozen = here / "firmware" / "frozen"
sys.path.insert(0, str(frozen))

# The pure-Python CRC (the fallback on a firmware without binascii) is
# checked first, with binascii hidden from the module.
_saved = {k: sys.modules.get(k) for k in ("binascii", "ubinascii")}
sys.modules["binascii"] = None
sys.modules["ubinascii"] = None
import catalyst_link as slow  # noqa: E402

assert slow.crc_kind == "python"
assert slow.crc32(b"123456789") == 0xCBF43926 == zlib.crc32(b"123456789")
assert slow.crc32(b"") == 0
assert slow.crc32(b"6789", slow.crc32(b"12345")) == zlib.crc32(b"123456789")
del sys.modules["catalyst_link"]
for k, v in _saved.items():
    if v is None:
        sys.modules.pop(k, None)
    else:
        sys.modules[k] = v

import catalyst_boot  # noqa: E402
import catalyst_link as L  # noqa: E402

frame, Decoder, crc32 = L.frame, L.Decoder, L.crc32
Files, settle, confirm = L.Files, L.settle, L.confirm

# The same bytes the handheld makes (smartcam.rs: the_wire_bytes_are_the_cameras).
known = frame(0x85, 1, b'{"n":1}')
assert known.hex() == "c57a85010700e77b226e223a317d" + zlib.crc32(known[2:-4]).to_bytes(4, "little").hex(), known.hex()
assert known.hex() == "c57a85010700e77b226e223a317d5ed2d160", known.hex()

# The kinds both ends use (smartcam.rs's frame module has the same numbers).
assert (L.IDLE, L.PIPE, L.SYNCH, L.HEALTH, L.AUDIO, L.AUDIO_END, L.CALIB) == (0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F)
assert (L.IDLE_BACK, L.PIPE_BACK, L.SYNC_BACK, L.HEALTH_BACK, L.HEARD, L.COMMAND, L.CALIB_BACK) == (0x8C, 0x8D, 0x8E, 0x8F, 0x90, 0x91, 0x92)

# Junk, a payload holding the sync word, a corrupted frame, cut every way.
a = frame(0x85, 1, b'{"n":1}')
b = frame(0x86, 2, bytes([0xC5, 0x7A, 1, 2, 3, 0xC5]))
bad = bytearray(frame(0x84, 3, b"x"))
bad[-1] ^= 0x40
c = frame(0x87, 4, b"after the bad one")
wire = b"boot noise \xc5" + a + b"\x00\xc5\xc5\x7a" + b + bytes(bad) + c
for cut in (1, 2, 3, 5, 7, 11, 64, len(wire)):
    d = Decoder()
    got = []
    for i in range(0, len(wire), cut):
        got += d.push(wire[i:i + cut])
    assert [(k, s) for k, s, _ in got] == [(0x85, 1), (0x86, 2), (0x87, 4)], (cut, got)
    assert got[1][2] == bytes([0xC5, 0x7A, 1, 2, 3, 0xC5])
    assert d.bad >= 1

assert L.u64(L.le64(0x0123456789ABCDEF)) == 0x0123456789ABCDEF

# ------------------------------------------------------------ the update ---


def write(root, name, data):
    with open(os.path.join(root, name), "wb") as f:
        f.write(data)


def read(root, name):
    try:
        with open(os.path.join(root, name), "rb") as f:
            return f.read()
    except OSError:
        return None


def send(files, name, data, upto=None, chunk=1000):
    """As the handheld does: begin, chunks from where the camera has got to,
    and (if all of it went) the end."""
    said = files.begin(name, len(data), zlib.crc32(data), "id-" + name)
    assert "have" in said, said
    have = said["have"]
    stop = len(data) if upto is None else upto
    while have < stop:
        piece = data[have:min(have + chunk, stop)]
        now = files.chunk(have, piece)
        assert now == have + len(piece), (now, have)
        have = now
    return said["have"], (files.end(name) if upto is None else None)


def boot(root):
    return catalyst_boot.run(root)


old_main, new_main = b"# old program\n" * 40, b"# new program\n" * 300
model = bytes(range(256)) * 50
kws = bytes(range(7, 250)) * 30

with tempfile.TemporaryDirectory() as root:
    write(root, "main.py", old_main)
    write(root, "catalyst.json", b'{"main":"old"}')
    files = Files(root)

    # A name it does not keep is refused; nothing open takes no chunk.
    assert files.begin("evil.py", 10, 0, "x").get("err") == "name"
    assert files.chunk(0, b"x") == 0xFFFFFFFF

    # The model, cut off part way (the power went): a fresh start resumes
    # from what is there, and a chunk out of place is not written.
    started, _ = send(files, "face.kmodel", model, upto=5000)
    assert started == 0
    files.close()
    files = Files(root)
    assert files.begin("face.kmodel", len(model), zlib.crc32(model), "id-face.kmodel") == {"name": "face.kmodel", "have": 5000}
    assert files.chunk(7000, model[7000:8000]) == 5000, "a chunk beyond the end is not written"
    assert files.chunk(4000, model[4000:5000]) == 5000, "a chunk again (its answer lost) is not written twice"
    files.close()
    started, ended = send(files, "face.kmodel", model)
    assert started == 5000 and ended == {"name": "face.kmodel", "ok": 1}, (started, ended)

    # A different file under the same name starts over.
    other = b"z" * 3000
    assert files.begin("face.kmodel", len(other), zlib.crc32(other), "another")["have"] == 0
    files.close()
    started, ended = send(files, "face.kmodel", model)
    assert started == 0 and ended["ok"] == 1

    # A file whose bytes do not match its CRC is refused and thrown away.
    files.begin("main.py", len(new_main), zlib.crc32(new_main) ^ 1, "bad")
    files.chunk(0, new_main)
    assert files.end("main.py") == {"name": "main.py", "ok": 0, "err": "crc"}
    assert read(root, "main.py.new") is None and read(root, "main.py.part") is None

    _, ended = send(files, "main.py", new_main)
    assert ended["ok"] == 1
    _, ended = send(files, "kws.kmodel", kws)
    assert ended["ok"] == 1

    # A commit naming a file that did not arrive does nothing.
    assert files.commit(["main.py", "kws.kmodel", "face.kmodel", "nothing.py"], "{}")["ok"] == 0
    assert read(root, "main.py") == old_main

    # All of them into place at once, the old ones kept, a trial begun. The
    # same through handle_file, as the program and the safe program do.
    back, said, committed = L.handle_file(files, L.COMMIT, json.dumps({"files": ["main.py", "kws.kmodel", "face.kmodel"], "stamp": {"main": "new"}}).encode(), json.loads, json.dumps)
    assert back == L.COMMIT_BACK and json.loads(said) == {"ok": 1} and committed
    assert read(root, "main.py") == new_main and read(root, "main.py.old") == old_main
    assert read(root, "kws.kmodel") == kws and read(root, "face.kmodel") == model
    assert read(root, "face.kmodel.old") is None, "the model was not a file before"
    assert json.loads(read(root, "catalyst.json")) == {"main": "new"} and read(root, "catalyst.json.old") == b'{"main":"old"}'
    trial = read(root, "catalyst.trial").decode().replace("\r", "").split("\n")
    assert trial[0] == "0" and "main.py 1" in trial and "kws.kmodel 0" in trial, trial
    assert not any(n.endswith((".new", ".part")) for n in os.listdir(root)), os.listdir(root)
    assert settle(root) is False, "nothing left to do"

    # The boot: the new program's starts counted; two that never reached the
    # handheld, and the third start has the old files back.
    for count in ("1", "2"):
        assert boot(root) == "trial " + count
        assert read(root, "catalyst.trial").decode().replace("\r", "").split("\n")[0] == count
        assert read(root, "main.py") == new_main
    assert boot(root) == "rolled back"
    assert read(root, "main.py") == old_main
    assert read(root, "kws.kmodel") is None and read(root, "face.kmodel") is None, "models that were not there before are gone again"
    assert read(root, "catalyst.json") == b'{"main":"old"}'
    assert read(root, "catalyst.trial") is None and read(root, "catalyst.rolledback") is not None
    assert boot(root) == "ok"
    assert read(root, "main.py") == old_main, "nothing more at the next start"

with tempfile.TemporaryDirectory() as root:
    # A program that starts and is heard ends its trial: nothing rolls back.
    write(root, "main.py", old_main)
    files = Files(root)
    send(files, "main.py", new_main)
    files.commit(["main.py"], "{}")
    boot(root)
    confirm(root)
    for _ in range(4):
        assert boot(root) == "ok"
    assert read(root, "main.py") == new_main and read(root, "catalyst.rolledback") is None
    # Removing what is not there is quiet (no SPIFFS error printed on the camera).
    confirm(root)
    L.remove(os.path.join(root, "nothing"))

with tempfile.TemporaryDirectory() as root:
    # The power gone in the middle of a commit: main.py already moved to
    # .old, its .new not yet in place. The next start finishes it.
    write(root, "main.py.old", old_main)
    write(root, "main.py.new", new_main)
    write(root, "catalyst.json.new", b'{"main":"new"}')
    write(root, "catalyst.commit", b"main.py 1\ncatalyst.json 0\n")
    assert boot(root) == "trial 1", "finished, then counted as its first start"
    assert read(root, "main.py") == new_main and read(root, "main.py.old") == old_main
    assert read(root, "catalyst.json") == b'{"main":"new"}'
    assert read(root, "catalyst.commit") is None
    assert settle(root) is False

# ----------------------------------------------------------- the program ---

space = {"__name__": "catalyst_test"}
exec(compile((here / "main.py").read_text(), "main.py", "exec"), space)
Program, Intrinsics = space["Program"], space["Intrinsics"]

# The keyword spotter's normalisation in main.py is kws.json's.
spec = json.loads((here / "speech" / "kws.json").read_text())
assert space["KWS_MEAN"] == [round(x, 6) for x in spec["normalisation"]["mean"]]
assert space["KWS_INV_STD"] == [round(x, 6) for x in spec["normalisation"]["inv_std"]]
assert spec["input"]["shape"] == [1, 1, 48, 40] and spec["frontend"]["hop"] == 320


class FakeK210:
    """The hardware as Program sees it, with a clock the test moves."""

    def __init__(self, root):
        self.root = root
        self.t = 1_000
        self.calls = []
        self.pcm = bytearray()
        self.vad_now = (0, 0, -1, 0)
        self.tag = None

    def now_ms(self):
        return self.t & 0x3FFFFFFF

    def now_us(self):
        return self.t * 1000 + 321

    def intrinsics(self):
        return Intrinsics(os.path.join(self.root, "intrinsics.json"))

    def wake(self, pipes):
        self.calls.append(("wake", tuple(pipes)))
        return 240

    def need(self, pipes):
        self.calls.append(("need", tuple(pipes)))

    def sleep(self):
        self.calls.append(("sleep",))

    def capture(self, pipes, n, intr):
        self.calls.append(("capture", tuple(pipes)))
        r = {"n": n, "t": self.now_ms(), "w": 320, "h": 240}
        if "tags" in pipes:
            r["a"] = [[3, 160, 120, 0, 0, 0, 1000]]
        return r, (b"\xff\xd8 jpeg" if "preview" in pipes else None)

    def audio_reset(self):
        self.pcm = bytearray()
        self.calls.append(("audio_reset",))

    def audio_push(self, pcm):
        self.pcm += bytes(pcm)

    def vad(self):
        return self.vad_now

    def kws(self, vad):
        return {"top": [[8, 0.93], [9, 0.04], [30, 0.01]], "emb": [0.125] * 64, "win_ms": 120, "ms": 2}

    def measure_tag(self, frames, intr):
        if self.tag is None:
            return None
        d = self.tag
        # A square-on tag at d mm: its side fx * S / d pixels, and a pose
        # whose z is in half-sides at the fx it was found with.
        true_fx = 250.0
        side = true_fx * 165.1 / d
        return {"side_px": side, "z_raw": d / (165.1 / 2) * intr.fx / true_fx, "fx_used": intr.fx, "seen": frames, "id": 7}

    def health(self):
        return {"gc_free": 500000, "heap": 3000000, "cpu_hz": 100000000, "kpu": False}


def frames_of(out):
    return [(k, json.loads(p) if k not in (L.PREVIEW, L.SYNC_BACK, L.PONG) else p) for k, p in out]


with tempfile.TemporaryDirectory() as root:
    hw = FakeK210(root)
    prog = Program(hw, {"v": 3, "fw": "catalyst-coprocessor", "fwv": 1})

    # Idle from the start: nothing is captured however long it waits.
    for _ in range(50):
        hw.t += 100
        assert prog.tick() == []
    assert prog.state == "idle" and not [c for c in hw.calls if c[0] == "capture"]

    # The hello says what it is, how long it has been up, that it is idle.
    (k, hello), = frames_of(prog.handle(L.HELLO, b'{"v":3}'))
    assert k == L.HELLO_BACK and hello["v"] == 3 and hello["state"] == "idle" and hello["up_ms"] == 5000 and hello["calibrated"] == 0

    # Pipelines: only those asked for (unknown names dropped), at the pace
    # asked for; the wake measured.
    (k, said), = frames_of(prog.handle(L.PIPE, b'{"p":["tags","lasers"],"fps":5,"idle_ms":3000}'))
    assert k == L.PIPE_BACK and said["p"] == ["tags"] and said["fps"] == 5 and said["state"] == "active" and said["wake_ms"] == 240
    assert hw.calls[-1] == ("wake", ("tags",))
    captured = []
    for _ in range(100):  # one second, in 10 ms steps
        for kind, p in prog.tick():
            assert kind == L.RESULTS
            r = json.loads(p)
            captured.append(r["t"])
            assert r["a"] == [[3, 160, 120, 0, 0, 0, 1000]] and "f" not in r
        hw.t += 10
        if hw.t % 1000 == 0:
            prog.handle(L.PING, b"alive")
    assert len(captured) == 5, captured
    assert all(b - a == 200 for a, b in zip(captured, captured[1:])), captured
    assert prog.due_in() <= 20

    # More pipelines while active: told what it needs now, not woken again.
    frames_of(prog.handle(L.PIPE, b'{"p":["tags","preview","faces"],"fps":0}'))
    assert hw.calls[-1] == ("need", ("faces", "tags", "preview"))
    out = frames_of(prog.tick())
    assert [k for k, _ in out] == [L.RESULTS, L.PREVIEW] and out[1][1].startswith(b"\xff\xd8")
    assert prog.due_in() == 0, "as fast as it can"

    # Quiet for longer than idle_ms (3 s, as asked): idle by itself.
    hw.t += 3001
    assert prog.tick() == [] and prog.state == "idle" and hw.calls[-1] == ("sleep",)

    # The old WANT still works (version 2 handhelds), as fast as it can.
    (k, echo), = prog.handle(L.WANT, b'{"faces":1,"codes":0,"tags":0,"blobs":0,"preview":0}')
    assert k == L.WANT_BACK and prog.pipes == ["faces"] and prog.fps == 0
    # IDLE, at once; and an empty PIPE is idle too.
    (k, said), = frames_of(prog.handle(L.IDLE, b"{}"))
    assert k == L.IDLE_BACK and said["state"] == "idle" and prog.state == "idle"
    prog.handle(L.PIPE, b'{"p":["codes"],"fps":10}')
    (k, said), = frames_of(prog.handle(L.PIPE, b'{"p":[]}'))
    assert said["state"] == "idle" and prog.pipes == []

    # Sync: the handheld's 8 bytes back, then the camera's microseconds.
    mine = struct.pack("<Q", 123456789012)
    (k, p), = prog.handle(L.SYNCH, mine)
    assert k == L.SYNC_BACK and p[:8] == mine and struct.unpack("<Q", p[8:16])[0] == hw.now_us()

    # Health: its state, pace, errors, memory, clock; no temperature sensor.
    (k, h), = frames_of(prog.handle(L.HEALTH, b""))
    assert k == L.HEALTH_BACK and h["state"] == "idle" and h["frames"] == 6 and h["errors"] == 0
    assert h["temp"] is None and h["cpu_hz"] == 100000000 and h["gc_free"] == 500000 and "last_error" in h

    # ------------------------------------------------------------ speech ---
    def chunk(offset, n, value=0):
        return struct.pack("<I", offset) + struct.pack("<%dh" % n, *([value] * n))

    assert prog.handle(L.AUDIO, chunk(0, 320)) == []
    assert hw.calls[-1] == ("audio_reset",) and len(hw.pcm) == 640
    # The voice starts: said once, with where.
    hw.vad_now = (1, 2, 9, 5)
    (k, heard), = frames_of(prog.handle(L.AUDIO, chunk(320, 320)))
    assert k == L.HEARD and heard == {"vad": 1, "start_ms": 40, "end_ms": 212, "speech_ms": 100, "n": 640}
    assert prog.handle(L.AUDIO, chunk(640, 320)) == [], "nothing new to say"
    # A chunk lost on the way: its time kept as silence; one sent again: dropped.
    prog.handle(L.AUDIO, chunk(1280, 320, 7))
    assert len(hw.pcm) == 2 * 1600 and hw.pcm[2 * 960:2 * 1280] == bytes(640)
    prog.handle(L.AUDIO, chunk(960, 320, 9))
    assert len(hw.pcm) == 2 * 1600, "already here"
    # Listening keeps it from idling but does not wake the sensor.
    assert prog.state == "idle"
    (k, h), = frames_of(prog.handle(L.HEALTH, b""))
    assert h["state"] == "listening"
    hw.vad_now = (2, 2, 20, 16)
    (k, heard), = frames_of(prog.handle(L.AUDIO, chunk(1600, 320)))
    assert heard["vad"] == 2, "the end of the voice, for dictation to stop by itself"
    # The end: the keyword spotter's top three and embedding, the speech's span.
    (k, said), = frames_of(prog.handle(L.AUDIO_END, b'{"mode":"command"}'))
    assert k == L.COMMAND and said["mode"] == "command" and said["top"][0] == [8, 0.93] and len(said["emb"]) == 64
    assert said["start_ms"] == 40 and said["end_ms"] == 432 and said["lost"] == 320 and said["n"] == 1920
    # Dictation: the span only, no keyword spotter.
    prog.handle(L.AUDIO, chunk(0, 320))
    (k, said), = frames_of(prog.handle(L.AUDIO_END, b'{"mode":"dictation"}'))
    assert said["mode"] == "dictation" and "top" not in said
    # Nothing arriving: let go after a while; an end with no audio says so.
    prog.handle(L.AUDIO, chunk(0, 320))
    hw.t += 3500
    prog.tick()
    (k, said), = frames_of(prog.handle(L.AUDIO_END, b'{"mode":"command"}'))
    assert said["err"] == "no audio"

    # ------------------------------------------------------- calibration ---
    (k, said), = frames_of(prog.handle(L.CALIB, b'{"do":"get"}'))
    assert said["calibrated"] == 0 and said["fx"] == 238.4
    (k, said), = frames_of(prog.handle(L.CALIB, b'{"do":"measure","d_mm":1000,"tag_mm":165.1}'))
    assert said == {**said, "ok": 0, "err": "no tag"}
    hw.tag = 500.0
    (k, m1), = frames_of(prog.handle(L.CALIB, b'{"do":"measure","d_mm":500,"tag_mm":165.1,"frames":5}'))
    assert m1["ok"] == 1 and abs(m1["fx"] - 250.0) < 0.01 and m1["kept"] == 1
    assert prog.state == "idle", "woken for the measurement only"
    hw.tag = 1000.0
    (k, m2), = frames_of(prog.handle(L.CALIB, b'{"do":"measure","d_mm":1000,"tag_mm":165.1}'))
    assert m2["kept"] == 2
    (k, saved), = frames_of(prog.handle(L.CALIB, b'{"do":"save","tag_mm":165.1}'))
    assert saved["ok"] == 1 and saved["calibrated"] == 1 and abs(saved["fx"] - 250.0) < 0.01
    # The pose's unit: half the tag's side, found from the distances.
    assert abs(saved["unit_mm"] - 165.1 / 2) < 0.01, saved
    stored = json.loads(read(root, "intrinsics.json"))
    assert stored["family"] == "36h11" and len(stored["measures"]) == 2
    # Kept across a start; reset forgets it.
    again = Program(hw, {"v": 3})
    assert again.intr.calibrated and abs(again.intr.fx - 250.0) < 0.01
    (k, said), = frames_of(again.handle(L.CALIB, b'{"do":"reset"}'))
    assert said["calibrated"] == 0 and read(root, "intrinsics.json") is None

print("the link, updates over Grove, the boot, the program's messages: all passed")

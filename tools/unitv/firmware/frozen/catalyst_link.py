# Catalyst Coprocessor: the Grove link's frames and the files that arrive over
# it, frozen into the firmware so they are there whatever /flash holds (the
# program, main.py, and the safe program both use them). Plain MicroPython,
# so the same file runs on a computer for the tests (tools/unitv/test_protocol.py).
#
# A frame (the handheld's smartcam.rs has the same):
#   C5 7A | kind | seq | len (2, LE) | header check | payload | CRC-32 (LE)
# The header check is (kind + seq + len_lo + len_hi + 0x5A) & 0xFF; the CRC
# (IEEE, as zlib's) covers kind through payload. Junk between frames is
# skipped to the next sync word.

import os

SYNC = b"\xc5\x7a"
MAX = 32 * 1024

# Asked of the camera.
HELLO, BAUD, WANT, PING, PUT, CHUNK, DONE, COMMIT = 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08
IDLE, PIPE, SYNCH, HEALTH, AUDIO, AUDIO_END, CALIB = 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F
# Its answers (and what it sends unasked, once asked to).
HELLO_BACK, BAUD_BACK, WANT_BACK, PONG, RESULTS, PREVIEW, LOG = 0x81, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87
PUT_BACK, CHUNK_BACK, DONE_BACK, COMMIT_BACK = 0x88, 0x89, 0x8A, 0x8B
IDLE_BACK, PIPE_BACK, SYNC_BACK, HEALTH_BACK, HEARD, COMMAND, CALIB_BACK = 0x8C, 0x8D, 0x8E, 0x8F, 0x90, 0x91, 0x92

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


def u64(b, at=0):
    return u32(b, at) | (u32(b, at + 4) << 32)


def le64(v):
    return le32(v & 0xFFFFFFFF) + le32((v >> 32) & 0xFFFFFFFF)


# ----------------------------------------------------------------- files ---
# A newer program (and models) from the handheld, over the Grove link, into
# /flash: each file into name.new beside a note of what it is to be
# (name.part: its size, CRC and id, so a transfer cut off resumes where it
# stopped), its CRC checked when it is whole, then all moved into place at
# once (COMMIT). The files they replace are kept as name.old, and the boot
# (catalyst_boot) counts the new program's starts: two that never heard the
# handheld, and the old files come back. A commit cut off part way is
# finished at the next start, from its journal.

FILES = ("main.py", "boot.py", "face.kmodel", "kws.kmodel")
CHUNK_MAX = 4096


def exists(path):
    try:
        os.stat(path)
        return True
    except OSError:
        return False


def remove(path):
    # Only what is there: MaixPy's os.remove prints "SPIFFS Error Code
    # -10002" (not found) before it raises, which read like a fault.
    if exists(path):
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
    begun for the boot. True if there was one."""
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


def handle_file(files, kind, payload, loads, dumps):
    """PUT, CHUNK, DONE and COMMIT: (answer kind, answer payload, committed)."""
    if kind == PUT:
        try:
            a = loads(payload)
            said = files.begin(a["name"], int(a["size"]), int(a["crc"]), str(a.get("id", "")))
        except Exception as e:
            said = {"err": str(e)[:80]}
        return PUT_BACK, dumps(said).encode(), False
    if kind == CHUNK:
        have = 0xFFFFFFFF
        if len(payload) >= 4:
            try:
                have = files.chunk(u32(payload), payload[4:])
            except Exception:
                have = 0xFFFFFFFF
        return CHUNK_BACK, le32(have), False
    if kind == DONE:
        try:
            said = files.end(loads(payload)["name"])
        except Exception as e:
            said = {"ok": 0, "err": str(e)[:80]}
        return DONE_BACK, dumps(said).encode(), False
    if kind == COMMIT:
        try:
            a = loads(payload)
            said = files.commit(a["files"], dumps(a["stamp"]))
        except Exception as e:
            said = {"ok": 0, "err": str(e)[:80]}
        return COMMIT_BACK, dumps(said).encode(), bool(said.get("ok"))
    return None, None, False

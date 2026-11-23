"""main.py's link and its update over Grove, checked on a computer (CPython):
its frames are the handheld's (smartcam.rs checks the same bytes), its CRC
is zlib's, its decoder skips junk and finds frames however the bytes are
cut; files arrive in chunks, resume after a cut, are refused on a bad CRC,
move into place together with the old ones kept; and boot.py finishes a
commit cut off part way and rolls a new program back after two starts that
never reached the handheld.

    python tools/unitv/test_protocol.py
"""

import os
import pathlib
import tempfile
import zlib

here = pathlib.Path(__file__).parent
source = (here / "main.py").read_text()
# Only the link's and the files' own code: from the sync word to where the
# camera's own (MaixPy) part begins.
part = source[source.index("SYNC = "): source.index("# ---------------------------------------------------------------- sensor")]
part = part.replace("from ubinascii import crc32", "raise ImportError").replace("from binascii import crc32", "raise ImportError")
space = {"MAX": 32 * 1024, "os": os}
exec(part, space)
frame, Decoder, crc32 = space["frame"], space["Decoder"], space["crc32"]
Files, settle, confirm = space["Files"], space["settle"], space["confirm"]

# The pure-Python CRC (the fallback on a firmware without binascii) is zlib's,
# and runs on from a value as zlib's does (a file's CRC chunk by chunk).
assert crc32(b"123456789") == 0xCBF43926 == zlib.crc32(b"123456789")
assert crc32(b"") == 0
assert crc32(b"6789", crc32(b"12345")) == zlib.crc32(b"123456789")

# The same bytes the handheld makes (smartcam.rs: the_wire_bytes_are_the_cameras).
known = frame(0x85, 1, b'{"n":1}')
assert known.hex() == "c57a85010700e77b226e223a317d" + zlib.crc32(known[2:-4]).to_bytes(4, "little").hex(), known.hex()
assert known.hex() == "c57a85010700e77b226e223a317d5ed2d160", known.hex()

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


old_main, new_main = b"# old program\n" * 40, b"# new program\n" * 300
model = bytes(range(256)) * 50

with tempfile.TemporaryDirectory() as root:
    write(root, "main.py", old_main)
    write(root, "boot.py", b"# old boot\n")
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
    _, ended = send(files, "boot.py", b"# new boot\n")
    assert ended["ok"] == 1

    # A commit naming a file that did not arrive does nothing.
    assert files.commit(["main.py", "boot.py", "face.kmodel", "nothing.py"], "{}")["ok"] == 0
    assert read(root, "main.py") == old_main

    # All of them into place at once, the old ones kept, a trial begun.
    assert files.commit(["main.py", "boot.py", "face.kmodel"], '{"main":"new"}') == {"ok": 1}
    assert read(root, "main.py") == new_main and read(root, "main.py.old") == old_main
    assert read(root, "face.kmodel") == model and read(root, "face.kmodel.old") is None, "the model was not a file before"
    assert read(root, "catalyst.json") == b'{"main":"new"}' and read(root, "catalyst.json.old") == b'{"main":"old"}'
    trial = read(root, "catalyst.trial").decode().replace("\r", "").split("\n")
    assert trial[0] == "0" and "main.py 1" in trial and "face.kmodel 0" in trial, trial
    assert not any(n.endswith((".new", ".part")) for n in os.listdir(root)), os.listdir(root)
    assert settle(root) is False, "nothing left to do"

    # boot.py: the new program's starts counted; two that never reached the
    # handheld, and the third start has the old files back.
    boot = (here / "boot.py").read_text().replace('ROOT = "/flash"', "ROOT = %r" % root)
    for count in ("1", "2"):
        exec(boot, {})
        assert read(root, "catalyst.trial").decode().replace("\r", "").split("\n")[0] == count
        assert read(root, "main.py") == new_main
    exec(boot, {})
    assert read(root, "main.py") == old_main and read(root, "boot.py") == b"# old boot\n"
    assert read(root, "face.kmodel") is None, "a model that was not there before is gone again"
    assert read(root, "catalyst.json") == b'{"main":"old"}'
    assert read(root, "catalyst.trial") is None and read(root, "catalyst.rolledback") is not None
    exec(boot, {})
    assert read(root, "main.py") == old_main, "nothing more at the next start"

with tempfile.TemporaryDirectory() as root:
    # A program that starts and is heard ends its trial: nothing rolls back.
    write(root, "main.py", old_main)
    files = Files(root)
    send(files, "main.py", new_main)
    files.commit(["main.py"], "{}")
    boot = (here / "boot.py").read_text().replace('ROOT = "/flash"', "ROOT = %r" % root)
    exec(boot, {})
    confirm(root)
    for _ in range(4):
        exec(boot, {})
    assert read(root, "main.py") == new_main and read(root, "catalyst.rolledback") is None

with tempfile.TemporaryDirectory() as root:
    # The power gone in the middle of a commit: main.py already moved to
    # .old, its .new not yet in place. The next start finishes it.
    write(root, "main.py.old", old_main)
    write(root, "main.py.new", new_main)
    write(root, "catalyst.json.new", b'{"main":"new"}')
    write(root, "catalyst.commit", b"main.py 1\ncatalyst.json 0\n")
    boot = (here / "boot.py").read_text().replace('ROOT = "/flash"', "ROOT = %r" % root)
    exec(boot, {})
    assert read(root, "main.py") == new_main and read(root, "main.py.old") == old_main
    assert read(root, "catalyst.json") == b'{"main":"new"}'
    assert read(root, "catalyst.commit") is None
    assert read(root, "catalyst.trial").decode().replace("\r", "").split("\n")[0] == "1", "finished, then counted as its first start"
    # main.py's own settle and boot.py's agree, and either may run twice.
    assert settle(root) is False

print("main.py's link and update, boot.py: all passed")

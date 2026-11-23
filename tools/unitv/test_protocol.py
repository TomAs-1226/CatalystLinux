"""main.py's link, checked on a computer (CPython): its frames are the
handheld's (smartcam.rs checks the same bytes), its CRC is zlib's, and its
decoder skips junk and finds frames however the bytes are cut.

    python tools/unitv/test_protocol.py
"""

import pathlib
import zlib

source = (pathlib.Path(__file__).parent / "main.py").read_text()
# Only the link's own code: from the CRC to where the sensor is set up.
part = source[source.index("SYNC = "): source.index("# ---------------------------------------------------------------- sensor")]
part = part.replace("from ubinascii import crc32", "raise ImportError").replace("from binascii import crc32", "raise ImportError")
space = {"MAX": 32 * 1024}
exec(part, space)
frame, Decoder, crc32 = space["frame"], space["Decoder"], space["crc32"]

# The pure-Python CRC (the fallback on a firmware without binascii) is zlib's.
assert crc32(b"123456789") == 0xCBF43926 == zlib.crc32(b"123456789")
assert crc32(b"") == 0

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

print("main.py's link: all passed")

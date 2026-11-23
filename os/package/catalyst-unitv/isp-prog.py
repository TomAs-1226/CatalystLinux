"""The K210's flash-mode loader out of kflash.py: its ISP_PROG, a hex string
of zlib-compressed code, written out as the bytes kflash.py loads into the
K210's SRAM (0x80000000) before it writes the flash. Checked against the
SHA-256 of the copy pinned in catalyst-unitv.mk.

    python3 isp-prog.py kflash.py isp.bin
"""

import binascii
import hashlib
import re
import sys
import zlib

# ISP_PROG of kflash.py at 5783d8d85e7397007a3ba1394a93d3f9f8ad982f.
SHA256 = "302db794751a88ec0385d929188717dfb721a45614b17cf7030d4363295c98e4"

source = open(sys.argv[1], encoding="utf-8").read()
found = re.search(r"ISP_PROG = '([0-9a-f]+)'", source)
if not found:
    sys.exit("no ISP_PROG in " + sys.argv[1])
prog = zlib.decompress(binascii.unhexlify(found.group(1)))
if hashlib.sha256(prog).hexdigest() != SHA256:
    sys.exit("ISP_PROG is not the one pinned")
with open(sys.argv[2], "wb") as out:
    out.write(prog)

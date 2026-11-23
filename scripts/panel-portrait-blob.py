#!/usr/bin/env python3
"""The CardputerZero panel's init sequence for the tear-free (portrait) mode,
derived from M5Stack's own (`cardputerzero,st7789v_lcd.bin` in
m5stack-linux-dtoverlays, modules/st7789v-1.0), with two changes:

- MADCTL (0x36) 0xA0 -> 0x00. M5Stack's 0xA0 is MY|MV: landscape by row/
  column exchange, so the ST7789 refreshes its gate lines across the
  picture's columns while frames arrive row by row, and a moving picture
  tears on a diagonal. 0x00 keeps the panel's own order: memory written from
  its first gate line onwards, the way it refreshes. The shell turns the
  picture a quarter turn as it sends it (platform.rs).
- FRCTRL2 (0xC6) 0x10, added after PORCTRL: 59.9 refreshes a second, just
  under the shell's 60 frames. The ST7789 refreshes at 10 MHz / ((320 rows
  + 5 + 5 of porch) x (250 + 16 x RTNA)) clocks; its default RTNA, 0x0F, is
  61.8 Hz with these porches, a little faster than the frames come, so about
  one frame in 33 was shown twice (a hitch about twice a second). At 59.9
  one frame in 600 is never shown, about once every ten seconds. The driver
  times every flush from the tearing-effect line, whatever the rate, so the
  write at 62.5 MHz (about 44 us a row, the scan 50.6) stays clear of the
  scan either way (patch 0008).

(The first version of the mode slowed the panel to 40 Hz so that a whole
frame could always lead the scan from the blank; that cost motion and bought
nothing once the driver timed each flush from the scan.)

The driver (patch 0003) counts PORCTRL's porches, 5 + 5 rows, as part of
each refresh: checked here.

Usage: panel-portrait-blob.py <stock .bin> <out .bin>
"""
import sys

MAGIC = b"MIPI DBI" + bytes(7)

# FRCTRL2: dot inversion (NLA 0), RTNA 0x10: 59.9 Hz with PORCTRL's 5 + 5.
FRCTRL2 = bytes([0xC6, 1, 0x10])


def portrait(stock: bytes) -> bytes:
    assert stock[:15] == MAGIC and stock[15] == 1, "a MIPI DBI command file, version 1"
    commands = bytearray(stock[16:])
    out = bytearray()
    i = 0
    seen_madctl = False
    while i < len(commands):
        command, count = commands[i], commands[i + 1]
        params = bytes(commands[i + 2 : i + 2 + count])
        i += 2 + count
        if command == 0x36:
            assert params == b"\xa0", f"M5Stack's MADCTL is 0xA0, not {params.hex()}"
            params = b"\x00"
            seen_madctl = True
        assert command != 0xC6, "the stock sequence sets FRCTRL2 already: check the rate it chose"
        out += bytes([command, count]) + params
        if command == 0xB2:
            assert params[:2] == b"\x05\x05", f"PORCTRL's porches are 5 + 5, not {params[:2].hex()} (patch 0003 counts them)"
            out += FRCTRL2
    assert seen_madctl, "MADCTL was in the stock sequence"
    assert FRCTRL2 in out, "PORCTRL was in the stock sequence"
    return bytes(stock[:16]) + bytes(out)


if __name__ == "__main__":
    stock = open(sys.argv[1], "rb").read()
    with open(sys.argv[2], "wb") as f:
        f.write(portrait(stock))

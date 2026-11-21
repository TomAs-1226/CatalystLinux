#!/usr/bin/env python3
"""The CardputerZero panel's init sequence for the tear-free (portrait) mode,
derived from M5Stack's own (`cardputerzero,st7789v_lcd.bin` in
m5stack-linux-dtoverlays, modules/st7789v-1.0), with one change:

- MADCTL (0x36) 0xA0 -> 0x00. M5Stack's 0xA0 is MY|MV: landscape by row/
  column exchange, so the ST7789 refreshes its gate lines across the
  picture's columns while frames arrive row by row, and a moving picture
  tears on a diagonal. 0x00 keeps the panel's own order: memory written from
  its first gate line onwards, the way it refreshes. The shell turns the
  picture a quarter turn as it sends it (platform.rs).

The refresh stays the panel's own 60 Hz. (The first version slowed it to
40 Hz so that a whole frame, 17 to 21 ms over SPI at about 42 to 50 MHz,
could always lead the scan from the blank. The driver now times each flush
from the tearing-effect line and, writing slower than the scan as it does
here, follows it instead: a band written just behind the scan is shown whole
at the next refresh. 40 Hz bought nothing then and cost motion.)

The driver (patch 0003) counts PORCTRL's porches, 5 + 5 rows, as part of
each refresh: checked here.

Usage: panel-portrait-blob.py <stock .bin> <out .bin>
"""
import sys

MAGIC = b"MIPI DBI" + bytes(7)


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
        if command == 0xB2:
            assert params[:2] == b"\x05\x05", f"PORCTRL's porches are 5 + 5, not {params[:2].hex()} (patch 0003 counts them)"
        assert command != 0xC6, "the sequence sets FRCTRL2: the driver assumes the panel's own 60 Hz"
        out += bytes([command, count]) + params
    assert seen_madctl, "MADCTL was in the stock sequence"
    return bytes(stock[:16]) + bytes(out)


if __name__ == "__main__":
    stock = open(sys.argv[1], "rb").read()
    with open(sys.argv[2], "wb") as f:
        f.write(portrait(stock))

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
- FRCTRL2 (0xC6) = 0x1E: 40 refreshes a second instead of 60. The SPI clock
  is the core clock over an even divisor (about 42 to 50 MHz for the 60 MHz
  asked), so a whole frame takes 17 to 21 ms to send; at 60 Hz (16.7 ms) the
  panel's scan would catch it, at 40 Hz (25 ms) it never does. It is also
  the shell's own beat: one frame a refresh.

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
        if command == 0xC6:
            raise SystemExit("the stock sequence sets FRCTRL2 itself: look again")
        if command == 0x29:
            # Display on comes last: the refresh rate is set before it.
            out += bytes([0xC6, 1, 0x1E])
        out += bytes([command, count]) + params
    assert seen_madctl, "MADCTL was in the stock sequence"
    return bytes(stock[:16]) + bytes(out)


if __name__ == "__main__":
    stock = open(sys.argv[1], "rb").read()
    with open(sys.argv[2], "wb") as f:
        f.write(portrait(stock))

#!/usr/bin/env python3
"""The towns Photos names a place by, offline: GeoNames' cities, cut small.

    python3 tools/gazetteer/build.py DIR     (Windows: py tools\\gazetteer\\build.py DIR)

DIR holds GeoNames' `cities5000.txt` (unzipped from cities5000.zip: every
town of 5,000 people or more, about 70,000) and `admin1CodesASCII.txt` (the
states and provinces), both from https://download.geonames.org/export/dump/.
Writes `userland/assets/gazetteer/towns.bin`, which the shell carries in
itself (photolog/gazetteer.rs), under 2 MB.

GeoNames' data is under the Creative Commons Attribution 4.0 licence
(https://creativecommons.org/licenses/by/4.0/); the note beside the file
says so, and About › Third-party pieces names it.

The file, little-endian:

    b"CTW1", u32 towns, u32 regions
    towns' latitudes, each i24, degrees × 10,000, in order of latitude
    towns' longitudes, each i24, the same
    towns' entries, each u24: where its entry starts in the strings
    regions' entries, each u24: where its name starts in the strings
    the strings: a town's entry is its country (2 ASCII letters), its region
    (u16, 0xFFFF for none), its name in UTF-8 and a 0; a region's is its
    name and a 0.

A region in the United States is named by its two letters ("NY"), as the
towns there are written; elsewhere by its name ("Ontario").
"""
import os
import struct
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
OUT = os.path.join(ROOT, "userland", "assets", "gazetteer", "towns.bin")


def i24(v):
    if not -(1 << 23) <= v < (1 << 23):
        raise ValueError(v)
    return struct.pack("<i", v)[:3]


def u24(v):
    if not 0 <= v < (1 << 24):
        raise ValueError(v)
    return struct.pack("<I", v)[:3]


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    src = sys.argv[1]
    regions = {}
    region_names = []
    with open(os.path.join(src, "admin1CodesASCII.txt"), encoding="utf-8") as f:
        for line in f:
            code, name = line.rstrip("\n").split("\t")[:2]
            cc, _, sub = code.partition(".")
            words = sub if cc == "US" else name
            regions[code] = len(region_names)
            region_names.append(words)
    towns = []
    with open(os.path.join(src, "cities5000.txt"), encoding="utf-8") as f:
        for line in f:
            p = line.rstrip("\n").split("\t")
            name, lat, lon, cc, admin1 = p[1], float(p[4]), float(p[5]), p[8], p[10]
            if len(cc) != 2 or not name:
                continue
            region = regions.get(f"{cc}.{admin1}", 0xFFFF)
            towns.append((round(lat * 10_000), round(lon * 10_000), cc, region, name))
    towns.sort(key=lambda t: (t[0], t[1]))
    strings = bytearray()
    region_at = []
    for words in region_names:
        region_at.append(len(strings))
        strings += words.encode("utf-8") + b"\0"
    town_at = []
    for _, _, cc, region, name in towns:
        town_at.append(len(strings))
        strings += cc.encode("ascii") + struct.pack("<H", region) + name.encode("utf-8") + b"\0"
    out = bytearray(b"CTW1")
    out += struct.pack("<II", len(towns), len(region_names))
    out += b"".join(i24(t[0]) for t in towns)
    out += b"".join(i24(t[1]) for t in towns)
    out += b"".join(u24(a) for a in town_at)
    out += b"".join(u24(a) for a in region_at)
    out += strings
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "wb") as f:
        f.write(out)
    print(f"{len(towns)} towns, {len(region_names)} regions, {len(out):,} bytes -> {OUT}")


if __name__ == "__main__":
    main()

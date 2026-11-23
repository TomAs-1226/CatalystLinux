"""Fetch M5Stack's published enclosure STLs and place them in device coordinates.

usage: python tools/prep_refs.py        (needs numpy + trimesh)

Downloads go to _ref/<name>/ (one new folder each, data only, never executed).
Output: _ref/derived/{cz_front,cz_back,uv_front,uv_back}.stl, where
  CardputerZero: X 0..84, Y 0..54 (0 = keyboard edge), Z 0..23.1 (0 = back)
  UnitV-M12:     its own frame, X 0..24, Y 0..12.7 (lens side +Y), Z 0..40 (lens end 0)
"""
import os, urllib.request
import numpy as np, trimesh

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
REF = os.path.join(HERE, "_ref")
RAW = "https://raw.githubusercontent.com/m5stack/M5_Hardware/master/Products/"
SRC = {
    "m5-cardputerzero-stl": RAW + "C154_CardputerZero/Structures/CardputerZero.stl",
    "m5-unitv-m12-stl": RAW + "U078-V-M12_UnitV-M12/Structures/UnitV-M12.stl",
}


def fetch(folder, url):
    d = os.path.join(REF, folder)
    os.makedirs(d, exist_ok=True)
    path = os.path.join(d, url.rsplit("/", 1)[1])
    if not os.path.exists(path):
        urllib.request.urlretrieve(url, path)
    return path


def main():
    out = os.path.join(REF, "derived")
    os.makedirs(out, exist_ok=True)
    # The CardputerZero file holds an assembled pair of shells plus loose copies;
    # the assembled pair is the one left of X = -14.
    cz = trimesh.load(fetch("m5-cardputerzero-stl", SRC["m5-cardputerzero-stl"]), force="mesh")
    for p in cz.split(only_watertight=False):
        if p.bounds[1][0] < -14 and len(p.faces) > 1000:
            p.apply_translation([98.4, -5.0, 23.25])
            p.export(os.path.join(out, "cz_front.stl" if p.bounds[0][2] > 10 else "cz_back.stl"))
    uv = trimesh.load(fetch("m5-unitv-m12-stl", SRC["m5-unitv-m12-stl"]), force="mesh")
    for p in uv.split(only_watertight=False):
        if p.bounds[0][0] > 0:     # the assembled pair
            p.apply_translation([-4.56, 0, 40.0])
            p.export(os.path.join(out, "uv_front.stl" if p.bounds[0][1] > 1 else "uv_back.stl"))
    for f in sorted(os.listdir(out)):
        m = trimesh.load(os.path.join(out, f))
        print(f, np.round(m.bounds, 2).tolist())


if __name__ == "__main__":
    main()

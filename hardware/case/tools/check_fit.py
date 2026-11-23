"""Check that the printed parts don't collide with the device, the frozen chassis and the UnitV.

usage: python tools/check_fit.py
Runs OpenSCAD intersections (part="chk" in case.scad) and prints each overlap's volume and
where it is; anything above ~0.5 mm3 is a real collision.
"""
import os, subprocess, sys, tempfile
import numpy as np, trimesh

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from build import HERE, openscad  # noqa: E402

CHECKS = [("camera_back", "device"), ("camera_back", "chassis"), ("camera_back", "uv"), ("camera_back", "plunger"),
          ("camera_back", "pins"), ("plain_back", "device"), ("plain_back", "chassis"), ("frame", "device"),
          ("frame", "camera_back"), ("frame", "plain_back"), ("frame", "chassis"), ("link", "camera_back"),
          ("lens_ring", "camera_back"), ("pins", "uv"), ("plunger", "uv"), ("chassis", "uv")]


def main():
    exe = openscad()
    with tempfile.TemporaryDirectory() as td:
        for a, b in CHECKS:
            out = os.path.join(td, "c.stl")
            subprocess.run([exe, "--backend", "Manifold", "-D", 'part="chk"', "-D", f'chk_a="{a}"',
                            "-D", f'chk_b="{b}"', "-o", out, os.path.join(HERE, "case.scad")],
                           capture_output=True, text=True)
            if not os.path.exists(out) or os.path.getsize(out) < 200:
                print(f"{a:12s} x {b:12s}     0.00 mm3")
                continue
            m = trimesh.load(out, force="mesh")
            v = abs(m.volume)
            where = np.round(m.bounds, 1).tolist() if v > 0.5 else ""
            print(f"{a:12s} x {b:12s} {v:8.2f} mm3 {'COLLISION ' + str(where) if v > 0.5 else ''}")
            os.remove(out)


if __name__ == "__main__":
    main()

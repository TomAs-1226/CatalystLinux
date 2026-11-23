"""Export case.scad to STL.

usage: python tools/build.py [render|print|all]   (default: all)
  render -> out/stl/<part>.stl      device coordinates, for tools/render.py
  print  -> stl/<name>.stl          oriented for the bed, named part__material-colour
OpenSCAD: $OPENSCAD, else the portable copy in hardware/case/.tools, else PATH.
"""
import concurrent.futures as cf, glob, os, shutil, subprocess, sys, time

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RENDER = ["frame", "chassis", "camera_back", "plain_back", "lens_ring", "link", "plunger", "pins",
          "keys", "lenses", "glass_imx", "glass_k210", "screw_heads"]
# The chassis and plunger are frozen (already printed in white PETG from revision 2):
# stl/chassis__white-PETG.stl and stl/plunger-camera__white-PETG.stl are never rewritten.
PRINT = {  # scad part -> file name in stl/
    "frame_print": "frame__black-PLA",
    "camera_back_print": "back-camera__black-PLA",
    "plain_back_print": "back-plain__black-PLA",
    "lens_ring_print": "lens-ring-camera__black-PLA",
    "link_print": "link-cover-camera__black-PLA",
    "pins_print": "pins-camera__black-PLA",
    "fit_print": "fit-test__black-PLA",
}


def openscad():
    if os.environ.get("OPENSCAD"):
        return os.environ["OPENSCAD"]
    found = sorted(glob.glob(os.path.join(HERE, ".tools", "OpenSCAD-*", "openscad.com")))
    return found[-1] if found else shutil.which("openscad")


def clean(path):
    """Drop the zero-area slivers OpenSCAD leaves where fillets meet tangentially."""
    try:
        import trimesh
    except ImportError:
        return ""
    m = trimesh.load(path)
    m.update_faces(m.nondegenerate_faces(height=1e-6))
    bodies = [b for b in m.split(only_watertight=False) if abs(b.volume) > 1e-2]
    m = trimesh.util.concatenate(bodies)
    m.merge_vertices()
    m.export(path)
    return "" if m.is_watertight else " (not watertight)"


def build(exe, part, out):
    t = time.time()
    r = subprocess.run([exe, "--backend", "Manifold", "-D", f'part="{part}"', "-o", out,
                        os.path.join(HERE, "case.scad")], capture_output=True, text=True)
    log = r.stderr + r.stdout
    ok = r.returncode == 0 and os.path.exists(out) and "WARNING" not in log and "ERROR" not in log
    msg = "" if ok else log[-900:]
    if ok and os.sep + "stl" + os.sep in out and os.sep + "out" + os.sep not in out:
        msg = clean(out)
    return part, ok, time.time() - t, msg


def main():
    exe = openscad()
    if not exe:
        sys.exit("OpenSCAD not found (set OPENSCAD)")
    mode = sys.argv[1] if len(sys.argv) > 1 else "all"
    jobs = []
    if mode in ("render", "all"):
        os.makedirs(os.path.join(HERE, "out", "stl"), exist_ok=True)
        jobs += [(p, os.path.join(HERE, "out", "stl", p + ".stl")) for p in RENDER]
    if mode in ("print", "all"):
        os.makedirs(os.path.join(HERE, "stl"), exist_ok=True)
        jobs += [(p, os.path.join(HERE, "stl", n + ".stl")) for p, n in PRINT.items()]
    failed = False
    with cf.ThreadPoolExecutor(max_workers=6) as ex:
        for part, ok, dt, msg in ex.map(lambda j: build(exe, *j), jobs):
            print(f"{part:18s} {'ok' if ok else 'FAILED'} {dt:5.1f}s {msg}")
            failed |= not ok
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()

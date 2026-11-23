"""Measure the UnitV's LEGO holes and the CardputerZero's X14/X70 slots from M5's STLs.

usage: python tools/measure_holes.py   (after tools/prep_refs.py)
"""
import os, numpy as np, trimesh

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
D = os.path.join(HERE, "_ref", "derived")


def solid_along(mesh, origin, direction, span):
    """Intervals along a ray (from origin, length span) that lie inside the mesh."""
    origin, direction = np.asarray(origin, float), np.asarray(direction, float)
    loc, _, _ = mesh.ray.intersects_location([origin], [direction], multiple_hits=True)
    t = sorted(float(np.dot(p - origin, direction)) for p in loc if 0 <= np.dot(p - origin, direction) <= span)
    return [round(x, 2) for x in t]


uv = trimesh.util.concatenate([trimesh.load(os.path.join(D, f)) for f in ("uv_front.stl", "uv_back.stl")])
print("UnitV LEGO hole at x=4, z=28.2 (frame: y 0 = back face, 12.7 = front)")
for r in (0.0, 1.5, 2.2, 2.4, 2.5, 2.7, 3.0, 3.3):
    hits = solid_along(uv, [4 + r, -5, 28.2], [0, 1, 0], 25)
    print(f"  r={r:3.1f}  surface crossings along +y: {[round(h - 5, 2) for h in hits]}")
# radius of the bore at several depths: shrink a ring of rays until they hit
for y in (0.5, 1.0, 2.0, 4.0, 6.0, 7.5, 8.0):
    sec = uv.section(plane_origin=[4, y, 28.2], plane_normal=[0, 1, 0])
    if sec is None:
        continue
    v = sec.vertices[:, [0, 2]]
    d = np.linalg.norm(v - [4, 28.2], axis=1)
    near = d[d < 4.5]
    print(f"  y={y:3.1f}  nearest wall radius {near.min() if near.size else float('nan'):.2f}")

cz = trimesh.util.concatenate([trimesh.load(os.path.join(D, f)) for f in ("cz_front.stl", "cz_back.stl")])
print("CardputerZero slot at X14 (cross-section at several depths z)")
for z in (1.0, 4.0, 8.0, 11.0, 11.5, 12.0, 13.0):
    sec = cz.section(plane_origin=[14, 51, z], plane_normal=[0, 0, 1])
    v = sec.vertices[:, :2]
    box = v[(abs(v[:, 0] - 14) < 4) & (v[:, 1] > 45)]
    xs = box[:, 0]
    left = xs[xs < 14].max() if (xs < 14).any() else float("nan")
    right = xs[xs > 14].min() if (xs > 14).any() else float("nan")
    lowy = box[(abs(box[:, 0] - 14) < 1.5)][:, 1]
    bottom = lowy[lowy < 50].max() if (lowy < 50).any() else float("nan")
    print(f"  z={z:4.1f}  slot X {left:.2f}..{right:.2f} (w {right - left:.2f})  lower edge Y {bottom:.2f}")
print("  M2 hole below the slot floor at (14,50):", solid_along(cz, [14, 50, -1], [0, 0, 1], 30))
print("  blind M2 at (28,50):", solid_along(cz, [28, 50, -1], [0, 0, 1], 30))

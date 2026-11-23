"""Find walls and edges thinner than a limit on every printed part.

usage: python tools/check_thin.py [limit_mm]   (default 1.0; after tools/build.py render)

Samples points over each part's surface (area-weighted), shoots a ray inward along the
surface normal and measures the distance to the opposite face (the local wall thickness).
Samples thinner than the limit are grouped into clusters on a 1.5 mm grid and reported with
their position (device coordinates), smallest thickness and approximate area. Hits at grazing
angles are ignored, so outside corners don't count as thin.
"""
import os, sys
import numpy as np, trimesh

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PARTS = ["frame", "camera_back", "plain_back", "lens_ring", "link", "pins", "chassis", "plunger"]
FROZEN = {"chassis", "plunger"}


def thickness(mesh, n_samples, rng):
    pts, fid = trimesh.sample.sample_surface(mesh, n_samples, seed=rng)
    nrm = mesh.face_normals[fid]
    origins = pts - nrm * 1e-3
    loc, ray, tri = mesh.ray.intersects_location(origins, -nrm, multiple_hits=False)
    t = np.full(len(pts), np.inf)
    d = np.linalg.norm(loc - origins[ray], axis=1)
    facing = np.einsum("ij,ij->i", mesh.face_normals[tri], nrm[ray]) < -0.3   # a real opposite wall
    t[ray[facing]] = d[facing]
    return pts, t


def clusters(pts, mask, cell=1.5):
    keys = np.floor(pts[mask] / cell).astype(int)
    idx = np.nonzero(mask)[0]
    groups, seen = [], {}
    for k, i in zip(map(tuple, keys), idx):
        seen.setdefault(k, []).append(i)
    visited = set()
    for k in seen:
        if k in visited:
            continue
        stack, members = [k], []
        visited.add(k)
        while stack:
            c = stack.pop()
            members += seen[c]
            for dx in (-1, 0, 1):
                for dy in (-1, 0, 1):
                    for dz in (-1, 0, 1):
                        nb = (c[0] + dx, c[1] + dy, c[2] + dz)
                        if nb in seen and nb not in visited:
                            visited.add(nb)
                            stack.append(nb)
        groups.append(np.array(members))
    return groups


def main():
    limit = float(sys.argv[1]) if len(sys.argv) > 1 else 1.0
    rng = 7
    for name in PARTS:
        path = os.path.join(HERE, "out", "stl", name + ".stl")
        if not os.path.exists(path):
            continue
        mesh = trimesh.load(path, force="mesh")
        n = int(min(60000, max(8000, mesh.area * 4)))
        pts, t = thickness(mesh, n, rng)
        thin = t < limit
        per = mesh.area / n
        tag = " (frozen)" if name in FROZEN else ""
        groups = sorted(clusters(pts, thin), key=lambda g: -len(g))
        groups = [g for g in groups if len(g) * per >= 0.5]   # ignore specks under 0.5 mm2
        print(f"{name}{tag}: {n} samples, {len(groups)} thin area(s) under {limit} mm")
        for g in groups:
            lo, hi = pts[g].min(0), pts[g].max(0)
            print(f"   min {t[g].min():4.2f} mm  ~{len(g) * per:6.1f} mm2  at X {lo[0]:6.1f}..{hi[0]:6.1f}"
                  f"  Y {lo[1]:6.1f}..{hi[1]:6.1f}  Z {lo[2]:6.1f}..{hi[2]:6.1f}")


if __name__ == "__main__":
    main()

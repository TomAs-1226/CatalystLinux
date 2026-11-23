"""Find openings in an enclosure by casting a ray grid at each face.
usage: openings.py OUTDIR STL [STL...]   (device coords: X right, Y up, Z toward viewer)"""
import sys, numpy as np, trimesh
from scipy import ndimage
from PIL import Image, ImageDraw

out, files = sys.argv[1], sys.argv[2:]
mesh = trimesh.util.concatenate([trimesh.load(f, force="mesh") for f in files])
lo, hi = mesh.bounds
step = 0.2
faces = {  # name: (ray axis, sign of ray dir, u axis, v axis)
    "left_-X": (0, +1, 1, 2), "right_+X": (0, -1, 1, 2),
    "bottom_-Y": (1, +1, 0, 2), "top_+Y": (1, -1, 0, 2),
    "back_-Z": (2, +1, 0, 1), "front_+Z": (2, -1, 0, 1),
}
inter = trimesh.ray.ray_triangle.RayMeshIntersector(mesh)
for name, (ax, sg, ua, va) in faces.items():
    us = np.arange(lo[ua], hi[ua] + 1e-6, step)
    vs = np.arange(lo[va], hi[va] + 1e-6, step)
    U, V = np.meshgrid(us, vs)
    n = U.size
    org = np.zeros((n, 3)); org[:, ua] = U.ravel(); org[:, va] = V.ravel()
    org[:, ax] = (lo[ax] - 5) if sg > 0 else (hi[ax] + 5)
    d = np.zeros((n, 3)); d[:, ax] = sg
    loc, idx, _ = inter.intersects_location(org, d, multiple_hits=False)
    depth = np.full(n, 99.0)
    depth[idx] = np.abs(loc[:, ax] - org[idx, ax]) - 5
    D = depth.reshape(U.shape)
    # surface level = median of hits; opening = deeper than 2.0 mm past it
    surf = np.median(D[D < 50])
    mask = D > surf + 2.0
    lab, k = ndimage.label(mask)
    print(f"== {name}  (u={'XYZ'[ua]}, v={'XYZ'[va]}, surface depth {surf:.2f})")
    for i, sl in enumerate(ndimage.find_objects(lab), 1):
        area = (lab[sl] == i).sum() * step * step
        if area < 0.6: continue
        u0, u1 = us[sl[1].start], us[sl[1].stop - 1]
        v0, v1 = vs[sl[0].start], vs[sl[0].stop - 1]
        dd = D[sl][lab[sl] == i]
        dep = np.median(dd[dd < 50]) if (dd < 50).any() else 99
        print(f"  {'XYZ'[ua]} {u0:6.1f}..{u1:6.1f}  {'XYZ'[va]} {v0:6.1f}..{v1:6.1f}  size {u1-u0+step:5.1f} x {v1-v0+step:5.1f}  area {area:6.1f}  depth {dep:5.1f}")
    img = np.clip(255 - (np.minimum(D, 30) - surf) * 10, 0, 255).astype(np.uint8)[::-1]
    im = Image.fromarray(img).convert("RGB").resize((img.shape[1] * 4, img.shape[0] * 4), Image.NEAREST)
    dr = ImageDraw.Draw(im)
    s = 4 / step
    for g in range(int(np.ceil(lo[ua] / 5) * 5), int(hi[ua]) + 1, 5):
        x = (g - lo[ua]) * s
        dr.line([(x, 0), (x, 8 if g % 10 else 16)], fill=(255, 0, 0)); dr.text((x + 1, 16), str(g), fill=(255, 0, 0))
    for g in range(int(np.ceil(lo[va] / 5) * 5), int(hi[va]) + 1, 5):
        y = im.size[1] - (g - lo[va]) * s
        dr.line([(0, y), (8 if g % 10 else 16, y)], fill=(0, 0, 255)); dr.text((18, y - 6), str(g), fill=(0, 0, 255))
    im.save(f"{out}/open_{name}.png")

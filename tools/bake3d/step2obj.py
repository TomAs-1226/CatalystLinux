"""A STEP assembly as a mesh Blender reads, each solid kept apart under its
own name (the CAD's part and assembly names, as "Assembly/Part"), with its
colour: what robot.sh needs from a STEP export.

    python step2obj.py ROBOT.step OUT.obj

Needs gmsh (OpenCascade inside): robot.sh makes a venv with it the first
time. Solids far smaller than the robot (screws, nuts, washers, rivets)
are left out, and the rest meshed coarsely: a 112-pixel picture needs the
shape, not the threads.
"""

import math
import os
import sys

import gmsh

src, out = sys.argv[1], sys.argv[2]
gmsh.initialize()
gmsh.option.setNumber("General.Terminal", 0)
# The STEP's own names and colours, on the entities they label.
gmsh.option.setNumber("Geometry.OCCImportLabels", 1)
gmsh.model.occ.importShapes(src)
gmsh.model.occ.synchronize()

volumes = gmsh.model.getEntities(3)
if not volumes:
    sys.exit("step2obj: no solids in " + src)
boxes = {tag: gmsh.model.getBoundingBox(3, tag) for _, tag in volumes}
xs = [b[0] for b in boxes.values()] + [b[3] for b in boxes.values()]
ys = [b[1] for b in boxes.values()] + [b[4] for b in boxes.values()]
zs = [b[2] for b in boxes.values()] + [b[5] for b in boxes.values()]
diag = math.dist((min(xs), min(ys), min(zs)), (max(xs), max(ys), max(zs)))

# Fasteners and other specks: gone before meshing.
small = [(3, t) for t, b in boxes.items() if math.dist(b[:3], b[3:]) < diag * 0.012]
if small:
    gmsh.model.occ.remove(small, recursive=True)
    gmsh.model.occ.synchronize()
kept = [t for _, t in gmsh.model.getEntities(3)]
print(f"step2obj: {len(volumes)} solids, {len(small)} too small to show left out", flush=True)

gmsh.option.setNumber("Mesh.MeshSizeMin", diag / 400)
gmsh.option.setNumber("Mesh.MeshSizeMax", diag / 35)
gmsh.option.setNumber("Mesh.MeshSizeFromCurvature", 10)
gmsh.option.setNumber("Mesh.Algorithm", 6)
gmsh.model.mesh.generate(2)

nodes, coords, _ = gmsh.model.mesh.getNodes()
where = {int(n): i for i, n in enumerate(nodes)}

mtl = os.path.splitext(out)[0] + ".mtl"
faces_total = 0
with open(out, "w", encoding="utf-8") as obj, open(mtl, "w", encoding="utf-8") as mats:
    obj.write(f"mtllib {os.path.basename(mtl)}\n")
    for i in range(len(nodes)):
        obj.write(f"v {coords[3 * i]:.5f} {coords[3 * i + 1]:.5f} {coords[3 * i + 2]:.5f}\n")
    for tag in kept:
        name = gmsh.model.getEntityName(3, tag) or f"solid{tag}"
        r, g, b, _ = gmsh.model.getColor(3, tag)
        mat = f"m{tag}"
        mats.write(f"newmtl {mat}\nKd {r / 255:.3f} {g / 255:.3f} {b / 255:.3f}\n")
        obj.write(f"o {name.replace(' ', '_')}\nusemtl {mat}\n")
        for _, surface in gmsh.model.getBoundary([(3, tag)], oriented=False):
            types, _, conn = gmsh.model.mesh.getElements(2, abs(surface))
            for kind, c in zip(types, conn):
                if kind != 2:  # three-node triangles
                    continue
                for k in range(0, len(c), 3):
                    a, b2, c2 = (where[int(c[k + j])] + 1 for j in range(3))
                    obj.write(f"f {a} {b2} {c2}\n")
                    faces_total += 1
gmsh.finalize()
print(f"step2obj: {len(kept)} solids, {faces_total} triangles in {out}", flush=True)

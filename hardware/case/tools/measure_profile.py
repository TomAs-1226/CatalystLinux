import sys, numpy as np, trimesh
ref = sys.argv[1]
u = trimesh.util.concatenate([trimesh.load(f"{ref}/uv_front.stl"), trimesh.load(f"{ref}/uv_back.stl")])
print("UnitV thickness (Y) profile along length Z:")
for z in np.arange(0.5, 40, 2.0):
    s = u.section(plane_origin=[0, 0, z], plane_normal=[0, 0, 1])
    if s is not None:
        b = s.bounds
        print(f"  z={z:5.1f}  X {b[0][0]:5.1f}..{b[1][0]:5.1f}  Y {b[0][1]:5.1f}..{b[1][1]:5.1f}")
c = trimesh.util.concatenate([trimesh.load(f"{ref}/cz_front.stl"), trimesh.load(f"{ref}/cz_back.stl")])
print("CardputerZero plan corner radius (section at several Z):")
for z in (2, 8, 12, 18, 22.5):
    s = c.section(plane_origin=[0, 0, z], plane_normal=[0, 0, 1])
    v = s.vertices[:, :2]
    # distance from the bounding-box corner to the nearest outline point -> radius r = d / (sqrt2 - 1)
    lo = v.min(0); d = np.min(np.linalg.norm(v - lo, axis=1))
    print(f"  z={z:5.1f}  bounds {np.round(v.min(0),2)} {np.round(v.max(0),2)}  corner r ~ {d/(np.sqrt(2)-1):.2f}")
print("CardputerZero side profile: X extent per Z (edge rounding)")
for z in (0.1, 0.5, 1, 1.5, 2, 3, 21, 22, 22.6, 23.0):
    s = c.section(plane_origin=[0, 0, z], plane_normal=[0, 0, 1])
    if s is not None:
        v = s.vertices; print(f"  z={z:5.2f}  X {v[:,0].min():5.2f}..{v[:,0].max():5.2f}  Y {v[:,1].min():5.2f}..{v[:,1].max():5.2f}")

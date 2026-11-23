"""Blender (Cycles + Freestyle) renders for the picture assembly guide.

usage: blender -b --factory-startup -P tools/assembly.py -- OUTDIR [KEY ...] [--samples N] [--scale S]
  KEY     only these shots (default: all), e.g. s03 s05 parts
  OUTDIR  gets <shot>.png (transparent) and manifest.json; tools/assembly_finish.py lays out the pages.
Needs out/stl/*.stl (tools/build.py render) and _ref/derived/*.stl (tools/prep_refs.py).
Every placement below is in device coordinates (mm) from case.scad; nothing here changes the model.
"""
import bpy, bmesh, json, math, os, sys
from mathutils import Matrix, Vector
from bpy_extras.object_utils import world_to_camera_view
import numpy as np

argv = sys.argv[sys.argv.index("--") + 1:]
OUT = os.path.abspath(argv[0])
SAMPLES, SCALE, ONLY = 40, 1.0, []
i = 1
while i < len(argv):
    if argv[i] == "--samples":
        SAMPLES = int(argv[i + 1]); i += 2
    elif argv[i] == "--scale":
        SCALE = float(argv[i + 1]); i += 2
    else:
        ONLY.append(argv[i]); i += 1
HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
STL, REF = os.path.join(HERE, "out", "stl"), os.path.join(HERE, "_ref", "derived")
SCREEN_IMG = os.path.join(HERE, "..", "..", "shots", "looks", "3-bezel-home.png")
MM = 0.001
W, H = 1600, 1200

# ---- case.scad numbers (revision 3.1) used for placement
NUTS = [(6.0, 6.0), (79.0, 6.0)]          # fz_nuts
BLIND = [(28.0, 50.0), (56.0, 50.0)]      # fz_blind: M2 x 4 countersunk
PINS = [(60.8, 23.8), (76.8, 23.8)]       # k_pins
PLUNGER = (55.4, 17.8)                    # fz_plunger
LENS = (68.8, 43.0)                       # lens
X1, PASS_Y = 87.55, 30.5                  # outer right wall, pass-through centre
SNAPS = [(15, -1.45), (69, -1.45), (70, 55.45), (-1.45, 50), (85.45, 50)]  # bezel snap points (z 17.4)
HOOKS = [(14.0, 53.5), (70.0, 53.5)]      # slot hooks

# ---- palettes
HI, HI2 = "#ff5b1f", "#e0480f"            # Catalyst signal orange
GREY = dict(pla="#8e9196", petg="#c6c4bc", dev="#5a5d63", devback="#4d5056", key="#2c2e32", glass="#0d0f12",
            barrel="#2b2d31", uvf="#5d6066", uvb="#b6b9bf", metal="#74787e", cable="#5b5e64", plug="#d0d2d5",
            glue="#e9e3c4", label="#2b5fb8")
REAL = dict(pla="#2e3034", petg="#c9c6bc", dev="#4a4d53", devback="#3d4045", key="#26272a", glass="#07090c",
            barrel="#1c1d20", uvf="#2d2f33", uvb="#c3c6cc", metal="#a9adb2", cable="#34363a", plug="#efefeb",
            glue="#e9e3c4", label="#2b5fb8")
KEEP = ("key", "glass", "barrel", "screen")   # roles that stay as they are when highlighted


def srgb(h):
    h = h.lstrip("#")
    c = [int(h[i:i + 2], 16) / 255 for i in (0, 2, 4)]
    return tuple(((x + 0.055) / 1.055) ** 2.4 if x > 0.04045 else x / 12.92 for x in c) + (1.0,)


def material(name, color, rough=0.55, metal=0.0, alpha=1.0, emit=0.0, img=None):
    m = bpy.data.materials.new(name)
    m.use_nodes = True
    nt = m.node_tree
    b = [n for n in nt.nodes if n.type == 'BSDF_PRINCIPLED'][0]
    b.inputs["Base Color"].default_value = srgb(color)
    b.inputs["Roughness"].default_value = rough
    b.inputs["Metallic"].default_value = metal
    b.inputs["Alpha"].default_value = alpha
    if emit:
        b.inputs["Emission Color"].default_value = srgb(color)
        b.inputs["Emission Strength"].default_value = emit
    if img and os.path.exists(img):
        tex = nt.nodes.new("ShaderNodeTexImage")
        tex.image = bpy.data.images.load(img)
        nt.links.new(tex.outputs["Color"], b.inputs["Emission Color"])
        b.inputs["Emission Strength"].default_value = 1.0
        b.inputs["Base Color"].default_value = (0, 0, 0, 1)
    return m


MATS = {}


def build_materials():
    for pal_name, pal in (("grey", GREY), ("real", REAL)):
        for role, c in pal.items():
            metal = 0.7 if role == "metal" else 0.0
            rough = 0.08 if role == "glass" else (0.35 if role == "metal" else 0.55)
            MATS[(pal_name, role)] = material(f"{pal_name}-{role}", c, rough, metal)
            MATS[("ghost", role)] = MATS.get(("ghost", role)) or material(f"ghost-{role}", GREY[role], 0.5, 0, alpha=0.22)
    MATS["hi"] = material("hi", HI, 0.45, emit=0.12)
    MATS["hi2"] = material("hi2", HI2, 0.5, emit=0.08)
    MATS["screen"] = material("screen", "#000000", 0.2, img=SCREEN_IMG)


# ---------------------------------------------------------------- geometry
OBJ, ROLE, BASE = {}, {}, {}


def register(o, name, role):
    o.name = name
    OBJ[name], ROLE[name], BASE[name] = o, role, o.matrix_world.copy()
    if not o.data.materials:
        o.data.materials.append(MATS[("grey", role if role != "screen" else "key")])
    return o


def imp(path, name, role, matrix=None):
    bpy.ops.wm.stl_import(filepath=path, global_scale=MM)
    o = bpy.context.selected_objects[0]
    bpy.context.view_layer.objects.active = o
    try:
        bpy.ops.object.shade_smooth_by_angle(angle=math.radians(30))
    except Exception:
        pass
    if matrix is not None:
        o.matrix_world = matrix @ o.matrix_world
    return register(o, name, role)


def mesh_obj(name, role, bm, at=(0, 0, 0), smooth=True):
    me = bpy.data.meshes.new(name)
    bm.to_mesh(me)
    bm.free()
    for p in me.polygons:
        p.use_smooth = smooth
    o = bpy.data.objects.new(name, me)
    bpy.context.scene.collection.objects.link(o)
    o.matrix_world = Matrix.Translation(Vector(at) * MM)
    return register(o, name, role)


def lathe(profile, seg=48):
    """Solid of revolution around +Z from (r, z) points in mm (r = 0 ends close the solid)."""
    bm = bmesh.new()
    rings = []
    for r, z in profile:
        if r < 1e-6:
            rings.append([bm.verts.new((0, 0, z * MM))])
        else:
            rings.append([bm.verts.new((r * MM * math.cos(2 * math.pi * k / seg), r * MM * math.sin(2 * math.pi * k / seg), z * MM))
                          for k in range(seg)])
    for a, b in zip(rings, rings[1:]):
        for k in range(seg):
            if len(a) == 1 and len(b) == 1:
                continue
            if len(a) == 1:
                bm.faces.new((a[0], b[(k + 1) % seg], b[k]))
            elif len(b) == 1:
                bm.faces.new((a[k], a[(k + 1) % seg], b[0]))
            else:
                bm.faces.new((a[k], a[(k + 1) % seg], b[(k + 1) % seg], b[k]))
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    return bm


def thread(z0, z1, r=1.0, rm=0.86, pitch=0.4):
    pts, z, k = [], z0, 0
    while z < z1 - 1e-6:
        pts.append((r if k % 2 == 0 else rm, z)); z += pitch / 2; k += 1
    return pts


def hex_socket(name, at_z, af=1.3, depth=0.55):
    bm = bmesh.new()
    bmesh.ops.create_cone(bm, cap_ends=True, segments=6, radius1=af / math.sqrt(3) * MM, radius2=af / math.sqrt(3) * MM,
                          depth=depth * MM)
    bmesh.ops.translate(bm, verts=bm.verts, vec=(0, 0, (at_z + depth / 2 - 0.05) * MM))
    return bm


def make_screw(name, kind, at):
    if kind == "button":    # ISO 7380 M2 x 3: dome apex at z = 0, underside 1.1, tip 4.1
        prof = [(0, 0)] + [(1.75 * math.sin(a), 0.9 * (1 - math.cos(a))) for a in np.linspace(0.25, math.pi / 2, 8)]
        prof += [(1.75, 1.1), (1.0, 1.1)] + thread(1.25, 3.9) + [(0.8, 4.1), (0, 4.1)]
    else:                   # ISO 10642 M2 x 4: flat face at z = 0, 90 deg cone, tip at 4.0
        prof = [(0, 0), (2.0, 0), (2.0, 0.12), (1.0, 1.12)] + thread(1.2, 3.8) + [(0.8, 4.0), (0, 4.0)]
    o = mesh_obj(name, "metal", lathe(prof, 40), at)
    o.data.polygons.foreach_set("use_smooth", [False] * len(o.data.polygons))
    s = mesh_obj(name + "-sock", "glass", hex_socket(name, -0.06), at, smooth=False)
    return o, s


def make_nut(name, at):        # M2 hex nut, 4 AF x 1.6, z 0..1.6
    bm = bmesh.new()
    R, r, n = 4.0 / math.sqrt(3), 1.0, 12
    def ring(rad_fn, z):
        return [bm.verts.new((rad_fn(k) * math.cos(math.pi * k / 6) * MM, rad_fn(k) * math.sin(math.pi * k / 6) * MM, z * MM)) for k in range(n)]
    outer = lambda k: R if k % 2 == 0 else R * math.cos(math.pi / 6)
    inner = lambda k: r
    ob, ot, ib, it = ring(outer, 0), ring(outer, 1.6), ring(inner, 0), ring(inner, 1.6)
    for k in range(n):
        j = (k + 1) % n
        bm.faces.new((ob[k], ob[j], ot[j], ot[k]))
        bm.faces.new((it[k], it[j], ib[j], ib[k]))
        bm.faces.new((ot[k], ot[j], it[j], it[k]))
        bm.faces.new((ib[k], ib[j], ob[j], ob[k]))
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    o = mesh_obj(name, "metal", bm, at, smooth=False)
    return o


def make_plug(name):
    """HY2.0-4P plug: tip at local x = 0, body toward -x, latch bumps on +z."""
    bm = bmesh.new()
    def box(x0, x1, y0, y1, z0, z1):
        res = bmesh.ops.create_cube(bm, size=1.0)
        v = res["verts"]
        bmesh.ops.scale(bm, verts=v, vec=((x1 - x0) * MM, (y1 - y0) * MM, (z1 - z0) * MM))
        bmesh.ops.translate(bm, verts=v, vec=((x0 + x1) / 2 * MM, (y0 + y1) / 2 * MM, (z0 + z1) / 2 * MM))
    box(-6.5, 0, -4.5, 4.5, -2.3, 2.3)        # housing
    box(-8.5, -6.4, -3.4, 3.4, -1.6, 1.6)     # strain relief
    for y in (-3.0, 3.0):
        box(-5.0, -1.4, y - 0.7, y + 0.7, 2.2, 3.1)   # latch bumps
    o = mesh_obj(name, "plug", bm, smooth=False)
    mod = o.modifiers.new("bevel", 'BEVEL'); mod.width = 0.35 * MM; mod.segments = 2
    return o


def plug_matrix(tip, axis, up=(0, 0, 1)):
    x = Vector(axis).normalized()
    z = Vector(up); z = (z - x * z.dot(x)).normalized()
    y = z.cross(x)
    m = Matrix((x, y, z)).transposed().to_4x4()
    m.translation = Vector(tip) * MM
    return m


def unitv_matrix():
    # UnitV frame (x, y, z) -> device (80.8 - x, 52.0 - z, -1.6 - y), lens end up (render.py)
    return Matrix(((-1, 0, 0, 80.8 * MM), (0, 0, -1, 52.0 * MM), (0, -1, 0, -1.6 * MM), (0, 0, 0, 1)))


def screen_plane():
    bpy.ops.mesh.primitive_plane_add(size=1)
    o = bpy.context.active_object
    o.scale = (42.2 * MM, 22.4 * MM, 1)
    o.location = (42.0 * MM, 39.9 * MM, 22.96 * MM)
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    o.data.materials.append(MATS["screen"])
    return register(o, "screen", "screen")


def bm_box(bm, x0, x1, y0, y1, z0, z1):
    v = bmesh.ops.create_cube(bm, size=1.0)["verts"]
    bmesh.ops.scale(bm, verts=v, vec=((x1 - x0) * MM, (y1 - y0) * MM, (z1 - z0) * MM))
    bmesh.ops.translate(bm, verts=v, vec=((x0 + x1) / 2 * MM, (y0 + y1) / 2 * MM, (z0 + z1) / 2 * MM))


def channel_is_open(o):
    inv = o.matrix_world.inverted()
    ok, loc, _, _ = o.ray_cast(inv @ (Vector((68.8, 15.0, 5.0)) * MM), (inv.to_3x3() @ Vector((0, 0, -1))).normalized())
    return ok and (o.matrix_world @ loc).z / MM < -13.0


def open_channel(o):
    """case.scad rev 3.1: channel2d() runs round2d(4.0), which erases the 5 mm channel2d(0), so the
    guides export as a solid 8.2 mm ridge with the lips buried in it. Until the model is fixed, the
    renders show the channel PRINTING.md section 4 documents: 5 mm wide, floor at -14.6, 1.2 mm lips.
    This touches only the render mesh, and switches itself off once the exported STL has the channel."""
    if channel_is_open(o):
        return False
    print("NOTE camera_back: cable channel is solid in the STL; showing the documented channel")
    bpy.context.view_layer.objects.active = o
    for i, (x0, x1, y0, y1) in enumerate(((66.3, 71.3, 3.0, 33.0), (66.3, 79.8, 28.0, 33.0))):
        bm = bmesh.new()
        bm_box(bm, x0, x1, y0, y1, -14.6, -11.0)
        me = bpy.data.meshes.new(f"cut{i}"); bm.to_mesh(me); bm.free()
        cut = bpy.data.objects.new(f"cut{i}", me)          # world-space metres, identity transform
        bpy.context.scene.collection.objects.link(cut)
        mod = o.modifiers.new(f"cut{i}", 'BOOLEAN')
        mod.operation, mod.object, mod.solver = 'DIFFERENCE', cut, 'EXACT'
        bpy.ops.object.modifier_apply(modifier=mod.name)
        bpy.data.objects.remove(cut, do_unlink=True)
    lips = [(70.1, 72.9, y, y + 3) for y in (9, 19)] + [(64.7, 67.5, y, y + 3) for y in (9, 19)] + [(75.25, 78.25, 31.8, 34.6)]
    bl = bmesh.new()
    for x0, x1, y0, y1 in lips:
        bm_box(bl, x0, x1, y0, y1, -12.3, -11.1)
    bmesh.ops.transform(bl, matrix=o.matrix_world.inverted(), verts=bl.verts)
    bm = bmesh.new()
    bm.from_mesh(o.data)
    tmp = bpy.data.meshes.new("lips"); bl.to_mesh(tmp); bl.free()
    bm.from_mesh(tmp)
    bm.to_mesh(o.data); bm.free()
    bpy.data.meshes.remove(tmp)
    return True


def load_all():
    s = lambda f: os.path.join(STL, f + ".stl")
    imp(os.path.join(REF, "cz_front.stl"), "dev_front", "dev")
    imp(os.path.join(REF, "cz_back.stl"), "dev_back", "devback")
    imp(s("keys"), "keys", "key")
    screen_plane()
    imp(s("glass_imx"), "glass_imx", "glass")
    imp(s("chassis"), "chassis", "petg")
    open_channel(imp(s("camera_back"), "camera_back", "pla"))
    imp(s("plain_back"), "plain_back", "pla")
    imp(s("frame"), "bezel", "pla")
    imp(s("link"), "link", "pla")
    imp(s("lens_ring"), "lens_ring", "pla")
    imp(s("pins"), "pins", "pla")
    imp(s("plunger"), "plunger", "petg")
    um = unitv_matrix()
    imp(os.path.join(REF, "uv_front.stl"), "uv_front", "uvf", um)
    imp(os.path.join(REF, "uv_back.stl"), "uv_back", "uvb", um)
    imp(s("lenses"), "uv_lens", "barrel")
    imp(s("glass_k210"), "uv_glass", "glass")
    for i, p in enumerate(NUTS):
        make_nut(f"nut{i + 1}", (p[0], p[1], -1.6))
    for i, p in enumerate(BLIND):
        make_screw(f"csk{i + 1}", "csk", (p[0], p[1], -1.6))
    for i, p in enumerate(NUTS):
        make_screw(f"btn{i + 1}", "button", (p[0], p[1], -4.5))
    make_plug("plugA"); make_plug("plugB")
    for i, (x, y) in enumerate(GLUE):      # optional superglue dabs on the plain back's floor
        bm = bmesh.new()
        bmesh.ops.create_uvsphere(bm, u_segments=24, v_segments=12, radius=1.0)
        bmesh.ops.scale(bm, vec=(2.4 * MM, 2.4 * MM, 1.0 * MM), verts=bm.verts)
        mesh_obj(f"dab{i + 1}", "glue", bm, (x, y, -1.6))
    # superglue tube, for the parts page (well away from the device)
    mesh_obj("glue_tube", "petg", lathe([(0, 0), (5.5, 0), (6, 0.6), (6, 30), (5.4, 33), (2.4, 36), (1.6, 37),
                                          (1.6, 40), (0.9, 46), (0, 46)], 40), (300, 0, 0))
    mesh_obj("glue_label", "label", lathe([(0, 9), (6.06, 9), (6.06, 24), (0, 24)], 40), (300, 0, 0))


GLUE = [(13.0, 3.6), (71.0, 3.6)]
GROUPS = {
    "dabs": ["dab1", "dab2"],
    "glue": ["glue_tube", "glue_label"],
    "device": ["dev_front", "dev_back", "keys", "screen", "glass_imx"],
    "chassis+": ["chassis", "nut1", "nut2", "csk1", "csk1-sock", "csk2", "csk2-sock"],
    "unitv": ["uv_front", "uv_back", "uv_lens", "uv_glass"],
    "nuts": ["nut1", "nut2"],
    "csk": ["csk1", "csk1-sock", "csk2", "csk2-sock"],
    "btn": ["btn1", "btn1-sock", "btn2", "btn2-sock"],
}


def expand(names):
    out = []
    for n in names:
        out += GROUPS.get(n, [n])
    return out


# ---------------------------------------------------------------- Grove cable
def catmull(pts, n=10):
    P = [Vector(p) for p in pts]
    P = [P[0] * 2 - P[1]] + P + [P[-1] * 2 - P[-2]]
    out = []
    for i in range(1, len(P) - 2):
        p0, p1, p2, p3 = P[i - 1], P[i], P[i + 1], P[i + 2]
        t0 = 0.0
        t1 = t0 + max((p1 - p0).length, 1e-4) ** 0.5
        t2 = t1 + max((p2 - p1).length, 1e-4) ** 0.5
        t3 = t2 + max((p3 - p2).length, 1e-4) ** 0.5
        for k in range(n):
            t = t1 + (t2 - t1) * k / n
            a1 = p0 * ((t1 - t) / (t1 - t0)) + p1 * ((t - t0) / (t1 - t0))
            a2 = p1 * ((t2 - t) / (t2 - t1)) + p2 * ((t - t1) / (t2 - t1))
            a3 = p2 * ((t3 - t) / (t3 - t2)) + p3 * ((t - t2) / (t3 - t2))
            b1 = a1 * ((t2 - t) / (t2 - t0)) + a2 * ((t - t0) / (t2 - t0))
            b2 = a2 * ((t3 - t) / (t3 - t1)) + a3 * ((t - t1) / (t3 - t1))
            out.append(b1 * ((t2 - t) / (t2 - t1)) + b2 * ((t - t1) / (t2 - t1)))
    out.append(P[-2])
    return out


CHANNEL = [(68.8, 9.0, -13.6), (68.8, 14, -13.6), (68.8, 20, -13.6), (68.8, 26.5, -13.6), (69.97, 29.33, -13.6),
           (72.8, 30.5, -13.6), (77.5, 30.5, -13.5), (81.5, 30.5, -11.8), (85.5, 30.5, -9.4)]
OUT_B = [(89.0, 30.5, -8.4)]


def cable_state(state):
    """-> (plugA pose, plugB pose, centre-line points), poses as (tip, axis, up)."""
    A_up = (1, 0, 0)
    if state == "loose":   # for the parts page, away from the device
        A = ((140, 0, 0), (-1, 0, 0), (0, 0, 1)); B = ((214, 0, 0), (1, 0, 0), (0, 0, 1))
        mid = [(148.5, 0, 0), (152, 0.5, 0), (164, 9, 0), (177, -7, 0), (190, 8, 0), (202, 0.5, 0), (205.5, 0, 0)]
        return A, B, mid
    if state == "free":      # step 5: routed, UnitV end standing at the bottom of the pod
        A = ((68.8, 4.0, -0.8), (0, 0, 1), (0, 1, 0))
        head = [(68.8, 4.0, -9.3), (68.8, 4.4, -11.2), (68.8, 5.6, -13.0), (68.8, 7.4, -13.6)]
    elif state == "lift_near":   # step 6: UnitV held above, plug 8 mm short of its socket
        A = ((68.8, 18.0, 22.3), (0, 1, 0), (0, 0, 1))
        head = [(68.8, 9.5, 22.3), (68.8, 6.5, 21.6), (68.8, 3.6, 17.5), (68.8, 2.2, 8.0), (68.8, 2.2, -3.0),
                (68.8, 2.8, -9.8), (68.8, 4.2, -12.2), (68.8, 6.0, -13.4), (68.8, 7.5, -13.6)]
    elif state == "lift_in":     # step 7: plugged into the UnitV, UnitV 30 mm above its pins
        A = ((68.8, 14.0, 24.3), (0, 1, 0), (0, 0, 1))
        head = [(68.8, 5.5, 24.3), (68.8, 2.8, 23.4), (68.8, 1.6, 19.5), (68.8, 1.6, 8.0), (68.8, 2.2, -3.0),
                (68.8, 2.8, -9.8), (68.8, 4.2, -12.2), (68.8, 6.0, -13.4), (68.8, 7.5, -13.6)]
    else:                        # plugged into the UnitV, which sits on its pins
        A = ((68.8, 14.0, -5.7), (0, 1, 0), (0, 0, 1))
        head = [(68.8, 5.5, -5.7), (68.8, 2.7, -6.6), (68.8, 2.2, -9.0), (68.8, 3.6, -11.4), (68.8, 5.2, -13.1),
                (68.8, 7.2, -13.6)]
    if state in ("free", "lift_near", "lift_in", "routed"):
        B = ((99.5, PASS_Y, -8.2), (1, 0, 0), (0, 0, 1))      # pushed out through the pass-through
        tail = OUT_B + [(91.0, PASS_Y, -8.2)]
    elif state == "port_near":   # step 9: in front of the device's Grove port
        B = ((92.0, 40.0, 13.8), (-1, 0, 0), (0, 0, 1))
        tail = [(89.2, 30.5, -7.6), (90.4, 31.5, -2.5), (91.0, 35.0, 4.0), (93.5, 39.0, 8.6), (99.0, 40.0, 10.6),
                (103.2, 40.0, 12.6), (102.0, 40.0, 13.8)]
    else:                        # "plugged": in the device's Grove port (Y34-46 Z10.6-17)
        B = ((83.0, 40.0, 13.8), (-1, 0, 0), (0, 0, 1))
        tail = [(89.0, 30.5, -7.8), (90.2, 31.5, -2.8), (90.6, 35.0, 3.5), (91.6, 38.8, 8.4), (93.6, 40.0, 11.0),
                (93.4, 40.0, 13.8)]
    return A, B, head + CHANNEL + tail


def plug_back(pose):
    tip, axis, _ = pose
    return tuple(Vector(tip) - Vector(axis) * 8.5)


CABLE = {}


def set_cable(state, role_mat, mat_a, mat_b, disp=(0, 0, 0)):
    if "obj" in CABLE:
        bpy.data.objects.remove(CABLE.pop("obj"), do_unlink=True)
    if state is None:
        for p in ("plugA", "plugB"):
            OBJ[p].hide_render = True
        return
    A, B, mid = cable_state(state)
    pts = [plug_back(A)] + mid + [plug_back(B)] if state != "loose" else [plug_back(A)] + mid + [plug_back(B)]
    pts = [Vector(p) + Vector(disp) for p in pts]
    cu = bpy.data.curves.new("cable", 'CURVE')
    cu.dimensions = '3D'
    cu.bevel_depth = 1.0 * MM
    cu.bevel_resolution = 3
    cu.use_fill_caps = True
    sp = cu.splines.new('POLY')
    dense = catmull(pts, 8)
    sp.points.add(len(dense) - 1)
    for p, v in zip(sp.points, dense):
        p.co = (v.x * MM, v.y * MM, v.z * MM, 1)
    o = bpy.data.objects.new("cable", cu)
    bpy.context.scene.collection.objects.link(o)
    o.data.materials.append(role_mat)
    CABLE["obj"] = o
    for name, pose, m in (("plugA", A, mat_a), ("plugB", B, mat_b)):
        tip = Vector(pose[0]) + Vector(disp)
        OBJ[name].matrix_world = plug_matrix(tip, pose[1], pose[2])
        OBJ[name].hide_render = False
        OBJ[name].material_slots[0].material = m
    return o


# ---------------------------------------------------------------- scene
def scene_setup():
    bpy.ops.wm.read_factory_settings(use_empty=True)
    sc = bpy.context.scene
    sc.render.engine = 'CYCLES'
    prefs = bpy.context.preferences.addons['cycles'].preferences
    for t in ('OPTIX', 'CUDA'):
        try:
            prefs.compute_device_type = t
            prefs.get_devices()
            if any(d.type == t for d in prefs.devices):
                break
        except Exception:
            pass
    for d in prefs.devices:
        d.use = True
    sc.cycles.device = 'GPU'
    sc.cycles.samples = SAMPLES
    sc.cycles.use_denoising = True
    sc.render.resolution_x, sc.render.resolution_y = W, H
    sc.render.resolution_percentage = int(100 * SCALE)
    sc.render.film_transparent = True
    sc.view_settings.view_transform = 'Standard'
    sc.view_settings.look = 'None'
    sc.view_settings.exposure = 0.0
    w = bpy.data.worlds.new("w"); sc.world = w; w.use_nodes = True
    bg = [n for n in w.node_tree.nodes if n.type == 'BACKGROUND'][0]
    bg.inputs[0].default_value = (1, 1, 1, 1)
    bg.inputs[1].default_value = 0.5
    # Freestyle: the manual's outlines
    sc.render.use_freestyle = True
    sc.render.line_thickness_mode = 'ABSOLUTE'
    sc.render.line_thickness = 1.0
    bpy.context.view_layer.use_freestyle = True
    fs = bpy.context.view_layer.freestyle_settings
    fs.crease_angle = math.radians(138)
    ls = fs.linesets[0] if fs.linesets else fs.linesets.new("lines")
    ls.select_by_visibility = True
    ls.visibility = 'VISIBLE'
    ls.select_silhouette = ls.select_border = ls.select_crease = True
    if ls.linestyle is None:
        ls.linestyle = bpy.data.linestyles.new("ink")
    ls.linestyle.color = (0.07, 0.075, 0.085)
    ls.linestyle.thickness = 2.6 * SCALE
    cam = bpy.data.objects.new("cam", bpy.data.cameras.new("cam"))
    sc.collection.objects.link(cam)
    sc.camera = cam
    cam.data.type = 'ORTHO'
    cam.data.clip_start, cam.data.clip_end = 0.005, 3.0
    for name, size, power in (("key", 0.35, 22.0), ("fill", 0.6, 7.0), ("top", 0.5, 8.0)):
        l = bpy.data.lights.new(name, 'AREA'); l.size, l.energy = size, power
        o = bpy.data.objects.new(name, l); sc.collection.objects.link(o)
    return sc


def look(d, up):
    z = Vector(d).normalized()
    x = Vector(up).cross(z).normalized()
    y = z.cross(x)
    return Matrix((x, y, z)).transposed()


def world_points(names):
    pts = []
    dg = bpy.context.evaluated_depsgraph_get()
    for n in names:
        o = OBJ.get(n) if n != "cable" else CABLE.get("obj")
        if o is None or o.hide_render:
            continue
        mw = np.array(o.matrix_world)
        if o.type == 'MESH' and len(o.data.vertices) > 8:
            co = np.empty(len(o.data.vertices) * 3); o.data.vertices.foreach_get("co", co)
            co = co.reshape(-1, 3)
        elif o.type == 'CURVE':
            co = np.array([p.co[:3] for p in o.data.splines[0].points])
            co = np.concatenate([co + np.array(d) * MM for d in
                                 ((1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1))])
        else:
            co = np.array([v[:] for v in o.bound_box])
        pts.append(co @ mw[:3, :3].T + mw[:3, 3])
    return np.concatenate(pts) if pts else np.zeros((1, 3))


def frame_camera(sc, d, up, fit_pts, rect):
    R = look(d, up)
    cam = sc.camera
    c = fit_pts.mean(0)
    loc = (fit_pts - c) @ np.array(R)            # camera-plane coords (x right, y up)
    x0, y0 = loc[:, 0].min(), loc[:, 1].min()
    x1, y1 = loc[:, 0].max(), loc[:, 1].max()
    rx0, ry0, rx1, ry1 = rect
    p = max((x1 - x0) / (rx1 - rx0), (y1 - y0) / (ry1 - ry0))
    cam.data.ortho_scale = p * W
    fx, fy = (rx0 + rx1) / 2, (ry0 + ry1) / 2
    cx = (x0 + x1) / 2 - (fx - W / 2) * p
    cy = (y0 + y1) / 2 + (fy - H / 2) * p
    Rm = Matrix(R)
    cam.matrix_world = (Matrix.Translation(Vector(c) + Rm @ Vector((cx, cy, 0)) + Rm @ Vector((0, 0, 0.8)))
                        @ Rm.to_4x4())
    tgt = Vector(c) + Rm @ Vector((cx, cy, 0))
    span = p * W
    for name, local in (("key", (-0.55, 0.75, 0.9)), ("fill", (0.9, -0.2, 0.7)), ("top", (0.0, 0.2, 1.0))):
        o = bpy.data.objects[name]
        loc_w = tgt + Rm @ (Vector(local).normalized() * max(0.35, span * 3))
        if name == "top":
            loc_w = tgt + Vector(up).normalized() * max(0.35, span * 3)
        o.location = loc_w
        o.rotation_euler = (tgt - loc_w).to_track_quat('-Z', 'Y').to_euler()
        o.data.energy = {"key": 3.5, "fill": 1.0, "top": 1.2}[name] * (max(0.35, span * 3) / 0.5) ** 2


def project(sc, p_mm):
    v = world_to_camera_view(sc, sc.camera, Vector(p_mm) * MM)
    return [round(v.x * W, 1), round((1 - v.y) * H, 1)]


# ---------------------------------------------------------------- the shots
UP, DOWN = (0, 0, 1), (0, 0, -1)
FACE_UP = UP           # device screen up (backplate inside up)
FACE_DOWN = DOWN       # device back up
PAGE_RECT = (150, 190, 1450, 975)
PANEL_RECT = (70, 70, 1530, 1130)

DEVICE_ALL = ["device", "chassis+"]
CAM_INSIDE = ["camera_back", "pins", "plunger"]
CAM_DONE = DEVICE_ALL + CAM_INSIDE + ["unitv", "cable"]


def arrow(*pts):
    return [list(p) for p in pts]


def pins_arrows(z0, z1):
    return [arrow((x, y, z0), (x, y, z1)) for x, y in PINS]


SHOTS = [
    # ---- what you need (thumbnails, real colours)
    *[dict(key=f"part-{k}", thumb=True, show=s, pal="real", dir=d, up=u, cable=c, rect=(60, 60, 740, 540))
      for k, s, d, u, c in [
          ("device", ["device"], (-0.35, -0.6, 0.75), UP, None),
          ("unitv", ["unitv"], (0.35, -0.55, -0.75), DOWN, None),
          ("cable", ["cable"], (-0.2, -0.45, 0.85), UP, "loose"),
          ("chassis", ["chassis"], (0.3, -0.55, -0.8), DOWN, None),
          ("nut", ["nut1"], (-0.35, -0.6, 0.75), UP, None),
          ("csk", ["csk1", "csk1-sock"], (0.3, -1.0, -0.55), DOWN, None),
          ("btn", ["btn1", "btn1-sock"], (0.3, -1.0, -0.55), DOWN, None),
          ("camera_back", ["camera_back"], (-0.3, -0.55, 0.8), UP, None),
          ("plain_back", ["plain_back"], (-0.3, -0.55, 0.8), UP, None),
          ("bezel", ["bezel"], (-0.3, -0.55, 0.8), UP, None),
          ("link", ["link"], (1.0, -0.45, 0.3), UP, None),
          ("lens_ring", ["lens_ring"], (0.3, -0.5, -0.8), DOWN, None),
          ("pins", ["pins"], (-0.25, -1.0, 0.35), UP, None),
          ("plunger", ["plunger"], (-0.7, -1.0, 0.3), UP, None),
          ("glue", ["glue"], (-0.45, -1.0, 0.35), UP, None),
      ]],
    # ---- callout insets (orange, enlarged)
    *[dict(key=f"hi-{k}", thumb=True, show=s, hi=s, dir=d, up=u, cable=None, rect=(50, 50, 750, 550))
      for k, s, d, u in [
          ("nut", ["nut1"], (-0.35, -0.6, 0.75), UP),
          ("csk", ["csk1", "csk1-sock"], (0.3, -1.0, -0.45), DOWN),
          ("btn", ["btn1", "btn1-sock"], (0.3, -1.0, -0.45), DOWN),
          ("pin", ["pins"], (-0.25, -1.0, 0.3), UP),
          ("plunger", ["plunger"], (-0.7, -1.0, 0.3), UP),
      ]],

    # ---- 1. nuts into the chassis (flat side up)
    dict(key="s01", show=["chassis", "nuts"], hi=["nuts"], move={"nuts": (0, 0, 16)},
         dir=(-0.25, -0.62, 0.75), up=UP,
         arrows=[arrow((x, y, 13.5), (x, y, 2.2)) for x, y in NUTS],
         callouts=[dict(thumb="hi-nut", label="M2 nut", count=2, at=(79, 6, 17.6))]),
    # ---- 2. chassis onto the device's back + 2 x M2 x 4
    dict(key="s02", show=DEVICE_ALL, hi=["chassis", "csk"], move={"chassis": (0, 0, -16), "nuts": (0, 0, -16), "csk": (0, 0, -38)},
         dir=(0.3, 0.62, -0.75), up=DOWN,
         arrows=[arrow((-6, 27, -16), (-6, 27, -3.5)), arrow((90, 27, -16), (90, 27, -3.5))]
                + [arrow((x, y, -32.5), (x, y, -19.0)) for x, y in BLIND],
         callouts=[dict(thumb="hi-csk", label="M2 × 4", count=2, at=(28, 50, -39.6))],
         marks=[dict(text="camera", at=(42, 47.8, -0.5))]),
    # ---- 3. two pins into the bosses (inside up)
    dict(key="s03", show=["camera_back", "pins"], hi=["pins"], move={"pins": (0, 0, 22)},
         dir=(-0.18, -0.42, 0.9), up=UP, fit_box=((30, -4, -17), (92, 58, 19)),
         arrows=pins_arrows(5.5, -8.0),
         clicks=[(x + 6.5, y, -9.5) for x, y in PINS],
         callouts=[dict(thumb="hi-pin", label="UnitV pin", count=2, at=(76.8, 23.8, 21.0))]),
    # ---- 4. plunger into its guide, pin first
    dict(key="s04", show=["camera_back", "pins", "plunger"], hi=["plunger"], move={"plunger": (0, 0, 24)},
         dir=(-0.18, -0.42, 0.9), up=UP, fit_box=((30, -4, -17), (92, 58, 19)),
         arrows=[arrow((PLUNGER[0], PLUNGER[1], 6.0), (PLUNGER[0], PLUNGER[1], -3.0))],
         callouts=[dict(thumb="hi-plunger", label="Plunger", count=1, at=(PLUNGER[0], PLUNGER[1], 16))]),
    # ---- 5. Grove cable: under the lips, plug out through the side
    dict(key="s05", show=CAM_INSIDE + ["cable"], hi=["cable"], cable="free",
         dir=(0.1, -0.22, 1.0), up=UP,
         arrows=[arrow((76, PASS_Y + 9, -6), (100, PASS_Y + 9, -6))]
                + [arrow((68.8 - 7, y, -4), (68.8 - 3.5, y, -11)) for y in (10.5, 20.5)]),
    # ---- 6. plug the cable into the UnitV
    dict(key="s06", show=CAM_INSIDE + ["unitv", "cable"], hi=["unitv"], move={"unitv": (0, 14, 28)}, cable="lift_near",
         dir=(-0.45, -0.8, 0.5), up=UP,
         arrows=[arrow((68.8 + 9, 9.0, 22.3), (68.8 + 9, 17.5, 22.3))]),
    # ---- 7. UnitV onto the pins, lens over the well
    dict(key="s07", show=CAM_INSIDE + ["unitv", "cable"], hi=["unitv"], move={"unitv": (0, 0, 30)}, cable="lift_in",
         dir=(-0.4, -0.7, 0.6), up=UP,
         arrows=[arrow((53.5, 30, 24), (53.5, 30, 6)), arrow((85.5, 30, 24), (85.5, 30, 6))],
         clicks=[(x, y, -2.0) for x, y in PINS]),
    # ---- 8. device (with its chassis) into the backplate until the hooks click
    dict(key="s08", show=CAM_DONE, hi=["dev_front", "dev_back", "chassis"], move={"device": (0, 0, 38), "chassis+": (0, 0, 38)},
         cable="routed", dir=(-0.32, -0.62, 0.72), up=UP,
         arrows=[arrow((-9, 27, 46), (-9, 27, 24)), arrow((93, 27, 46), (93, 27, 24))],
         clicks=[]),
    # ---- 9. Grove cable into the device's side port
    dict(key="s09", show=CAM_DONE, hi=["cable"], cable="port_near",
         dir=(0.9, -0.4, 0.42), up=UP, fit=["camera_back", "cable", "dev_front"],
         arrows=[arrow((104, 40, 21.5), (91, 40, 21.5))]),
    # ---- 10. two M2 x 3 into the bottom corners (face down)
    dict(key="s10", show=CAM_DONE + ["btn"], hi=["btn"], move={"btn": (0, 0, -30)}, cable="plugged",
         dir=(0.25, -0.62, -0.75), up=DOWN,
         arrows=[arrow((x, y, -30.0), (x, y, -18.0)) for x, y in NUTS],
         callouts=[dict(thumb="hi-btn", label="M2 × 3", count=2, at=(79, 6, -34.5))]),
    # ---- 11. bezel snaps on from the front
    dict(key="s11", show=CAM_DONE + ["btn", "bezel"], hi=["bezel"], move={"bezel": (0, 0, 28)}, cable="plugged",
         dir=(-0.32, -0.62, 0.72), up=UP,
         arrows=[arrow((-9, 27, 44), (-9, 27, 26)), arrow((93, 27, 44), (93, 27, 26))],
         clicks=[(x, y, 17.4) for x, y in SNAPS]),
    # ---- 12. link cover over the pass-through
    dict(key="s12", show=CAM_DONE + ["btn", "bezel", "link"], hi=["link"], move={"link": (16, 0, 0)}, cable="plugged",
         dir=(0.9, -0.42, 0.4), up=UP, fit=["camera_back", "link", "bezel"],
         arrows=[arrow((106, 17, -2), (92, 17, -2)), arrow((106, 55, -2), (92, 55, -2))],
         clicks=[(X1 + 1, PASS_Y - 6.5, -8.2), (X1 + 1, PASS_Y + 6.5, -8.2)]),
    # ---- 13. lens ring into the lens well (face down)
    dict(key="s13", show=CAM_DONE + ["btn", "bezel", "link", "lens_ring"], hi=["lens_ring"], move={"lens_ring": (0, 0, -18)},
         cable="plugged", dir=(0.3, -0.55, -0.78), up=DOWN,
         arrows=[arrow((LENS[0] + 15, LENS[1], -34), (LENS[0] + 15, LENS[1], -20))]),
    # ---- plain guide: optional glue, then full-page versions of the plain steps
    dict(key="g1", show=["plain_back", "dabs"], hi=["dabs"], dir=(-0.3, 0.62, 0.8), up=UP,
         arrows=[arrow((x, y, 24), (x, y, 5.0)) for x, y in GLUE],
         callouts=[dict(thumb="part-glue", label="Small dab", count=2, at=(GLUE[1][0], GLUE[1][1], -0.9))]),
    dict(key="q4", show=["plain_back"] + DEVICE_ALL, hi=["dev_front", "dev_back", "chassis"],
         move={"device": (0, 0, 34), "chassis+": (0, 0, 34)}, dir=(-0.32, -0.62, 0.72), up=UP,
         arrows=[arrow((-9, 27, 42), (-9, 27, 22)), arrow((93, 27, 42), (93, 27, 22))],
         clicks=[]),
    dict(key="q5", show=["plain_back", "btn"] + DEVICE_ALL, hi=["btn"], move={"btn": (0, 0, -24)},
         dir=(0.25, -0.62, -0.75), up=DOWN,
         arrows=[arrow((x, y, -19.0), (x, y, -6.5)) for x, y in NUTS],
         callouts=[dict(thumb="hi-btn", label="M2 × 3", count=2, at=(79, 6, -28.5))]),
    dict(key="q6", show=["plain_back", "btn", "bezel"] + DEVICE_ALL, hi=["bezel"], move={"bezel": (0, 0, 28)},
         dir=(-0.32, -0.62, 0.72), up=UP,
         arrows=[arrow((-9, 27, 44), (-9, 27, 26)), arrow((93, 27, 44), (93, 27, 26))],
         clicks=[(x, y, 17.4) for x, y in SNAPS]),
    # ---- plain backplate: three panels
    dict(key="p1", panel=True, show=["plain_back"] + DEVICE_ALL, hi=["dev_front", "dev_back", "chassis"],
         move={"device": (0, 0, 34), "chassis+": (0, 0, 34)}, dir=(-0.32, -0.62, 0.72), up=UP,
         arrows=[arrow((-9, 27, 42), (-9, 27, 22)), arrow((93, 27, 42), (93, 27, 22))],
         clicks=[]),
    dict(key="p2", panel=True, show=["plain_back", "btn"] + DEVICE_ALL, hi=["btn"], move={"btn": (0, 0, -22)},
         dir=(0.25, -0.62, -0.75), up=DOWN,
         arrows=[arrow((x, y, -22.0), (x, y, -7.5)) for x, y in NUTS],
         callouts=[dict(thumb="hi-btn", label="M2 × 3", count=2, at=(79, 6, -26.5))]),
    dict(key="p3", panel=True, show=["plain_back", "btn", "bezel"] + DEVICE_ALL, hi=["bezel"], move={"bezel": (0, 0, 28)},
         dir=(-0.32, -0.62, 0.72), up=UP,
         arrows=[arrow((-9, 27, 44), (-9, 27, 26)), arrow((93, 27, 44), (93, 27, 26))],
         clicks=[(x, y, 17.4) for x, y in SNAPS]),
    # ---- swapping backs: three panels (plain back shown; the camera back comes off the same way)
    dict(key="w1", panel=True, show=["plain_back", "btn", "bezel"] + DEVICE_ALL, hi=["bezel"], move={"bezel": (0, 0, 28)},
         dir=(-0.32, -0.62, 0.72), up=UP,
         arrows=[arrow((-9, 27, 26), (-9, 27, 46)), arrow((93, 27, 26), (93, 27, 46))]),
    dict(key="w2", panel=True, show=["plain_back", "btn"] + DEVICE_ALL, hi=["btn"], move={"btn": (0, 0, -22)},
         dir=(0.25, -0.62, -0.75), up=DOWN,
         arrows=[arrow((x, y, -7.5), (x, y, -22.0)) for x, y in NUTS],
         callouts=[dict(thumb="hi-btn", label="M2 × 3", count=2, at=(79, 6, -26.5))]),
    dict(key="w3", panel=True, show=["plain_back"] + DEVICE_ALL, hi=["dev_front", "dev_back", "chassis"],
         tilt=("device chassis+", 14, (42, -1.0, -1.6)), dir=(-0.8, -0.35, 0.45), up=UP,
         arrows=[[[-10, 62 * math.cos(math.radians(a)) - 1.0, -1.6 + 62 * math.sin(math.radians(a)) + 6]
                  for a in range(4, 44, 4)]]),
]


def apply_shot(sc, s):
    show = set(expand(s["show"]))
    hi = set(expand(s.get("hi", [])))
    ghost = set(expand(s.get("ghost", [])))
    pal = s.get("pal", "grey")
    for n, o in OBJ.items():
        o.hide_render = n not in show
        o.matrix_world = BASE[n].copy()
        role = ROLE[n]
        if n.startswith("plug"):
            continue
        if role == "screen":
            continue
        if n in hi and role not in KEEP:
            m = MATS["hi2"] if role in ("devback", "uvb") else MATS["hi"]
        elif n in ghost:
            m = MATS[("ghost", role)]
        else:
            m = MATS[(pal, role)]
        o.material_slots[0].material = m
    for g, d in s.get("move", {}).items():
        for n in expand([g]):
            OBJ[n].matrix_world = Matrix.Translation(Vector(d) * MM) @ BASE[n]
    if "tilt" in s:
        names, ang, pivot = s["tilt"]
        R = (Matrix.Translation(Vector(pivot) * MM) @ Matrix.Rotation(math.radians(ang), 4, 'X')
             @ Matrix.Translation(-Vector(pivot) * MM))
        for n in expand(names.split()):
            OBJ[n].matrix_world = Matrix.Translation(Vector((0, 0, 6)) * MM) @ R @ BASE[n]
    # cable
    cs = s.get("cable")
    if "cable" not in show:
        cs = None
    pal_cable = MATS["hi"] if "cable" in hi else MATS[(pal, "cable")]
    pal_plug = MATS["hi"] if "cable" in hi else MATS[(pal, "plug")]
    if cs == "plugged_free":     # device lifted: draw the cable as plugged into the UnitV, port end hidden
        cs = "routed"
    disp = s.get("move", {}).get("unitv", (0, 0, 0)) if cs in ("lift_near", "lift_in") else (0, 0, 0)
    set_cable(cs, pal_cable, pal_plug, pal_plug)
    if cs in ("lift_near", "lift_in"):
        pass
    return show


def render_shot(sc, s, manifest):
    global W, H
    show = apply_shot(sc, s)
    names = [n for n in show if n in OBJ and not OBJ[n].hide_render] + (["cable", "plugA", "plugB"] if CABLE.get("obj") else [])
    fit = expand(s["fit"]) + (["cable"] if "cable" in s["fit"] else []) if s.get("fit") else names
    pts = world_points(fit)
    if s.get("fit_box"):
        (a0, b0, c0), (a1, b1, c1) = s["fit_box"]
        pts = np.array([(x, y, z) for x in (a0, a1) for y in (b0, b1) for z in (c0, c1)]) * MM
    extra =[np.array(p) * MM for a in s.get("arrows", []) for p in a]
    extra += [np.array(c["at"]) * MM for c in s.get("callouts", [])]
    if extra:
        pts = np.concatenate([pts, np.array(extra)])
    rect = s.get("rect") or (PANEL_RECT if s.get("panel") else PAGE_RECT)
    if s.get("thumb"):
        sc.render.resolution_x, sc.render.resolution_y = 800, 600
        W, H = 800, 600
    else:
        sc.render.resolution_x, sc.render.resolution_y = 1600, 1200
        W, H = 1600, 1200
    sc.render.line_thickness = 1.0
    bpy.context.view_layer.freestyle_settings.linesets[0].linestyle.thickness = (2.0 if s.get("thumb") else 2.6) * SCALE
    frame_camera(sc, s["dir"], s["up"], pts, rect)
    bpy.context.view_layer.update()
    sc.render.filepath = os.path.join(OUT, s["key"] + ".png")
    bpy.ops.render.render(write_still=True)
    entry = dict(key=s["key"], size=[W, H])
    entry["arrows"] = [[project(sc, p) for p in a] for a in s.get("arrows", [])]
    entry["clicks"] = [project(sc, p) for p in s.get("clicks", [])]
    entry["callouts"] = [dict(c, at=project(sc, c["at"])) for c in s.get("callouts", [])]
    entry["marks"] = [dict(m, at=project(sc, m["at"])) for m in s.get("marks", [])]
    manifest[s["key"]] = entry


def main():
    os.makedirs(OUT, exist_ok=True)
    sc = scene_setup()
    build_materials()
    load_all()
    mpath = os.path.join(OUT, "manifest.json")
    manifest = json.load(open(mpath)) if os.path.exists(mpath) else {}
    for s in SHOTS:
        if ONLY and not any(s["key"] == k or s["key"].startswith(k + "-") or (k == "parts" and s["key"].startswith("part-"))
                            or (k == "hi" and s["key"].startswith("hi-")) for k in ONLY):
            continue
        render_shot(sc, s, manifest)
        with open(mpath, "w") as f:
            json.dump(manifest, f, indent=1)
        print("SHOT", s["key"], flush=True)


main()

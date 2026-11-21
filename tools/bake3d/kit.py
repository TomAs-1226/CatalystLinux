"""The modelling kit the code-built objects are made with, and the stage
they are rendered on. Runs inside Blender (5.2, Cycles).

Every object is made of a handful of materials that say what a surface is,
not what colour it is. Colour comes later, on the handheld, from the palette
in use: see `userland/apps/shell/src/object3d.rs`. So each material renders
twice:

* lit: a neutral grey of its own lightness (dark glass, graphite, the body,
  white), lit by Bezel's one light from above and to the left, with soft
  shadows and the ambient occlusion global illumination gives;
* data: flat emission saying which tint it takes (red channel: tint A, the
  signal; green: tint B, ice or amber as the object chooses) and that it is
  the object (blue = 1), so the shadow catcher's pixels can be told apart.
"""

import math

import bmesh
import bpy
import numpy as np
from mathutils import Vector

# ----------------------------------------------------------------- materials --

# name: (lightness, roughness, metallic, coat, tint A, tint B, glow)
# Lightness is the albedo of the neutral render. Tinted materials sit at the
# lightness their tint's colour is anchored to (see ACCENT_ANCHOR in pack.py).
MATERIALS = {
    "body": (0.62, 0.42, 0.0, 0.25, 0, 0, 0.0),
    "white": (0.86, 0.36, 0.0, 0.30, 0, 0, 0.0),
    "soft": (0.36, 0.48, 0.0, 0.10, 0, 0, 0.0),
    "graphite": (0.10, 0.42, 0.0, 0.25, 0, 0, 0.0),
    "dark": (0.025, 0.16, 0.0, 0.60, 0, 0, 0.0),
    "rubber": (0.05, 0.80, 0.0, 0.00, 0, 0, 0.0),
    "metal": (0.78, 0.26, 1.0, 0.00, 0, 0, 0.0),
    "accent": (0.55, 0.36, 0.0, 0.35, 1, 0, 0.0),
    "accent_glow": (0.55, 0.40, 0.0, 0.00, 1, 0, 0.5),
    "second": (0.55, 0.36, 0.0, 0.35, 0, 1, 0.0),
    "second_glow": (0.55, 0.40, 0.0, 0.00, 0, 1, 0.5),
    "glow": (0.90, 0.40, 0.0, 0.00, 0, 0, 0.9),
}

_lit = {}
_data = {}


def _principled(m, lightness, rough, metal, coat):
    nt = m.node_tree
    bsdf = nt.nodes.get("Principled BSDF")
    bsdf.inputs["Base Color"].default_value = (lightness, lightness, lightness, 1.0)
    bsdf.inputs["Roughness"].default_value = rough
    bsdf.inputs["Metallic"].default_value = metal
    bsdf.inputs["Coat Weight"].default_value = coat
    bsdf.inputs["Coat Roughness"].default_value = 0.18
    return bsdf


def material(name):
    """The lit material of a class (made once per scene)."""
    if name in _lit and _lit[name].name in bpy.data.materials:
        return _lit[name]
    lightness, rough, metal, coat, ta, tb, glow = MATERIALS[name]
    m = bpy.data.materials.new(name)
    m.use_nodes = True
    bsdf = _principled(m, lightness, rough, metal, coat)
    if glow > 0:
        bsdf.inputs["Emission Color"].default_value = (lightness, lightness, lightness, 1.0)
        bsdf.inputs["Emission Strength"].default_value = glow
    _lit[name] = m
    d = bpy.data.materials.new(name + "~data")
    d.use_nodes = True
    nt = d.node_tree
    nt.nodes.clear()
    out = nt.nodes.new("ShaderNodeOutputMaterial")
    em = nt.nodes.new("ShaderNodeEmission")
    em.inputs["Color"].default_value = (ta, tb, 1.0, 1.0)
    em.inputs["Strength"].default_value = 1.0
    nt.links.new(em.outputs["Emission"], out.inputs["Surface"])
    _data[name] = d
    m["data"] = d.name
    return m


def vertex_material():
    """For a generated model (glb_prep.py): the neutral lightness in the red
    of its vertex colours, the accent in the green."""
    if "vertex" in _lit and _lit["vertex"].name in bpy.data.materials:
        return _lit["vertex"]
    m = bpy.data.materials.new("vertex")
    m.use_nodes = True
    nt = m.node_tree
    bsdf = _principled(m, 0.5, 0.42, 0.0, 0.2)
    attr = nt.nodes.new("ShaderNodeVertexColor")
    attr.layer_name = "neutral"
    sep = nt.nodes.new("ShaderNodeSeparateColor")
    nt.links.new(attr.outputs["Color"], sep.inputs["Color"])
    # Where it is the accent, the lightness the accent is anchored to.
    mix = nt.nodes.new("ShaderNodeMix")
    mix.data_type = "FLOAT"
    mix.inputs["B"].default_value = MATERIALS["accent"][0]
    nt.links.new(sep.outputs["Green"], mix.inputs["Factor"])
    nt.links.new(sep.outputs["Red"], mix.inputs["A"])
    grey = nt.nodes.new("ShaderNodeCombineColor")
    for ch in ("Red", "Green", "Blue"):
        nt.links.new(mix.outputs["Result"], grey.inputs[ch])
    nt.links.new(grey.outputs["Color"], bsdf.inputs["Base Color"])
    _lit["vertex"] = m
    d = bpy.data.materials.new("vertex~data")
    d.use_nodes = True
    nt = d.node_tree
    nt.nodes.clear()
    out = nt.nodes.new("ShaderNodeOutputMaterial")
    em = nt.nodes.new("ShaderNodeEmission")
    attr = nt.nodes.new("ShaderNodeVertexColor")
    attr.layer_name = "neutral"
    sep = nt.nodes.new("ShaderNodeSeparateColor")
    comb = nt.nodes.new("ShaderNodeCombineColor")
    nt.links.new(attr.outputs["Color"], sep.inputs["Color"])
    nt.links.new(sep.outputs["Green"], comb.inputs["Red"])
    comb.inputs["Blue"].default_value = 1.0
    nt.links.new(comb.outputs["Color"], em.inputs["Color"])
    nt.links.new(em.outputs["Emission"], out.inputs["Surface"])
    _data["vertex"] = d
    m["data"] = d.name
    return m


def import_model(path):
    """A model kept by glb_prep.py, with its vertex-colour material."""
    before = set(bpy.data.objects)
    bpy.ops.import_scene.gltf(filepath=path)
    new = [o for o in bpy.data.objects if o not in before]
    root = empty("model")
    for o in new:
        if o.type == "MESH":
            # glTF brings the colours back as COLOR_0, named "Color".
            if o.data.color_attributes:
                o.data.color_attributes[0].name = "neutral"
            o.data.materials.clear()
            o.data.materials.append(vertex_material())
            for p in o.data.polygons:
                p.use_smooth = True
        if o.parent is None:
            o.parent = root
    return root


def tag(obj, part):
    """Say which part of a robot (or other object with parts) `obj` and
    everything under it belong to: the shell colours each part on its own."""
    obj["part"] = part
    return obj


def part_of(obj):
    """The part an object belongs to: its own tag, or the nearest above it."""
    while obj is not None:
        if "part" in obj:
            return obj["part"]
        obj = obj.parent
    return None


def part_material(index):
    """Flat emission carrying a part's number (out of 16) in its red."""
    name = f"part~{index}"
    if name in bpy.data.materials:
        return bpy.data.materials[name]
    d = bpy.data.materials.new(name)
    d.use_nodes = True
    nt = d.node_tree
    nt.nodes.clear()
    out = nt.nodes.new("ShaderNodeOutputMaterial")
    em = nt.nodes.new("ShaderNodeEmission")
    em.inputs["Color"].default_value = (index / 16.0, 0.0, 0.0, 1.0)
    em.inputs["Strength"].default_value = 1.0
    nt.links.new(em.outputs["Emission"], out.inputs["Surface"])
    return d


def reset_materials():
    _lit.clear()
    _data.clear()


# ------------------------------------------------------------------ geometry --

def _link(obj):
    bpy.context.scene.collection.objects.link(obj)
    return obj


def _mesh_object(name, bm, mat, smooth=True):
    me = bpy.data.meshes.new(name)
    bm.to_mesh(me)
    bm.free()
    obj = bpy.data.objects.new(name, me)
    _link(obj)
    if mat:
        obj.data.materials.append(material(mat))
    if smooth:
        for p in obj.data.polygons:
            p.use_smooth = True
    return obj


def bevel(obj, width, segments=5, angle=50.0, harden=True):
    """Round the object's edges, keeping flat faces flat."""
    mod = obj.modifiers.new("bevel", "BEVEL")
    mod.width = width
    mod.segments = segments
    mod.limit_method = "ANGLE"
    mod.angle_limit = math.radians(angle)
    mod.harden_normals = harden
    mod.use_clamp_overlap = True
    mod.miter_outer = "MITER_ARC"
    return obj


def subsurf(obj, levels=2):
    mod = obj.modifiers.new("subsurf", "SUBSURF")
    mod.levels = levels
    mod.render_levels = levels
    return obj


def place(obj, loc=(0, 0, 0), rot=(0, 0, 0), scale=None, parent=None):
    obj.location = Vector(loc)
    obj.rotation_euler = tuple(math.radians(a) for a in rot)
    if scale is not None:
        obj.scale = (scale, scale, scale) if isinstance(scale, (int, float)) else scale
    if parent is not None:
        obj.parent = parent
    return obj


def box(size, loc=(0, 0, 0), r=0.06, mat="body", rot=(0, 0, 0), segments=5, parent=None, name="box"):
    """A box `size` = (x, y, z) with its edges rounded by `r`."""
    bm = bmesh.new()
    bmesh.ops.create_cube(bm, size=1.0)
    bmesh.ops.scale(bm, vec=Vector(size), verts=bm.verts)
    obj = _mesh_object(name, bm, mat)
    if r > 0:
        bevel(obj, r, segments)
    return place(obj, loc, rot, parent=parent)


def cyl(radius, depth, loc=(0, 0, 0), r=0.04, mat="body", rot=(0, 0, 0), verts=64, parent=None, radius2=None, name="cyl", segments=4):
    """A cylinder (or a cone, with `radius2`) along its own z, rounded rims."""
    bm = bmesh.new()
    bmesh.ops.create_cone(bm, cap_ends=True, cap_tris=False, segments=verts, radius1=radius, radius2=radius if radius2 is None else radius2, depth=depth)
    obj = _mesh_object(name, bm, mat)
    if r > 0:
        bevel(obj, r, segments, angle=40)
    return place(obj, loc, rot, parent=parent)


def sphere(radius, loc=(0, 0, 0), mat="body", parent=None, scale=None, name="sphere", rot=(0, 0, 0)):
    bm = bmesh.new()
    bmesh.ops.create_uvsphere(bm, u_segments=64, v_segments=32, radius=radius)
    obj = _mesh_object(name, bm, mat)
    return place(obj, loc, rot, scale=scale, parent=parent)


def torus(major, minor, loc=(0, 0, 0), mat="body", rot=(0, 0, 0), parent=None, arc=360.0, start=0.0, name="torus", caps=True):
    """A torus, or an arc of one from `start` through `arc` degrees (round caps)."""
    bm = bmesh.new()
    seg_major = max(8, int(96 * arc / 360.0))
    seg_minor = 24
    full = arc >= 359.9
    rings = []
    n = seg_major if full else seg_major + 1
    for i in range(n):
        a = math.radians(start + arc * i / seg_major)
        centre = Vector((math.cos(a) * major, math.sin(a) * major, 0))
        out = Vector((math.cos(a), math.sin(a), 0))
        ring = []
        for j in range(seg_minor):
            b = 2 * math.pi * j / seg_minor
            ring.append(bm.verts.new(centre + out * math.cos(b) * minor + Vector((0, 0, math.sin(b) * minor))))
        rings.append(ring)
    pairs = range(len(rings)) if full else range(len(rings) - 1)
    for i in pairs:
        a, b = rings[i], rings[(i + 1) % len(rings)]
        for j in range(seg_minor):
            bm.faces.new((a[j], b[j], b[(j + 1) % seg_minor], a[(j + 1) % seg_minor]))
    obj = _mesh_object(name, bm, mat)
    place(obj, loc, rot, parent=parent)
    if not full and caps:
        for k, end in ((0, start), (1, start + arc)):
            a = math.radians(end)
            c = Vector((math.cos(a) * major, math.sin(a) * major, 0))
            s = sphere(minor, c, mat, name=name + "-cap")
            s.parent = obj
    return obj


def prism(points, depth, loc=(0, 0, 0), r=0.03, mat="body", rot=(0, 0, 0), parent=None, name="prism", segments=4):
    """A flat shape (points in x, y, counter-clockwise) extruded `depth` along z, rounded."""
    bm = bmesh.new()
    verts = [bm.verts.new((x, y, -depth / 2)) for x, y in points]
    face = bm.faces.new(verts)
    bmesh.ops.recalc_face_normals(bm, faces=[face])
    ext = bmesh.ops.extrude_face_region(bm, geom=[face])
    top = [v for v in ext["geom"] if isinstance(v, bmesh.types.BMVert)]
    bmesh.ops.translate(bm, vec=Vector((0, 0, depth)), verts=top)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    obj = _mesh_object(name, bm, mat)
    if r > 0:
        bevel(obj, r, segments, angle=30)
    return place(obj, loc, rot, parent=parent)


def tube(points, radius, mat="body", parent=None, closed=False, name="tube", resolution=12, loc=(0, 0, 0), rot=(0, 0, 0)):
    """A round tube along a path of 3D points (a smooth Bezier through them)."""
    cu = bpy.data.curves.new(name, "CURVE")
    cu.dimensions = "3D"
    cu.bevel_depth = radius
    cu.bevel_resolution = 6
    cu.use_fill_caps = True
    cu.resolution_u = resolution
    sp = cu.splines.new("BEZIER")
    sp.bezier_points.add(len(points) - 1)
    for bp, p in zip(sp.bezier_points, points):
        bp.co = Vector(p)
        bp.handle_left_type = bp.handle_right_type = "AUTO"
    sp.use_cyclic_u = closed
    obj = bpy.data.objects.new(name, cu)
    _link(obj)
    obj.data.materials.append(material(mat))
    return place(obj, loc, rot, parent=parent)


def lathe(profile, loc=(0, 0, 0), mat="body", rot=(0, 0, 0), parent=None, name="lathe", segments=72):
    """A solid of revolution about z from a profile of (radius, z) points, bottom to top."""
    bm = bmesh.new()
    rings = []
    for i in range(segments):
        a = 2 * math.pi * i / segments
        rings.append([bm.verts.new((rr * math.cos(a), rr * math.sin(a), z)) for rr, z in profile])
    for i in range(segments):
        a, b = rings[i], rings[(i + 1) % segments]
        for j in range(len(profile) - 1):
            bm.faces.new((a[j], b[j], b[j + 1], a[j + 1]))
    bmesh.ops.remove_doubles(bm, verts=bm.verts, dist=1e-5)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    obj = _mesh_object(name, bm, mat)
    return place(obj, loc, rot, parent=parent)


def text(s, size, depth, loc=(0, 0, 0), mat="white", rot=(0, 0, 0), parent=None, font=None, bevel_depth=0.01, align="CENTER", name="text"):
    cu = bpy.data.curves.new(name, "FONT")
    cu.body = s
    cu.size = size
    cu.extrude = depth / 2
    cu.bevel_depth = bevel_depth
    cu.bevel_resolution = 3
    cu.align_x = align
    cu.align_y = "CENTER"
    if font:
        cu.font = bpy.data.fonts.load(font, check_existing=True)
    obj = bpy.data.objects.new(name, cu)
    _link(obj)
    obj.data.materials.append(material(mat))
    return place(obj, loc, rot, parent=parent)


def empty(name="root", loc=(0, 0, 0), rot=(0, 0, 0), parent=None):
    e = bpy.data.objects.new(name, None)
    _link(e)
    return place(e, loc, rot, parent=parent)


def parent_all(objs, root):
    for o in objs:
        if o.parent is None:
            o.parent = root
    return root


def rounded_rect(w, h, r, n=8):
    """The outline of a rounded rectangle, centred, counter-clockwise."""
    pts = []
    for cx, cy, a0 in ((w / 2 - r, h / 2 - r, 0), (-w / 2 + r, h / 2 - r, 90), (-w / 2 + r, -h / 2 + r, 180), (w / 2 - r, -h / 2 + r, 270)):
        for i in range(n + 1):
            a = math.radians(a0 + 90 * i / n)
            pts.append((cx + r * math.cos(a), cy + r * math.sin(a)))
    return pts


def ring_band(r_in, r_out, depth, start, sweep, loc=(0, 0, 0), mat="body", rot=(0, 0, 0), parent=None, round_ends=True, name="band", r=0.02):
    """A flat arc band (annulus sector) extruded along z, its ends rounded."""
    n = max(12, int(abs(sweep) / 3))
    outer = [(r_out * math.cos(math.radians(start + sweep * i / n)), r_out * math.sin(math.radians(start + sweep * i / n))) for i in range(n + 1)]
    inner = [(r_in * math.cos(math.radians(start + sweep * i / n)), r_in * math.sin(math.radians(start + sweep * i / n))) for i in range(n, -1, -1)]
    pts = outer
    mid = (r_in + r_out) / 2
    half = (r_out - r_in) / 2
    if round_ends:
        # A half circle at the far end, then the inner edge, then one at the start.
        a1 = math.radians(start + sweep)
        c1 = (mid * math.cos(a1), mid * math.sin(a1))
        for i in range(1, 12):
            t = math.pi * i / 12
            d = a1 + t * (1 if sweep > 0 else -1)
            pts.append((c1[0] + half * math.cos(d), c1[1] + half * math.sin(d)))
        pts = pts + inner
        a0 = math.radians(start)
        c0 = (mid * math.cos(a0), mid * math.sin(a0))
        for i in range(1, 12):
            t = math.pi * i / 12
            d = a0 + math.pi + t * (1 if sweep > 0 else -1)
            pts.append((c0[0] + half * math.cos(d), c0[1] + half * math.sin(d)))
    else:
        pts = pts + inner
    if sweep < 0:
        pts = pts[::-1]
    return prism(pts, depth, loc, r=r, mat=mat, rot=rot, parent=parent, name=name)


def svg_glyph(path, depth, size, loc=(0, 0, 0), mat="white", rot=(0, 0, 0), parent=None, r=0.01):
    """A Material Symbol's outline, extruded: imported as curves, joined, and filled."""
    before = set(bpy.data.objects)
    bpy.ops.import_curve.svg(filepath=path)
    new = [o for o in bpy.data.objects if o not in before]
    curves = [o for o in new if o.type == "CURVE"]
    for o in curves:
        o.data.extrude = depth / 2
        o.data.bevel_depth = r
        o.data.fill_mode = "BOTH"
        o.data.materials.clear()
        o.data.materials.append(material(mat))
    # The symbols are 960 units across; scale to `size` and centre.
    root = empty("glyph", loc, rot, parent)
    for o in curves:
        o.parent = root
    xs, ys = [], []
    for o in curves:
        for sp in o.data.splines:
            for bp in sp.bezier_points:
                xs.append(bp.co.x)
                ys.append(bp.co.y)
    cx, cy = (min(xs) + max(xs)) / 2, (min(ys) + max(ys)) / 2
    span = max(max(xs) - min(xs), max(ys) - min(ys))
    k = size / span
    for o in curves:
        o.location = Vector((-cx * k, -cy * k, 0))
        o.scale = (k, k, k)
    return root


# --------------------------------------------------------------------- stage --

def new_scene():
    bpy.ops.wm.read_factory_settings(use_empty=True)
    reset_materials()
    scene = bpy.context.scene
    scene.render.engine = "CYCLES"
    prefs = bpy.context.preferences.addons["cycles"].preferences
    for kind in ("OPTIX", "CUDA"):
        try:
            prefs.compute_device_type = kind
            prefs.get_devices()
            if any(d.type == kind for d in prefs.devices):
                break
        except TypeError:
            continue
    for d in prefs.devices:
        d.use = d.type != "CPU"
    scene.cycles.device = "GPU" if any(d.use for d in prefs.devices) else "CPU"
    scene.render.film_transparent = True
    scene.render.use_persistent_data = True
    scene.cycles.seed = 5805
    scene.cycles.use_animated_seed = False
    scene.view_settings.view_transform = "Standard"
    scene.view_settings.look = "None"
    scene.view_settings.exposure = 0.0
    scene.view_settings.gamma = 1.0
    scene.render.image_settings.file_format = "PNG"
    scene.render.image_settings.color_mode = "RGBA"
    scene.render.image_settings.color_depth = "8"
    scene.render.filter_size = 1.2
    return scene


def world(scene):
    """The ambient: a soft neutral sky, brighter above, darker below, so
    tops catch light, undersides fall off and crevices darken."""
    w = bpy.data.worlds.new("sky")
    scene.world = w
    w.use_nodes = True
    nt = w.node_tree
    nt.nodes.clear()
    out = nt.nodes.new("ShaderNodeOutputWorld")
    bg = nt.nodes.new("ShaderNodeBackground")
    coords = nt.nodes.new("ShaderNodeTexCoord")
    sep = nt.nodes.new("ShaderNodeSeparateXYZ")
    ramp = nt.nodes.new("ShaderNodeValToRGB")
    ramp.color_ramp.elements[0].position = 0.0
    ramp.color_ramp.elements[0].color = (0.035, 0.035, 0.035, 1)
    ramp.color_ramp.elements[1].position = 1.0
    ramp.color_ramp.elements[1].color = (0.30, 0.30, 0.30, 1)
    mid = ramp.color_ramp.elements.new(0.5)
    mid.color = (0.14, 0.14, 0.14, 1)
    mapr = nt.nodes.new("ShaderNodeMapRange")
    mapr.inputs["From Min"].default_value = -1.0
    mapr.inputs["From Max"].default_value = 1.0
    nt.links.new(coords.outputs["Generated"], sep.inputs["Vector"])
    # Generated on the world is the view direction.
    nt.links.new(sep.outputs["Z"], mapr.inputs["Value"])
    nt.links.new(mapr.outputs["Result"], ramp.inputs["Fac"])
    nt.links.new(ramp.outputs["Color"], bg.inputs["Color"])
    bg.inputs["Strength"].default_value = 1.0
    nt.links.new(bg.outputs["Background"], out.inputs["Surface"])
    return w


def evaluated_points(objs):
    """Every vertex of the objects as rendered (modifiers applied), in world space."""
    dg = bpy.context.evaluated_depsgraph_get()
    pts = []
    for o in objs:
        if o.type not in ("MESH", "CURVE", "FONT", "SURFACE"):
            continue
        ev = o.evaluated_get(dg)
        try:
            me = ev.to_mesh()
        except RuntimeError:
            continue
        if me is None:
            continue
        mw = ev.matrix_world
        co = np.empty(len(me.vertices) * 3, np.float32)
        me.vertices.foreach_get("co", co)
        co = co.reshape(-1, 3)[:: max(1, len(me.vertices) // 4000)]
        pts.extend(mw @ Vector(c) for c in co)
        ev.to_mesh_clear()
    return pts


def normalise(root, size=2.0):
    """Scale and move a model so it is `size` across at its widest and stands
    centred on the origin, its lowest point on z = 0."""
    bpy.context.view_layer.update()
    objs = [o for o in bpy.data.objects if o.type != "EMPTY" or o is root]
    pts = evaluated_points(objs)
    lo = Vector((min(p.x for p in pts), min(p.y for p in pts), min(p.z for p in pts)))
    hi = Vector((max(p.x for p in pts), max(p.y for p in pts), max(p.z for p in pts)))
    span = max(hi.x - lo.x, hi.y - lo.y, hi.z - lo.z)
    k = size / span
    centre = Vector(((lo.x + hi.x) / 2, (lo.y + hi.y) / 2, lo.z))
    holder = empty("holder")
    root.parent = holder
    holder.scale = (k, k, k)
    holder.location = -centre * k
    pivot = empty("pivot")
    holder.parent = pivot
    bpy.context.view_layer.update()
    return pivot

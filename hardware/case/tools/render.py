"""Blender (Cycles) product renders of the case concept.

usage: blender -b --factory-startup -P tools/render.py -- VARIANT VIEWS OUTDIR [SAMPLES]
  VARIANT camera | plain   (which backplate)
  VIEWS   comma list of: front, back, side, three-quarter, back-three-quarter, exploded
Needs out/stl/*.stl (tools/build.py), _ref/derived/*.stl (tools/prep_refs.py) and
the Bezel home screenshot in shots/looks/3-bezel-home.png.
"""
import bpy, json, math, os, sys
from bpy_extras.object_utils import world_to_camera_view
from mathutils import Vector

argv = sys.argv[sys.argv.index("--") + 1:]
VARIANT, VIEWS, OUT = argv[0], argv[1].split(","), argv[2]
SAMPLES = int(argv[3]) if len(argv) > 3 else 160
HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
STL, REF = os.path.join(HERE, "out", "stl"), os.path.join(HERE, "_ref", "derived")
SCREEN_IMG = os.path.join(HERE, "..", "..", "shots", "looks", "3-bezel-home.png")
MM = 0.001
CENTER = (42.0, 27.0, 4.0)        # device-coordinate point placed at the rig origin


def srgb(h):
    h = h.lstrip("#")
    c = [int(h[i:i + 2], 16) / 255 for i in (0, 2, 4)]
    return tuple(((x + 0.055) / 1.055) ** 2.4 if x > 0.04045 else x / 12.92 for x in c) + (1.0,)


def material(name, color, rough=0.5, metal=0.0, sss=0.0, coat=0.0, emit_img=None):
    m = bpy.data.materials.new(name)
    m.use_nodes = True
    nt = m.node_tree
    b = [n for n in nt.nodes if n.type == 'BSDF_PRINCIPLED'][0]
    b.inputs["Base Color"].default_value = srgb(color)
    b.inputs["Roughness"].default_value = rough
    b.inputs["Metallic"].default_value = metal
    if sss:
        b.inputs["Subsurface Weight"].default_value = sss
        b.inputs["Subsurface Radius"].default_value = (1.0, 1.0, 1.0)
        b.inputs["Subsurface Scale"].default_value = 0.002
    if coat:
        b.inputs["Coat Weight"].default_value = coat
        b.inputs["Coat Roughness"].default_value = 0.03
    if emit_img:
        tex = nt.nodes.new("ShaderNodeTexImage")
        tex.image = bpy.data.images.load(emit_img)
        nt.links.new(tex.outputs["Color"], b.inputs["Emission Color"])
        b.inputs["Emission Strength"].default_value = 1.6
        b.inputs["Base Color"].default_value = (0, 0, 0, 1)
    return m


SCHEMES = {
    # recommended: matte black body, white PETG inside, one gold signal, ice link
    "night": dict(frame="black", back="black", link="ice", pipe="ice", trim="gold", chassis="petg"),
    # alternative: light tonal body
    "ice": dict(frame="petg", back="ice", link="petg", pipe="petg", trim="gold", chassis="petg"),
}


def build_materials():
    return {
        "black": material("black PLA", "#17181b", rough=0.62),
        "petg": material("white PETG", "#dddcd6", rough=0.36, sss=0.15),
        "ice": material("light blue PLA", "#a7cbe8", rough=0.55),
        "gold": material("gold PLA", "#c2913f", rough=0.30, metal=1.0),
        "dev": material("device shell", "#4a4d53", rough=0.42),
        "devback": material("device back", "#3d4045", rough=0.48),
        "key": material("keycaps", "#26272a", rough=0.32),
        "glass": material("lens glass", "#07090c", rough=0.03, coat=1.0),
        "barrel": material("lens barrel", "#1c1d20", rough=0.55),
        "screw": material("black oxide screw", "#2a2b2e", rough=0.35, metal=0.8),
        "screen": material("screen", "#000000", rough=0.18, coat=0.25, emit_img=SCREEN_IMG),
        "uvf": material("UnitV front", "#2d2f33", rough=0.4),
        "uvb": material("UnitV back", "#c3c6cc", rough=0.45),
        "cell": material("LiPo pouch", "#9aa3b2", rough=0.35, metal=0.5),
        "pcb": material("PCB", "#1f3d2c", rough=0.4),
        "floor": material("floor", "#d9d7d3", rough=0.9),
    }


def load(path, mat, rig, offset=(0, 0, 0), matrix=None):
    bpy.ops.wm.stl_import(filepath=path, global_scale=MM)
    o = bpy.context.selected_objects[0]
    o.data.materials.append(mat)
    bpy.context.view_layer.objects.active = o
    try:
        bpy.ops.object.shade_smooth_by_angle(angle=math.radians(32))
    except Exception:
        pass
    if matrix is not None:
        o.matrix_world = matrix @ o.matrix_world   # keep the import's mm->m scale
    o.location = Vector(o.location) + Vector([(offset[i] - CENTER[i]) * MM for i in range(3)])
    o.parent = rig
    return o


def unitv_matrix():
    # UnitV frame (x, y, z) -> device (80.8 - x, 52.0 - z, -1.6 - y), lens end up
    from mathutils import Matrix
    m = Matrix(((-1, 0, 0, 80.8 * MM), (0, 0, -1, 52.0 * MM), (0, -1, 0, -1.6 * MM), (0, 0, 0, 1)))
    return m


def screen_plane(mat, rig, dz=0.0):
    bpy.ops.mesh.primitive_plane_add(size=1)
    o = bpy.context.active_object
    o.scale = (42.2 * MM, 22.4 * MM, 1)
    o.location = Vector([(42.0 - CENTER[0]) * MM, (39.9 - CENTER[1]) * MM, (22.96 + dz - CENTER[2]) * MM])
    o.data.materials.append(mat)
    o.parent = rig
    return o


def sweep(mat, z0):
    """Seamless studio sweep: floor curving up into a back wall at +Y."""
    import bmesh
    prof = [(y, z0) for y in (-3.0, -0.6, 0.0, 0.25)]
    r = 0.45
    for i in range(1, 17):
        a = math.radians(90 * i / 16)
        prof.append((0.25 + r * math.sin(a), z0 + r * (1 - math.cos(a))))
    prof += [(0.25 + r, z0 + r + 0.6), (0.25 + r, z0 + 3.0)]
    bm = bmesh.new()
    rows = [[bm.verts.new((x, y, z)) for (y, z) in prof] for x in (-3.0, 3.0)]
    for i in range(len(prof) - 1):
        bm.faces.new((rows[0][i], rows[1][i], rows[1][i + 1], rows[0][i + 1]))
    me = bpy.data.meshes.new("sweep")
    bm.to_mesh(me)
    o = bpy.data.objects.new("sweep", me)
    bpy.context.scene.collection.objects.link(o)
    for p in me.polygons:
        p.use_smooth = True
    me.materials.append(mat)


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
    sc.render.resolution_x, sc.render.resolution_y = 1600, 1200
    try:
        sc.view_settings.view_transform = 'AgX'
        sc.view_settings.look = 'AgX - Medium High Contrast'
        sc.view_settings.exposure = -0.7
    except Exception:
        pass
    w = bpy.data.worlds.new("w"); sc.world = w; w.use_nodes = True
    bg = [n for n in w.node_tree.nodes if n.type == 'BACKGROUND'][0]
    bg.inputs[0].default_value = srgb("#d9d7d3")
    bg.inputs[1].default_value = 0.12
    return sc


def light(name, loc, size, power, target=(0, 0, 0.03)):
    l = bpy.data.lights.new(name, 'AREA')
    l.size, l.energy = size, power
    o = bpy.data.objects.new(name, l)
    bpy.context.scene.collection.objects.link(o)
    o.location = loc
    o.rotation_euler = (Vector(target) - Vector(loc)).to_track_quat('-Z', 'Y').to_euler()


def camera(loc, target, lens=85):
    sc = bpy.context.scene
    cam = sc.camera or bpy.data.objects.new("cam", bpy.data.cameras.new("cam"))
    if cam.name not in sc.collection.objects:
        sc.collection.objects.link(cam)
    sc.camera = cam
    cam.data.lens = lens
    cam.data.clip_start, cam.data.clip_end = 0.01, 20
    cam.location = loc
    cam.rotation_euler = (Vector(target) - Vector(loc)).to_track_quat('-Z', 'Y').to_euler()


def assemble(mats, variant, explode=False):
    """Revision 3: everything black PLA; the frozen chassis and plunger are white PETG."""
    rig = bpy.data.objects.new("rig", None)
    bpy.context.scene.collection.objects.link(rig)
    e = (lambda dz, dx=0: (dx, 0, dz)) if explode else (lambda dz, dx=0: (0, 0, 0))
    tag = {}
    tag["Frame (bezel)"] = load(os.path.join(STL, "frame.stl"), mats["black"], rig, e(34))
    tag["CardputerZero"] = load(os.path.join(REF, "cz_front.stl"), mats["dev"], rig, e(0))
    load(os.path.join(REF, "cz_back.stl"), mats["devback"], rig, e(0))
    load(os.path.join(STL, "keys.stl"), mats["key"], rig, e(0))
    screen_plane(mats["screen"], rig)
    load(os.path.join(STL, "glass_imx.stl"), mats["glass"], rig, e(0))
    tag["Chassis"] = load(os.path.join(STL, "chassis.stl"), mats["petg"], rig, e(-18))
    if variant == "camera":
        load(os.path.join(STL, "lenses.stl"), mats["barrel"], rig, e(-42))
        load(os.path.join(STL, "glass_k210.stl"), mats["glass"], rig, e(-42))
        um = unitv_matrix()
        for f, m in (("uv_front.stl", "uvf"), ("uv_back.stl", "uvb")):
            tag["UnitV K210"] = load(os.path.join(REF, f), mats[m], rig, e(-42), matrix=um)
        tag["UnitV pins"] = load(os.path.join(STL, "pins.stl"), mats["black"], rig, e(-56))
        tag["Plunger"] = load(os.path.join(STL, "plunger.stl"), mats["petg"], rig, e(-62))
        tag["Camera backplate"] = load(os.path.join(STL, "camera_back.stl"), mats["black"], rig, e(-80))
        tag["Lens ring"] = load(os.path.join(STL, "lens_ring.stl"), mats["black"], rig, e(-104))
        tag["Link cover"] = load(os.path.join(STL, "link.stl"), mats["black"], rig, e(-80, 30))
        load(os.path.join(STL, "screw_heads.stl"), mats["screw"], rig, e(-98))
    else:
        tag["Plain backplate"] = load(os.path.join(STL, "plain_back.stl"), mats["black"], rig, e(-48))
        load(os.path.join(STL, "screw_heads.stl"), mats["screw"], rig, e(-62))
    return rig, tag


VIEWS_DEF = {
    #  name: (rig yaw deg, camera loc, target, lens)
    "front": (0, (0.0, -0.40, 0.045), (0, 0, 0.030), 88),
    "back": (180, (0.0, -0.40, 0.045), (0, 0, 0.030), 88),
    "side": (-90, (0.0, -0.40, 0.040), (0, 0, 0.030), 88),
    "three-quarter": (-34, (-0.04, -0.38, 0.105), (0, 0, 0.027), 84),
    "back-three-quarter": (180 + 36, (0.04, -0.38, 0.12), (0, 0, 0.027), 84),
    "exploded": (-60, (0.0, -0.50, 0.27), (0.030, 0.017, 0.03), 46),
    "exploded-plain": (-60, (-0.01, -0.44, 0.24), (0.012, 0.007, 0.03), 50),
}


def main():
    os.makedirs(OUT, exist_ok=True)
    for view in VIEWS:
        sc = scene_setup()
        mats = build_materials()
        rig, tag = assemble(mats, VARIANT, explode=(view == "exploded"))
        key = "exploded-plain" if (view == "exploded" and VARIANT == "plain") else view
        yaw, cam_loc, tgt, lens = VIEWS_DEF[key]
        rig.rotation_euler = (math.radians(90), 0, math.radians(yaw))
        rig.location = (0, 0, (CENTER[1] + 2.25) * MM)
        sweep(mats["floor"], -0.10 if view == "exploded" else 0.0)
        # one key light above-left (Bezel's single light), soft fill, cool rim
        light("key", (-0.45, -0.40, 0.55), 0.45, 32)
        light("fill", (0.55, -0.35, 0.15), 0.7, 7)
        light("rim", (0.25, 0.45, 0.40), 0.35, 18)
        camera(cam_loc, tgt, lens)
        name = f"{VARIANT}-{view}.png"
        sc.render.filepath = os.path.join(OUT, name)
        bpy.ops.render.render(write_still=True)
        if view == "exploded":   # label anchors for tools/finish.py
            bpy.context.view_layer.update()
            anchors = {}
            for label, o in tag.items():
                c = sum((o.matrix_world @ Vector(b) for b in o.bound_box), Vector()) / 8
                p = world_to_camera_view(sc, sc.camera, c)
                anchors[label] = (p.x, 1 - p.y)
            with open(os.path.join(OUT, name + ".labels.json"), "w") as f:
                json.dump(anchors, f, indent=1)


main()

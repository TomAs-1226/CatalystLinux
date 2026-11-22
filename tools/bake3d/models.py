"""Catalyst's robot in three dimensions, and what it is rendered as.

Only the Robot app's robot is 3D: everything else in the shell is flat.
The built-in robot is Numbers, FRC 5805's, from the team website's own
model (fetched by numbers.sh); a team's own robot is made by robot.sh from
its CAD. Front faces -y (toward the camera at rest); z is up; the size does
not matter (bake.py scales every object to the same size).

What a surface is made of, not its colour: "accent" takes the signal (the
bumpers, unless the alliance paints them), "second" takes the object's
second tint (for Numbers its own blue), "dark" is glass and screens,
"graphite" dark plastic, "body" and "white" bare aluminium and printed
parts, "metal" bare metal.
"""

import json
import math
import os

from kit import box, empty, tag

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))

# Views. The robot turns slowly on its stage, two views blended for the
# angles between: 4 degrees apart, so an edge moves 3 px from one to the
# next and the blend never shows as a double edge. The previews' whole turn
# is rendered apart, every 15 degrees.
TURN = list(range(0, 360, 15))
SPIN = list(range(0, 360, 4))
ROBOT = dict(sizes=[["hero", 112, SPIN, [0]]], res=300, frame_shadow=False, margin=0.03)


# ================================================================ numbers ===

NUMBERS_DIR = os.path.join(HERE, "numbers")
NUMBERS_PARTS = ["drivetrain", "intake", "hopper", "shooter", "flywheel", "hood", "bumpers", "frame"]

# Which part each of the model's named nodes is (everything under a node
# goes with it): the website's own names (robot.json, robotRig.js).
NUMBERS_NODES = {
    "module-fl": "drivetrain", "module-fr": "drivetrain", "module-bl": "drivetrain", "module-br": "drivetrain",
    "intake": "intake", "assembly-intake": "intake",
    "assembly-hopper": "hopper", "assembly-floor": "hopper",
    "assembly-shooter": "shooter",
    "roller-flywheel": "flywheel", "roller-flywheel-inertia": "flywheel",
    "hood": "hood",
    "assembly-drive": "frame", "static": "frame",
}

# The model's materials, as what each surface is made of. Its blue anodised
# aluminium is the robot's own colour (`second`, the tint the pack carries
# as Numbers' blue); bare and gold aluminium and printed parts are light;
# belts, motors, plastics and dark electronics are graphite. Polycarbonate
# is left out: it is clear, and what it holds shows through.
NUMBERS_MATERIALS = {
    "aluminium": "second", "aluminium.001": "body", "aluminium.002": "white",
    "belt": "graphite", "black": "graphite", "print": "body", "print.001": "graphite", "print.002": "graphite",
    "motor": "graphite", "motor.001": "metal", "steel": "graphite", "steel.001": "metal",
    "electronics": "soft", "electronics.001": "graphite", "electronics.002": "dark", "electronics.003": "white",
    "electronics.004": "soft", "electronics.005": "dark",
    "other": "soft", "tread": "soft", "tread.001": "rubber",
}

# Its mechanisms as baked, each in the robot code's own units (the steps the
# shell shows them at; between two it blends, only while one moves): the
# intake's carriage in Deploy/LengthInches (stowed 5.0, out 11.8)
# and the hood in Hood/AngleDegrees (11 is its stop, the CAD's pose, and
# idle 13 shows as it; 25 for a shot from the hub's near side, 40 from far
# off or feeding). Each step costs a patch in every view: these are the
# ones worth showing apart.
INTAKE_STEPS = [5.0, 11.8]
HOOD_STEPS = [11.0, 25.0, 40.0]


def numbers_offset(length_inches):
    """The intake carriage's slide out from the CAD's pose, in metres, for
    a Deploy length: a linear share of the stop-to-stop travel (4.85 to
    13.203 inches in the code, 0.3023 m in the CAD)."""
    return max(0.0, (length_inches - 4.85) / (13.203 - 4.85) * 0.3023)


def numbers(folder=NUMBERS_DIR):
    """Numbers, FRC 5805's robot, from the website's model and manifest
    (robot.glb, robot.json; numbers.sh fetches them): its parts named, its
    bumpers added (the CAD has none; the manifest gives their size), and
    its intake and hood posed as the shell asks: a spec for bake.py."""
    model = os.path.join(folder, "robot.glb")
    with open(os.path.join(folder, "robot.json"), encoding="utf-8") as f:
        manifest = json.load(f)
    nodes = {}

    def build():
        import bmesh as bm
        import bpy
        import kit
        before = set(bpy.data.objects)
        bpy.ops.import_scene.gltf(filepath=model)
        new = [o for o in bpy.data.objects if o not in before]
        # The model's x is forward: turned so its front (the intake) faces -y.
        root = empty("numbers", (0, 0, 0), (0, 0, -90))
        for o in new:
            if o.parent is None:
                o.parent = root
        for o in new:
            if o.name in NUMBERS_NODES:
                tag(o, NUMBERS_NODES[o.name])
            nodes[o.name] = o
        for o in new:
            if o.type != "MESH":
                continue
            # Clear polycarbonate out, and every surface by what it is made of.
            mats = [s.material.name if s.material else "" for s in o.material_slots]
            mesh = bm.new()
            mesh.from_mesh(o.data)
            gone = [f for f in mesh.faces if f.material_index < len(mats) and mats[f.material_index].startswith("poly")]
            bm.ops.delete(mesh, geom=gone, context="FACES")
            mesh.to_mesh(o.data)
            mesh.free()
            for slot in o.material_slots:
                kind = NUMBERS_MATERIALS.get(slot.material.name if slot.material else "", "body")
                slot.material = kit.material(kind)
            for p in o.data.polygons:
                p.use_smooth = True
        # Bumpers all round, as the manifest has them (the CAD has none): in
        # the model's metres, x forward, z right, on its own floor.
        b = manifest.get("bumpers", {})
        thick, height, bottom = b.get("thickness", 0.0825), b.get("height", 0.127), b.get("bottom", 0.019)
        length, width = b.get("length", 0.8509), b.get("width", 0.889)
        bumpers = tag(empty("bumpers", parent=root), "bumpers")
        zc = bottom + height / 2
        r = min(thick, height) * 0.22
        for side in (-1, 1):
            # Front and back (across), then the sides (along), in Blender's
            # axes before the turn: x forward, y left.
            box((thick, width, height), (side * (length - thick) / 2, 0, zc), r=r, mat="accent", parent=bumpers, segments=4)
            box((length - 2 * thick + 0.002, thick, height), (0, side * (width - thick) / 2, zc), r=r, mat="accent", parent=bumpers, segments=4)
        bpy.context.view_layer.update()
        # The rest pose of what moves, kept to pose from.
        rest = {n: (nodes[n].location.copy(), nodes[n].rotation_quaternion.copy()) for n in ("intake", "hood") if n in nodes}
        for n in ("intake", "hood"):
            nodes[n].rotation_mode = "QUATERNION"
        nodes["_rest"] = rest
        return root

    intake = manifest.get("intake", {})
    axis = intake.get("axis", [0.9954, -0.0956, 0.0])
    # glTF (x forward, y up, z right) to Blender's (x, -z, y).
    axis_b = (axis[0], -axis[2], axis[1])
    hood = manifest.get("hood", {})
    cad_hood = hood.get("cadAngle", 11.0)

    def set_intake(length_inches):
        # The model rests with the carriage at its inner stop.
        loc, _ = nodes["_rest"]["intake"]
        d = numbers_offset(length_inches)
        nodes["intake"].location = (loc[0] + axis_b[0] * d, loc[1] + axis_b[1] * d, loc[2] + axis_b[2] * d)

    def set_hood(degrees):
        from mathutils import Quaternion
        _, q = nodes["_rest"]["hood"]
        # About the glTF's +z (Blender's -y): positive lifts the hood's front.
        nodes["hood"].rotation_quaternion = Quaternion((0.0, -1.0, 0.0), math.radians(degrees - cad_hood)) @ q

    mechanisms = [
        dict(name="intake", steps=INTAKE_STEPS, set=set_intake),
        dict(name="hood", steps=HOOD_STEPS, set=set_hood),
    ]
    return dict(build=build, **ROBOT, parts=NUMBERS_PARTS, rest_part="frame", tints=["signal", "own"], own="#0057b8",
                mechanisms=mechanisms, source="team website (smblyrequired) robot.glb",
                about="Numbers, FRC 5805's robot: the Robot app's own, its intake and hood posed as the robot reports them")


# ============================================================ team robot ===

def team_robot(model, mapping):
    """A team's own robot from its CAD (glTF/GLB, or OBJ from step2obj.py;
    STL, FBX and PLY too), its parts named by the mapping file (see
    robot.sh): a spec for bake.py.

    A real assembly is large: its fasteners and specks (anything under 1.2%
    of the robot across, and whatever `hide` names) are left out, and the
    rest reduced to about 250,000 triangles in all (each solid in proportion,
    small ones kept whole): plenty for a 112-pixel picture."""
    rules, turn, up, hide = read_mapping(mapping)
    parts = [p for p, _ in rules]
    if "frame" not in parts:
        parts.append("frame")

    def build():
        import fnmatch

        import bmesh as bm
        import bpy
        import kit
        from mathutils import Vector
        before = set(bpy.data.objects)
        ext = os.path.splitext(model)[1].lower()
        axis = "Y" if up == "y" else "Z"
        if ext in (".glb", ".gltf"):
            bpy.ops.import_scene.gltf(filepath=model)
        elif ext == ".obj":
            bpy.ops.wm.obj_import(filepath=model, up_axis=axis)
        elif ext == ".stl":
            bpy.ops.wm.stl_import(filepath=model, up_axis=axis)
        elif ext == ".ply":
            bpy.ops.wm.ply_import(filepath=model, up_axis=axis)
        elif ext == ".fbx":
            bpy.ops.import_scene.fbx(filepath=model)
        else:
            raise SystemExit(f"robot.sh: {ext} is not a model this reads: export glTF (.glb) from Onshape, or STEP")
        new = [o for o in bpy.data.objects if o not in before]
        root = empty("team-robot", (0, 0, 0), (0, 0, turn))
        for o in new:
            if o.parent is None:
                o.parent = root
        bpy.context.view_layer.update()
        meshes = [o for o in new if o.type == "MESH"]

        def levels_of(o):
            # The names it sits under, outermost first, down to its own, as
            # CAD wrote them. A STEP's part comes as one path (step2obj.py
            # writes gmsh's "Shapes/Robot/Intake/Roller"): each step of it is
            # a level, gmsh's own "Shapes" left off.
            levels = []
            a = o
            while a is not None and a is not root:
                names = [a.name.lower()] + ([a.data.name.lower()] if a.type == "MESH" and a.data else [])
                steps = [s for s in names[0].split("/") if s]
                if len(steps) > 1 and steps[0] == "shapes":
                    steps = steps[1:]
                levels += [[s] for s in reversed(steps)] if len(steps) > 1 else [names]
                a = a.parent
            return levels[::-1]

        # What matches nothing: the part a bare `*` is for, else the frame.
        fallback = next((p for p, globs in rules if "*" in globs), "frame")

        def part_for(o):
            # The outermost assembly that a line matches decides, so a "Drive
            # Pulley" in the Intake is the intake's. The robot's own top
            # assembly is passed over: its name ("Warmonger") is the whole
            # robot's, not a part's. Among lines matching one name, the first
            # wins; what nothing matches is the fallback.
            levels = levels_of(o)
            for names in levels[1:] if len(levels) > 1 else levels:
                for p, globs in rules:
                    if any(g != "*" and fnmatch.fnmatch(n, g) for g in globs for n in names):
                        return p
            return fallback

        def size_of(o):
            corners = [o.matrix_world @ Vector(c) for c in o.bound_box]
            lo = Vector((min(c.x for c in corners), min(c.y for c in corners), min(c.z for c in corners)))
            hi = Vector((max(c.x for c in corners), max(c.y for c in corners), max(c.z for c in corners)))
            return lo, hi

        boxes = {o.name: size_of(o) for o in meshes}
        lo = Vector((min(b[0].x for b in boxes.values()), min(b[0].y for b in boxes.values()), min(b[0].z for b in boxes.values())))
        hi = Vector((max(b[1].x for b in boxes.values()), max(b[1].y for b in boxes.values()), max(b[1].z for b in boxes.values())))
        whole = (hi - lo).length
        gone = 0
        for o in list(meshes):
            b = boxes[o.name]
            # `hide` is for fasteners and the like: matched against the
            # part's own name, never its assembly's (a "Drivetrain" holds
            # its screws, and is not one).
            own = [o.name.lower()] + ([o.data.name.lower()] if o.data else [])
            if (b[1] - b[0]).length < whole * 0.012 or any(fnmatch.fnmatch(n, g) for g in hide for n in own):
                meshes.remove(o)
                bpy.data.objects.remove(o, do_unlink=True)
                gone += 1
        faces = sum(len(o.data.polygons) for o in meshes)
        ratio = min(1.0, 250_000 / max(faces, 1))
        counts = {}
        for o in meshes:
            part = part_for(o)
            o["part"] = part
            counts[part] = counts.get(part, 0) + 1
            # The CAD's own colours, as our lightness: what each surface is made of.
            lightness = []
            for slot in o.material_slots:
                m = slot.material
                c = None
                if m and m.use_nodes:
                    bsdf = next((n for n in m.node_tree.nodes if n.type == "BSDF_PRINCIPLED"), None)
                    if bsdf:
                        c = bsdf.inputs["Base Color"].default_value
                elif m:
                    c = m.diffuse_color
                lightness.append(0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2] if c is not None else 0.5)
            mat = "body"
            if lightness:
                v = sum(lightness) / len(lightness)
                mat = "white" if v > 0.55 else "body" if v > 0.22 else "soft" if v > 0.08 else "graphite"
            if part == "bumpers":
                mat = "accent"
            # Faces turned one way (a STEP's surfaces may not agree), smooth.
            mesh = bm.new()
            mesh.from_mesh(o.data)
            bm.ops.recalc_face_normals(mesh, faces=mesh.faces)
            mesh.to_mesh(o.data)
            mesh.free()
            o.data.materials.clear()
            o.data.materials.append(kit.material(mat))
            for poly in o.data.polygons:
                poly.use_smooth = True
            if ratio < 1.0 and len(o.data.polygons) > 400:
                mod = o.modifiers.new("reduce", "DECIMATE")
                mod.ratio = ratio
        print(f"robot.sh: {len(meshes)} solids kept, {gone} small ones or hidden left out, {faces} faces reduced by {ratio:.2f}", flush=True)
        print("robot.sh: parts:", ", ".join(f"{p} {counts.get(p, 0)}" for p in parts), flush=True)
        return root

    return dict(build=build, parts=parts, rest_part="frame", **ROBOT, tints=["signal", "ice"], source="team", about="The team's own robot")


def read_mapping(path):
    """A mapping file: `part = glob glob …` lines (matched against the CAD's
    part and assembly names, ignoring case; the first that matches wins),
    `hide = glob …` for what to leave out, `turn = degrees` to bring the
    robot's front round to face the camera, `up = y` or `z` (OBJ, STL)."""
    rules, turn, up, hide = [], 0.0, "z", []
    with open(path, encoding="utf-8") as f:
        for line in f:
            line = line.split("#", 1)[0].strip()
            if "=" not in line:
                continue
            key, value = (s.strip() for s in line.split("=", 1))
            if key == "turn":
                turn = float(value)
            elif key == "up":
                up = value.lower()
            elif key == "hide":
                hide += [g.lower() for g in value.split()]
            else:
                rules.append((key, [g.lower() for g in value.split()]))
    return rules, turn, up, hide


# What bake.py makes, each by a function giving its spec (Numbers reads its
# model only when it is baked).
ASSETS = {
    # The Robot app's own robot: Numbers, FRC 5805's.
    "robot": numbers,
}

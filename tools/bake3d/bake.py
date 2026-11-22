"""Render Catalyst's 3D objects: every view of every object, as the neutral
lit pass and the data pass (see kit.py), into a folder for pack.py.

    blender -b --factory-startup -P tools/bake3d/bake.py -- OUT [names...]

With no names, every object in models.ASSETS. The same Blender (5.2 LTS),
the same seed and the same samples give the same pictures: the renders are
reproducible, and pack.py turns them into what the shell carries.

The light is Bezel's one light, resting above and to the left: an area
light fixed to the camera (so it stays above-left in every view, however
the object turns), large enough for soft shadows, over a soft sky that
gives the ambient occlusion. A shadow catcher under the object keeps its
contact shadow, which the light throws down and to the right.
"""

import itertools
import json
import math
import os
import sys
import time

import bpy
from bpy_extras.object_utils import world_to_camera_view
from mathutils import Vector

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import kit  # noqa: E402
import models  # noqa: E402

# Where the light is, seen from the object, in the camera's own axes (right,
# up, toward the camera): above and to the left, a little in front.
LIGHT = Vector((-0.55, 0.62, 0.56)).normalized()
LENS = 70.0
SAMPLES = 96
DATA_SAMPLES = 24


def camera_pose(cam, target, elevation, distance):
    e = math.radians(elevation)
    back = Vector((0, -math.cos(e), math.sin(e)))
    cam.location = target + back * distance
    cam.rotation_euler = (math.radians(90 - elevation), 0, 0)
    return back


def light_pose(light, target, elevation, distance):
    e = math.radians(elevation)
    right = Vector((1, 0, 0))
    back = Vector((0, -math.cos(e), math.sin(e)))
    up = back.cross(right).normalized()
    d = (right * LIGHT.x + up * LIGHT.y + back * LIGHT.z).normalized()
    light.location = target + d * distance
    # Point it at the target: an area light shines down its own -z.
    light.rotation_euler = (-d).to_track_quat("-Z", "Y").to_euler()
    return d


def render(scene, path, samples, denoise):
    scene.cycles.samples = samples
    scene.cycles.use_denoising = denoise
    scene.render.filepath = path
    bpy.ops.render.render(write_still=True)


def bake(name, spec, out):
    scene = kit.new_scene()
    kit.world(scene)
    root = spec["build"]()
    pivot = kit.normalise(root, 2.0)
    objs = [o for o in bpy.data.objects if o.type not in ("EMPTY",)]
    res = spec.get("res", 240)
    scene.render.resolution_x = scene.render.resolution_y = res
    scene.render.resolution_percentage = 100

    # The ground: a shadow catcher just under the object.
    bpy.ops.mesh.primitive_plane_add(size=40, location=(0, 0, 0.0))
    ground = bpy.context.active_object
    ground.is_shadow_catcher = True
    ground.data.materials.append(kit.material("body"))

    cam_data = bpy.data.cameras.new("cam")
    cam_data.lens = LENS
    cam = bpy.data.objects.new("cam", cam_data)
    scene.collection.objects.link(cam)
    scene.camera = cam
    light_data = bpy.data.lights.new("key", "AREA")
    light_data.shape = "DISK"
    light_data.size = 3.2
    light_data.energy = spec.get("light", 180.0)
    light = bpy.data.objects.new("key", light_data)
    scene.collection.objects.link(light)

    bpy.context.view_layer.update()
    height = max(p.z for p in kit.evaluated_points(objs))
    target = Vector((0, 0, height * spec.get("aim", 0.45)))
    distance = 9.0

    rest = spec.get("rest", -28.0)
    elevation = spec.get("elevation", 24.0)
    # What ships: every view any size needs. The whole turn is rendered
    # apart, framed for itself, for the previews' turntables.
    shipped = sorted({(y, p) for _, _, yaws, pitches in spec["sizes"] for p in pitches for y in yaws})
    turn = [(y, 0) for y in models.TURN]
    if QUICK:
        shipped = [v for v in shipped if v in ((0, 0), (-48, 0), (48, 0), (0, -12), (0, 12), (44, 0), (88, 0), (180, 0), (272, 0))]
        turn = [(0, 0), (90, 0), (180, 0)]

    def pose_view(yaw, pitch):
        pivot.rotation_euler = (0, 0, math.radians(rest + yaw))
        camera_pose(cam, target, elevation + pitch, distance)
        light_pose(light, target, elevation + pitch, 6.0)
        bpy.context.view_layer.update()

    # Framing: one lens and shift for a set of views, so the object never
    # jumps between frames, as large as the union of every view (and its
    # shadow) allows.
    def bounds(views):
        xs, ys = [], []
        for yaw, pitch in views[:: max(1, len(views) // 24)] + [views[-1]]:
            pose_view(yaw, pitch)
            pts = kit.evaluated_points(objs)
            d = (light.location - target).normalized()
            # Its shadow too, unless it may run off the frame (it fades out
            # before the edge: object3d.rs), so a turning robot fills more of it.
            shadow = [p - d * (p.z / d.z) for p in pts[::3]] if d.z > 0.05 and spec.get("frame_shadow", True) else []
            for p in pts + shadow:
                v = world_to_camera_view(scene, cam, p)
                xs.append(v.x)
                ys.append(v.y)
        return min(xs), max(xs), min(ys), max(ys)

    margin = spec.get("margin", 0.05)

    def frame(views):
        cam_data.lens, cam_data.shift_x, cam_data.shift_y = LENS, 0.0, 0.0
        for _ in range(3):
            x0, x1, y0, y1 = bounds(views)
            cam_data.lens /= max(x1 - x0, y1 - y0) / (1 - 2 * margin)
            x0, x1, y0, y1 = bounds(views)
            cam_data.shift_x += (x0 + x1) / 2 - 0.5
            cam_data.shift_y += (y0 + y1) / 2 - 0.5
        return cam_data.lens, cam_data.shift_x, cam_data.shift_y

    # Its mechanisms (Numbers' intake and hood): each pose is a step of
    # each, the first all at their first (the rest pose, which the turn and
    # the parts' facing use). Every pose is rendered at every shipped view;
    # pack.py keeps, for each pose, only what differs from the rest pose.
    mechanisms = spec.get("mechanisms", [])
    poses = list(itertools.product(*[range(len(m["steps"])) for m in mechanisms]))

    def strike(pose):
        for m, i in zip(mechanisms, pose):
            m["set"](m["steps"][i])
        bpy.context.view_layer.update()

    def tag_of(pose):
        return "" if not any(pose) else "@" + "".join(str(i) for i in pose)

    # Framed with every mechanism at its farthest, so no pose leaves the frame.
    if mechanisms:
        strike(tuple(len(m["steps"]) - 1 for m in mechanisms))
    framing = {"": frame(shipped), "turn_": frame(turn)}
    if mechanisms:
        strike(poses[0])
    sets = {"": shipped, "turn_": turn}

    def each(kind, samples, denoise):
        """Render every view of every pose (the turn at rest only)."""
        for prefix, views in sets.items():
            use(prefix)
            for pose in poses if prefix == "" else poses[:1]:
                if mechanisms:
                    strike(pose)
                for yaw, pitch in views:
                    pose_view(yaw, pitch)
                    render(scene, os.path.join(folder, f"{prefix}{kind}_{yaw:+04d}_{pitch:+03d}{tag_of(pose)}.png"), samples, denoise)
        if mechanisms:
            strike(poses[0])

    def use(prefix):
        cam_data.lens, cam_data.shift_x, cam_data.shift_y = framing[prefix]

    # Where the object stands (the middle of its footprint) in the rest view,
    # as a fraction of the frame from the top left: the shell sets objects
    # down on a surface by it.
    use("")
    pose_view(0, 0)
    foot = world_to_camera_view(scene, cam, Vector((0, 0, 0)))
    middle = world_to_camera_view(scene, cam, target)

    folder = os.path.join(out, name)
    os.makedirs(folder, exist_ok=True)
    started = time.time()
    each("lit", SAMPLES, True)

    # The data pass: flat tint and coverage, no shadow, no light needed.
    ground.hide_render = True
    scene.world.node_tree.nodes["Background"].inputs["Strength"].default_value = 0.0
    scene.cycles.max_bounces = 0
    for o in objs:
        for slot in o.material_slots:
            if slot.material and "data" in slot.material:
                slot.material = bpy.data.materials[slot.material["data"]]
        if o.type in ("CURVE", "FONT"):
            mats = o.data.materials
            for i, m in enumerate(mats):
                if m and "data" in m:
                    mats[i] = bpy.data.materials[m["data"]]
    each("data", DATA_SAMPLES, False)

    # The parts pass, for an object whose parts the shell colours one by one
    # (a robot's drivetrain, intake, arm...): each part's number as flat
    # emission, one sample a pixel and no filter, so no pixel mixes two
    # numbers; pack.py brings it down by the commonest number under each
    # pixel. And each part's facing: the turn that brings it nearest the camera.
    parts = spec.get("parts", [])
    facing = []
    if parts:
        pose_view(0, 0)
        owner = {o.name: kit.part_of(o) for o in objs}
        centroids = {}
        for o in objs:
            p = owner[o.name]
            if p in parts:
                for v in kit.evaluated_points([o])[::7]:
                    centroids.setdefault(p, []).append(v)
        for p in parts:
            pts = centroids.get(p, [])
            if not pts:
                facing.append(0)
                continue
            cx = sum(v.x for v in pts) / len(pts)
            cy = sum(v.y for v in pts) / len(pts)
            # Seen from the camera at -y: a part toward -y faces it. Its
            # angle about the pivot now (at rest), and the turn on from rest
            # that brings it round to the front.
            a = math.degrees(math.atan2(cy, cx))
            turn_to = (-90.0 - a + 180.0) % 360.0 - 180.0
            facing.append(0 if math.hypot(cx, cy) < 0.15 else int(round(turn_to)))
        for o in objs:
            p = owner[o.name]
            index = parts.index(p) + 1 if p in parts else (1 + parts.index(spec.get("rest_part", parts[-1])))
            m = kit.part_material(index)
            if o.type in ("CURVE", "FONT"):
                for i in range(len(o.data.materials)):
                    o.data.materials[i] = m
            for slot in o.material_slots:
                slot.material = m
        scene.cycles.filter_width = 0.01
        each("part", 1, False)

    meta = {
        "name": name,
        "res": res,
        "turn": [y for y, _ in turn],
        "sizes": spec["sizes"],
        "tints": spec.get("tints", ["signal", "ice"]),
        "foot": [foot.x, 1 - foot.y],
        "middle": [middle.x, 1 - middle.y],
        "source": spec.get("source", "code"),
        "about": spec.get("about", ""),
        "parts": parts,
        "facing": facing,
        "own": spec.get("own"),
        "mechanisms": [dict(name=m["name"], steps=m["steps"]) for m in mechanisms],
    }
    with open(os.path.join(folder, "meta.json"), "w") as f:
        json.dump(meta, f, indent=1)
    print(f"bake3d: {name}: {len(shipped) * len(poses) + len(turn)} views ({len(poses)} poses) in {time.time() - started:.1f} s", flush=True)


QUICK = False


def main():
    global QUICK
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    if "--quick" in argv:
        argv.remove("--quick")
        QUICK = True
    # Whole paths: Blender reads a relative one against the drive's root.
    out = os.path.abspath(argv[0])
    # A team's own robot, from its CAD: --robot MODEL MAP (see robot.sh).
    if "--robot" in argv:
        i = argv.index("--robot")
        model, mapping = os.path.abspath(argv[i + 1]), os.path.abspath(argv[i + 2])
        bake("robot-team", models.team_robot(model, mapping), out)
        return
    names = argv[1:] or list(models.ASSETS)
    for name in names:
        bake(name, models.ASSETS[name](), out)


main()

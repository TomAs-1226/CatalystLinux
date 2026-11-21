"""A first look at a generated model as it came: its size, its materials and
four views with its own textures, before it is made into one of ours.

    blender -b --factory-startup -P tools/bake3d/glb_look.py -- MODEL.glb OUT.png
"""

import math
import os
import sys

import bpy
from mathutils import Vector

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import kit  # noqa: E402

argv = sys.argv[sys.argv.index("--") + 1:]
path, out = argv[0], argv[1]
scene = kit.new_scene()
kit.world(scene)
bpy.ops.import_scene.gltf(filepath=path)
meshes = [o for o in bpy.data.objects if o.type == "MESH"]
tris = sum(len(o.data.polygons) for o in meshes)
print(f"glb_look: {len(meshes)} meshes, {tris} faces, materials: {[m.name for m in bpy.data.materials]}")
for img in bpy.data.images:
    print(f"glb_look: image {img.name} {tuple(img.size)}")
pts = kit.evaluated_points(meshes)
lo = Vector((min(p.x for p in pts), min(p.y for p in pts), min(p.z for p in pts)))
hi = Vector((max(p.x for p in pts), max(p.y for p in pts), max(p.z for p in pts)))
print(f"glb_look: bounds {tuple(round(v, 3) for v in lo)} to {tuple(round(v, 3) for v in hi)}")
centre = (lo + hi) / 2
radius = (hi - lo).length / 2
cam_data = bpy.data.cameras.new("cam")
cam = bpy.data.objects.new("cam", cam_data)
scene.collection.objects.link(cam)
scene.camera = cam
sun = bpy.data.lights.new("sun", "SUN")
sun.energy = 3.0
so = bpy.data.objects.new("sun", sun)
so.rotation_euler = (math.radians(40), math.radians(-30), 0)
scene.collection.objects.link(so)
scene.render.resolution_x = scene.render.resolution_y = 320
scene.cycles.samples = 32
scene.render.film_transparent = False
files = []
for i, az in enumerate((0, 90, 180, 270)):
    a = math.radians(az)
    d = radius * 3.2
    cam.location = centre + Vector((math.sin(a) * d, -math.cos(a) * d, d * 0.45))
    cam.rotation_euler = (centre - cam.location).to_track_quat("-Z", "Y").to_euler()
    f = out.replace(".png", f"-{i}.png")
    scene.render.filepath = f
    bpy.ops.render.render(write_still=True)
    files.append(f)
print("glb_look: done")

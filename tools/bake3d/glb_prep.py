"""Make a generated model one of ours: reduced to what a 96-pixel picture
needs, turned to face the camera, and its textures read into what our
materials need, kept in the model's own vertex colours:

* red: the neutral lightness of the surface there (linear);
* green: how much it is the accent (the model's orange: it takes the signal).

    blender -b --factory-startup -P tools/bake3d/glb_prep.py -- RAW.glb OUT.glb TURN FACES

TURN is the degrees about the vertical that bring its front to face the
camera; FACES what it is reduced to. The result is small enough to keep in
the repository (Draco-compressed), so the bake needs nothing downloaded.
"""

import math
import os
import sys

import bmesh
import bpy
import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import kit  # noqa: E402

argv = sys.argv[sys.argv.index("--") + 1:]
raw, out, turn, faces = argv[0], argv[1], float(argv[2]), int(argv[3])

kit.new_scene()
bpy.ops.import_scene.gltf(filepath=raw)
obj = next(o for o in bpy.data.objects if o.type == "MESH")
bpy.context.view_layer.objects.active = obj
obj.select_set(True)
obj.rotation_euler = (0, 0, math.radians(turn))
bpy.ops.object.transform_apply(location=False, rotation=True, scale=True)

# The base colour, read once at a quarter of its size.
mat = obj.active_material
tex = next(n.image for n in mat.node_tree.nodes if n.type == "TEX_IMAGE" and n.image and n.image.name.startswith("Color"))
tex.scale(1024, 1024)
w, h = tex.size
pix = np.empty(w * h * 4, np.float32)
tex.pixels.foreach_get(pix)
pix = pix.reshape(h, w, 4)[..., :3]

# Reduce first: colours are then read for what is left.
mod = obj.modifiers.new("reduce", "DECIMATE")
mod.ratio = min(1.0, faces / len(obj.data.polygons))
bpy.ops.object.modifier_apply(modifier=mod.name)

me = obj.data
uv = me.uv_layers.active.data
n = len(me.loops)
uvs = np.empty(n * 2, np.float32)
uv.foreach_get("uv", uvs)
uvs = uvs.reshape(-1, 2)
xs = np.clip((uvs[:, 0] % 1.0) * (w - 1), 0, w - 1).astype(int)
ys = np.clip((uvs[:, 1] % 1.0) * (h - 1), 0, h - 1).astype(int)
rgb = pix[ys, xs]  # the texture's own values: sRGB-encoded

# The accent: the model's orange (and its reds and golds), saturated and lit.
mx, mn = rgb.max(axis=1), rgb.min(axis=1)
sat = np.where(mx > 0, (mx - mn) / np.maximum(mx, 1e-6), 0)
r, g, b = rgb[:, 0], rgb[:, 1], rgb[:, 2]
hue = np.degrees(np.arctan2(np.sqrt(3) * (g - b), 2 * r - g - b)) % 360
warm = np.clip(1 - np.abs(((hue + 180) % 360) - 180 - 22) / 30, 0, 1)
accent = np.clip(warm * np.clip((sat - 0.35) / 0.25, 0, 1) * np.clip((mx - 0.25) / 0.15, 0, 1), 0, 1)
lin = np.where(rgb <= 0.04045, rgb / 12.92, ((rgb + 0.055) / 1.055) ** 2.4)
lum = 0.2126 * lin[:, 0] + 0.7152 * lin[:, 1] + 0.0722 * lin[:, 2]
# Neutral lightness, kept within what our own materials span (dark glass to white).
light = np.clip(lum * 1.15, 0.03, 0.82)

# Per vertex (smaller to keep than per corner): the mean of its corners.
verts = np.empty(n, np.int32)
me.loops.foreach_get("vertex_index", verts)
count = np.bincount(verts, minlength=len(me.vertices)).astype(np.float32)
mean = lambda v: np.bincount(verts, weights=v, minlength=len(me.vertices)) / np.maximum(count, 1)
cols = np.stack([mean(light), mean(accent), np.zeros(len(me.vertices)), np.ones(len(me.vertices))], axis=1).astype(np.float32)
attr = me.color_attributes.new("neutral", "FLOAT_COLOR", "POINT")
attr.data.foreach_set("color", cols.ravel())
me.color_attributes.active_color = attr

# Drop the textures and UVs: what they said is in the colours now.
while me.uv_layers:
    me.uv_layers.remove(me.uv_layers[0])
me.materials.clear()
for p in me.polygons:
    p.use_smooth = True
bpy.ops.export_scene.gltf(
    filepath=out, export_format="GLB", use_selection=True, export_texcoords=False, export_normals=True,
    export_vertex_color="ACTIVE", export_materials="NONE", export_draco_mesh_compression_enable=True,
    export_draco_mesh_compression_level=10, export_draco_position_quantization=14, export_draco_color_quantization=8,
)
print(f"glb_prep: {os.path.basename(out)}: {len(me.polygons)} faces, accent on {float((accent > 0.5).mean()) * 100:.0f}% of corners")

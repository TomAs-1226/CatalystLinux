"""Turn bake.py's renders into what the shell carries: `objects.pack`.

    py -3 tools/bake3d/pack.py BAKED OUT.pack                 the shipped sizes of
                                                              the objects in shipped.txt
    py -3 tools/bake3d/pack.py BAKED OUT.pack --preview 96    the whole turn of
                                                              every object (app
                                                              icons at 0.6 of it),
                                                              for the previews only
    py -3 tools/bake3d/pack.py BAKED --peek OUT.png [names]   a quick look

Each view is brought down from the render's size to the size it is shown
at (an area filter in linear light, from about seven times the pixels), and
kept as four planes of a byte each:

* shade: how light the object is there, sRGB-encoded (the neutral render);
* tint: how much of it takes tint A (high four bits) and tint B (low four);
* cover: how much of the pixel the object covers;
* shadow: how dark its shadow makes what is under the rest of the pixel.

Colour is put on at run time, from the palette in use (object3d.rs, and
`colour()` below, which is the same arithmetic): so one bake serves dark and
light, every accent and the user's own colour.

The pack is little-endian: "C3DP", a version, then each object (its name,
tints, where it stands, its sizes; each size its views and, per view, where
its frame lies: a PNG whose red, green, blue and alpha are the four planes).
"""

import io
import json
import os
import struct
import sys

import numpy as np
from PIL import Image

VERSION = 1
ROLES = {"signal": 0, "ice": 1, "amber": 2, "fault": 3, "ink": 4}
# The neutral lightness each kind of surface was rendered at (kit.MATERIALS):
# a palette colour is anchored there, so a surface facing the light shows
# the palette's colour itself.
BODY_ALBEDO = 0.62
ACCENT_ALBEDO = 0.55


def srgb_to_linear(v):
    return np.where(v <= 0.04045, v / 12.92, ((v + 0.055) / 1.055) ** 2.4)


def linear_to_srgb(v):
    v = np.clip(v, 0, 1)
    return np.where(v <= 0.0031308, v * 12.92, 1.055 * np.power(v, 1 / 2.4) - 0.055)


def load(path):
    return np.asarray(Image.open(path).convert("RGBA")).astype(np.float32) / 255.0


def resize(plane, px):
    """An area filter in two steps: a box filter to twice the size, then Lanczos."""
    im = Image.fromarray(plane.astype(np.float32))
    if im.width > 2 * px:
        im = im.resize((2 * px, 2 * px), Image.Resampling.BOX)
    im = im.resize((px, px), Image.Resampling.LANCZOS)
    return np.asarray(im)


def planes(folder, yaw, pitch, px, prefix=""):
    """The four planes of one view at `px` across (`prefix` "turn_" for the
    previews' whole turn)."""
    lit = load(os.path.join(folder, f"{prefix}lit_{yaw:+04d}_{pitch:+03d}.png"))
    data = load(os.path.join(folder, f"{prefix}data_{yaw:+04d}_{pitch:+03d}.png"))
    alpha = lit[..., 3]
    cover = data[..., 3]
    shade = srgb_to_linear(lit[..., :3].mean(axis=2))
    ta = srgb_to_linear(data[..., 0])
    tb = srgb_to_linear(data[..., 1])
    # Premultiplied, so the filter weighs each by how much of the pixel it is.
    obj = resize(shade * alpha, px)
    cov = np.clip(resize(cover, px), 0, 1)
    ta = resize(ta * cover, px)
    tb = resize(tb * cover, px)
    sh = resize(np.clip(alpha - cover, 0, 1), px)
    some = cov > 1e-3
    safe = np.where(some, cov, 1)
    shade = np.where(some, obj / safe, 0)
    ta = np.where(some, ta / safe, 0)
    tb = np.where(some, tb / safe, 0)
    open_ = np.clip(1 - cov, 1e-3, 1)
    shadow = np.clip(np.where(cov < 0.999, sh / open_, 0), 0, 1)
    out = np.zeros((4, px, px), np.uint8)
    out[0] = np.round(linear_to_srgb(shade) * 255)
    tint = (np.round(np.clip(ta, 0, 1) * 15).astype(np.uint8) << 4) | np.round(np.clip(tb, 0, 1) * 15).astype(np.uint8)
    out[1] = tint
    out[2] = np.round(cov * 255)
    out[3] = np.round(shadow * 255)
    return out


def png_bytes(p):
    """One view's planes as a PNG: shade, tint, cover and shadow in its red,
    green, blue and alpha. Lossless, filtered and deflated, and the shell
    already reads PNG."""
    out = io.BytesIO()
    Image.fromarray(np.ascontiguousarray(p.transpose(1, 2, 0)), "RGBA").save(out, "PNG", optimize=True)
    return out.getvalue()


def assets(baked):
    found = []
    for name in sorted(os.listdir(baked)):
        meta = os.path.join(baked, name, "meta.json")
        if os.path.exists(meta):
            with open(meta) as f:
                found.append(json.load(f))
    return found


def write_pack(baked, out, preview=None, only=None):
    metas = [m for m in assets(baked) if not only or m["name"] in only]
    header = bytearray(b"C3DP")
    header += struct.pack("<HH", VERSION, len(metas))
    blobs = bytearray()
    entries = []
    raw = 0
    for meta in metas:
        folder = os.path.join(baked, meta["name"])
        sizes = meta["sizes"]
        prefix = ""
        if preview:
            # App icons at about their size on the carousel, the rest at a hero's.
            px = round(preview * 0.6) if meta["name"].startswith("app-") else preview
            sizes = [["preview", px, meta["turn"], [0]]]
            prefix = "turn_"
        entry = bytearray()
        name = meta["name"].encode()
        entry += struct.pack("<B", len(name)) + name
        tints = meta["tints"]
        entry += struct.pack("<BB", ROLES[tints[0]], ROLES[tints[1]])
        fx, fy = meta["foot"]
        entry += struct.pack("<HH", int(round(fx * 65535)), int(round(fy * 65535)))
        entry += struct.pack("<B", len(sizes))
        for label, px, yaws, pitches in sizes:
            label = label.encode()
            entry += struct.pack("<B", len(label)) + label
            entry += struct.pack("<HBB", px, len(yaws), len(pitches))
            entry += struct.pack(f"<{len(yaws)}h", *yaws)
            entry += struct.pack(f"<{len(pitches)}b", *pitches)
            for pitch in pitches:
                for yaw in yaws:
                    p = planes(folder, yaw, pitch, px, prefix)
                    raw += p.size
                    blob = png_bytes(p)
                    entry += struct.pack("<II", len(blobs), len(blob))
                    blobs += blob
        entries.append(entry)
        print(f"  {meta['name']}: {', '.join(f'{s[0]} {s[1]} px × {len(s[2]) * len(s[3])}' for s in sizes)}")
    body = header
    for e in entries:
        body += e
    # Frame offsets count from the start of the blobs, which follow the index.
    data = bytes(body) + struct.pack("<I", len(body) + 4) + bytes(blobs)
    with open(out, "wb") as f:
        f.write(data)
    print(f"{out}: {len(metas)} objects, {len(data) / 1024:.0f} KB ({raw / 1024:.0f} KB of planes before compression)")


# ------------------------------------------------------------------ colour --

def hexc(h):
    return np.array([int(h[i:i + 2], 16) / 255 for i in (1, 3, 5)], np.float32)


# Bezel's own palettes (bezel_tokens), for looking at renders without the shell.
PALETTES = {
    "dark": dict(ground="#0b0c0e", surface="#16181b", body="#c0c3c7", signal="#ff5b1f", ice="#cfe0ee", amber="#f4c542", fault="#ff6b5e", ink="#eef0f2", shadow=0.85),
    "light": dict(ground="#e8eaed", surface="#fbfbfc", body="#fbfbfc", signal="#ea5a1a", ice="#9fbfda", amber="#ffc21a", fault="#b3261e", ink="#15181b", shadow=0.55),
}


def ramp(colour, albedo):
    """What each shade (0 to 255) looks like on a surface of `colour`: the
    neutral render relit, overexposure bleeding to white."""
    s = srgb_to_linear(np.arange(256, dtype=np.float32) / 255)[:, None]
    c = s * srgb_to_linear(colour)[None, :] / albedo
    m = c.max(axis=1, keepdims=True)
    over = np.clip((m - 1) / 1.5, 0, 1)
    base = c / np.maximum(m, 1)
    c = base + (1 - base) * over
    return linear_to_srgb(c)


def colour(p, pal, tints):
    """One view's planes in a palette: premultiplied colour and what is left
    of what is under it."""
    body = ramp(hexc(pal["body"]), BODY_ALBEDO)
    a = ramp(hexc(pal[tints[0]]), ACCENT_ALBEDO)
    b = ramp(hexc(pal[tints[1]]), ACCENT_ALBEDO)
    s = p[0]
    ta = (p[1] >> 4).astype(np.float32)[..., None] / 15
    tb = (p[1] & 15).astype(np.float32)[..., None] / 15
    c = body[s] * (1 - ta - tb) + a[s] * ta + b[s] * tb
    cov = p[2].astype(np.float32)[..., None] / 255
    sh = p[3].astype(np.float32)[..., None] / 255
    return c * cov, (1 - cov) * (1 - sh * pal["shadow"])


def peek(baked, out, only, yaw=0):
    """Each object at 120 px in dark and light, and at its smallest shipped
    size (3x) in dark: six objects to a row."""
    cells = []
    for meta in assets(baked):
        if only and meta["name"] not in only:
            continue
        folder = os.path.join(baked, meta["name"])
        small = min(s[1] for s in meta["sizes"])
        parts = []
        for tone, px, k in (("dark", 120, 1), ("light", 120, 1), ("dark", small, max(1, 120 // small))):
            pal = PALETTES[tone]
            p = planes(folder, yaw, 0, px)
            pm, keep = colour(p, pal, meta["tints"])
            img = pm + np.ones((px, px, 3), np.float32) * hexc(pal["surface"]) * keep
            img = np.repeat(np.repeat(img, k, 0), k, 1)
            parts.append(np.pad(img, ((0, 120 - img.shape[0]), (0, 4), (0, 0)), constant_values=0.3))
        cells.append(np.pad(np.concatenate(parts, axis=1), ((0, 6), (0, 10), (0, 0)), constant_values=0.3))
    rows = [np.concatenate(cells[i:i + 4], axis=1) for i in range(0, len(cells), 4)]
    w = max(r.shape[1] for r in rows)
    rows = [np.pad(r, ((0, 0), (0, w - r.shape[1]), (0, 0)), constant_values=0.3) for r in rows]
    Image.fromarray((np.concatenate(rows, axis=0) * 255).astype(np.uint8)).save(out)
    print(out)


def main():
    args = sys.argv[1:]
    baked = args.pop(0)
    if args[0] == "--peek":
        return peek(baked, args[1], set(args[2:]))
    out = args.pop(0)
    preview = None
    if "--preview" in args:
        i = args.index("--preview")
        preview = int(args[i + 1])
        del args[i:i + 2]
    only = set(args)
    if not only and not preview:
        # The objects the shell carries.
        with open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "shipped.txt")) as f:
            only = {line.strip() for line in f if line.strip() and not line.startswith("#")}
    write_pack(baked, out, preview, only)


main()

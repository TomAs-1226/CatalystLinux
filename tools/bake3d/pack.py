"""Turn bake.py's renders into what the shell carries: `objects.pack`.

    py -3 tools/bake3d/pack.py BAKED OUT.pack [names]         the shipped sizes and
                                                              poses of the objects
                                                              named (shipped.txt's
                                                              by default)
    py -3 tools/bake3d/pack.py BAKED OUT.pack --preview 96    the whole turn at
                                                              rest, 15° apart
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

An object with parts (a robot's drivetrain, intake, arm...; a battery's
cells) has a fifth plane: which part each pixel is, by the commonest part
under it. The shell colours each part on its own, from what it stands for.

An object with mechanisms (Numbers' intake and hood) was rendered in every
pose, each a step of each mechanism. Its rest pose is kept whole; every
other pose only where it differs from the rest, as a patch: the 8-pixel
tiles of the view it changes, eight to a row in one PNG, each byte as its
difference from what it is laid over (wrapping; part numbers exclusive-
or'd), so what a tile kept is zeros and deflates to almost nothing. A pose
of one mechanism moved is one patch, laid over the rest pose; two moved are
their two patches added together, then a patch of the tiles where the pose
itself differs from that (where they meet, or one shades the other). So
the intake's two steps and the hood's three cost a fraction of six whole
turns.

The pack is little-endian: "C3DP", a version (3), then each object (its
name, tints and its own colour, where it stands, its parts with the turn
that faces each to the camera, its mechanisms with their steps, its sizes;
each size its views and, per view, where its frame lies: a PNG whose red,
green, blue and alpha are the four planes, and for an object with parts a
greyscale PNG of part numbers right after it; then its tile size and its
patches, each the view and pose it is for, its tiles (numbered across the
view, row by row) and where its PNGs lie).
"""

import io
import itertools
import json
import os
import struct
import sys

import numpy as np
from PIL import Image

VERSION = 3
ROLES = {"signal": 0, "ice": 1, "amber": 2, "fault": 3, "ink": 4, "own": 5}
# What counts as a difference from the rest pose (render noise is less;
# a step this small at a tile's edge does not show), and a patch's tiles.
SHADE_STEP, COVER_STEP, SHADOW_STEP = 8, 4, 10
TILE = 8
ACROSS = 8
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


def planes(folder, yaw, pitch, px, prefix="", pose=""):
    """The four planes of one view at `px` across (`prefix` "turn_" for the
    previews' whole turn; `pose` "@12" for a pose other than the rest)."""
    lit = load(os.path.join(folder, f"{prefix}lit_{yaw:+04d}_{pitch:+03d}{pose}.png"))
    data = load(os.path.join(folder, f"{prefix}data_{yaw:+04d}_{pitch:+03d}{pose}.png"))
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


def part_plane(folder, yaw, pitch, px, prefix, parts, cover, pose=""):
    """Which part each pixel is, at `px` across: the commonest part number
    under it (0 where the object is not)."""
    raw = load(os.path.join(folder, f"{prefix}part_{yaw:+04d}_{pitch:+03d}{pose}.png"))
    ids = np.round(srgb_to_linear(raw[..., 0]) * 16).astype(np.int32) * (raw[..., 3] > 0.5)
    best = np.zeros((px, px), np.float32)
    out = np.zeros((px, px), np.uint8)
    for k in range(1, len(parts) + 1):
        share = np.asarray(Image.fromarray((ids == k).astype(np.float32)).resize((px, px), Image.Resampling.BOX))
        better = share > best
        out[better] = k
        best[better] = share[better]
    out[cover == 0] = 0
    return out


def l8_bytes(plane):
    out = io.BytesIO()
    Image.fromarray(np.ascontiguousarray(plane), "L").save(out, "PNG", optimize=True)
    return out.getvalue()


def changed(a, la, b, lb):
    """The tiles where view b differs from view a (planes and part
    numbers), numbered across the view row by row."""
    d = np.abs(a.astype(np.int16) - b.astype(np.int16))
    ta, tb = a[1].astype(np.int16), b[1].astype(np.int16)
    tint = (np.abs((ta >> 4) - (tb >> 4)) > 1) | (np.abs((ta & 15) - (tb & 15)) > 1)
    mask = (d[0] > SHADE_STEP) | tint | (d[2] > COVER_STEP) | (d[3] > SHADOW_STEP)
    if la is not None:
        mask |= la != lb
    px = a.shape[1]
    n = -(-px // TILE)
    grid = np.zeros((n * TILE, n * TILE), bool)
    grid[:px, :px] = mask
    return [int(i) for i in np.flatnonzero(grid.reshape(n, TILE, n, TILE).any(axis=(1, 3)))]


def tile_box(t, px):
    n = -(-px // TILE)
    y0, x0 = divmod(t, n)
    y0, x0 = y0 * TILE, x0 * TILE
    return y0, x0, min(TILE, px - y0), min(TILE, px - x0)


def add(q, lq, base, lbase, tiles, onto, lonto):
    """View q's difference from view base, in its tiles, added to a copy of
    view onto: as the shell lays a patch down (bytes wrapping, part numbers
    exclusive-or'd)."""
    out, lout = onto.copy(), (lonto.copy() if lonto is not None else None)
    px = q.shape[1]
    for t in tiles:
        y0, x0, h, w = tile_box(t, px)
        d = q[:, y0:y0 + h, x0:x0 + w].astype(np.int16) - base[:, y0:y0 + h, x0:x0 + w]
        out[:, y0:y0 + h, x0:x0 + w] = (out[:, y0:y0 + h, x0:x0 + w] + d) & 255
        if lout is not None:
            lout[y0:y0 + h, x0:x0 + w] ^= lq[y0:y0 + h, x0:x0 + w] ^ lbase[y0:y0 + h, x0:x0 + w]
    return out, lout


def strip(q, lq, onto, lonto, tiles):
    """View q's tiles, eight to a row, as their difference from view onto:
    its planes, and its part numbers."""
    px = q.shape[1]
    across = min(ACROSS, len(tiles))
    rows = -(-len(tiles) // ACROSS)
    planes_ = np.zeros((4, rows * TILE, across * TILE), np.uint8)
    parts_ = np.zeros((rows * TILE, across * TILE), np.uint8)
    for k, t in enumerate(tiles):
        y0, x0, h, w = tile_box(t, px)
        r, c = divmod(k, ACROSS)
        d = q[:, y0:y0 + h, x0:x0 + w].astype(np.int16) - onto[:, y0:y0 + h, x0:x0 + w]
        planes_[:, r * TILE:r * TILE + h, c * TILE:c * TILE + w] = d & 255
        if lq is not None:
            parts_[r * TILE:r * TILE + h, c * TILE:c * TILE + w] = lq[y0:y0 + h, x0:x0 + w] ^ lonto[y0:y0 + h, x0:x0 + w]
    return planes_, parts_


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
            px = preview
            sizes = [["preview", px, meta["turn"], [0]]]
            prefix = "turn_"
        entry = bytearray()
        name = meta["name"].encode()
        entry += struct.pack("<B", len(name)) + name
        tints = meta["tints"]
        entry += struct.pack("<BB", ROLES[tints[0]], ROLES[tints[1]])
        own = meta.get("own") or "#000000"
        entry += bytes(int(own[i:i + 2], 16) for i in (1, 3, 5))
        fx, fy = meta["foot"]
        entry += struct.pack("<HH", int(round(fx * 65535)), int(round(fy * 65535)))
        parts = meta.get("parts", [])
        entry += struct.pack("<B", len(parts))
        for part, facing in zip(parts, meta.get("facing", [0] * len(parts))):
            b = part.encode()
            entry += struct.pack("<B", len(b)) + b + struct.pack("<h", facing)
        # Its mechanisms and their steps (in tenths), and its poses: every
        # step of each, the rest first, numbered as the shell numbers them
        # (the first mechanism's step, plus the second's times the first's
        # count, and so on).
        mechs = [] if preview else meta.get("mechanisms", [])
        entry += struct.pack("<B", len(mechs))
        for m in mechs:
            b = m["name"].encode()
            entry += struct.pack("<B", len(b)) + b + struct.pack("<B", len(m["steps"]))
            entry += struct.pack(f"<{len(m['steps'])}h", *[int(round(v * 10)) for v in m["steps"]])
        counts = [len(m["steps"]) for m in mechs]
        poses = list(itertools.product(*[range(n) for n in counts]))[1:]

        def code(pose):
            c, k = 0, 1
            for i, n in zip(pose, counts):
                c += i * k
                k *= n
            return c

        entry += struct.pack("<B", len(sizes))
        for label, px, yaws, pitches in sizes:
            label = label.encode()
            entry += struct.pack("<B", len(label)) + label
            entry += struct.pack("<HBB", px, len(yaws), len(pitches))
            entry += struct.pack(f"<{len(yaws)}h", *yaws)
            entry += struct.pack(f"<{len(pitches)}b", *pitches)
            patches = []
            whole, patched = len(blobs), 0
            for pitch in pitches:
                for yaw in yaws:
                    p = planes(folder, yaw, pitch, px, prefix)
                    raw += p.size
                    blob = png_bytes(p)
                    lp = part_plane(folder, yaw, pitch, px, prefix, parts, p[2]) if parts else None
                    extra = l8_bytes(lp) if parts else b""
                    entry += struct.pack("<III", len(blobs), len(blob), len(extra))
                    blobs += blob + extra
                    if not poses:
                        continue
                    # Each other pose: what it changes, as a patch.
                    view = pitches.index(pitch) * len(yaws) + yaws.index(yaw)
                    got = {}
                    for pose in poses:
                        tag = "@" + "".join(str(i) for i in pose)
                        q = planes(folder, yaw, pitch, px, prefix, tag)
                        lq = part_plane(folder, yaw, pitch, px, prefix, parts, q[2], tag) if parts else None
                        got[pose] = (q, lq)
                    singles = {}
                    for pose in poses:
                        q, lq = got[pose]
                        moved = [i for i, step in enumerate(pose) if step]
                        onto, lonto = p, lp
                        if len(moved) == 1:
                            # One mechanism moved: what it changes from the rest.
                            tiles = changed(p, lp, q, lq)
                            singles[pose] = tiles
                        else:
                            # Two: their patches added together, then where
                            # the pose itself differs from that.
                            for i in moved:
                                one = tuple(step if j == i else 0 for j, step in enumerate(pose))
                                onto, lonto = add(got[one][0], got[one][1], p, lp, singles[one], onto, lonto)
                            tiles = changed(onto, lonto, q, lq)
                        if not tiles:
                            continue
                        sp, sl = strip(q, lq, onto, lonto, tiles)
                        pb = png_bytes(sp)
                        pe = l8_bytes(sl) if parts else b""
                        patches.append(struct.pack("<HHH", view, code(pose), len(tiles)) + struct.pack(f"<{len(tiles)}H", *tiles) + struct.pack("<III", len(blobs), len(pb), len(pe)))
                        blobs += pb + pe
                        patched += len(pb) + len(pe)
            entry += struct.pack("<BBI", TILE, ACROSS, len(patches))
            for patch in patches:
                entry += patch
            if poses:
                print(f"  {meta['name']} {label}: rest pose {(len(blobs) - whole - patched) / 1024:.0f} KB; {len(patches)} patches for {len(poses)} poses, {patched / 1024:.0f} KB")
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
            pal = dict(PALETTES[tone], own=meta.get("own") or "#808080")
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

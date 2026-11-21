"""Catalyst's 3D objects, and what each is rendered as.

Each builder makes its object out of kit.py's shapes and materials and
returns its root. Front faces -y (toward the camera at rest); z is up; the
size does not matter (bake.py scales every object to the same size).

What a surface is made of, not its colour: "accent" takes the signal (the
accent colour chosen in Settings, or the user's own), "second" takes the
object's second tint (ice or amber, as `tints` says), "dark" is glass and
screens, "graphite" dark plastic, "body" and "white" the shell, "metal"
bare metal. "_glow" ones are lit from within.

Two sources: code (the builders below) and models generated with Higgsfield
(text to 3D, Tripo), reduced and kept in glb/ by glb_prep.py.
"""

import math
import os
import random

from kit import (box, cyl, empty, import_model, lathe, prism, ring_band, sphere, text, torus, tube)

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))
FONTS = os.path.join(ROOT, "userland", "assets", "fonts")
SANS = os.path.join(FONTS, "GoogleSansFlex.ttf")
CODE = os.path.join(FONTS, "GoogleSansCode.ttf")

# Views. An app icon turns either way from where it rests and tips up and
# down a little (Home turns it with the focus ring's motion and with where
# its tile sits); a hero turns all the way round (setup swings it from page
# to page). Previews add a whole turn to every object.
ARC = list(range(-48, 49, 8))
SMALL_ARC = list(range(-48, 49, 12))
TURN = list(range(0, 360, 15))
PITCHES = [-12, 0, 12]


ICON = dict(sizes=[["s", 30, SMALL_ARC, PITCHES], ["l", 52, SMALL_ARC, [0]]], res=208)
HERO = dict(sizes=[["hero", 92, TURN, [0]]], res=276)
THING = dict(sizes=[["m", 60, ARC, [0]]], res=240)
HERO_AND_THING = dict(sizes=[["hero", 92, TURN, [0]], ["m", 60, ARC, [0]]], res=276)


def upright(loc=(0, 0, 0), parent=None, name="face"):
    """A frame whose x, y plane faces the camera: its +z points at the
    viewer (-y), its +y up. Flat shapes made in it stand facing front."""
    return empty(name, loc, (90, 0, 0), parent)


def heart_outline(n=72, k=1.0):
    pts = []
    for i in range(n):
        t = 2 * math.pi * i / n
        x = 16 * math.sin(t) ** 3
        y = 13 * math.cos(t) - 5 * math.cos(2 * t) - 2 * math.cos(3 * t) - math.cos(4 * t)
        pts.append((x / 17 * k, y / 17 * k))
    return pts[::-1]


def star(points, r_out, r_in, n_round=1):
    pts = []
    for i in range(points * 2):
        a = math.pi / 2 + math.pi * i / points
        r = r_out if i % 2 == 0 else r_in
        pts.append((r * math.cos(a), r * math.sin(a)))
    return pts


def sparkle(r, pinch=0.22, n=12):
    """A four-pointed sparkle with curved sides."""
    pts = []
    for q in range(4):
        a0 = math.pi / 2 * q
        for i in range(n):
            t = i / n
            a = a0 + t * math.pi / 2
            # From one point to the next through a pinched middle.
            rr = r * (pinch + (1 - pinch) * (abs(1 - 2 * t) ** 2.2))
            pts.append((rr * math.cos(a), rr * math.sin(a)))
    return pts


def gear_outline(teeth=10, r_out=1.0, r_in=0.8, n=8):
    pts = []
    step = 2 * math.pi / teeth
    for i in range(teeth):
        a = i * step
        for j in range(n):
            t = j / n
            # A trapezoid tooth: rising flank, top, falling flank, root.
            if t < 0.2:
                r = r_in + (r_out - r_in) * (t / 0.2)
            elif t < 0.5:
                r = r_out
            elif t < 0.7:
                r = r_out - (r_out - r_in) * ((t - 0.5) / 0.2)
            else:
                r = r_in
            pts.append((r * math.cos(a + t * step), r * math.sin(a + t * step)))
    return pts


BOLT = [(0.18, 1.0), (-0.42, -0.08), (-0.02, -0.08), (-0.2, -1.0), (0.42, 0.12), (0.02, 0.12)]


def bolt(depth, scale, loc, mat="accent_glow", parent=None):
    return prism([(x * scale, y * scale) for x, y in BOLT], depth, loc, r=min(depth * 0.3, scale * 0.08), mat=mat, parent=parent, name="bolt")


def tick(face, x, y, z, s=0.3, r=0.05, mat="accent"):
    tube([(x - s * 0.5, y + s * 0.02, z), (x - s * 0.15, y - s * 0.32, z), (x + s * 0.55, y + s * 0.42, z)], r, mat, parent=face, resolution=4)


# ================================================================== system ===

def handheld():
    """The CardputerZero, stood up and leaning back a little: its screen lit
    with Catalyst's bolt, its 46 keys below, Fn in the accent."""
    root = empty("handheld")
    dev = empty("device", (0, 0, 0), (-14, 0, 0), root)
    w, d, h = 3.0, 0.42, 2.0
    box((w, d, h), (0, 0, h / 2), r=0.16, mat="white", parent=dev, segments=6)
    box((1.86, 0.06, 0.98), (0, -d / 2 - 0.005, 1.42), r=0.06, mat="graphite", parent=dev)
    box((1.72, 0.05, 0.86), (0, -d / 2 - 0.03, 1.42), r=0.035, mat="dark", parent=dev)
    face = upright((0, -d / 2 - 0.06, 1.42), dev)
    bolt(0.03, 0.27, (0, 0, 0), parent=face)
    kw, kh, gap = 0.215, 0.15, 0.03
    for r, n in enumerate([12, 12, 11, 11]):
        z = 0.78 - r * (kh + gap)
        x0 = -(n * kw + (n - 1) * gap) / 2 + kw / 2
        for i in range(n):
            mat = "accent" if (r, i) == (3, 0) else "graphite"
            box((kw, 0.07, kh), (x0 + i * (kw + gap), -d / 2 - 0.02, z), r=0.03, mat=mat, parent=dev, segments=3)
    for i, x in enumerate((-1.15, -1.0, -0.85)):
        cyl(0.025, 0.04, (x, -d / 2 - 0.01, 0.93), r=0.0, mat="second_glow" if i == 1 else "soft", rot=(90, 0, 0), parent=dev, verts=16)
    return root


def wifi():
    """Wi-Fi: a dot and three arcs (two lit, as when joined) on a round base."""
    root = empty("wifi")
    cyl(0.9, 0.22, (0, 0, 0.11), r=0.08, mat="white", parent=root)
    face = upright((0, 0, 0.22), root)
    sphere(0.17, (0, 0.32, 0), "accent", parent=face)
    for i, (ri, ro) in enumerate(((0.55, 0.78), (0.98, 1.21), (1.41, 1.64))):
        ring_band(ri, ro, 0.26, 45, 90, (0, 0.06, 0), mat="accent" if i < 2 else "white", parent=face, r=0.05)
    return root


def battery():
    """A battery cell standing up: a white shell, a dark window and its
    charge glowing inside in the second tint."""
    root = empty("battery")
    w, d, h = 1.0, 0.62, 1.7
    box((w, d, h), (0, 0, h / 2), r=0.2, mat="white", parent=root, segments=6)
    box((0.42, 0.34, 0.16), (0, 0, h + 0.05), r=0.06, mat="graphite", parent=root)
    box((0.74, 0.05, 1.42), (0, -d / 2 - 0.005, h / 2), r=0.1, mat="dark", parent=root)
    for i in range(3):
        box((0.6, 0.05, 0.36), (0, -d / 2 - 0.03, 0.27 + i * 0.42), r=0.06, mat="second_glow", parent=root)
    face = upright((0, -d / 2 - 0.07, h / 2), root)
    bolt(0.04, 0.36, (0, 0, 0), mat="white", parent=face)
    return root


def chip():
    """The hardware check: a processor, pins all round, the bolt on its lid."""
    root = empty("chip")
    tilt = empty("tilt", (0, 0, 0.0), (52, 0, 0), root)
    box((1.9, 1.9, 0.2), (0, 0, 0.1), r=0.06, mat="graphite", parent=tilt)
    box((1.25, 1.25, 0.14), (0, 0, 0.26), r=0.08, mat="white", parent=tilt)
    for side in range(4):
        for i in range(7):
            t = -0.75 + i * 0.25
            x, y = (t, 1.06) if side == 0 else (t, -1.06) if side == 1 else (1.06, t) if side == 2 else (-1.06, t)
            sx, sy = (0.11, 0.3) if side < 2 else (0.3, 0.11)
            box((sx, sy, 0.06), (x, y, 0.08), r=0.02, mat="metal", parent=tilt, segments=2)
    face = empty("lid", (0, 0, 0.34), (0, 0, 0), tilt)
    bolt(0.05, 0.4, (0, 0, 0), mat="accent", parent=face)
    return root


def keycaps():
    """The keyboard lesson: Fn, Shift and Sym as keycaps, their lights above."""
    root = empty("keycaps")
    tilt = empty("tilt", (0, 0, 0), (38, 0, 0), root)
    box((3.3, 1.5, 0.18), (0, 0, 0.09), r=0.08, mat="graphite", parent=tilt)
    for i, (label, mat) in enumerate((("shift", "white"), ("fn", "accent"), ("sym", "white"))):
        x = -1.08 + i * 1.08
        cap = box((0.92, 0.92, 0.36), (x, -0.12, 0.36), r=0.16, mat=mat, parent=tilt, segments=5)
        text(label, 0.3, 0.04, (x, -0.12, 0.555), mat="graphite" if mat == "white" else "white", parent=tilt, font=CODE, bevel_depth=0.004)
        cyl(0.07, 0.05, (x, 0.5, 0.2), r=0.0, mat="second_glow" if i == 1 else "soft", parent=tilt, verts=20)
    return root


def key():
    """Keys for online services: a key, its bow in the accent."""
    root = empty("key")
    face = upright((0, 0, 1.0), root)
    tilt = empty("tilt", (0, 0, 0), (0, 0, -35), face)
    torus(0.55, 0.2, (-0.95, 0, 0), "accent", parent=tilt)
    box((1.9, 0.28, 0.2), (0.55, 0, 0), r=0.08, mat="metal", parent=tilt)
    for x, h in ((1.0, 0.38), (1.25, 0.26), (1.43, 0.34)):
        box((0.14, h, 0.2), (x, -0.14 - h / 2, 0), r=0.04, mat="metal", parent=tilt, segments=3)
    cyl(0.18, 0.24, (-0.32, 0, 0), r=0.05, mat="metal", parent=tilt)
    return root


def toggle():
    """The choices: a switch, switched on."""
    root = empty("toggle")
    face = upright((0, 0, 0.75), root)
    box((2.4, 1.2, 0.5), (0, 0, 0), r=0.6, mat="accent", parent=face, segments=10)
    sphere(0.5, (0.58, 0, 0.18), "white", parent=face)
    return root


def palette():
    """The look: swatches fanned out, the accent on top."""
    root = empty("palette")
    face = upright((0, 0, 0.0), root)
    for i, mat in enumerate(("soft", "white", "second", "accent")):
        a = -42 + i * 26
        sw = empty("swatch", (0, 0, i * 0.08), (0, 0, a), face)
        box((0.62, 2.1, 0.08), (0, 1.15, 0), r=0.04, mat="body", parent=sw, segments=3)
        box((0.5, 0.9, 0.06), (0, 1.62, 0.03), r=0.05, mat=mat, parent=sw, segments=3)
    cyl(0.12, 0.5, (0, 0.18, 0.15), r=0.03, mat="metal", parent=face, rot=(0, 0, 0))
    return root


def crescent(r=1.0, cx=0.48, cy=0.34, r2=0.86, n=72):
    """A crescent: a circle of radius r less one of r2 centred at (cx, cy),
    counter-clockwise."""
    d = math.hypot(cx, cy)
    # Where the two circles cross, as angles on each.
    a = (r * r - r2 * r2 + d * d) / (2 * d)
    h = math.sqrt(max(r * r - a * a, 0))
    bx, by = a * cx / d, a * cy / d
    p1 = (bx + h * cy / d, by - h * cx / d)
    p2 = (bx - h * cy / d, by + h * cx / d)
    o1, o2 = math.atan2(p1[1], p1[0]), math.atan2(p2[1], p2[0])
    # The outer arc runs the long way round, away from the inner circle.
    if o1 < o2:
        o1 += 2 * math.pi
    pts = [(r * math.cos(o2 + (o1 - o2) * i / n), r * math.sin(o2 + (o1 - o2) * i / n)) for i in range(n + 1)]
    i1, i2 = math.atan2(p1[1] - cy, p1[0] - cx), math.atan2(p2[1] - cy, p2[0] - cx)
    # Back along the inner circle, clockwise: its side nearer the middle,
    # which lies inside the outer one.
    if i2 > i1:
        i2 -= 2 * math.pi
    m = n // 2
    for k in range(1, m):
        t = i1 + (i2 - i1) * k / m
        pts.append((cx + r2 * math.cos(t), cy + r2 * math.sin(t)))
    return pts


def moon():
    """Deep standby: a crescent moon, two stars beside it."""
    root = empty("moon")
    face = upright((0, 0, 1.1), root)
    prism(crescent(), 0.5, (0, 0, 0), r=0.16, mat="second", parent=face, segments=6)
    for x, y, s in ((0.62, 0.62, 0.26), (1.05, 0.05, 0.16)):
        prism(star(4, s, s * 0.36), 0.14, (x, y, 0), r=0.03, mat="white", parent=face)
    return root


def empty_tray():
    """An empty state: an open tray with nothing in it."""
    root = empty("tray")
    box((2.2, 1.6, 0.16), (0, 0, 0.08), r=0.08, mat="white", parent=root)
    for x, y, sx, sy in ((0, 0.74, 2.2, 0.12), (0, -0.74, 2.2, 0.12), (1.04, 0, 0.12, 1.6), (-1.04, 0, 0.12, 1.6)):
        box((sx, sy, 0.5), (x, y, 0.25), r=0.05, mat="white", parent=root, segments=3)
    box((1.6, 1.1, 0.03), (0, 0, 0.17), r=0.08, mat="soft", parent=root)
    return root


def sunmoon():
    """Light and dark: a coin with the sun on its face and the moon on its
    back, so half a turn goes from one to the other."""
    root = empty("sunmoon")
    face = upright((0, 0, 1.05), root)
    cyl(1.0, 0.3, (0, 0, 0), r=0.1, mat="white", parent=face)
    sphere(0.34, (0, 0, 0.16), "second", parent=face, scale=(1, 1, 0.45))
    for i in range(8):
        ray = empty("ray", (0, 0, 0.17), (0, 0, i * 45), face)
        box((0.1, 0.2, 0.08), (0, 0.6, 0), r=0.04, mat="second", parent=ray, segments=3)
    back = empty("back", (0, 0, -0.15), (180, 0, 0), face)
    prism([(x * 0.62, y * 0.62) for x, y in crescent()], 0.12, (-0.05, 0.0, 0.02), r=0.04, mat="second", parent=back)
    return root


# ==================================================================== apps ===

def app_robot():
    """Robot: a friendly robot's head, visor and lit eyes, an antenna."""
    root = empty("robot")
    box((1.5, 1.2, 1.15), (0, 0, 0.62), r=0.32, mat="white", parent=root, segments=7)
    box((1.18, 0.1, 0.62), (0, -0.58, 0.66), r=0.22, mat="dark", parent=root, segments=6)
    for x in (-0.28, 0.28):
        box((0.2, 0.08, 0.28), (x, -0.64, 0.68), r=0.09, mat="accent_glow", parent=root, segments=4)
    for x in (-0.79, 0.79):
        cyl(0.22, 0.14, (x, 0, 0.66), r=0.05, mat="graphite", rot=(0, 90, 0), parent=root)
    cyl(0.04, 0.34, (0, 0, 1.33), r=0.0, mat="graphite", parent=root, verts=16)
    sphere(0.12, (0, 0, 1.52), "accent", parent=root)
    return root


def app_health():
    """Health: a heart, its pulse drawn across it."""
    root = empty("health")
    face = upright((0, 0, 1.0), root)
    prism(heart_outline(k=1.05), 0.55, (0, 0, 0), r=0.2, mat="accent", parent=face, segments=6)
    z = 0.29
    pts = [(-0.95, 0.05), (-0.42, 0.05), (-0.24, 0.42), (-0.06, -0.38), (0.12, 0.22), (0.26, 0.05), (0.95, 0.05)]
    for (x0, y0), (x1, y1) in zip(pts, pts[1:]):
        tube([(x0, y0, z), (x1, y1, z)], 0.055, "white", parent=face, resolution=2)
    for x, y in pts[1:-1]:
        sphere(0.055, (x, y, z), "white", parent=face)
    return root


def page(face, w, h, cut, depth, mat, loc=(0, 0, 0), rot=0.0, name="page"):
    pts = [(-w / 2, -h / 2), (w / 2, -h / 2), (w / 2, h / 2 - cut), (w / 2 - cut, h / 2), (-w / 2, h / 2)]
    holder = empty(name, loc, (0, 0, rot), face)
    prism(pts, depth, (0, 0, 0), r=0.035, mat=mat, parent=holder)
    return holder


def app_logs():
    """Logs: a page of a log, a second behind it, one line marked."""
    root = empty("logs")
    face = upright((0, 0, 1.0), root)
    page(face, 1.25, 1.65, 0.0, 0.1, "body", (0.28, 0.12, -0.12), -8)
    p = page(face, 1.25, 1.65, 0.38, 0.12, "white", (0, 0, 0))
    box((0.36, 0.36, 0.1), (0.44, 0.64, 0.0), r=0.04, mat="soft", rot=(0, 0, 0), parent=p)
    for i, (w, mat) in enumerate(((0.62, "soft"), (0.85, "soft"), (0.7, "accent"), (0.85, "soft"), (0.5, "soft"))):
        y = 0.5 - i * 0.25
        box((w, 0.09, 0.06), (-0.5 + w / 2 - 0.05, y, 0.07), r=0.03, mat=mat, parent=p, segments=2)
    return root


def app_schedule():
    """Schedule: a desk calendar, the day marked in the accent."""
    root = empty("schedule")
    box((1.7, 0.55, 1.45), (0, 0, 0.725), r=0.16, mat="white", parent=root)
    box((1.72, 0.57, 0.42), (0, 0, 1.25), r=0.16, mat="accent", parent=root)
    for x in (-0.45, 0.45):
        torus(0.16, 0.06, (x, -0.05, 1.47), "graphite", rot=(90, 0, 90), parent=root)
    face = upright((0, -0.28, 0), root)
    for row in range(3):
        for col in range(4):
            mat = "accent" if (row, col) == (1, 2) else "soft"
            box((0.24, 0.2, 0.06), (-0.48 + col * 0.32, 0.78 - row * 0.27, 0.0), r=0.05, mat=mat, parent=face, segments=3)
    return root


def app_tables():
    """Tables: a tree of values, its root in the accent."""
    root = empty("tables")
    face = upright((0, 0, 1.0), root)
    box((0.62, 0.62, 0.5), (-0.62, 0.55, 0), r=0.14, mat="accent", parent=face)
    box((0.58, 0.58, 0.46), (0.62, 0.05, 0), r=0.14, mat="white", parent=face)
    box((0.58, 0.58, 0.46), (0.62, -0.75, 0), r=0.14, mat="white", parent=face)
    for y in (0.05, -0.75):
        tube([(-0.62, 0.24, 0), (-0.62, y + 0.05, 0), (-0.55, y, 0), (0.33, y, 0)], 0.07, "graphite", parent=face, resolution=8)
    return root


def app_loop():
    """Loop: a gauge, the needle well inside its arc."""
    root = empty("loop")
    face = upright((0, 0, 1.0), root)
    cyl(1.0, 0.3, (0, 0, 0), r=0.08, mat="white", parent=face)
    torus(1.0, 0.1, (0, 0, 0.08), "graphite", parent=face)
    ring_band(0.66, 0.8, 0.08, -30, 240, (0, 0, 0.17), mat="soft", parent=face, r=0.02)
    ring_band(0.66, 0.8, 0.1, 150, -95, (0, 0, 0.18), mat="accent", parent=face, r=0.02)
    needle = empty("needle", (0, 0, 0.22), (0, 0, 50), face)
    prism([(-0.07, 0), (0, 0.72), (0.07, 0)], 0.06, (0, 0, 0), r=0.015, mat="graphite", parent=needle)
    cyl(0.13, 0.12, (0, 0, 0.24), r=0.03, mat="graphite", parent=face)
    return root


def app_motors():
    """Motors: a brushless motor, its can ringed in the accent, its shaft out."""
    root = empty("motors")
    face = upright((0, 0, 0.95), root)
    cyl(0.85, 1.1, (0, 0, -0.1), r=0.1, mat="graphite", parent=face)
    cyl(0.86, 0.16, (0, 0, 0.12), r=0.04, mat="accent", parent=face)
    cyl(0.72, 0.14, (0, 0, 0.48), r=0.06, mat="white", parent=face)
    for i in range(6):
        a = i * 60
        holder = empty("slot", (0, 0, 0.56), (0, 0, a), face)
        box((0.14, 0.34, 0.04), (0, 0.4, 0), r=0.03, mat="dark", parent=holder, segments=2)
    cyl(0.12, 0.45, (0, 0, 0.72), r=0.03, mat="metal", parent=face)
    return root


def app_console():
    """Console: the robot's controller board, a chip and two lit lights."""
    root = empty("console")
    tilt = empty("tilt", (0, 0, 0), (55, 0, 0), root)
    box((2.0, 1.45, 0.12), (0, 0, 0.06), r=0.08, mat="graphite", parent=tilt)
    box((0.7, 0.7, 0.14), (-0.3, 0.05, 0.19), r=0.06, mat="white", parent=tilt)
    for i in range(5):
        t = -0.24 + i * 0.12
        for x, y, sx, sy in ((t - 0.3, 0.44, 0.06, 0.12), (t - 0.3, -0.34, 0.06, 0.12), (0.09, t + 0.05, 0.12, 0.06), (-0.69, t + 0.05, 0.12, 0.06)):
            box((sx, sy, 0.04), (x, y, 0.14), r=0.0, mat="metal", parent=tilt)
    for i, y in enumerate((0.35, 0.05)):
        box((0.16, 0.12, 0.08), (0.62, y, 0.16), r=0.03, mat="accent_glow" if i == 0 else "second_glow", parent=tilt, segments=2)
    for y in (-0.35, -0.48):
        box((0.7, 0.04, 0.02), (0.45, y, 0.13), r=0.0, mat="soft", parent=tilt)
    box((0.3, 0.5, 0.22), (0.78, -0.4, 0.2), r=0.05, mat="body", parent=tilt)
    return root


def app_terminal():
    """Terminal: a dark screen with a prompt lit on it."""
    root = empty("terminal")
    box((2.0, 0.36, 1.4), (0, 0, 0.7), r=0.16, mat="graphite", parent=root)
    box((1.8, 0.06, 1.2), (0, -0.17, 0.7), r=0.1, mat="dark", parent=root)
    face = upright((0, -0.21, 0.7), root)
    prism([(-0.62, 0.32), (-0.18, 0.02), (-0.62, -0.28), (-0.62, -0.12), (-0.38, 0.02), (-0.62, 0.16)], 0.05, (0, 0, 0), r=0.015, mat="accent_glow", parent=face)
    box((0.48, 0.12, 0.05), (0.2, -0.24, 0), r=0.03, mat="accent_glow", parent=face, segments=2)
    return root


def app_files():
    """Files: a folder in the accent, a page inside it."""
    root = empty("files")
    face = upright((0, 0, 0.0), root)
    back = [(-1.0, 0), (1.0, 0), (1.0, 1.25), (-0.1, 1.25), (-0.25, 1.45), (-1.0, 1.45)]
    prism(back, 0.12, (0, 0, -0.2), r=0.05, mat="accent", parent=face)
    page(face, 1.6, 1.2, 0.0, 0.05, "white", (0.05, 0.78, -0.05), -4)
    front = empty("front", (0, 0, 0.0), (-16, 0, 0), face)
    prism([(-1.0, 0), (1.0, 0), (1.04, 1.05), (-1.04, 1.05)], 0.12, (0, 0, 0.05), r=0.05, mat="accent", parent=front)
    return root


def app_git():
    """Git: a branch leaving the main line, its last commit in the accent."""
    root = empty("git")
    face = upright((0, 0, 1.05), root)
    tube([(-0.45, -0.95, 0), (-0.45, 0.95, 0)], 0.09, "graphite", parent=face, resolution=2)
    tube([(-0.45, -0.25, 0), (-0.2, 0.1, 0), (0.45, 0.25, 0), (0.5, 0.6, 0)], 0.09, "graphite", parent=face, resolution=12)
    for y in (-0.8, -0.15, 0.75):
        sphere(0.24, (-0.45, y, 0), "white", parent=face)
    sphere(0.28, (0.5, 0.62, 0), "accent", parent=face)
    return root


def app_catalogue():
    """Catalogue: a shopping bag, its handles in the accent."""
    root = empty("catalogue")
    box((1.5, 0.75, 1.45), (0, 0, 0.725), r=0.1, mat="white", parent=root)
    box((1.52, 0.77, 0.14), (0, 0, 1.38), r=0.04, mat="body", parent=root, segments=3)
    for y in (-0.38, 0.38):
        torus(0.36, 0.06, (0, y, 1.45), "accent", rot=(90, 0, 0), parent=root, arc=180, start=0)
    face = upright((0, -0.39, 0.75), root)
    prism(star(5, 0.28, 0.12), 0.06, (0, 0, 0), r=0.02, mat="accent", parent=face)
    return root


def app_monitor():
    """Monitor: bars rising, the last in the accent."""
    root = empty("monitor")
    box((2.2, 0.9, 0.14), (0, 0, 0.07), r=0.07, mat="white", parent=root)
    for i, h in enumerate((0.55, 0.95, 0.75, 1.45)):
        box((0.38, 0.38, h), (-0.72 + i * 0.48, 0, 0.14 + h / 2), r=0.1, mat="accent" if i == 3 else "body", parent=root)
    return root


def app_nettools():
    """Net tools: a radar dish on its mount, listening."""
    root = empty("nettools")
    cyl(0.6, 0.18, (0, 0, 0.09), r=0.06, mat="graphite", parent=root)
    cyl(0.12, 0.6, (0, 0, 0.45), r=0.03, mat="graphite", parent=root)
    turn = empty("turn", (0, 0, 0.85), (0, 0, -38), root)
    dish = empty("dish", (0, 0, 0), (62, 0, 0), turn)
    prof = []
    for i in range(13):
        r = i / 12 * 1.0
        prof.append((r, r * r * 0.42))
    for i in range(12, -1, -1):
        r = i / 12 * 0.96
        prof.append((r, r * r * 0.42 + 0.07))
    lathe(prof, (0, 0, 0), "white", parent=dish)
    tube([(0, 0, 0.1), (0, 0, 0.62)], 0.035, "graphite", parent=dish, resolution=2)
    sphere(0.12, (0, 0, 0.66), "accent_glow", parent=dish)
    return root


def app_connect():
    """Connect: a network plug on its cable, its latch in the accent."""
    root = empty("connect")
    # The plug points out toward the viewer, a little to the left.
    plug = empty("plug", (0, 0, 0.9), (12, 0, 38), root)
    box((0.9, 1.0, 0.7), (0, 0, 0), r=0.1, mat="white", parent=plug)
    box((0.7, 0.06, 0.4), (0, -0.5, -0.05), r=0.03, mat="dark", parent=plug)
    for i in range(6):
        box((0.06, 0.08, 0.1), (-0.25 + i * 0.1, -0.5, 0.2), r=0.0, mat="metal", parent=plug)
    box((0.4, 0.62, 0.12), (0, -0.05, 0.4), r=0.05, mat="accent", parent=plug)
    box((0.72, 0.5, 0.6), (0, 0.72, 0), r=0.16, mat="graphite", parent=plug)
    tube([(0, 0.95, 0), (0, 1.25, -0.1), (0.1, 1.35, -0.55)], 0.14, "graphite", parent=plug)
    return root


def app_actions():
    """Actions: a play button, raised."""
    root = empty("actions")
    face = upright((0, 0, 1.0), root)
    cyl(1.0, 0.32, (0, 0, 0), r=0.12, mat="white", parent=face)
    prism([(-0.28, -0.45), (0.5, 0.0), (-0.28, 0.45)], 0.3, (0.04, 0, 0.18), r=0.08, mat="accent", parent=face)
    return root


def app_camera():
    """Camera: a camera with a big lens, the shutter in the accent."""
    root = empty("camera")
    box((1.9, 0.75, 1.25), (0, 0, 0.625), r=0.2, mat="white", parent=root)
    box((0.6, 0.55, 0.25), (-0.45, 0, 1.3), r=0.1, mat="white", parent=root)
    face = upright((0.12, -0.37, 0.6), root)
    cyl(0.5, 0.3, (0, 0, 0.1), r=0.05, mat="graphite", parent=face)
    torus(0.42, 0.05, (0, 0, 0.26), "accent", parent=face)
    cyl(0.36, 0.06, (0, 0, 0.24), r=0.02, mat="dark", parent=face)
    sphere(0.09, (-0.1, 0.12, 0.26), "glow", parent=face)
    cyl(0.14, 0.12, (0.55, 0, 1.28), r=0.04, mat="accent", parent=root)
    box((0.3, 0.06, 0.16), (0.62, -0.38, 1.0), r=0.04, mat="dark", parent=root, segments=2)
    return root


def app_photos():
    """Photos: two prints, a landscape on the front one with the sun out."""
    root = empty("photos")
    face = upright((0, 0, 0.95), root)
    back = empty("back", (0.22, 0.18, -0.12), (0, 0, 10), face)
    box((1.7, 1.3, 0.08), (0, 0, 0), r=0.06, mat="body", parent=back)
    front = empty("front", (-0.1, -0.08, 0.02), (0, 0, -6), face)
    box((1.7, 1.3, 0.1), (0, 0, 0), r=0.06, mat="white", parent=front)
    box((1.45, 1.05, 0.04), (0, 0.0, 0.05), r=0.04, mat="graphite", parent=front, segments=2)
    prism([(-0.72, -0.52), (-0.25, 0.08), (0.02, -0.22), (0.3, 0.2), (0.72, -0.52)], 0.06, (0, 0, 0.08), r=0.02, mat="body", parent=front)
    sphere(0.16, (0.38, 0.3, 0.1), "second", parent=front)
    return root


def app_apriltag():
    """AprilTag: a tag, framed by the scanner's corners in the accent."""
    root = empty("apriltag")
    face = upright((0, 0, 1.0), root)
    box((1.6, 1.6, 0.12), (0, 0, 0), r=0.06, mat="white", parent=face)
    box((1.24, 1.24, 0.06), (0, 0, 0.05), r=0.02, mat="graphite", parent=face)
    rng = random.Random(5805)
    cell = 1.24 / 8
    for i in range(6):
        for j in range(6):
            if rng.random() < 0.5:
                box((cell * 0.98, cell * 0.98, 0.07), (-0.62 + (i + 1.5) * cell, -0.62 + (j + 1.5) * cell, 0.07), r=0.0, mat="white", parent=face)
    for sx in (-1, 1):
        for sy in (-1, 1):
            c = empty("corner", (sx * 1.0, sy * 1.0, 0.3), (0, 0, 0), face)
            box((0.42, 0.1, 0.1), (-sx * 0.16, 0, 0), r=0.04, mat="accent", parent=c, segments=2)
            box((0.1, 0.42, 0.1), (0, -sy * 0.16, 0), r=0.04, mat="accent", parent=c, segments=2)
    return root


def app_graphs():
    """Graphs: a line rising across a board, its points marked."""
    root = empty("graphs")
    tilt = empty("tilt", (0, 0, 0), (62, 0, 0), root)
    box((2.1, 1.5, 0.12), (0, 0, 0.06), r=0.08, mat="white", parent=tilt)
    for y in (-0.4, 0.0, 0.4):
        box((1.8, 0.03, 0.02), (0, y, 0.13), r=0.0, mat="soft", parent=tilt)
    pts = [(-0.8, -0.45), (-0.35, -0.05), (0.05, -0.25), (0.45, 0.25), (0.82, 0.5)]
    for (x0, y0), (x1, y1) in zip(pts, pts[1:]):
        tube([(x0, y0, 0.3), (x1, y1, 0.3)], 0.07, "accent", parent=tilt, resolution=2)
    for x, y in pts:
        sphere(0.12, (x, y, 0.3), "white", parent=tilt)
    return root


def app_pit():
    """Pit: a clipboard checklist, two ticked."""
    root = empty("pit")
    face = upright((0, 0, 1.0), root)
    box((1.5, 1.95, 0.1), (0, 0, -0.05), r=0.1, mat="body", parent=face)
    box((1.3, 1.65, 0.05), (0, -0.08, 0.03), r=0.04, mat="white", parent=face)
    box((0.7, 0.3, 0.18), (0, 0.92, 0.07), r=0.08, mat="graphite", parent=face)
    for i in range(3):
        y = 0.4 - i * 0.48
        box((0.28, 0.28, 0.06), (-0.38, y, 0.07), r=0.06, mat="soft" if i == 2 else "accent", parent=face, segments=3)
        if i < 2:
            tick(face, -0.38, y, 0.12, 0.2, 0.03, "white")
        box((0.5, 0.08, 0.05), (0.16, y, 0.07), r=0.03, mat="soft", parent=face, segments=2)
    return root


def app_remote():
    """Remote: an infrared remote, its big button in the accent."""
    root = empty("remote")
    tilt = empty("tilt", (0, 0, 0), (60, 0, -18), root)
    box((0.9, 2.3, 0.3), (0, 0, 0.15), r=0.18, mat="white", parent=tilt)
    box((0.4, 0.16, 0.05), (0, 1.02, 0.29), r=0.06, mat="dark", parent=tilt, segments=2)
    cyl(0.22, 0.12, (0, 0.5, 0.3), r=0.04, mat="accent", parent=tilt)
    for row in range(3):
        for col in range(3):
            cyl(0.08, 0.08, (-0.25 + col * 0.25, 0.0 - row * 0.28, 0.3), r=0.025, mat="soft", parent=tilt, verts=24)
    return root


def app_level():
    """Level: a spirit level, its bubble in the middle of the glass."""
    root = empty("level")
    box((2.6, 0.5, 0.5), (0, 0, 0.25), r=0.08, mat="accent", parent=root)
    box((0.95, 0.08, 0.32), (0, -0.24, 0.27), r=0.06, mat="dark", parent=root)
    cyl(0.12, 0.8, (0, -0.27, 0.27), r=0.05, mat="second_glow", rot=(0, 90, 0), parent=root)
    sphere(0.09, (0.0, -0.36, 0.3), "glow", parent=root, scale=(1.6, 1.0, 0.8))
    for x in (-0.18, 0.18):
        box((0.02, 0.02, 0.3), (x, -0.3, 0.27), r=0.0, mat="graphite", parent=root)
    for x in (-0.95, 0.95):
        box((0.28, 0.06, 0.3), (x, -0.25, 0.25), r=0.04, mat="graphite", parent=root, segments=2)
    return root


def app_compass():
    """Compass: north in the accent."""
    root = empty("compass")
    face = upright((0, 0, 1.0), root)
    cyl(1.0, 0.32, (0, 0, 0), r=0.12, mat="white", parent=face)
    cyl(0.8, 0.06, (0, 0, 0.14), r=0.02, mat="graphite", parent=face)
    for i in range(12):
        a = i * 30
        h = empty("mark", (0, 0, 0.18), (0, 0, a), face)
        box((0.04, 0.12 if i % 3 else 0.2, 0.02), (0, 0.66, 0), r=0.0, mat="soft", parent=h)
    needle = empty("needle", (0, 0, 0.22), (0, 0, -24), face)
    prism([(-0.14, 0), (0, 0.62), (0.14, 0)], 0.08, (0, 0, 0), r=0.025, mat="accent", parent=needle)
    prism([(0.14, 0), (0, -0.62), (-0.14, 0)], 0.08, (0, 0, 0), r=0.025, mat="white", parent=needle)
    cyl(0.08, 0.12, (0, 0, 0.27), r=0.03, mat="metal", parent=face)
    return root


def cloud(parent, loc, k=1.0, mat="white"):
    c = empty("cloud", loc, (0, 0, 0), parent)
    for x, y, r in ((-0.5, 0, 0.42), (0.0, 0.2, 0.58), (0.52, 0.02, 0.44), (0.05, -0.15, 0.4)):
        sphere(r * k, (x * k, 0, y * k), mat, parent=c)
    box((1.6 * k, 0.7 * k, 0.4 * k), (0, 0, -0.18 * k), r=0.2 * k, mat=mat, parent=c, segments=6)
    return c


def app_weather():
    """Weather: the sun, a cloud drifting in front of it."""
    root = empty("weather")
    sphere(0.75, (-0.3, 0.3, 1.35), "second", parent=root)
    cloud(root, (0.25, -0.35, 0.55), 1.0)
    return root


def app_browse():
    """Browse: a magnifier over a page."""
    root = empty("browse")
    face = upright((0, 0, 1.0), root)
    page(face, 1.3, 1.7, 0.36, 0.1, "white", (-0.3, 0.1, -0.2), 0)
    for i in range(4):
        box((0.7 if i % 2 else 0.85, 0.08, 0.05), (-0.48, 0.55 - i * 0.25, -0.12), r=0.03, mat="soft", parent=face, segments=2)
    lens = empty("lens", (0.25, -0.25, 0.25), (0, 0, 0), face)
    torus(0.48, 0.1, (0, 0, 0), "accent", parent=lens)
    cyl(0.44, 0.04, (0, 0, 0), r=0.0, mat="dark", parent=lens)
    h = empty("handle", (0, 0, 0), (0, 0, -45), lens)
    box((0.2, 0.75, 0.2), (0, -0.85, 0), r=0.09, mat="graphite", parent=h)
    return root


def app_calculator():
    """Calculator: its display lit, = in the accent."""
    root = empty("calculator")
    tilt = empty("tilt", (0, 0, 0), (62, 0, 0), root)
    box((1.4, 2.0, 0.24), (0, 0, 0.12), r=0.14, mat="graphite", parent=tilt)
    box((1.1, 0.45, 0.05), (0, 0.68, 0.26), r=0.06, mat="dark", parent=tilt)
    box((0.5, 0.12, 0.03), (0.2, 0.68, 0.29), r=0.03, mat="second_glow", parent=tilt, segments=2)
    for row in range(4):
        for col in range(4):
            mat = "accent" if (row, col) == (3, 3) else ("body" if col == 3 else "white")
            box((0.22, 0.22, 0.1), (-0.42 + col * 0.28, 0.25 - row * 0.28, 0.27), r=0.06, mat=mat, parent=tilt, segments=3)
    return root


def app_clock():
    """Clock: a stopwatch, its hand in the accent."""
    root = empty("clock")
    face = upright((0, 0, 1.0), root)
    cyl(0.95, 0.36, (0, 0, 0), r=0.13, mat="white", parent=face)
    cyl(0.78, 0.05, (0, 0, 0.17), r=0.02, mat="dark", parent=face)
    cyl(0.12, 0.2, (0, 1.06, 0), r=0.04, mat="graphite", rot=(90, 0, 0), parent=face)
    box((0.36, 0.14, 0.2), (0, 1.2, 0), r=0.06, mat="accent", parent=face, segments=3)
    h = empty("side", (0, 0, 0), (0, 0, -42), face)
    cyl(0.07, 0.16, (0, 1.0, 0), r=0.03, mat="graphite", rot=(90, 0, 0), parent=h)
    for i in range(12):
        m = empty("mark", (0, 0, 0.2), (0, 0, i * 30), face)
        box((0.04, 0.12, 0.02), (0, 0.62, 0), r=0.0, mat="soft", parent=m)
    hand = empty("hand", (0, 0, 0.23), (0, 0, -60), face)
    box((0.07, 0.6, 0.04), (0, 0.25, 0), r=0.03, mat="accent", parent=hand, segments=2)
    cyl(0.07, 0.08, (0, 0, 0.24), r=0.02, mat="accent", parent=face)
    return root


def app_notes():
    """Notes: a pad of notes in the second tint, a pencil across it."""
    root = empty("notes")
    tilt = empty("tilt", (0, 0, 0), (58, 0, 0), root)
    box((1.55, 1.55, 0.3), (0, 0, 0.15), r=0.06, mat="body", parent=tilt)
    box((1.55, 1.55, 0.06), (0, 0, 0.33), r=0.05, mat="second", parent=tilt)
    for i in range(3):
        box((1.0 - i * 0.15, 0.06, 0.02), (-0.15 + i * 0.07, 0.35 - i * 0.28, 0.37), r=0.0, mat="soft", parent=tilt)
    pen = empty("pencil", (0.15, -0.35, 0.5), (0, 0, 32), tilt)
    cyl(0.09, 1.5, (0, 0, 0), r=0.0, mat="accent", rot=(0, 90, 0), parent=pen, verts=6)
    cyl(0.09, 0.22, (0.86, 0, 0), r=0.0, mat="body", rot=(0, 90, 0), parent=pen, radius2=0.02, verts=6)
    cyl(0.095, 0.16, (-0.82, 0, 0), r=0.02, mat="metal", rot=(0, 90, 0), parent=pen)
    return root


def app_units():
    """Units: a tape measure, a little of the tape out."""
    root = empty("units")
    face = upright((0, 0, 0.95), root)
    box((1.5, 1.5, 0.6), (0, 0, 0), r=0.42, mat="accent", parent=face, segments=8)
    cyl(0.42, 0.06, (0, 0, 0.31), r=0.02, mat="graphite", parent=face)
    box((0.3, 0.22, 0.5), (0.68, -0.6, 0), r=0.06, mat="graphite", parent=face)
    box((1.1, 0.04, 0.32), (1.25, -0.68, 0), r=0.01, mat="second", parent=face)
    for i in range(6):
        box((0.02, 0.03, 0.14), (0.8 + i * 0.16, -0.66, 0.12), r=0.0, mat="graphite", parent=face)
    box((0.06, 0.18, 0.36), (1.8, -0.6, 0), r=0.02, mat="metal", parent=face)
    return root


def app_http():
    """HTTP: braces around a request on its way."""
    root = empty("http")
    face = upright((0, 0, 1.0), root)
    text("{", 2.1, 0.42, (-0.6, 0.08, 0), "white", parent=face, font=SANS, bevel_depth=0.012)
    text("}", 2.1, 0.42, (0.6, 0.08, 0), "white", parent=face, font=SANS, bevel_depth=0.012)
    sphere(0.16, (-0.17, 0, 0), "accent", parent=face)
    sphere(0.16, (0.17, 0, 0), "accent", parent=face)
    return root


def app_share():
    """Share: a QR code, its three corners in the accent."""
    root = empty("share")
    face = upright((0, 0, 1.0), root)
    box((1.7, 1.7, 0.12), (0, 0, 0), r=0.12, mat="white", parent=face)
    rng = random.Random(254)
    cell = 1.36 / 11
    finder = lambda i, j: (i < 3 and j < 3) or (i > 7 and j < 3) or (i < 3 and j > 7)
    for i in range(11):
        for j in range(11):
            if not finder(i, j) and rng.random() < 0.48:
                box((cell * 0.92, cell * 0.92, 0.06), (-0.68 + (i + 0.5) * cell, -0.68 + (j + 0.5) * cell, 0.07), r=0.012, mat="graphite", parent=face, segments=2)
    for cx, cy in ((-0.68 + 1.5 * cell, -0.68 + 1.5 * cell), (-0.68 + 9.5 * cell, -0.68 + 1.5 * cell), (-0.68 + 1.5 * cell, -0.68 + 9.5 * cell)):
        box((cell * 3, cell * 3, 0.1), (cx, cy, 0.08), r=0.06, mat="accent", parent=face, segments=3)
        box((cell * 1.6, cell * 1.6, 0.12), (cx, cy, 0.1), r=0.03, mat="white", parent=face, segments=2)
        box((cell * 0.9, cell * 0.9, 0.14), (cx, cy, 0.12), r=0.03, mat="accent", parent=face, segments=2)
    return root


def app_assistant():
    """Assistant: sparkles, lit."""
    root = empty("assistant")
    face = upright((0, 0, 1.0), root)
    prism(sparkle(1.0), 0.32, (-0.15, -0.1, 0), r=0.1, mat="accent_glow", parent=face, segments=5)
    prism(sparkle(0.42), 0.2, (0.72, 0.62, 0.05), r=0.06, mat="white", parent=face)
    prism(sparkle(0.26), 0.16, (0.78, -0.55, 0.05), r=0.04, mat="second", parent=face)
    return root


def app_web():
    """Web: a globe drawn in lines of latitude and longitude."""
    root = empty("web")
    sphere(0.95, (0, 0, 1.0), "white", parent=root)
    for a in (0, 60, 120):
        torus(0.97, 0.035, (0, 0, 1.0), "accent" if a == 0 else "soft", rot=(90, 0, a), parent=root)
    for z, r in ((0.0, 0.97), (0.5, 0.84), (-0.5, 0.84)):
        torus(r, 0.035, (0, 0, 1.0 + z), "soft", parent=root)
    return root


def app_settings():
    """Settings: a gear, its hub in the accent."""
    root = empty("settings")
    face = upright((0, 0, 1.0), root)
    prism(gear_outline(10, 1.0, 0.78), 0.42, (0, 0, 0), r=0.08, mat="white", parent=face, segments=4)
    cyl(0.36, 0.5, (0, 0, 0.04), r=0.08, mat="accent", parent=face)
    cyl(0.16, 0.56, (0, 0, 0.05), r=0.04, mat="graphite", parent=face)
    return root


# =========================================================== generated ===

def generated(name, about):
    def build():
        return import_model(os.path.join(HERE, "glb", name + ".glb"))
    build.__doc__ = about
    return build


A = dict

ASSETS = {
    # Setup's pages, and the system's big moments.
    "handheld": A(build=handheld, **HERO_AND_THING, tints=["signal", "amber"], about="Setup's welcome, and its last page; About's header: the handheld itself"),
    "wifi": A(build=wifi, **HERO, tints=["signal", "ice"], about="Setup: Wi-Fi"),
    "chip": A(build=chip, **HERO, tints=["signal", "ice"], about="Setup: the hardware check"),
    "keycaps": A(build=keycaps, **HERO, tints=["signal", "amber"], about="Setup: the keyboard lesson"),
    "key": A(build=key, **HERO, tints=["signal", "ice"], about="Setup: keys for online services"),
    "toggle": A(build=toggle, **HERO, tints=["signal", "ice"], about="Setup: choices"),
    "battery": A(build=battery, **HERO_AND_THING, tints=["signal", "ice"], about="Setup: battery and power; Settings › Battery"),
    "palette": A(build=palette, **HERO, tints=["signal", "ice"], about="Setup: the look"),
    "moon": A(build=moon, **HERO_AND_THING, tints=["signal", "amber"], about="Deep standby, and the screen going off"),
    "tray": A(build=empty_tray, **THING, tints=["signal", "ice"], about="Empty states: nothing here yet"),
    "sunmoon": A(build=sunmoon, sizes=[["s", 30, list(range(0, 181, 15)), [0]], ["m", 60, ARC, [0]]], res=240, tints=["signal", "amber"], about="The control centre's Light: a sun, and a moon half a turn behind it"),
    "robot": A(build=generated("robot", "An FRC robot"), **HERO, tints=["signal", "ice"], source="higgsfield", about="Setup: the team; a robot arriving"),
    "globe": A(build=generated("globe", "A desk globe"), **HERO, tints=["signal", "ice"], source="higgsfield", about="Setup: name and time zone"),
    "laptop": A(build=generated("laptop", "A laptop"), **HERO, tints=["signal", "ice"], source="higgsfield", about="Setup from a computer"),
    "trophy": A(build=generated("trophy", "A trophy"), **HERO, tints=["signal", "amber"], source="higgsfield", about="You're all set; the match report"),
    "rocket": A(build=generated("rocket", "A rocket"), **HERO, tints=["signal", "ice"], source="higgsfield", about="System update: ready to install"),
    # Home's apps, by name.
    "app-robot": A(build=app_robot, **ICON, tints=["signal", "ice"], about="Robot"),
    "app-health": A(build=app_health, **ICON, tints=["signal", "ice"], about="Health"),
    "app-logs": A(build=app_logs, **ICON, tints=["signal", "ice"], about="Logs"),
    "app-schedule": A(build=app_schedule, **ICON, tints=["signal", "ice"], about="Schedule"),
    "app-tables": A(build=app_tables, **ICON, tints=["signal", "ice"], about="Tables"),
    "app-loop": A(build=app_loop, **ICON, tints=["signal", "ice"], about="Loop"),
    "app-motors": A(build=app_motors, **ICON, tints=["signal", "ice"], about="Motors"),
    "app-console": A(build=app_console, **ICON, tints=["signal", "ice"], about="Console"),
    "app-terminal": A(build=app_terminal, **ICON, tints=["signal", "ice"], about="Terminal"),
    "app-files": A(build=app_files, **ICON, tints=["signal", "ice"], about="Files"),
    "app-git": A(build=app_git, **ICON, tints=["signal", "ice"], about="Git"),
    "app-catalogue": A(build=app_catalogue, **ICON, tints=["signal", "ice"], about="Catalogue"),
    "app-monitor": A(build=app_monitor, **ICON, tints=["signal", "ice"], about="Monitor"),
    "app-nettools": A(build=app_nettools, **ICON, tints=["signal", "ice"], about="Net tools"),
    "app-connect": A(build=app_connect, **ICON, tints=["signal", "ice"], about="Connect"),
    "app-actions": A(build=app_actions, **ICON, tints=["signal", "ice"], about="Actions"),
    "app-camera": A(build=app_camera, **ICON, tints=["signal", "ice"], about="Camera"),
    "app-photos": A(build=app_photos, **ICON, tints=["signal", "amber"], about="Photos"),
    "app-apriltag": A(build=app_apriltag, **ICON, tints=["signal", "ice"], about="AprilTag"),
    "app-graphs": A(build=app_graphs, **ICON, tints=["signal", "ice"], about="Graphs"),
    "app-pit": A(build=app_pit, **ICON, tints=["signal", "ice"], about="Pit"),
    "app-remote": A(build=app_remote, **ICON, tints=["signal", "ice"], about="Remote"),
    "app-level": A(build=app_level, **ICON, tints=["signal", "ice"], about="Level"),
    "app-compass": A(build=app_compass, **ICON, tints=["signal", "ice"], about="Compass"),
    "app-weather": A(build=app_weather, **ICON, tints=["signal", "amber"], about="Weather"),
    "app-browse": A(build=app_browse, **ICON, tints=["signal", "ice"], about="Browse"),
    "app-calculator": A(build=app_calculator, **ICON, tints=["signal", "ice"], about="Calculator"),
    "app-clock": A(build=app_clock, **ICON, tints=["signal", "ice"], about="Clock"),
    "app-notes": A(build=app_notes, **ICON, tints=["signal", "amber"], about="Notes"),
    "app-units": A(build=app_units, **ICON, tints=["signal", "ice"], about="Units"),
    "app-http": A(build=app_http, **ICON, tints=["signal", "ice"], about="HTTP"),
    "app-share": A(build=app_share, **ICON, tints=["signal", "ice"], about="Share"),
    "app-assistant": A(build=app_assistant, **ICON, tints=["signal", "amber"], about="Assistant"),
    "app-web": A(build=app_web, **ICON, tints=["signal", "ice"], about="Web"),
    "app-settings": A(build=app_settings, **ICON, tints=["signal", "ice"], about="Settings"),
}

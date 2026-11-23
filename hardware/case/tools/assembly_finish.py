"""Lay out the picture assembly guide from tools/assembly.py's renders.

usage: python tools/assembly_finish.py RAW_DIR OUT_DIR
  RAW_DIR  renders + manifest.json from tools/assembly.py
  OUT_DIR  gets 00-parts.png, 01-...png (one page per step), assembly-guide.pdf and
           assembly-guide-phone.png (one tall overview, 1080 wide, under 4000 px)
"""
import json, math, os, sys
from PIL import Image, ImageDraw, ImageFilter, ImageFont

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FONT = os.path.join(HERE, "..", "..", "userland", "assets", "fonts", "GoogleSansFlex.ttf")
W, H = 1600, 1200
SS = 2                                   # overlay supersampling

INK, INK2, MUTED = (21, 24, 27), (60, 64, 70), (104, 109, 116)
GROUND, SURFACE = (232, 234, 237), (251, 251, 252)
SIGNAL, AMBER, BLUE = (255, 91, 31), (255, 194, 26), (21, 101, 216)
GREY_SW = (142, 145, 150)

# One page per step. Captions: 8 words at most. Notes: ("warn" | "info" | "noscreen", short text).
PLAIN_PAGES = [
    dict(kind="parts", file="plain-00-parts"),
    dict(n=1, shot="s01", file="plain-01-nuts", caption="Optional: drop 2 nuts in, flat side up",
         notes=[("info", "Only for the M2 × 3 screws"), ("info", "Already built? Go to step 3")]),
    dict(n=2, shot="s02", file="plain-02-chassis", caption="Screw the chassis on, window over camera",
         notes=[("warn", "Snug, don't force"), ("info", "Already built? Go to step 3")]),
    dict(n=3, shot="g1", file="plain-03-glue", caption="Optional: a small glue dab per corner",
         notes=[("noscreen", "Keep away from the screen"), ("warn", "Never on the handheld"),
                ("info", "Back to white chassis only")]),
    dict(n=4, shot="q4", file="plain-04-device", caption="Press the device in until both hooks click",
         notes=[("warn", "Don't press the screen")]),
    dict(n=5, shot="q5", file="plain-05-screws", caption="Optional: fit 2 screws in the bottom corners",
         notes=[("info", "Only if you have M2 nuts"), ("warn", "Snug, don't force")]),
    dict(n=6, shot="q6", file="plain-06-bezel", caption="Snap the bezel on: 5 clicks",
         notes=[("info", "Glued? Wait 10 minutes first"), ("warn", "Don't press the screen")]),
]
PLAIN_PARTS = [
    ("part-device", "CardputerZero", 1, "the device", "CardputerZero"),
    ("part-chassis", "Chassis", 1, "white PETG", "Chassis"),
    ("part-csk", "M2 × 4 countersunk", 2, "chassis to device", "M2 × 4"),
    ("part-plain_back", "Plain backplate", 1, "black PLA", "Plain back"),
    ("part-bezel", "Bezel", 1, "black PLA", "Bezel"),
    ("part-nut", "M2 hex nut", 2, "optional", "M2 nut, optional"),
    ("part-btn", "M2 × 3 button head", 2, "optional, if you have nuts", "M2 × 3, optional"),
    ("part-glue", "Superglue", 1, "optional", "Glue, optional"),
]

PAGES = [
    dict(kind="parts", file="00-parts"),
    dict(n=1, shot="s01", file="01-nuts", caption="Flat side up: drop in 2 nuts",
         notes=[("info", "Already built? Go to step 3")]),
    dict(n=2, shot="s02", file="02-chassis", caption="Screw the chassis on, window over camera",
         notes=[("warn", "Snug, don't force"), ("info", "Already built? Go to step 3")]),
    dict(n=3, shot="s03", file="03-pins", caption="Press the 2 pins in until they click", notes=[("info", "Short end down")]),
    dict(n=4, shot="s04", file="04-plunger", caption="Drop the plunger in, pin end first"),
    dict(n=5, shot="s05", file="05-cable", caption="Cable under the lips, plug out the side",
         notes=[("info", "UnitV end stays at the bottom")]),
    dict(n=6, shot="s06", file="06-plug-unitv", caption="Plug the cable into the UnitV"),
    dict(n=7, shot="s07", file="07-unitv", caption="Press the UnitV onto the pins: click",
         notes=[("info", "Lens over the round hole")]),
    dict(n=8, shot="s08", file="08-device", caption="Press the device in until both hooks click",
         notes=[("warn", "Don't press the screen")]),
    dict(n=9, shot="s09", file="09-grove-port", caption="Plug the cable into the side port", notes=[("info", "The lower port")]),
    dict(n=10, shot="s10", file="10-screws", caption="Fit 2 screws in the bottom corners", notes=[("warn", "Snug, don't force")]),
    dict(n=11, shot="s11", file="11-bezel", caption="Snap the bezel on: 5 clicks", notes=[("warn", "Don't press the screen")]),
    dict(n=12, shot="s12", file="12-link-cover", caption="Clip the cover over the side hole"),
    dict(n=13, shot="s13", file="13-lens-ring", caption="Press the lens ring into the lens well",
         notes=[("info", "Pull off to swap the lens")]),
    dict(n="Plain", file="14-plain-back", panels=["p1", "p2", "p3"], sub=["Press in: click", "2 screws", "Bezel: 5 clicks"],
         caption="Plain back: device in, 2 screws, bezel", notes=[("warn", "Don't press the screen")]),
    dict(n="Swap", file="15-swap-backs", panels=["w1", "w2", "w3"], sub=["Bezel off", "2 screws out", "Rock at the top, lift"],
         caption="Swap backs: bezel off, unscrew, rock out",
         notes=[("info", "Chassis stays on the device"), ("info", "Camera back: unplug the cable first")]),
]

PARTS = [  # thumb, label, count, sub, short label for the phone
    ("part-device", "CardputerZero", 1, "the device", "CardputerZero"),
    ("part-chassis", "Chassis", 1, "white PETG", "Chassis"),
    ("part-nut", "M2 hex nut", 2, "in the chassis", "M2 nut"),
    ("part-csk", "M2 × 4 countersunk", 2, "chassis to device", "M2 × 4"),
    ("part-unitv", "UnitV K210", 1, "camera", "UnitV"),
    ("part-cable", "Grove cable", 1, "10 cm", "Grove cable"),
    ("part-camera_back", "Camera backplate", 1, "black PLA", "Camera back"),
    ("part-pins", "UnitV pins", 2, "print 2 spares too", "Pins"),
    ("part-plunger", "Plunger", 1, "white PETG", "Plunger"),
    ("part-btn", "M2 × 3 button head", 2, "backplate to chassis", "M2 × 3"),
    ("part-bezel", "Bezel", 1, "black PLA", "Bezel"),
    ("part-link", "Link cover", 1, "black PLA", "Link cover"),
    ("part-lens_ring", "Lens ring", 1, "black PLA", "Lens ring"),
    ("part-plain_back", "Plain backplate", 1, "optional, black PLA", "Plain back"),
]

_fonts = {}


def font(size, weight=400):
    k = (size, weight)
    if k not in _fonts:
        f = ImageFont.truetype(FONT, size)
        try:
            f.set_variation_by_axes([min(72, max(6, size * 0.75)), 100, weight, 0])
        except Exception:
            pass
        _fonts[k] = f
    return _fonts[k]


def backdrop(w, h):
    bg = Image.new("RGB", (w, h), GROUND)
    glow = Image.new("L", (w, h), 0)
    ImageDraw.Draw(glow).ellipse([w * 0.12, h * 0.05, w * 0.88, h * 0.85], fill=150)
    glow = glow.filter(ImageFilter.GaussianBlur(w * 0.08))
    return Image.composite(Image.new("RGB", (w, h), (247, 248, 249)), bg, glow).convert("RGBA")


# ---------------------------------------------------------------- icons
def icon_warn(d, x, y, s):
    d.polygon([(x + s / 2, y), (x + s, y + s * 0.9), (x, y + s * 0.9)], fill=AMBER, outline=INK, width=max(2, int(s / 14)))
    f = font(int(s * 0.62), 800)
    d.text((x + s / 2, y + s * 0.55), "!", font=f, fill=INK, anchor="mm")


def icon_info(d, x, y, s):
    d.ellipse([x, y, x + s, y + s], fill=BLUE)
    d.text((x + s / 2, y + s * 0.53), "i", font=font(int(s * 0.66), 800), fill=(255, 255, 255), anchor="mm")


def burst(d, x, y, r, width, color=INK):
    for k in range(8):
        a = k * math.pi / 4 + math.pi / 8
        d.line([(x + math.cos(a) * r * 0.45, y + math.sin(a) * r * 0.45), (x + math.cos(a) * r, y + math.sin(a) * r)],
               fill=color, width=width)


def icon_noscreen(d, x, y, s):
    """A screen with a glue drop, struck through: keep glue away from the screen."""
    w = max(2, int(s / 12))
    d.rounded_rectangle([x + s * 0.12, y + s * 0.22, x + s * 0.88, y + s * 0.78], radius=s * 0.08, fill=(40, 44, 50), outline=INK, width=w)
    dx, dy = x + s * 0.5, y + s * 0.52
    d.polygon([(dx, dy - s * 0.2), (dx - s * 0.11, dy + s * 0.02), (dx + s * 0.11, dy + s * 0.02)], fill=(250, 226, 120))
    d.ellipse([dx - s * 0.11, dy - s * 0.06, dx + s * 0.11, dy + s * 0.15], fill=(250, 226, 120))
    red = (211, 47, 47)
    d.ellipse([x, y, x + s, y + s], outline=red, width=max(3, int(s / 9)))
    d.line([(x + s * 0.17, y + s * 0.83), (x + s * 0.83, y + s * 0.17)], fill=red, width=max(3, int(s / 9)))



def chip(d, x, y, kind, text, size):
    """Icon + short text; returns the width used."""
    s = int(size * 1.15)
    {"warn": icon_warn, "info": icon_info, "noscreen": icon_noscreen}[kind](d, x, y + (size * 1.3 - s) / 2, s)
    f = font(size, 600)
    d.text((x + s + size * 0.35, y + size * 0.65), text, font=f, fill=INK, anchor="lm")
    return s + size * 0.35 + d.textlength(text, font=f)


# ---------------------------------------------------------------- picture layer
def scaled(p, k):
    return (p[0] * k, p[1] * k)


def draw_arrow(d, pts, k, width=15, head=50, color=BLUE, min_len=120):
    pts = [scaled(p, k) for p in pts]
    if len(pts) == 2:     # foreshortened arrows still read as arrows
        (x0, y0), (x1, y1) = pts
        L = math.hypot(x1 - x0, y1 - y0)
        if L < min_len * k and L > 1e-3:      # grow toward the target, never back over the part
            ext = (min_len * k - L)
            ux, uy = (x1 - x0) / L, (y1 - y0) / L
            pts = [(x0, y0), (x1 + ux * ext, y1 + uy * ext)]
    (xa, ya), (xb, yb) = pts[-2], pts[-1]
    L = math.hypot(xb - xa, yb - ya) or 1
    ux, uy = (xb - xa) / L, (yb - ya) / L
    hl, hw = head * k, head * k * 0.95
    base = (xb - ux * hl, yb - uy * hl)
    shaft = pts[:-1] + [(xb - ux * hl * 0.6, yb - uy * hl * 0.6)]
    tri = [(xb, yb), (base[0] - uy * hw / 2, base[1] + ux * hw / 2), (base[0] + uy * hw / 2, base[1] - ux * hw / 2)]
    for col, extra in (((255, 255, 255), 7 * k), (color, 0)):
        d.line(shaft, fill=col, width=int(width * k + 2 * extra), joint="curve")
        if extra:
            r = (width * k + 2 * extra) / 2
            d.ellipse([shaft[0][0] - r, shaft[0][1] - r, shaft[0][0] + r, shaft[0][1] + r], fill=col)
            cx, cy = sum(p[0] for p in tri) / 3, sum(p[1] for p in tri) / 3
            big = [(cx + (px - cx) * 1.0 + (px - cx) / max(1e-6, math.hypot(px - cx, py - cy)) * extra * 1.6,
                    cy + (py - cy) * 1.0 + (py - cy) / max(1e-6, math.hypot(px - cx, py - cy)) * extra * 1.6) for px, py in tri]
            d.polygon(big, fill=col)
        else:
            r = width * k / 2
            d.ellipse([shaft[0][0] - r, shaft[0][1] - r, shaft[0][0] + r, shaft[0][1] + r], fill=col)
            d.polygon(tri, fill=col)


def draw_click(d, at, k, label=True):
    x, y = scaled(at, k)
    burst(d, x, y, 26 * k, int(5 * k), (255, 255, 255))
    burst(d, x, y, 24 * k, int(3.5 * k))
    if label:
        f = font(int(27 * k), 700)
        d.text((x + 30 * k, y - 22 * k), "click", font=f, fill=INK, anchor="ls", stroke_width=int(4 * k), stroke_fill=(255, 255, 255))


def crop_alpha(im, pad=6):
    bb = im.getchannel("A").point(lambda a: 255 if a > 8 else 0).getbbox()
    if not bb:
        return im
    return im.crop((max(0, bb[0] - pad), max(0, bb[1] - pad), min(im.width, bb[2] + pad), min(im.height, bb[3] + pad)))


def fit_into(im, w, h):
    s = min(w / im.width, h / im.height)
    return im.resize((max(1, int(im.width * s)), max(1, int(im.height * s))), Image.LANCZOS)


def callout_spot(alpha, anchor, r, taken):
    """Pick a circle centre in a free corner area, near the anchor."""
    cands = [(W - 60 - r, 245 + r * 0.2), (W - 60 - r, 560), (W - 60 - r, 960 - r), (60 + r, 300 + r),
             (60 + r, 960 - r), (W / 2 + 260, 245 + r * 0.2), (W / 2 - 260, 960 - r)]
    best, score = None, 1e18
    a = alpha.resize((W // 8, H // 8))
    px = a.load()
    for cx, cy in cands:
        cover = 0
        for yy in range(int((cy - r) / 8), int((cy + r) / 8)):
            for xx in range(int((cx - r) / 8), int((cx + r) / 8)):
                if 0 <= xx < W // 8 and 0 <= yy < H // 8 and (xx * 8 - cx) ** 2 + (yy * 8 - cy) ** 2 < r * r:
                    cover += px[xx, yy] > 20
        clash = any(math.hypot(cx - tx, cy - ty) < r * 2.1 for tx, ty in taken)
        sc = cover * 60 + math.hypot(cx - anchor[0], cy - anchor[1]) * 0.6 + (1e9 if clash else 0)
        if sc < score:
            best, score = (cx, cy), sc
    return best


def draw_callout(img, d, c, raw, k, taken, alpha):
    r = 150
    ax, ay = c["at"]
    cx, cy = callout_spot(alpha, (ax, ay), r, taken)
    taken.append((cx, cy))
    # leader: anchor dot -> circle edge
    L = math.hypot(cx - ax, cy - ay) or 1
    ex, ey = cx - (cx - ax) / L * r, cy - (cy - ay) / L * r
    d.line([scaled((ax, ay), k), scaled((ex, ey), k)], fill=(255, 255, 255), width=int(8 * k))
    d.line([scaled((ax, ay), k), scaled((ex, ey), k)], fill=INK, width=int(3.5 * k))
    d.ellipse([(ax - 8) * k, (ay - 8) * k, (ax + 8) * k, (ay + 8) * k], fill=INK, outline=(255, 255, 255), width=int(3 * k))
    d.ellipse([(cx - r) * k, (cy - r) * k, (cx + r) * k, (cy + r) * k], fill=(255, 255, 255), outline=INK, width=int(5 * k))
    th = Image.open(os.path.join(raw, c["thumb"] + ".png")).convert("RGBA")
    th = fit_into(crop_alpha(th), int(1.25 * r * k), int(1.0 * r * k))
    img.alpha_composite(th, (int(cx * k - th.width / 2), int((cy - 22) * k - th.height / 2)))
    d.text((cx * k, (cy + r * 0.66) * k), c["label"], font=font(int(40 * k), 700), fill=INK, anchor="mm")
    if c.get("count", 1) > 1:
        bx, by = (cx + r * 0.72) * k, (cy - r * 0.78) * k
        t = f"{c['count']}×"
        f = font(int(40 * k), 800)
        tw = d.textlength(t, font=f)
        d.rounded_rectangle([bx - tw / 2 - 18 * k, by - 30 * k, bx + tw / 2 + 18 * k, by + 30 * k], radius=30 * k, fill=INK)
        d.text((bx, by + 1 * k), t, font=f, fill=(255, 255, 255), anchor="mm")


def draw_mark(d, m, k):
    ax, ay = m["at"]
    tx, ty = ax - 150, ay - 90
    d.line([scaled((ax, ay), k), scaled((tx, ty), k)], fill=INK, width=int(3 * k))
    d.ellipse([(ax - 7) * k, (ay - 7) * k, (ax + 7) * k, (ay + 7) * k], fill=INK, outline=(255, 255, 255), width=int(3 * k))
    f = font(int(30 * k), 600)
    tw = d.textlength(m["text"], font=f)
    d.rounded_rectangle([(tx * k - tw - 32 * k), (ty - 24) * k, tx * k, (ty + 24) * k], radius=24 * k, fill=(255, 255, 255), outline=INK, width=int(3 * k))
    d.text((tx * k - tw / 2 - 16 * k, ty * k), m["text"], font=f, fill=INK, anchor="mm")


def picture(raw, man, key, bg=True):
    """The render with its arrows, clicks, callouts: 1600 x 1200 RGBA."""
    e = man[key]
    ren = Image.open(os.path.join(raw, key + ".png")).convert("RGBA")
    if ren.size != (W, H):
        ren = ren.resize((W, H), Image.LANCZOS)
    base = backdrop(W, H) if bg else Image.new("RGBA", (W, H), (0, 0, 0, 0))
    base.alpha_composite(ren)
    k = SS
    over = Image.new("RGBA", (W * k, H * k), (0, 0, 0, 0))
    d = ImageDraw.Draw(over)
    for a in e["arrows"]:
        draw_arrow(d, a, k)
    for c in e["clicks"]:
        draw_click(d, c, k, label=len(e["clicks"]) <= 2)
    if len(e["clicks"]) > 2:    # many clicks: one label
        x, y = min(e["clicks"], key=lambda p: p[1])
        d.text(((x - 30) * k, (y - 22) * k), f"{len(e['clicks'])} clicks", font=font(int(27 * k), 700), fill=INK,
               anchor="rs", stroke_width=int(4 * k), stroke_fill=(255, 255, 255))
    for m in e.get("marks", []):
        draw_mark(d, m, k)
    taken = []
    for c in e["callouts"]:
        draw_callout(over, d, c, raw, k, taken, ren.getchannel("A"))
    base.alpha_composite(over.resize((W, H), Image.LANCZOS))
    return base


# ---------------------------------------------------------------- pages
def page_frame(img, num, caption, notes, idx, total, label="Catalyst case rev 3.1"):
    d = ImageDraw.Draw(img)
    if num is not None:
        txt = str(num)
        f = font(150 if isinstance(num, int) else 92, 760)
        d.text((64, 40), txt, font=f, fill=INK, anchor="la")
    y0 = 1000
    d.rounded_rectangle([36, y0, W - 36, H - 30], radius=28, fill=SURFACE, outline=(214, 217, 221), width=2)
    d.text((80, y0 + 56), caption, font=font(58, 640), fill=INK, anchor="lm")
    x = 82
    for kind, text in notes or []:
        x += chip(d, x, y0 + 100, kind, text, 34) + 38
    d.text((W - 64, 58), f"{label} · {idx}/{total}", font=font(24, 500), fill=MUTED, anchor="ra")
    return img


def panels_picture(raw, man, keys, subs):
    img = backdrop(W, H)
    pw = (W - 80) // 3
    d = ImageDraw.Draw(img)
    for i, (key, sub) in enumerate(zip(keys, subs)):
        p = picture(raw, man, key, bg=False)
        p = fit_into(crop_alpha(p, 20), pw - 30, 660)
        x0 = 40 + i * pw
        d.rounded_rectangle([x0 + 8, 200, x0 + pw - 8, 960], radius=26, fill=(255, 255, 255, 140), outline=(214, 217, 221), width=2)
        img.alpha_composite(p, (int(x0 + (pw - p.width) / 2), int(250 + (680 - p.height) / 2)))
        d = ImageDraw.Draw(img)
        d.ellipse([x0 + 26, 218, x0 + 86, 278], fill=INK)
        d.text((x0 + 56, 249), "abc"[i], font=font(38, 760), fill=(255, 255, 255), anchor="mm")
        d.text((x0 + 104, 249), sub, font=font(36, 640), fill=INK, anchor="lm")
    return img


def parts_page(raw, parts, subtitle):
    img = backdrop(W, H)
    d = ImageDraw.Draw(img)
    d.text((64, 52), "What you need", font=font(86, 760), fill=INK, anchor="la")
    d.text((68, 160), subtitle, font=font(28, 500), fill=MUTED, anchor="la")
    cols, cw, ch, gx, gy = (5, 300, 300, 50, 225) if len(parts) > 8 else (3, 500, 300, 50, 225)
    cells = parts + [None]
    for i, it in enumerate(cells):
        cx, cy = gx + (i % cols) * cw, gy + (i // cols) * ch
        d.rounded_rectangle([cx + 6, cy + 6, cx + cw - 6, cy + ch - 6], radius=22, fill=(255, 255, 255), outline=(220, 222, 226), width=2)
        if it is None:
            legend(d, cx + 26, cy + 30)
            continue
        th = fit_into(crop_alpha(Image.open(os.path.join(raw, it[0] + ".png")).convert("RGBA")), cw - 56, 186)
        img.alpha_composite(th, (int(cx + (cw - th.width) / 2), int(cy + 14 + (190 - th.height) / 2)))
        d = ImageDraw.Draw(img)
        d.text((cx + 24, cy + 222), it[1], font=font(27, 680), fill=INK, anchor="lm")
        d.text((cx + 24, cy + 258), it[3], font=font(23, 500), fill=MUTED, anchor="lm")
        if it[2] > 1:
            t = f"{it[2]}×"
            f = font(34, 800)
            tw = d.textlength(t, font=f)
            d.rounded_rectangle([cx + cw - 30 - tw - 28, cy + 22, cx + cw - 30, cy + 74], radius=26, fill=INK)
            d.text((cx + cw - 30 - tw / 2 - 14, cy + 49), t, font=f, fill=(255, 255, 255), anchor="mm")
    return img


def legend(d, x, y, size=26):
    f = font(size, 600)
    rows = [("sw_hi", "add this now"), ("sw_grey", "already in place"), ("arrow", "push this way"),
            ("click", "it clicks in"), ("warn", "careful")]
    d.text((x, y + 4), "How to read", font=font(int(size * 1.08), 760), fill=INK, anchor="la")
    for i, (kind, text) in enumerate(rows):
        yy = y + 58 + i * size * 1.75
        if kind == "sw_hi":
            d.rounded_rectangle([x, yy - 14, x + 44, yy + 14], radius=7, fill=SIGNAL, outline=INK, width=2)
        elif kind == "sw_grey":
            d.rounded_rectangle([x, yy - 14, x + 44, yy + 14], radius=7, fill=GREY_SW, outline=INK, width=2)
        elif kind == "arrow":
            d.line([(x, yy), (x + 26, yy)], fill=BLUE, width=9)
            d.polygon([(x + 44, yy), (x + 24, yy - 13), (x + 24, yy + 13)], fill=BLUE)
        elif kind == "click":
            burst(d, x + 22, yy, 17, 4)
        else:
            icon_warn(d, x + 4, yy - 17, 36)
        d.text((x + 60, yy), text, font=f, fill=INK, anchor="lm")


# ---------------------------------------------------------------- phone overview
def phone(raw, man, pics, out, pages, parts, title, subtitle):
    PW, col = 1080, 540
    pc = 5 if len(parts) > 8 else 3                       # parts per row
    rows_steps = math.ceil((len(pages) - 1) / 2)
    head, cell_h = 118, (392 if rows_steps > 4 else 520)
    pic_h = 262 if rows_steps > 4 else 300
    tw_, th_ = (PW - 40) // pc, 196
    parts_h = 56 + math.ceil((len(parts) + 1) / pc) * th_ + 24
    total_h = head + parts_h + rows_steps * cell_h + 20
    img = Image.new("RGBA", (PW, total_h), GROUND + (255,))
    d = ImageDraw.Draw(img)
    d.text((32, 30), title, font=font(48, 760), fill=INK, anchor="la")
    d.text((34, 88), subtitle, font=font(24, 500), fill=MUTED, anchor="la")
    y0 = head
    d.text((32, y0 + 10), "What you need", font=font(32, 720), fill=INK, anchor="la")
    for i, it in enumerate(parts):
        cx, cy = 20 + (i % pc) * tw_, y0 + 56 + (i // pc) * th_
        d.rounded_rectangle([cx + 4, cy + 4, cx + tw_ - 4, cy + th_ - 4], radius=16, fill=(255, 255, 255))
        t = fit_into(crop_alpha(Image.open(os.path.join(raw, it[0] + ".png")).convert("RGBA")), tw_ - 30, 112)
        img.alpha_composite(t, (int(cx + (tw_ - t.width) / 2), int(cy + 12 + (116 - t.height) / 2)))
        d = ImageDraw.Draw(img)
        lab = it[4] + (f"  {it[2]}×" if it[2] > 1 else "")
        d.text((cx + tw_ / 2, cy + 156), lab, font=font(25, 680), fill=INK, anchor="mm")
    # legend in the cell after the parts
    cx, cy = 20 + (len(parts) % pc) * tw_, y0 + 56 + (len(parts) // pc) * th_
    d.rounded_rectangle([cx + 4, cy + 4, cx + tw_ - 4, cy + th_ - 4], radius=16, fill=(255, 255, 255))
    for j, (kind, text) in enumerate([("hi", "add now"), ("arrow", "push"), ("warn", "careful")]):
        yy = cy + 40 + j * 52
        if kind == "hi":
            d.rounded_rectangle([cx + 16, yy - 12, cx + 52, yy + 12], radius=6, fill=SIGNAL, outline=INK, width=2)
        elif kind == "arrow":
            d.line([(cx + 16, yy), (cx + 36, yy)], fill=BLUE, width=8)
            d.polygon([(cx + 54, yy), (cx + 34, yy - 12), (cx + 34, yy + 12)], fill=BLUE)
        else:
            icon_warn(d, cx + 18, yy - 16, 32)
        d.text((cx + 64, yy), text, font=font(24, 620), fill=INK, anchor="lm")
    # steps: 2 columns
    y0 = head + parts_h
    for i, pg in enumerate(pages[1:]):
        cx, cy = (i % 2) * col, y0 + (i // 2) * cell_h
        d.rounded_rectangle([cx + 12, cy + 8, cx + col - 12, cy + cell_h - 8], radius=20, fill=(255, 255, 255))
        pic = crop_alpha(pics[i], 10)
        pic = fit_into(pic, col - 40, pic_h)
        img.alpha_composite(pic, (int(cx + (col - pic.width) / 2), int(cy + 20 + (pic_h - pic.height) / 2)))
        d = ImageDraw.Draw(img)
        num = str(pg["n"])
        f = font(34 if len(num) < 3 else 24, 780)
        bw = max(56, d.textlength(num, font=f) + 26)
        d.rounded_rectangle([cx + 24, cy + 20, cx + 24 + bw, cy + 76], radius=28, fill=INK)
        d.text((cx + 24 + bw / 2, cy + 49), num, font=f, fill=(255, 255, 255), anchor="mm")
        lines = wrap(d, pg["caption"], font(29, 680), col - 56)
        for j, ln in enumerate(lines[:2]):
            d.text((cx + 28, cy + pic_h + 40 + j * 34), ln, font=font(29, 680), fill=INK, anchor="lm")
        notes = pg.get("notes") or []
        room = (cell_h - 8 - (pic_h + 40 + 34 * min(2, len(lines)) - 14)) // 32
        for j, (kind, text) in enumerate(notes[:max(0, room)]):
            chip(d, cx + 28, cy + pic_h + 40 + 34 * min(2, len(lines)) - 12 + j * 32, kind, text, 22)
    # crop to the used height
    used = y0 + math.ceil((len(pages) - 1) / 2) * cell_h + 12
    img = img.crop((0, 0, PW, used)).convert("RGB")
    img.save(out, optimize=True)
    return img.size


def wrap(d, text, f, width):
    words, lines, cur = text.split(), [], ""
    for w_ in words:
        t = (cur + " " + w_).strip()
        if d.textlength(t, font=f) <= width:
            cur = t
        else:
            lines.append(cur); cur = w_
    lines.append(cur)
    return lines


GUIDES = {
    "plain": dict(pages=PLAIN_PAGES, parts=PLAIN_PARTS, pdf="assembly-guide-plain.pdf", phone="assembly-guide-plain-phone.png",
                  label="Catalyst case rev 3.1 · plain back", title="Catalyst case: plain back",
                  phone_sub="revision 3.1 · no camera",
                  parts_sub="Catalyst case rev 3.1 · plain back · screws shown enlarged"),
    "camera": dict(pages=PAGES, parts=PARTS, pdf="assembly-guide.pdf", phone="assembly-guide-phone.png",
                   label="Catalyst case rev 3.1", title="Catalyst case: assembly", phone_sub="revision 3.1 · camera back",
                   parts_sub="Catalyst case rev 3.1 · camera back, plain back optional · screws shown enlarged"),
}


def build(raw, out, man, g):
    pages, pics = [], []
    total = len(g["pages"])
    for idx, pg in enumerate(g["pages"], 1):
        if pg.get("kind") == "parts":
            img = parts_page(raw, g["parts"], g["parts_sub"])
        elif "panels" in pg:
            pic = panels_picture(raw, man, pg["panels"], pg["sub"])
            pics.append(pic.crop((20, 190, W - 20, 975)))
            img = page_frame(pic.copy(), pg["n"], pg["caption"], pg.get("notes"), idx, total, g["label"])
        else:
            pic = picture(raw, man, pg["shot"])
            pics.append(picture(raw, man, pg["shot"], bg=False))
            img = page_frame(pic, pg["n"], pg["caption"], pg.get("notes"), idx, total, g["label"])
        rgb = img.convert("RGB")
        rgb.save(os.path.join(out, pg["file"] + ".png"), optimize=True)
        pages.append(rgb)
        print("page", pg["file"])
    pages[0].save(os.path.join(out, g["pdf"]), save_all=True, append_images=pages[1:], resolution=150.0)
    size = phone(raw, man, pics, os.path.join(out, g["phone"]), g["pages"], g["parts"], g["title"], g["phone_sub"])
    print("phone", g["phone"], size)


def main():
    raw, out = sys.argv[1], sys.argv[2]
    which = sys.argv[3:] or list(GUIDES)
    os.makedirs(out, exist_ok=True)
    man = json.load(open(os.path.join(raw, "manifest.json")))
    for k in which:
        build(raw, out, man, GUIDES[k])


if __name__ == "__main__":
    main()

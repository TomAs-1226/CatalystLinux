"""Put the transparent Blender renders on a studio backdrop; label the exploded view.

usage: python tools/finish.py RAW_DIR OUT_DIR
"""
import glob, json, os, sys
from PIL import Image, ImageDraw, ImageFont, ImageFilter

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FONT = os.path.join(HERE, "..", "..", "userland", "assets", "fonts", "GoogleSansCode.ttf")
MATERIAL = {   # label -> material, Night scheme (Bezel labels are lowercase)
    "Frame": "black pla", "Chassis": "white petg", "Camera backplate": "black pla",
    "Plain backplate": "black pla", "Camera trim": "gold pla", "Gold accent": "gold pla",
    "Link cover": "light blue pla", "Plunger": "white petg",
}


def backdrop(w, h):
    top, bot = (236, 234, 230), (214, 211, 206)
    g = Image.new("RGB", (1, h))
    for y in range(h):
        t = y / (h - 1)
        g.putpixel((0, y), tuple(int(top[i] + (bot[i] - top[i]) * t) for i in range(3)))
    bg = g.resize((w, h)).convert("RGBA")
    glow = Image.new("L", (w, h), 0)
    ImageDraw.Draw(glow).ellipse([w * 0.18, h * 0.12, w * 0.82, h * 0.78], fill=70)
    glow = glow.filter(ImageFilter.GaussianBlur(w * 0.08))
    white = Image.new("RGBA", (w, h), (250, 249, 246, 255))
    return Image.composite(white, bg, glow)


def labels(img, anchors):
    w, h = img.size
    d = ImageDraw.Draw(img)
    f = ImageFont.truetype(FONT, 26) if os.path.exists(FONT) else ImageFont.load_default()
    fs = ImageFont.truetype(FONT, 20) if os.path.exists(FONT) else f
    sides = {"l": [], "r": []}
    for name, (x, y) in anchors.items():
        sides["l" if x < 0.5 else "r"].append((y * h, x * w, name))
    for side, items in sides.items():
        items.sort()
        last = -1e9
        for ay, ax, name in items:
            ly = max(ay, last + 84)
            last = ly
            lx = 70 if side == "l" else w - 70
            d.line([(ax, ay), (lx + (260 if side == "l" else -260), ly), (lx, ly)], fill=(120, 116, 110), width=2)
            d.ellipse([ax - 5, ay - 5, ax + 5, ay + 5], fill=(40, 40, 42))
            text = name.lower()
            sub = MATERIAL.get(name, "")
            tx = lx if side == "l" else lx - d.textlength(text, font=f)
            d.text((tx, ly - 34), text, font=f, fill=(34, 34, 36))
            if sub:
                sx = lx if side == "l" else lx - d.textlength(sub, font=fs)
                d.text((sx, ly + 6), sub, font=fs, fill=(110, 106, 100))


def overview(out):
    order = ["camera-three-quarter", "camera-back-three-quarter", "camera-front", "camera-back",
             "camera-side", "camera-exploded", "plain-back-three-quarter", "plain-exploded"]
    tiles = [Image.open(os.path.join(out, n + ".png")) for n in order if os.path.exists(os.path.join(out, n + ".png"))]
    tw, th = 800, 600
    sheet = Image.new("RGB", (tw * 2, th * ((len(tiles) + 1) // 2)), (236, 234, 230))
    for i, t in enumerate(tiles):
        sheet.paste(t.resize((tw, th), Image.LANCZOS), ((i % 2) * tw, (i // 2) * th))
    sheet.save(os.path.join(out, "overview.png"), optimize=True)


def main():
    raw, out = sys.argv[1], sys.argv[2]
    os.makedirs(out, exist_ok=True)
    for p in sorted(glob.glob(os.path.join(raw, "*.png"))):
        if os.path.basename(p) == "overview.png":
            continue
        name = os.path.basename(p)
        r = Image.open(p).convert("RGBA")
        img = backdrop(*r.size)
        img.alpha_composite(r)
        meta = p + ".labels.json"
        if os.path.exists(meta):
            labels(img, json.load(open(meta)))
        img.convert("RGB").save(os.path.join(out, name), optimize=True)
        print("wrote", name)
    overview(out)


if __name__ == "__main__":
    main()

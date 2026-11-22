#!/usr/bin/env python3
"""The CJK font, for the simulator: fetch Noto Sans CJK, check it, cut it.

    python3 scripts/cjk-font.py              (Windows: py scripts\\cjk-font.py)
    python3 scripts/cjk-font.py --characters (write the character list again)

The image gets the same font from its own Buildroot package
(os/package/catalyst-fonts-cjk), built from the same pinned file by the same
subset.py with the same fontTools. This puts it where the simulator and the
tests look for it, userland/assets/fonts/cjk/NotoSansCJK-Catalyst.otf, which
is not in git: it is made again from the pinned inputs whenever wanted.

Inputs, each pinned to its SHA-256 (downloads are kept in
~/.cache/catalyst-linux/fonts, or $CATALYST_FONT_CACHE):
- Noto Sans CJK JP, variable (weights 100 to 900), from the Sans2.004
  release of github.com/notofonts/noto-cjk, at the commit that tag names;
- its licence (SIL Open Font License 1.1), copied next to the font;
- fontTools 4.60.0's wheel (pure Python), used if that exact version is not
  the one installed, so the output is the same file wherever it is made.

--characters writes os/package/catalyst-fonts-cjk/characters.txt from
Python's own tables of the national character sets (it needs a Python with
its CJK codecs, which Buildroot's host Python is built without; that is why
the list itself is kept in git).
"""
import hashlib
import os
import sys
import urllib.request
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
PACKAGE = os.path.join(ROOT, "os", "package", "catalyst-fonts-cjk")
OUT_DIR = os.path.join(ROOT, "userland", "assets", "fonts", "cjk")
OUT = os.path.join(OUT_DIR, "NotoSansCJK-Catalyst.otf")

COMMIT = "523d033d6cb47f4a80c58a35753646f5c3608a78"  # tag Sans2.004
NOTO = f"https://github.com/notofonts/noto-cjk/raw/{COMMIT}"
INPUTS = {
    "NotoSansCJKjp-VF.otf": (f"{NOTO}/Sans/Variable/OTF/NotoSansCJKjp-VF.otf", "ab2728702f90d2ae900309f299dc3c2b075010888a1a8a67fbd5b4c6aff713a0"),
    "LICENSE": (f"{NOTO}/LICENSE", "6a73f9541c2de74158c0e7cf6b0a58ef774f5a780bf191f2d7ec9cc53efe2bf2"),
    "fonttools-4.60.0-py3-none-any.whl": (
        "https://files.pythonhosted.org/packages/f9/a4/247d3e54eb5ed59e94e09866cfc4f9567e274fbf310ba390711851f63b3b/fonttools-4.60.0-py3-none-any.whl",
        "496d26e4d14dcccdd6ada2e937e4d174d3138e3d73f5c9b6ec6eb2fd1dab4f66",
    ),
}
FONTTOOLS = "4.60.0"
# What subset.py makes of the pinned font with that fontTools (the same from
# Python 3.13 on Windows and from Buildroot's host Python 3.14).
OUT_SHA256 = "6cb74defbce7d589b97202fad2c58a689617f903159dba260ec8d6ea176934e3"


def cache_dir():
    d = os.environ.get("CATALYST_FONT_CACHE") or os.path.join(os.path.expanduser("~"), ".cache", "catalyst-linux", "fonts")
    os.makedirs(d, exist_ok=True)
    return d


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def fetch(name):
    url, want = INPUTS[name]
    path = os.path.join(cache_dir(), name)
    if os.path.exists(path) and sha256(path) == want:
        return path
    print(f"fetching {url}")
    part = path + ".part"
    with urllib.request.urlopen(url, timeout=300) as r, open(part, "wb") as f:
        while block := r.read(1 << 20):
            f.write(block)
    got = sha256(part)
    if got != want:
        os.remove(part)
        sys.exit(f"{name}: SHA-256 {got}, expected {want}: not using it")
    os.replace(part, path)
    return path


def use_pinned_fonttools():
    try:
        import fontTools

        if fontTools.version == FONTTOOLS:
            return
    except ImportError:
        pass
    wheel = fetch(f"fonttools-{FONTTOOLS}-py3-none-any.whl")
    unpacked = os.path.join(cache_dir(), f"fonttools-{FONTTOOLS}")
    if not os.path.isdir(os.path.join(unpacked, "fontTools")):
        zipfile.ZipFile(wheel).extractall(unpacked)
    sys.path.insert(0, unpacked)
    for name in [m for m in sys.modules if m == "fontTools" or m.startswith("fontTools.")]:
        del sys.modules[name]


# ------------------------------------------------------------ characters ---


def double_byte(codec, leads, trails):
    out = []
    for lead in leads:
        for trail in trails:
            try:
                ch = bytes([lead, trail]).decode(codec)
            except UnicodeDecodeError:
                continue
            if len(ch) == 1:
                out.append(ch)
    return out


def big5(first, last):
    out = []
    for lead in range(0xA1, 0xFA):
        for trail in list(range(0x40, 0x7F)) + list(range(0xA1, 0xFF)):
            if first <= (lead << 8 | trail) <= last:
                try:
                    out.append(bytes([lead, trail]).decode("big5"))
                except UnicodeDecodeError:
                    pass
    return [c for c in out if len(c) == 1]


def block(first, last):
    return [chr(c) for c in range(first, last + 1)]


# Characters at least once in a million in a large corpus that the national
# sets lack: Jun Da's Modern Chinese frequency list (193 million characters)
# and kanji counted in Japanese Wikipedia (59 million; scriptin/kanji-frequency).
# Mostly name characters: 髙 and 﨑 (Takahashi, Yamasaki) above all.
FREQUENT_EXTRAS = "廆彅彧暠曺濊濵燁瓌禕窣竑箓郞鋆镕闿髙龢﨑"

SYMBOLS = "→←↑↓✓▶◀▲▼■●○◆★☆♪⌘⇧≈≠≤≥∞−"


def groups():
    """The character list, in groups, each with what it is for."""
    ws = range(0xA1, 0xFF)
    return [
        ("GB2312 levels 1 and 2: Simplified Chinese (99.98% of Jun Da's 193-million-character corpus)", double_byte("gb2312", range(0xB0, 0xF8), ws)),
        ("Big5 level 1, the frequently used characters: Traditional Chinese", big5(0xA440, 0xC67E)),
        ("JIS X 0208 levels 1 and 2: Japanese kanji (99.96% of the kanji in Japanese Wikipedia), the joyo kanji among them", double_byte("euc_jp", range(0xB0, 0xF5), ws)),
        # The 2010 joyo list added five kanji outside JIS X 0208: 剝 and 頰
        # are in Big5, 塡 is here, and 𠮟 is outside the Basic Multilingual
        # Plane (its everyday form 叱 is in JIS level 1).
        ("The one joyo kanji the sets above lack", ["塡"]),
        ("JIS X 0208's other characters: kana, punctuation, symbols, Greek and Cyrillic", double_byte("euc_jp", range(0xA1, 0xB0), ws)),
        ("Characters common in names and text that the sets above lack", list(FREQUENT_EXTRAS)),
        ("KS X 1001's 2,350 Hangul syllables (modern Korean), and the compatibility jamo", [c for c in double_byte("euc_kr", range(0xB0, 0xC9), ws) if "가" <= c <= "힣"] + block(0x3131, 0x318E)),
        (
            "Kana in full, CJK punctuation, Bopomofo, full- and half-width forms",
            block(0x3000, 0x303F) + block(0x3041, 0x3096) + block(0x3099, 0x30FF) + block(0x31F0, 0x31FF) + block(0x3105, 0x312F) + block(0xFF01, 0xFF60) + block(0xFF61, 0xFF9F) + block(0xFFE0, 0xFFE6),
        ),
        ("Greek and Cyrillic (the Latin-only Google Sans lacks them)", block(0x0391, 0x03C9) + block(0x0400, 0x045F)),
        # scripts/subset-fonts.sh asks Google Sans for these, and it has none of them.
        ("Symbols the shell uses that Google Sans lacks", list(SYMBOLS)),
    ]


def write_characters():
    path = os.path.join(PACKAGE, "characters.txt")
    seen = set()
    lines = [
        "# The characters the CJK font keeps, by what they are for. Written by",
        "# scripts/cjk-font.py --characters; read by subset.py. A line starting",
        "# with # is a comment; every other character on every line is kept.",
    ]
    for what, chars in groups():
        new = sorted(set(chars) - seen, key=ord)
        seen.update(new)
        lines.append(f"# {what}: {len(new)} more")
        for i in range(0, len(new), 64):
            lines.append("".join(new[i : i + 64]))
    lines.append(f"# {len(seen)} in all")
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(lines) + "\n")
    print(f"{path}: {len(seen)} characters")


# ------------------------------------------------------------------ main ---


def main():
    if "--characters" in sys.argv[1:]:
        return write_characters()
    font = fetch("NotoSansCJKjp-VF.otf")
    licence = fetch("LICENSE")
    use_pinned_fonttools()
    import fontTools

    print(f"fontTools {fontTools.version}")
    sys.path.insert(0, PACKAGE)
    import subset as cut

    os.makedirs(OUT_DIR, exist_ok=True)
    part = OUT + ".part"
    cut.main(font, os.path.join(PACKAGE, "characters.txt"), part)
    os.replace(part, OUT)
    with open(licence, "rb") as src, open(os.path.join(OUT_DIR, "OFL-NotoSansCJK.txt"), "wb") as dest:
        dest.write(src.read())
    got = sha256(OUT)
    print(f"{OUT}: {os.path.getsize(OUT)} bytes, SHA-256 {got}")
    if got != OUT_SHA256:
        print(f"note: not the pinned result ({OUT_SHA256}); the image's may differ from this one", file=sys.stderr)


if __name__ == "__main__":
    main()

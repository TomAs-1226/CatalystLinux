"""Cut Noto Sans CJK down to the characters in characters.txt.

    python3 subset.py <NotoSansCJKjp-VF.otf> <characters.txt> <out.otf>

Run by the Buildroot package (with Buildroot's host Python and the pinned
fontTools wheel) and by scripts/cjk-font.py for the simulator: the same
fontTools on the same input gives the same file, byte for byte.

What is kept, and why (see characters.txt for the list itself):
- the characters only: no OpenType layout features (no regional forms, no
  vertical forms, no proportional kana; the shell draws horizontal text,
  one form of each character);
- the weight axis (100 to 900), so a CJK title is as heavy as the Latin
  beside it;
- outlines without hints (the shell rasterises unhinted) and without
  subroutines: each glyph's outline stands alone in the file, so drawing
  one character reads one place of the memory-mapped font, not the
  shared subroutines scattered through it;
- the .notdef box, drawn for a character the subset lacks;
- no vertical metrics (vhea, vmtx, VORG, VVAR): nothing is set vertically.
"""
import sys

from fontTools import subset


def characters(path):
    out = set()
    with open(path, encoding="utf-8") as f:
        for line in f:
            if line.startswith("#"):
                continue
            out.update(ord(c) for c in line.rstrip("\n"))
    return sorted(out)


def main(src, chars, dest):
    unicodes = characters(chars)
    opts = subset.Options()
    opts.layout_features = []
    opts.desubroutinize = True
    opts.hinting = False
    opts.notdef_outline = True
    opts.notdef_glyph = True
    opts.name_IDs = ["*"]
    opts.name_languages = ["*"]
    opts.ignore_missing_unicodes = True
    opts.drop_tables += ["vhea", "vmtx", "VORG", "VVAR", "BASE", "JSTF", "DSIG"]
    font = subset.load_font(src, opts)
    sub = subset.Subsetter(opts)
    sub.populate(unicodes=unicodes)
    sub.subset(font)
    subset.save_font(font, dest, opts)
    print(f"{dest}: {len(unicodes)} characters asked for, {len(font.getGlyphOrder())} glyphs")


if __name__ == "__main__":
    if len(sys.argv) != 4:
        sys.exit(__doc__)
    main(*sys.argv[1:])

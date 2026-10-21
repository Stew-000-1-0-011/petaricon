#!/usr/bin/env python3
"""Regenerates NotoSansJP-subset.otf from Noto Sans CJK (JP face).

Usage: python3 make_subset.py [path/to/NotoSansCJK-Regular.ttc]
Requires fontTools (pip install fonttools).
"""
import sys
from fontTools.ttLib import TTCollection
from fontTools import subset

SRC = sys.argv[1] if len(sys.argv) > 1 else "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc"
OUT = "NotoSansJP-subset.otf"
RANGES = [(0x20, 0x7E), (0xA0, 0xFF), (0x2000, 0x206F), (0x2190, 0x21FF), (0x2460, 0x24FF),
          (0x25A0, 0x25FF), (0x3000, 0x30FF), (0x31F0, 0x31FF), (0xFF00, 0xFFEF)]


def jis_x_0208_kanji():
    chars = set()
    for lead in range(0x81, 0xFD):
        for trail in range(0x40, 0xFD):
            try:
                c = bytes([lead, trail]).decode("shift_jis")
            except UnicodeDecodeError:
                continue
            if len(c) == 1 and 0x4E00 <= ord(c) <= 0x9FFF:
                chars.add(ord(c))
    return chars


def main():
    ttc = TTCollection(SRC)
    face = None
    for f in ttc.fonts:
        names = {n.toUnicode() for n in f["name"].names if n.nameID in (1, 4, 6)}
        if any("CJK JP" in n or "CJKjp" in n for n in names):
            face = f
            break
    if face is None:
        sys.exit("Noto Sans CJK JP face not found")
    face.save("/tmp/NotoSansCJKjp-face.otf")
    unicodes = set()
    for a, b in RANGES:
        unicodes.update(range(a, b + 1))
    unicodes |= jis_x_0208_kanji()
    opts = subset.Options()
    opts.layout_features = ["kern", "palt", "vert", "vrt2", "locl", "ccmp"]
    opts.name_IDs = ["*"]
    opts.notdef_outline = True
    opts.hinting = False
    opts.desubroutinize = False
    font = subset.load_font("/tmp/NotoSansCJKjp-face.otf", opts)
    s = subset.Subsetter(opts)
    s.populate(unicodes=sorted(unicodes))
    s.subset(font)
    subset.save_font(font, OUT, opts)


main()

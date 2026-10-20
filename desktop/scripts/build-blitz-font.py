"""Convert the original BlitzMain OTF to the app's TrueType font.

Usage: python3 desktop/scripts/build-blitz-font.py /path/to/BlitzMain.otf
Requires fonttools. Normal app builds use the checked-in TTF and need no Python.
CFF outlines are converted to quadratic TrueType outlines to avoid missing
strokes when OBS scales the browser source. Preserve all glyphs and layout data.
"""
import argparse
from pathlib import Path

from fontTools.fontBuilder import FontBuilder
from fontTools.pens.cu2quPen import Cu2QuPen
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib import TTFont

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("source", type=Path)
args = parser.parse_args()
source = TTFont(args.source)
glyph_set = source.getGlyphSet()
glyphs = {}
for name in source.getGlyphOrder():
    pen = TTGlyphPen(None)
    glyph_set[name].draw(Cu2QuPen(pen, max_err=1.0, reverse_direction=True))
    glyphs[name] = pen.glyph()

builder = FontBuilder(source["head"].unitsPerEm, isTTF=True)
builder.setupGlyphOrder(source.getGlyphOrder())
builder.setupCharacterMap(source.getBestCmap())
builder.setupGlyf(glyphs)
builder.setupHorizontalMetrics(dict(source["hmtx"].metrics))
builder.setupHorizontalHeader()
builder.setupOS2()
builder.setupPost()
# Retain names, attribution, metrics, kerning and substitutions from the original.
for tag in ("name", "hhea", "OS/2", "GPOS", "GSUB"):
    if tag in source:
        builder.font[tag] = source[tag]
builder.font["head"].created = source["head"].created
builder.font["head"].modified = source["head"].modified
builder.font.recalcTimestamp = False
builder.save(Path(__file__).resolve().parents[1] / "ui/fonts/BlitzMain.ttf")

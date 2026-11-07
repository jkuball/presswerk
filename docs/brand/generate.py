"""Generate the Presswerk brand assets.

The mark is a monochrome split die: a hexagonal pressing tool, split at the
height of the record's centre, holding a record with a lambda-flake label and a
round spindle hole. It comes in two detail levels: the full mark with grooves
and flake for large sizes (from about 64 px), and the reduced icon for headers
and favicons.

Every asset is a single filled path with one colour and a transparent
background. The shapes are combined as geometry instead of SVG masks, because
WebKit rasterises masked SVG images at low resolution and they turn blurry.

The wordmark is set in Archivo (SIL Open Font License) at width 62 and weight
800 and converted to outlines, so the assets need no font at runtime.

Usage, from the repository root:

    nix shell --impure --expr 'with import <nixpkgs> {}; python3.withPackages (p: [p.fonttools p.shapely])' \\
      -c python3 docs/brand/generate.py path/to/Archivo[wdth,wght].ttf

The variable Archivo font is available from https://github.com/google/fonts/tree/main/ofl/archivo.
"""

import math
import sys
from pathlib import Path

from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont
from shapely import affinity
from shapely.geometry import LineString, Point, Polygon, box
from shapely.ops import unary_union

INK_LIGHT = "#111111"  # ink on the light theme
INK_DARK = "#ECECEA"  # ink on the dark theme

REPO = Path(__file__).resolve().parents[2]
ASSETS = REPO / "docs/site/src/assets/brand"
PUBLIC = REPO / "docs/site/public"

CENTRE = (50, 50)
SEGMENTS = 96  # per quarter circle; smooth even at hero size


def disc(r):
    return Point(CENTRE).buffer(r, quad_segs=SEGMENTS)


def ring(r, width):
    return disc(r + width / 2).difference(disc(r - width / 2))


def hexagon(r):
    return Polygon(
        (CENTRE[0] + r * math.cos(math.radians(60 * k)), CENTRE[1] + r * math.sin(math.radians(60 * k)))
        for k in range(6)
    )


def flake(scale):
    """Six lambdas around the centre: a long leg at 60 degrees and a short leg from its middle."""
    length, width, a = 34, 6.5, math.radians(60)
    b = (length * math.cos(a), length * math.sin(a))
    m = (b[0] / 2, b[1] / 2)
    c = (m[0] - length / 2 * math.cos(a), m[1] + length / 2 * math.sin(a))
    arm = unary_union(
        [
            LineString([(0, 0), b]).buffer(width / 2, cap_style="flat", join_style="mitre"),
            LineString([m, c]).buffer(width / 2, cap_style="flat", join_style="mitre"),
        ]
    )
    arm = affinity.translate(affinity.rotate(arm, 90, origin=(0, 0)), 0, -8)
    arms = unary_union([affinity.rotate(arm, 60 * k, origin=(0, 0)) for k in range(6)])
    return affinity.translate(affinity.scale(arms, scale, scale, origin=(0, 0)), *CENTRE)


def mark_geometry(full):
    """The ink of the mark on a 100 x 100 canvas."""
    record = 39 if full else 32
    frame = hexagon(48).difference(box(0, 47.25, 100, 52.75)).difference(disc(record + 2.5))
    vinyl = disc(record).difference(disc(17))
    if full:
        vinyl = vinyl.difference(ring(32.8, 2.2)).difference(ring(26.5, 2.2))
    parts = [frame, vinyl, disc(3.4 if full else 5)]
    if full:
        parts.append(flake(17 / 45))
    return unary_union(parts)


def to_path(geometry):
    """SVG path data; holes become subpaths, drawn with fill-rule evenodd."""
    polygons = getattr(geometry, "geoms", [geometry])

    def ring_data(coords):
        points = list(coords)[:-1]
        return "M" + "L".join(f"{x:.2f},{y:.2f}" for x, y in points) + "Z"

    return "".join(
        ring_data(p.exterior.coords) + "".join(ring_data(i.coords) for i in p.interiors) for p in polygons
    )


def mark_path(full, fill, attrs=""):
    return f'<path d="{to_path(mark_geometry(full))}" fill="{fill}" fill-rule="evenodd"{attrs}/>'


def svg(width, height, body, title):
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width:.2f} {height:.2f}" '
        f'role="img" aria-label="{title}"><title>{title}</title>{body}</svg>\n'
    )


def wordmark(font, text="PRESSWERK", tracking=0.03):
    """Outline the wordmark at 100 units per em; returns path data, width and cap height."""
    glyphs, cmap, hmtx = font.getGlyphSet(), font.getBestCmap(), font["hmtx"]
    scale = 100 / font["head"].unitsPerEm
    cap = font["OS/2"].sCapHeight * scale
    pen = SVGPathPen(glyphs)
    x = 0.0
    for i, ch in enumerate(text):
        name = cmap[ord(ch)]
        glyphs[name].draw(TransformPen(pen, (scale, 0, 0, -scale, x, cap)))
        x += hmtx[name][0] * scale + (tracking * 100 if i < len(text) - 1 else 0)
    return pen.getCommands(), x, cap


def lockup(font, fill):
    """Icon left, wordmark right; the icon is a little taller than the capitals."""
    path, width, cap = wordmark(font)
    icon = cap * 1.4
    gap = icon * 0.32
    top = (icon - cap) / 2
    body = (
        mark_path(False, fill, f' transform="scale({icon / 100:.4f})"')
        + f'<path d="{path}" fill="{fill}" transform="translate({icon + gap:.2f},{top:.2f})"/>'
    )
    return svg(icon + gap + width, icon, body, "Presswerk")


def favicon():
    """The reduced icon; the browser tab follows the OS theme, so a media query is right here."""
    style = (
        f"<style>.ink{{fill:{INK_LIGHT}}}"
        f"@media (prefers-color-scheme: dark){{.ink{{fill:{INK_DARK}}}}}</style>"
    )
    return svg(100, 100, style + mark_path(False, INK_LIGHT, ' class="ink"'), "Presswerk")


def main(archivo_path):
    font = instantiateVariableFont(TTFont(archivo_path), {"wdth": 62, "wght": 800})
    ASSETS.mkdir(parents=True, exist_ok=True)
    PUBLIC.mkdir(parents=True, exist_ok=True)
    for theme, ink in (("light", INK_LIGHT), ("dark", INK_DARK)):
        (ASSETS / f"mark-{theme}.svg").write_text(svg(100, 100, mark_path(True, ink), "Presswerk"))
        (ASSETS / f"icon-{theme}.svg").write_text(svg(100, 100, mark_path(False, ink), "Presswerk"))
        (ASSETS / f"lockup-{theme}.svg").write_text(lockup(font, ink))
    (PUBLIC / "favicon.svg").write_text(favicon())


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    main(sys.argv[1])

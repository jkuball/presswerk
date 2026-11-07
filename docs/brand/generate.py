"""Generate the Presswerk brand assets.

The mark is a monochrome split die: a hexagonal pressing tool, split at the
height of the record's centre, holding a record with a lambda-flake label and a
round spindle hole. It comes in two detail levels: the full mark with grooves
and flake for large sizes (from about 64 px), and the reduced icon for headers
and favicons. Every shape is cut out with an SVG mask, so the assets have one
fill colour and a transparent background.

The wordmark is set in Archivo (SIL Open Font License) at width 62 and weight
800 and converted to outlines, so the assets need no font at runtime.

Usage, from the repository root:

    nix shell --impure --expr 'with import <nixpkgs> {}; python3.withPackages (p: [p.fonttools])' \\
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

INK_LIGHT = "#111111"  # ink on the light theme
INK_DARK = "#ECECEA"  # ink on the dark theme

REPO = Path(__file__).resolve().parents[2]
ASSETS = REPO / "docs/site/src/assets/brand"
PUBLIC = REPO / "docs/site/public"


def hex_points(r, cx=50, cy=50, offset=0):
    return " ".join(
        f"{cx + r * math.cos(math.radians(offset + 60 * k)):.2f},{cy + r * math.sin(math.radians(offset + 60 * k)):.2f}"
        for k in range(6)
    )


def lambda_path(length):
    """One lambda: a long leg at 60 degrees and a short leg branching from its middle."""
    a = math.radians(60)
    bx, by = length * math.cos(a), length * math.sin(a)
    mx, my = bx / 2, by / 2
    cx, cy = mx - length / 2 * math.cos(a), my + length / 2 * math.sin(a)
    return f"M0,0 L{bx:.2f},{by:.2f} M{mx:.2f},{my:.2f} L{cx:.2f},{cy:.2f}"


def flake(scale, colour):
    """Six lambdas rotated around the centre; drawn small enough to sit on the label."""
    arm = lambda_path(34)
    arms = "".join(
        f'<g transform="rotate({k * 60}) translate(0,-8) rotate(90)">'
        f'<path d="{arm}" stroke="{colour}" stroke-width="6.5" fill="none"/></g>'
        for k in range(6)
    )
    return f'<g transform="translate(50,50) scale({scale:.4f})">{arms}</g>'


def mark_mask(full):
    """White keeps ink, black cuts it away."""
    disc = 39 if full else 32
    parts = [
        f'<polygon points="{hex_points(48)}" fill="white"/>',
        '<rect x="0" y="47.25" width="100" height="5.5" fill="black"/>',
        f'<circle cx="50" cy="50" r="{disc + 2.5}" fill="black"/>',
        f'<circle cx="50" cy="50" r="{disc}" fill="white"/>',
    ]
    if full:
        parts += [
            f'<circle cx="50" cy="50" r="{r}" fill="none" stroke="black" stroke-width="2.2"/>'
            for r in (32.8, 26.5)
        ]
    parts.append('<circle cx="50" cy="50" r="17" fill="black"/>')
    if full:
        parts.append(flake(17 / 45, "white"))
    parts.append(f'<circle cx="50" cy="50" r="{3.4 if full else 5}" fill="white"/>')
    return "".join(parts)


def mark_body(full, fill, mask_id, css_class=""):
    return (
        f'<mask id="{mask_id}" maskUnits="userSpaceOnUse" x="0" y="0" width="100" height="100">'
        f'<rect width="100" height="100" fill="black"/>{mark_mask(full)}</mask>'
        f'<rect width="100" height="100" fill="{fill}" mask="url(#{mask_id})"'
        + (f' class="{css_class}"' if css_class else "")
        + "/>"
    )


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


def lockup(font, fill, mask_id):
    """Icon left, wordmark right; the icon is a little taller than the capitals."""
    path, width, cap = wordmark(font)
    icon = cap * 1.4
    gap = icon * 0.32
    top = (icon - cap) / 2
    body = (
        f'<g transform="scale({icon / 100:.4f})">{mark_body(False, fill, mask_id)}</g>'
        f'<path d="{path}" fill="{fill}" transform="translate({icon + gap:.2f},{top:.2f})"/>'
    )
    return svg(icon + gap + width, icon, body, "Presswerk")


def favicon():
    """The reduced icon; the browser tab follows the OS theme, so a media query is right here."""
    style = (
        f"<style>.ink{{fill:{INK_LIGHT}}}"
        f"@media (prefers-color-scheme: dark){{.ink{{fill:{INK_DARK}}}}}</style>"
    )
    return svg(100, 100, style + mark_body(False, INK_LIGHT, "m", "ink"), "Presswerk")


def main(archivo_path):
    font = TTFont(archivo_path)
    font = instantiateVariableFont(font, {"wdth": 62, "wght": 800})
    ASSETS.mkdir(parents=True, exist_ok=True)
    PUBLIC.mkdir(parents=True, exist_ok=True)
    for theme, ink in (("light", INK_LIGHT), ("dark", INK_DARK)):
        (ASSETS / f"mark-{theme}.svg").write_text(svg(100, 100, mark_body(True, ink, "m"), "Presswerk"))
        (ASSETS / f"icon-{theme}.svg").write_text(svg(100, 100, mark_body(False, ink, "m"), "Presswerk"))
        (ASSETS / f"lockup-{theme}.svg").write_text(lockup(font, ink, "m"))
    (PUBLIC / "favicon.svg").write_text(favicon())


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    main(sys.argv[1])

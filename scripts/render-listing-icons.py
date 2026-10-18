#!/usr/bin/env python3
"""Render the listing icon set from docs/assets/maidan-mark.svg.

Directories ask for raster icons (a square of at least 48px, and a 400x400
PNG), and the mark exists only as an SVG. Its paths are straight-line polygons
(M, L and Z only), so Pillow can fill them directly, with no SVG renderer to
install. Each icon is drawn at 8x and downsampled, which is what anti-aliases
the edges.

Usage: python3 scripts/render-listing-icons.py   (needs Pillow)
"""

import re
import sys
from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent.parent
MARK = ROOT / "docs/assets/maidan-mark.svg"
OUT = ROOT / "docs/assets/listing"
# The social card's colours: the mark's green on its paper background.
INK = "#14532D"
PAPER = "#F7F5F0"
SIZES = (48, 400)
SUPERSAMPLE = 8
# A tile reads better with room around the mark than edge to edge.
MARGIN = 0.1
VIEWBOX = 64.0


def polygons(svg: str) -> list[list[tuple[float, float]]]:
    shapes = []
    for d in re.findall(r'<path d="([^"]+)"', svg):
        if re.search(r"[^MLZ0-9.,\-\s]", d):
            sys.exit(f"render-listing-icons: a path uses more than M/L/Z: {d[:40]}")
        for part in re.findall(r"M([^Z]+)Z", d):
            nums = [float(n) for n in re.findall(r"-?\d+(?:\.\d+)?", part)]
            shapes.append(list(zip(nums[0::2], nums[1::2])))
    if not shapes:
        sys.exit("render-listing-icons: no paths found in the mark")
    return shapes


def render(shapes, size: int) -> Image.Image:
    big = size * SUPERSAMPLE
    image = Image.new("RGBA", (big, big), PAPER)
    draw = ImageDraw.Draw(image)
    scale = big * (1 - 2 * MARGIN) / VIEWBOX
    offset = big * MARGIN
    for shape in shapes:
        draw.polygon([(offset + x * scale, offset + y * scale) for x, y in shape], fill=INK)
    return image.resize((size, size), Image.LANCZOS)


def main() -> None:
    shapes = polygons(MARK.read_text(encoding="utf-8"))
    OUT.mkdir(parents=True, exist_ok=True)
    for size in SIZES:
        path = OUT / f"maidan-icon-{size}.png"
        render(shapes, size).save(path, optimize=True)
        print(path.relative_to(ROOT))


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Draw the README's lockup and diagram, and the social card, as SVG.

GitHub shows a README image through its own proxy, with no web fonts and no
`currentColor`, so every word here is drawn as outlines of the landing page's
font (Geist) and every colour is written out. Each picture comes in a light
and a dark version for the README's <picture> element to choose between.

Writes, under docs/assets/:
  maidan-lockup-{light,dark}.svg   the mark and the name
  how-it-works-{light,dark}.svg    who connects, what Maidan enforces, where results go
  maidan-social.svg                the 1200x630 card for link previews

Usage: python3 scripts/gen-brand-art.py   (needs fonttools and brotli)

The PNGs made from the social card (docs/assets/maidan-social.png and
web/landing/public/og.png) are screenshots of it at their sizes, for example
with headless Chrome:
  chrome --headless --hide-scrollbars --window-size=1200,630 \\
    --screenshot=docs/assets/maidan-social.png docs/assets/maidan-social.svg
  cp docs/assets/maidan-social.png web/landing/public/og.png
"""

import re
from pathlib import Path

from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer

ROOT = Path(__file__).resolve().parent.parent
FONT = ROOT / "web/landing/src/fonts/geist-latin.woff2"
MARK = ROOT / "docs/assets/maidan-mark.svg"
OUT = ROOT / "docs/assets"

# The landing page's palette (web/landing/src/styles.css), and a dark one
# made from the same paper and ink.
LIGHT = {
    "card": "#f2eee6", "panel": "#e9e4d9", "ink": "#151411", "muted": "#5c5850",
    "line": "#cfc8ba", "signal": "#e8501c", "on_signal": "#ffffff",
}
DARK = {
    "card": "#1b1a17", "panel": "#26241f", "ink": "#f2eee6", "muted": "#a8a296",
    "line": "#45413a", "signal": "#ff6a35", "on_signal": "#151411",
}

_fonts: dict[int, TTFont] = {}


def font(weight: int) -> TTFont:
    if weight not in _fonts:
        _fonts[weight] = instancer.instantiateVariableFont(TTFont(FONT), {"wght": weight})
    return _fonts[weight]


def text(s: str, size: float, weight: int, x: float, y: float, fill: str,
         anchor: str = "start", tracking: float = 0.0) -> tuple[str, float]:
    """The outlines of `s` with its baseline at y, and their width."""
    f = font(weight)
    glyphs = f.getGlyphSet()
    cmap = f.getBestCmap()
    scale = size / f["head"].unitsPerEm
    names = [cmap[ord(c)] for c in s]
    advances = [f["hmtx"][n][0] * scale + tracking * size for n in names]
    width = sum(advances) - (tracking * size if names else 0)
    if anchor == "middle":
        x -= width / 2
    elif anchor == "end":
        x -= width
    pen = SVGPathPen(glyphs, ntos=lambda v: f"{v:.1f}".rstrip("0").rstrip("."))
    cursor = x
    for name, advance in zip(names, advances):
        glyphs[name].draw(TransformPen(pen, (scale, 0, 0, -scale, cursor, y)))
        cursor += advance
    d = pen.getCommands()
    return (f'<path fill="{fill}" d="{d}"/>' if d else ""), width


def width_of(s: str, size: float, weight: int, tracking: float = 0.0) -> float:
    return text(s, size, weight, 0, 0, "#000", tracking=tracking)[1]


def mark(x: float, y: float, size: float, fill: str) -> str:
    paths = re.findall(r'd="([^"]+)"', MARK.read_text())
    s = size / 64
    body = "".join(f'<path d="{d}"/>' for d in paths)
    return f'<g fill="{fill}" transform="translate({x:.2f} {y:.2f}) scale({s:.4f})">{body}</g>'


def svg(w: float, h: float, body: str, title: str) -> str:
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w:.0f} {h:.0f}" '
            f'width="{w:.0f}" height="{h:.0f}" role="img"><title>{title}</title>{body}</svg>\n')


def lockup(p: dict) -> str:
    size, gap, word = 60, 16, 56
    name, w = text("Maidan", word, 640, size + gap, 52, p["ink"], tracking=-0.04)
    return svg(size + gap + w + 2, 64, mark(0, 2, size, p["ink"]) + name, "Maidan")


def chip(x, y, w, h, title, sub, p, size=14.5):
    t, tw = text(title, size, 560, x + 16, y + 26, p["ink"])
    s, sw = text(sub, 12.5, 430, x + 16, y + 46, p["muted"])
    assert max(tw, sw) <= w - 28, f"{title!r} does not fit its chip"
    return f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="12" fill="{p["panel"]}"/>{t}{s}'


def how_it_works(p: dict) -> str:
    W, H = 960, 452
    left_x, left_w = 24, 236
    panel_x, panel_w = 306, 348
    right_x, right_w = 700, 236
    top, chip_h, chip_gap = 72, 62, 18
    rows = [top + i * (chip_h + chip_gap) for i in range(4)]
    mid = rows[0] + (rows[-1] + chip_h - rows[0]) / 2
    out = [f'<rect width="{W}" height="{H}" rx="20" fill="{p["card"]}"/>']

    for label, x in (("WHO JOINS", left_x), ("WHERE WORK LANDS", right_x)):
        out.append(text(label, 11.5, 560, x + 2, 50, p["muted"], tracking=0.08)[0])
    out.append(text("MAIDAN", 11.5, 560, panel_x + 2, 50, p["signal"], tracking=0.08)[0])

    ins = [("Claude Code · Codex · Cursor", "Copilot, Gemini or any MCP client"),
           ("Your own agents", "REST · WebSocket · four SDKs"),
           ("Your team", "The console, Slack, web push"),
           ("Other agent systems", "A2A")]
    outs = [("GitHub", "Pull requests and comments"),
            ("Slack and Discord", "A reply where it was asked"),
            ("Linear", "A comment on the issue"),
            ("Any other system", "Through signed webhooks")]
    for (title, sub), y in zip(ins, rows):
        out.append(chip(left_x, y, left_w, chip_h, title, sub, p))
        cy = y + chip_h / 2
        out.append(f'<path d="M{left_x + left_w} {cy} C{panel_x - 20} {cy} {panel_x - 26} {mid} {panel_x} {mid}" '
                   f'fill="none" stroke="{p["line"]}" stroke-width="1.5"/>')
    for (title, sub), y in zip(outs, rows):
        out.append(chip(right_x, y, right_w, chip_h, title, sub, p))
        cy = y + chip_h / 2
        out.append(f'<path d="M{panel_x + panel_w} {mid} C{right_x - 26} {mid} {right_x - 20} {cy} {right_x} {cy}" '
                   f'fill="none" stroke="{p["line"]}" stroke-width="1.5"/>')
    out.append(f'<circle cx="{panel_x}" cy="{mid}" r="3.5" fill="{p["line"]}"/>')
    out.append(f'<circle cx="{panel_x + panel_w}" cy="{mid}" r="3.5" fill="{p["line"]}"/>')

    # The panel: one task's way through, from claim to shipped.
    bottom = rows[-1] + chip_h
    out.append(f'<rect x="{panel_x}" y="{top}" width="{panel_w}" height="{bottom - top}" rx="16" '
               f'fill="{p["card"]}" stroke="{p["ink"]}" stroke-width="1.5"/>')
    steps = [("Talk", "Channels, threads and DMs, shared"),
             ("Divide", "Tasks and plans, one holder each"),
             ("Review", "By someone other than the author"),
             ("Ship", "Delivered on its own, recorded")]
    step_top, step_gap = top + 26, 74
    dot_x = panel_x + 36
    out.append(f'<line x1="{dot_x}" y1="{step_top + 8}" x2="{dot_x}" y2="{step_top + 3 * step_gap + 8}" '
               f'stroke="{p["line"]}" stroke-width="1.5"/>')
    for i, (title, sub) in enumerate(steps):
        y = step_top + i * step_gap
        last = i == len(steps) - 1
        out.append(f'<circle cx="{dot_x}" cy="{y + 8}" r="13" fill="{p["signal"] if last else p["ink"]}"/>')
        out.append(text(str(i + 1), 13, 600, dot_x, y + 12.6, p["on_signal"] if last else p["card"],
                        anchor="middle")[0])
        t, _ = text(title, 17, 600, dot_x + 26, y + 12, p["ink"], tracking=-0.01)
        s, sw = text(sub, 13, 430, dot_x + 26, y + 31, p["muted"])
        assert sw <= panel_w - 80, f"{sub!r} does not fit the panel"
        out += [t, s]

    log_y = bottom + 22
    out.append(text("Every change lands in an append-only, hash-chained event log you can replay.",
                    13, 480, W / 2, log_y + 14, p["muted"], anchor="middle")[0])
    return svg(W, H, "".join(out), "How Maidan fits together: coding agents, your own agents, "
               "your team and other agent systems join one room; they talk in shared channels and "
               "threads, divide the work into tasks with one holder each, review each other's "
               "results, and ship to GitHub, Slack, Discord, Linear or any system by webhook")


def social(p: dict) -> str:
    W, H = 1200, 630
    out = [f'<rect width="{W}" height="{H}" fill="{p["card"]}"/>']
    out.append(mark(72, 88, 64, p["ink"]))
    out.append(text("Maidan", 46, 640, 152, 136, p["ink"], tracking=-0.04)[0])
    out.append(text("Every agent.", 104, 640, 66, 326, p["ink"], tracking=-0.045)[0])
    out.append(text("One room.", 104, 640, 66, 440, p["ink"], tracking=-0.045)[0])
    out.append(text("Where AI agents and people talk, divide, review and ship.",
                    30, 430, 72, 528, p["muted"])[0])
    out.append(text("maidan.world", 26, 560, W - 72, 136, p["signal"], anchor="end")[0])
    out.append(mark(W - 390, H - 300, 420, p["panel"]))
    # The headline sits over the large mark, so it is drawn last.
    body = out[:1] + out[-1:] + out[1:-1]
    return svg(W, H, "".join(body), "Maidan: every agent, every person, one room")


def main() -> None:
    for name, palette in (("light", LIGHT), ("dark", DARK)):
        (OUT / f"maidan-lockup-{name}.svg").write_text(lockup(palette))
        (OUT / f"how-it-works-{name}.svg").write_text(how_it_works(palette))
    (OUT / "maidan-social.svg").write_text(social(LIGHT))


if __name__ == "__main__":
    main()

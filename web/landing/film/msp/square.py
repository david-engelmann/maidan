"""The square itself, the room the model is built round, and its light: a
shallow court of satin brass sunk in a black frame, its curving walks and
the plaza's rosette polished and cut into it, the oval lawn an inlay of
black glass, the trees frosted beads on fine brass pins with light piped up
into each, the fountain a brass ring round a ring of light with a plume
rising from it, lamp posts that are pins tipped with light, and the people:
tiny turned figures, white, and a few of them orange.
"""

import math

import bpy
import numpy as np

from . import craft, site
from .common import MB, TAU, add_object, box, capsule, collection, ellipsoid, revolve, rng, tube
from .site import MM, SCALE

PANEL_Z = 0.3 * MM  # the court's floor
FRAME_Z = 2.0 * MM  # the top of the frame round it


# ------------------------------------------------------------------ engraving


def engraving(px=0.15):
    """The laser's drawing for the park panel, as an image: red where an
    area is engraved (the walks, the plaza), green for the hairlines (kerbs,
    the plaza's rings and spokes, the beds), blue for the scorch that
    spreads a little from every line. `px` is the pixel size in model mm."""
    x0, y0, x1, y1 = site.PARK
    W = int(round((x1 - x0) * SCALE / (px * MM)))
    H = int(round((y1 - y0) * SCALE / (px * MM)))
    mpp = (x1 - x0) / W  # city metres per pixel
    clear = np.full((H, W), 1e3, np.float32)
    xs = x0 + (np.arange(W) + 0.5) * mpp
    ys = y0 + (np.arange(H) + 0.5) * mpp
    for pts, hw in site.all_paths():
        for k in range(len(pts) - 1):
            a, b = pts[k], pts[k + 1]
            pad = hw + 2.0
            i0 = max(0, int((min(a[0], b[0]) - pad - x0) / mpp))
            i1 = min(W, int((max(a[0], b[0]) + pad - x0) / mpp) + 1)
            j0 = max(0, int((min(a[1], b[1]) - pad - y0) / mpp))
            j1 = min(H, int((max(a[1], b[1]) + pad - y0) / mpp) + 1)
            if i1 <= i0 or j1 <= j0:
                continue
            X, Y = np.meshgrid(xs[i0:i1], ys[j0:j1])
            ab = b - a
            t = np.clip(((X - a[0]) * ab[0] + (Y - a[1]) * ab[1]) / (ab @ ab + 1e-9), 0, 1)
            d = np.hypot(X - (a[0] + t * ab[0]), Y - (a[1] + t * ab[1])) - hw
            clear[j0:j1, i0:i1] = np.minimum(clear[j0:j1, i0:i1], d)
    X, Y = np.meshgrid(xs, ys)
    R = np.hypot(X, Y)
    A = np.arctan2(Y, X)
    aa = mpp * 1.2
    lw = 0.055 / SCALE * MM  # half a hairline's width, in city metres
    # the plaza joins the walks
    clear = np.minimum(clear, R - site.PLAZA_R)
    area = np.clip(0.5 - clear / (2 * aa), 0, 1)
    line = np.clip(1.0 - (np.abs(clear) - lw) / aa, 0, 1)
    # the plaza's rosette: rings of paving, a collar, sixteen spokes
    plaza = R < site.PLAZA_R - 0.3
    rings = np.zeros_like(R)
    for rr, w in ((7.5, 1.6), (10.0, 1.0), (12.5, 1.0), (13.9, 1.6), (15.2, 1.0), (17.6, 1.0)):
        rings = np.maximum(rings, np.clip(1.0 - (np.abs(R - rr) - lw * w) / aa, 0, 1))
    seg = (A / TAU * 16.0) % 1.0
    spoke = np.clip(1.0 - (np.abs(seg - 0.5) * (R * TAU / 16.0) - lw) / aa, 0, 1) * (R > 7.5) * (R < 13.9)
    seg2 = ((A / TAU + 1 / 64) * 32.0) % 1.0
    spoke2 = np.clip(1.0 - (np.abs(seg2 - 0.5) * (R * TAU / 32.0) - lw) / aa, 0, 1) * (R > 13.9)
    line = np.maximum(line, np.maximum(rings, np.maximum(spoke, spoke2)) * plaza)
    # crescent beds between the walks, just outside the plaza
    bed = (R > site.PLAZA_R + 1.2) & (R < site.PLAZA_R + 4.6) & (clear > 1.6)
    edge = bed & ~(np.roll(bed, 1, 0) & np.roll(bed, -1, 0) & np.roll(bed, 1, 1) & np.roll(bed, -1, 1))
    line = np.maximum(line, edge.astype(np.float32) * 0.9)
    hatch = ((X * 0.9 + Y * 0.6) / 0.9) % 1.0
    area = np.maximum(area, bed * (hatch < 0.22) * 0.75)
    # the lawn's edge, where the felt is let in, and a line inside the park's kerb
    le = np.hypot((X - site.LAWN_C[0]) / site.LAWN_R[0], (Y - site.LAWN_C[1]) / site.LAWN_R[1])
    lawn_edge = np.abs(le - 1.0) * min(site.LAWN_R)
    line = np.maximum(line, np.clip(1.0 - (lawn_edge - lw * 1.6) / aa, 0, 1))
    fence = np.minimum(np.minimum(X - x0, x1 - X), np.minimum(Y - y0, y1 - Y))
    line = np.maximum(line, np.clip(1.0 - (np.abs(fence - 2.2) - lw) / aa, 0, 1) * (clear > 0.4))
    # scorch: the line, spread
    halo = line.copy()
    for _ in range(3):
        halo = (halo + np.roll(halo, 1, 0) + np.roll(halo, -1, 0) + np.roll(halo, 1, 1) + np.roll(halo, -1, 1)) / 5.0
        halo = (halo + np.roll(halo, 2, 0) + np.roll(halo, -2, 0) + np.roll(halo, 2, 1) + np.roll(halo, -2, 1)) / 5.0
    rgba = np.stack([area, line, np.clip(halo * 2.2, 0, 1), np.ones_like(area)], axis=-1).astype(np.float32)
    old = bpy.data.images.get("ParkEngraving")
    if old is not None:
        bpy.data.images.remove(old)
    img = bpy.data.images.new("ParkEngraving", W, H, alpha=False, float_buffer=False)
    img.colorspace_settings.name = "Non-Color"
    img.pixels.foreach_set(rgba.ravel())
    img.pack()
    return img


# -------------------------------------------------------------------- pieces


def _figure(mb, r, x, y, z, turn=0.0, scale=1.0, seated=False, lean=0.0, orange=False):
    """A scale figure: a turned body and a bead for a head. A seated one is
    shorter, with its legs out in front of it."""
    s = scale * MM
    mat = 1 if orange else 0
    a_line = r.uniform(0.0, 1.0)
    if seated:
        prof = [(0.0001, 0.0), (0.5, 0.0), (0.54, 0.25), (0.5, 1.1), (0.6, 1.75), (0.5, 2.05), (0.22, 2.2), (0.0001, 2.25)]
        head_z = 2.72
    else:
        foot = 0.42 + 0.26 * a_line
        prof = [(0.0001, 0.0), (foot, 0.0), (foot + 0.03, 0.2), (0.44 + 0.06 * a_line, 2.0), (0.6, 3.0), (0.52, 3.38), (0.22, 3.55), (0.0001, 3.6)]
        head_z = 4.08
    v, q, uv = revolve([(pr * s, pz * s) for pr, pz in prof], 20)
    hv, hq, _ = ellipsoid((0, 0, head_z * s), (0.47 * s, 0.47 * s, 0.5 * s), sides=16, rings=10)
    parts = [(v, q), (hv, hq)]
    if seated:
        for sx in (-1, 1):
            cv, cq, _ = capsule((sx * 0.24 * s, 0.1 * s, 0.32 * s), (sx * 0.26 * s, 1.15 * s, 0.3 * s), 0.27 * s, 0.24 * s, sides=8, rings=2)
            parts.append((cv, cq))
            cv, cq, _ = capsule((sx * 0.26 * s, 1.15 * s, 0.3 * s), (sx * 0.26 * s, 1.3 * s, -1.25 * s), 0.24 * s, 0.2 * s, sides=8, rings=2)
            parts.append((cv, cq))
    c, sn = math.cos(turn), math.sin(turn)
    cl, sl = math.cos(lean), math.sin(lean)
    for pv, pq in parts:
        pv = np.asarray(pv, np.float64)
        # lean forward (about x), then turn, then place
        yy = pv[:, 1] * cl + pv[:, 2] * sl
        zz = -pv[:, 1] * sl + pv[:, 2] * cl
        xx = pv[:, 0]
        out = np.stack([x + xx * c - yy * sn, y + xx * sn + yy * c, z + zz], axis=-1)
        mb.add(out, pq, mat=mat, smooth=True)


def cast(seed=11):
    """Who stands where in the square, in city metres: (x, y, turn, kind,
    scale). Kinds: stand, walk, sit (on the basin's rim), rest (on the felt)."""
    r = rng(seed)
    out = []

    def face(dx, dy):
        return math.atan2(-dx, dy)

    # seated on the fountain's rim, looking out over the plaza
    used = []
    while len(used) < 13:
        a = r.uniform(0, TAU)
        if any(abs((a - b + math.pi) % TAU - math.pi) < 0.24 for b in used):
            continue
        used.append(a)
        if len(used) % 3 == 0:
            used.append(a + 0.2)
    for a in used:
        out.append((5.42 * math.cos(a), 5.42 * math.sin(a), face(math.cos(a), math.sin(a)) + r.normal(0, 0.2), "sit", r.uniform(0.94, 1.06)))
    # gathered round the fountain in twos and threes
    for _ in range(11):
        a = r.uniform(0, TAU)
        rad = r.uniform(7.6, 12.5)
        c = np.array([rad * math.cos(a), rad * math.sin(a)])
        k = int(r.choice([2, 2, 3, 3, 4]))
        for j in range(k):
            ang = j * TAU / k + a
            off = np.array([math.cos(ang), math.sin(ang)]) * (0.62 + 0.1 * k)
            out.append((c[0] + off[0], c[1] + off[1], face(-off[0], -off[1]) + r.normal(0, 0.25), "stand", 0.62 if (k >= 3 and j == k - 1 and r.random() < 0.4) else r.uniform(0.92, 1.08)))
    # crossing the plaza
    n = 0
    while n < 12:
        a = r.uniform(0, TAU)
        rad = math.sqrt(r.uniform(9.0 ** 2, (site.PLAZA_R - 1.5) ** 2))
        p = (rad * math.cos(a), rad * math.sin(a))
        heading = a + math.pi / 2 * (1 if r.random() < 0.5 else -1) + r.normal(0, 0.5)
        out.append((p[0], p[1], face(math.cos(heading), math.sin(heading)), "walk", r.uniform(0.92, 1.08)))
        if r.random() < 0.6:
            out.append((p[0] + 1.25 * math.cos(heading + math.pi / 2), p[1] + 1.25 * math.sin(heading + math.pi / 2), face(math.cos(heading), math.sin(heading)), "walk", r.uniform(0.9, 1.06)))
        n += 1
    # in pairs along the walks
    for pts, hw in site.all_paths():
        d = np.concatenate(([0], np.cumsum(np.linalg.norm(np.diff(pts, axis=0), axis=1))))
        count = int(d[-1] / 17.0) + (1 if r.random() < 0.5 else 0)
        for _ in range(count):
            s0 = r.uniform(3.0, max(3.1, d[-1] - 2.0))
            i = min(max(int(np.searchsorted(d, s0)), 1), len(pts) - 2)
            t = pts[i + 1] - pts[i - 1]
            t = t / np.linalg.norm(t)
            sgn = 1 if r.random() < 0.5 else -1
            p = pts[i] + np.array([-t[1], t[0]]) * r.uniform(-hw * 0.45, hw * 0.45)
            if math.hypot(p[0], p[1]) < site.PLAZA_R + 0.5:
                continue
            out.append((p[0], p[1], face(t[0] * sgn, t[1] * sgn), "walk", r.uniform(0.92, 1.08)))
            if r.random() < 0.72:
                out.append((p[0] - t[1] * 1.25, p[1] + t[0] * 1.25, face(t[0] * sgn, t[1] * sgn), "walk", r.uniform(0.9, 1.06)))
    # on the lawn, sitting on the felt
    c = np.array(site.LAWN_C)
    for _ in range(6):
        a = r.uniform(0, TAU)
        rad = r.uniform(0.1, 0.75)
        g = c + np.array([site.LAWN_R[0] * rad * math.cos(a), site.LAWN_R[1] * rad * math.sin(a)])
        for j in range(int(r.integers(2, 4))):
            off = r.normal(0, 1.2, 2)
            out.append((g[0] + off[0], g[1] + off[1], r.uniform(0, TAU), "rest", r.uniform(0.92, 1.05)))
    return out


def build(col=None):
    col = col or collection("Square")
    r = rng(77)
    floor = craft.court(engraving())
    frame = craft.satin("park_frame", (0.012, 0.012, 0.014), rough=0.28)
    brass = craft.brass("brass_fountain", turned=True, rough=0.14)
    liner = craft.brass("brass_liner", turned=False, rough=0.2, bevel=False)
    wire = craft.brass("brass_wire", turned=False, rough=0.2, bevel=False)
    pearl = craft.pearl()
    lawn = craft.black_glass("lawn_glass", tone=(0.003, 0.009, 0.006), rough=0.05)
    pool = craft.black_glass("pool_glass", ripple=0.3)
    ring = craft.light("ring_light", power=70.0, tint=(1.0, 0.8, 0.58))
    tip = craft.light("lamp_tip", power=220.0, tint=(1.0, 0.64, 0.32))
    jet = craft.plume()
    white = craft.paint("figure_white", (0.88, 0.88, 0.86), rough=0.45)
    orange = craft.paint("figure_orange", craft.ORANGE, rough=0.42, glow=0.3)
    x0, y0, x1, y1 = [v * SCALE for v in site.PARK]
    m = MM

    # ---- the court's floor
    mb = MB()
    mb.add([(x0, y0, PANEL_Z), (x1, y0, PANEL_Z), (x1, y1, PANEL_Z), (x0, y1, PANEL_Z)], [[0, 1, 2, 3]], uv=[(0, 0), (1, 0), (1, 1), (0, 1)])
    add_object("ParkPanel", mb.build("ParkPanel", [floor]), col)
    # ---- its frame: satin black, lined with brass on the inside
    mb = MB()
    f = 4.0 * SCALE
    t = 0.5 * m
    for (ax, ay, bx, by) in ((x0 - f, y0 - f, x1 + f, y0), (x0 - f, y1, x1 + f, y1 + f), (x0 - f, y0, x0, y1), (x1, y0, x1 + f, y1)):
        v, q = box((ax, ay, 0.0), (bx, by, FRAME_Z))
        mb.add(v, q, mat=0)
    for (ax, ay, bx, by) in ((x0, y0, x1, y0 + t), (x0, y1 - t, x1, y1), (x0, y0 + t, x0 + t, y1 - t), (x1 - t, y0 + t, x1, y1 - t)):
        v, q = box((ax, ay, 0.0), (bx, by, FRAME_Z + 0.05 * m))
        mb.add(v, q, mat=1)
    add_object("ParkFrame", mb.build("ParkFrame", [frame, liner]), col)

    # ---- the lawn: black glass in a brass rim
    lx, ly = site.LAWN_C[0] * SCALE, site.LAWN_C[1] * SCALE
    rx, ry = site.LAWN_R[0] * SCALE, site.LAWN_R[1] * SCALE
    segs = 160
    a = np.arange(segs) * TAU / segs

    def oval(fx, zz):
        return np.stack([lx + (rx + fx) * np.cos(a), ly + (ry + fx) * np.sin(a), np.full(segs, zz)], axis=-1)

    zt = PANEL_Z + 0.35 * m
    mb = MB()
    mb.add(oval(0.0, zt), [list(range(segs))], mat=0)
    rim = np.concatenate([oval(0.0, zt), oval(0.0, zt + 0.25 * m), oval(0.6 * m, zt + 0.25 * m), oval(0.6 * m, PANEL_Z)])
    i = np.arange(3)[:, None]
    j = np.arange(segs)[None, :]
    j2 = (j + 1) % segs
    q = np.stack([i * segs + j, i * segs + j2, (i + 1) * segs + j2, (i + 1) * segs + j], axis=-1).reshape(-1, 4)
    mb.add(rim, q[:, ::-1], mat=1, smooth=True)
    add_object("Lawn", mb.build("Lawn", [lawn, liner], smooth_angle=math.radians(40)), col)

    # ---- the fountain: a brass ring, a ring of light inside it, a dark pool, a plume
    z = PANEL_Z
    fb = MB()
    prof = [(12.3 * m, z), (12.3 * m, z + 1.15 * m), (12.6 * m, z + 1.55 * m), (13.3 * m, z + 1.75 * m), (14.0 * m, z + 1.55 * m), (14.25 * m, z + 1.15 * m), (14.25 * m, z)]
    v, q, uv = revolve(prof, 128)
    fb.add(v, q, smooth=True)
    v, q, uv = revolve([(1.5 * m, z + 0.9 * m), (1.5 * m, z + 1.5 * m), (1.1 * m, z + 1.8 * m), (0.75 * m, z + 2.6 * m), (0.0001, z + 2.6 * m)], 48)
    fb.add(v, q, smooth=True)
    add_object("FountainBrass", fb.build("FountainBrass", [brass], smooth_angle=math.radians(50)), col)
    fb = MB()
    v, q, uv = revolve([(0.0001, z + 0.95 * m), (10.9 * m, z + 0.95 * m)], 96)
    fb.add(v, q, mat=0, smooth=True)
    v, q, uv = revolve([(10.9 * m, z + 1.0 * m), (12.3 * m, z + 1.0 * m)], 128)
    fb.add(v, q, mat=1)
    add_object("FountainPool", fb.build("FountainPool", [pool, ring]), col)
    fb = MB()
    hj = 17.0 * m
    prof = [(0.75 * m, 0.0), (0.62 * m, 0.25 * hj), (0.42 * m, 0.6 * hj), (0.2 * m, 0.9 * hj), (0.0001, hj)]
    v, q, uv = revolve(prof, 20)
    fb.add(v + np.array([0, 0, z + 2.6 * m], np.float32), q, smooth=True, vattr={"h": v[:, 2] / hj})
    jet_ob = add_object("FountainJet", fb.build("FountainJet", [jet]), col)
    jet_ob.visible_shadow = False
    ld = bpy.data.lights.new("FountainLamp", "POINT")
    ld.energy = 0.05
    ld.color = (1.0, 0.76, 0.5)
    ld.shadow_soft_size = 0.003
    lo = add_object("FountainLamp", ld, col, loc=(0.0, 0.0, z + 5.0 * m))
    lo["film_lamp"] = 0.05

    # ---- the trees: frosted beads on fine brass pins, light piped up each
    beads = MB()
    pins = MB()
    sites = site.bead_sites()
    for (x, y, size) in sites:
        d = size * SCALE
        hb = (3.2 + size * 0.55 + r.uniform(-0.8, 2.6)) * SCALE
        px, py = x * SCALE, y * SCALE
        lean = r.normal(0, 0.012, 2)
        cz = PANEL_Z + hb + d * 0.5
        cx, cy = px + lean[0] * cz, py + lean[1] * cz
        v, q, uv = ellipsoid((cx, cy, cz), (d / 2, d / 2, d / 2), sides=28, rings=14)
        dist = math.hypot(x, y)
        glow = r.uniform(0.55, 1.25) * (0.3 + 0.7 * math.exp(-(dist / 120.0) ** 2))
        if r.random() < 0.07:
            glow *= 0.12
        beads.add(v, q, smooth=True, vattr={"h": (v[:, 2] - (cz - d / 2)) / d}, fattr={"glow": np.full(len(q), glow), "hue": np.full(len(q), r.random())})
        v, q, uv = tube(np.array([(px, py, PANEL_Z - 0.1 * m), (cx, cy, cz - d * 0.42)]), 0.11 * m, 5)
        pins.add(v, q, smooth=True)
    add_object("Beads", beads.build("Beads", [pearl]), col)

    # ---- lamp posts: pins with a point of light
    tips = MB()
    lamps = [(x * SCALE, y * SCALE) for x, y in site.lamp_sites()]
    for (x, y) in lamps:
        h = 8.6 * m * r.uniform(0.94, 1.06)
        v, q, uv = tube(np.array([(x, y, PANEL_Z - 0.1 * m), (x, y, PANEL_Z + h)]), 0.085 * m, 4)
        pins.add(v, q, smooth=True)
        v, q, uv = ellipsoid((x, y, PANEL_Z + h + 0.2 * m), (0.3 * m, 0.3 * m, 0.3 * m), sides=8, rings=5)
        tips.add(v, q, smooth=True)
    add_object("Pins", pins.build("Pins", [wire]), col)
    add_object("LampTips", tips.build("LampTips", [tip]), col).visible_shadow = False

    # ---- the people
    fig = MB()
    people = cast()
    n_orange = 0
    for k, (x, y, turn, kind, scale) in enumerate(people):
        if kind == "rest":
            continue
        is_orange = r.random() < 0.16
        n_orange += is_orange
        px, py = x * SCALE, y * SCALE
        if kind == "sit":
            _figure(fig, r, px, py, PANEL_Z + 1.72 * m, turn, scale, seated=True, orange=is_orange)
        else:
            _figure(fig, r, px, py, PANEL_Z, turn, scale, lean=(0.1 if kind == "walk" else 0.0) + r.normal(0, 0.03), orange=is_orange)
    add_object("Figures", fig.build("Figures", [white, orange]), col)
    print(f"[square] {len(sites)} beads, {len(lamps)} lamps, {len(people)} figures ({n_orange} orange)")
    return col

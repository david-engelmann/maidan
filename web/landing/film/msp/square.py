"""The square itself, the room the model is built round: a maple panel sunk
between the blocks, its curving walks and the plaza's rosette engraved by
laser, the oval lawn a piece of sage felt, the trees opal beads on brass
pins, the fountain a brass ring over a lit disc of resin with a rod of
frosted acrylic for its jet, lamp posts that are brass pins tipped with
light, a wire of tiny lamps strung round the plaza, and the people: small
turned figures, white, and some of them orange.
"""

import math

import bpy
import numpy as np

from . import craft, site
from .common import MB, TAU, add_object, capsule, collection, ellipsoid, revolve, rng, tube
from .site import MM, SCALE

PANEL_Z = 0.22 * MM  # the park's floor: below the plates round it


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
    maple = craft.wood("wood_park", "white_maple_veneer", value=0.9, sat=0.86, etch=False, engraving=engraving())
    bead_wood = craft.wood("wood_bead", "white_maple_veneer", value=0.8, sat=1.0, etch=False)
    brass = craft.brass("brass_fountain", turned=True)
    wire = craft.brass("brass_wire", turned=False, rough=0.22)
    opal = craft.opal()
    felt = craft.felt()
    white = craft.paint("figure_white", (0.86, 0.86, 0.83), rough=0.5, sss=0.35)
    orange = craft.paint("figure_orange", craft.ORANGE, rough=0.46)
    lamp_tip = craft.led("lamp_tip", power=34.0)
    micro = craft.led("micro_lamp", power=13.0, tint=(1.0, 0.5, 0.2))
    x0, y0, x1, y1 = [v * SCALE for v in site.PARK]

    # ---- the panel
    mb = MB()
    v = [(x0, y0, PANEL_Z), (x1, y0, PANEL_Z), (x1, y1, PANEL_Z), (x0, y1, PANEL_Z)]
    mb.add(v, [[0, 1, 2, 3]], uv=[(0, 0), (1, 0), (1, 1), (0, 1)], fattr={"stock": np.array([[0.31, 0.62, 0.55]], np.float32)})
    add_object("ParkPanel", mb.build("ParkPanel", [maple]), col)

    # ---- the lawn: felt, a millimetre proud, its edge rolled over
    mb = MB()
    lx, ly = site.LAWN_C[0] * SCALE, site.LAWN_C[1] * SCALE
    rx, ry = site.LAWN_R[0] * SCALE, site.LAWN_R[1] * SCALE
    zt = PANEL_Z + 0.9 * MM
    segs = 128
    a = np.arange(segs) * TAU / segs
    rings = [(0.0005, zt), (0.5, zt + 0.05 * MM), (0.93, zt), (0.975, zt - 0.1 * MM), (0.995, zt - 0.32 * MM), (1.0, zt - 0.6 * MM), (1.0, PANEL_Z)]
    vv = []
    for f, zz in rings:
        vv.append(np.stack([lx + rx * f * np.cos(a), ly + ry * f * np.sin(a), np.full(segs, zz)], axis=-1))
    vv = np.concatenate(vv)
    i = np.arange(len(rings) - 1)[:, None]
    j = np.arange(segs)[None, :]
    j2 = (j + 1) % segs
    q = np.stack([i * segs + j, (i + 1) * segs + j, (i + 1) * segs + j2, i * segs + j2], axis=-1).reshape(-1, 4)
    mb.add(vv, q, smooth=True)
    mb.add(vv[:segs], [list(range(segs))])
    add_object("Lawn", mb.build("Lawn", [felt]), col)

    # ---- the fountain: brass over light
    m = MM
    z = PANEL_Z
    fb = MB()
    ring = [(12.3 * m, z), (12.3 * m, z + 1.15 * m), (12.6 * m, z + 1.55 * m), (13.3 * m, z + 1.75 * m), (14.0 * m, z + 1.55 * m), (14.25 * m, z + 1.15 * m), (14.25 * m, z)]
    v, q, uv = revolve(ring, 128)
    fb.add(v, q, smooth=True)
    # the pedestal's sleeve and the upper bowl, a turned saucer
    sleeve = [(1.25 * m, z + 0.9 * m), (1.25 * m, z + 1.3 * m), (0.95 * m, z + 1.6 * m), (0.85 * m, z + 4.9 * m), (1.1 * m, z + 5.3 * m), (3.5 * m, z + 6.05 * m), (4.35 * m, z + 6.45 * m), (4.4 * m, z + 6.7 * m), (4.1 * m, z + 6.72 * m), (3.4 * m, z + 6.45 * m), (0.62 * m, z + 6.2 * m), (0.62 * m, z + 6.9 * m)]
    v, q, uv = revolve(sleeve, 64)
    fb.add(v, q, smooth=True)
    add_object("FountainBrass", fb.build("FountainBrass", [brass], smooth_angle=math.radians(50)), col)
    fb = MB()
    disc = [(0.0001, z + 0.95 * m), (4.0 * m, z + 0.97 * m), (9.0 * m, z + 0.93 * m), (12.4 * m, z + 0.88 * m)]
    v, q, uv = revolve(disc, 96)
    fb.add(v, q, smooth=True)
    add_object("FountainWater", fb.build("FountainWater", [craft.resin()]), col)
    fb = MB()
    jet = [(0.0001, z + 0.9 * m), (0.52 * m, z + 0.9 * m), (0.52 * m, z + 14.6 * m), (0.4 * m, z + 15.0 * m), (0.0001, z + 15.15 * m)]
    v, q, uv = revolve(jet, 24)
    fb.add(v, q, smooth=True)
    jet_ob = add_object("FountainJet", fb.build("FountainJet", [craft.rod()]), col)
    jet_ob.visible_shadow = False
    ld = bpy.data.lights.new("FountainLamp", "POINT")
    ld.energy = 0.07
    ld.color = (1.0, 0.72, 0.42)
    ld.shadow_soft_size = 0.004
    lo = add_object("FountainLamp", ld, col, loc=(0.0, 0.0, z + 3.2 * m))
    lo["film_lamp"] = 0.07

    # ---- the trees: an opal bead (or now and then a wooden one) on a brass pin
    crowns = MB()
    beads = MB()
    pins = MB()
    for k, (x, y, size) in enumerate(site.tree_sites()):
        d = size * SCALE * r.uniform(0.96, 1.04)
        hb = (4.2 + size * 0.24) * SCALE
        px, py = x * SCALE, y * SCALE
        lean = r.normal(0, 0.016, 2)
        cz = PANEL_Z + hb + d * 0.46
        cx, cy = px + lean[0] * (hb + d * 0.5), py + lean[1] * (hb + d * 0.5)
        squash = r.uniform(0.86, 0.98)
        v, q, uv = ellipsoid((cx, cy, cz), (d / 2, d / 2, d / 2 * squash), sides=36, rings=20)
        if r.random() < 0.16:
            beads.add(v, q, smooth=True, fattr={"stock": np.tile(r.random(3).astype(np.float32)[None], (len(q), 1))})
        else:
            crowns.add(v, q, smooth=True)
        v, q, uv = tube(np.array([(px, py, PANEL_Z - 0.1 * m), (cx, cy, cz)]), 0.3 * m, 8)
        pins.add(v, q, smooth=True)
        # a turned collar where the pin enters the bead
        coll = [(0.3 * m, -0.9 * m), (0.62 * m, -0.7 * m), (0.62 * m, -0.2 * m), (0.3 * m, 0.0)]
        v, q, uv = revolve(coll, 12)
        pins.add(v + np.array([cx, cy, cz - d / 2 * squash + 0.25 * m], np.float32), q, smooth=True)
    add_object("TreeCrowns", crowns.build("TreeCrowns", [opal]), col)
    add_object("TreeBeads", beads.build("TreeBeads", [bead_wood]), col)
    add_object("TreePins", pins.build("TreePins", [wire]), col)

    # ---- lamp posts: brass pins with a point of light
    posts = MB()
    tips = MB()
    lamps = [(x * SCALE, y * SCALE) for x, y in site.lamp_sites()]
    for k, (x, y) in enumerate(lamps):
        h = 9.4 * m * r.uniform(0.97, 1.03)
        lean = r.normal(0, 0.012, 2)
        top = (x + lean[0] * h, y + lean[1] * h, PANEL_Z + h)
        v, q, uv = tube(np.array([(x, y, PANEL_Z - 0.1 * m), top]), 0.19 * m, 6)
        posts.add(v, q, smooth=True)
        v, q, uv = ellipsoid((top[0], top[1], top[2] + 0.3 * m), (0.4 * m, 0.4 * m, 0.42 * m), sides=10, rings=6)
        tips.add(v, q, smooth=True)
        ld = bpy.data.lights.new(f"LampLight{k:03d}", "POINT")
        ld.energy = 0.013
        ld.color = (1.0, 0.62, 0.3)
        ld.shadow_soft_size = 0.0006
        lo = add_object(f"LampLight{k:03d}", ld, col, loc=(top[0], top[1], top[2] + 0.3 * m))
        lo["film_lamp"] = 0.013
    # the flagpole, and a ball on its top
    fx, fy = 17.0 * SCALE, 27.5 * SCALE
    v, q, uv = tube(np.array([(fx, fy, PANEL_Z), (fx + 0.3 * m, fy, PANEL_Z + 52 * m)]), 0.32 * m, 8)
    posts.add(v, q, smooth=True)
    v, q, uv = ellipsoid((fx + 0.3 * m, fy, PANEL_Z + 52.4 * m), (0.6 * m, 0.6 * m, 0.6 * m), sides=12, rings=8)
    posts.add(v, q, smooth=True)

    # ---- the wire of tiny lamps round the plaza
    n = 14
    rad = (site.PLAZA_R - 1.9) * SCALE
    h = 13.5 * m
    tops = []
    for k in range(n):
        ang = (k + 0.5) / n * TAU
        x, y = rad * math.cos(ang), rad * math.sin(ang)
        v, q, uv = tube(np.array([(x, y, PANEL_Z - 0.1 * m), (x, y, PANEL_Z + h)]), 0.2 * m, 6)
        posts.add(v, q, smooth=True)
        v, q, uv = ellipsoid((x, y, PANEL_Z + h + 0.15 * m), (0.3 * m, 0.3 * m, 0.3 * m), sides=8, rings=5)
        posts.add(v, q, smooth=True)
        tops.append(np.array([x, y, PANEL_Z + h]))
    bulbs = MB()
    count = 0
    for k in range(n):
        a_, b_ = tops[k], tops[(k + 1) % n]
        span = np.linalg.norm(b_ - a_)
        sag = 0.085 * span * r.uniform(0.85, 1.15)
        steps = int(span / (1.75 * m))
        s = np.linspace(0, 1, steps + 1)
        pts = a_[None] + (b_ - a_)[None] * s[:, None]
        pts[:, 2] -= sag * 4 * s * (1 - s)
        v, q, uv = tube(pts, 0.06 * m, 4)
        posts.add(v, q, smooth=True)
        for p in pts[1:-1]:
            v, q, uv = ellipsoid((p[0], p[1], p[2] - 0.32 * m), (0.24 * m, 0.24 * m, 0.3 * m), sides=8, rings=5)
            bulbs.add(v, q, smooth=True)
            count += 1
        mid = pts[len(pts) // 2]
        ld = bpy.data.lights.new(f"WireLight{k:02d}", "POINT")
        ld.energy = 0.011
        ld.color = (1.0, 0.6, 0.28)
        ld.shadow_soft_size = 0.004
        lo = add_object(f"WireLight{k:02d}", ld, col, loc=(mid[0], mid[1], mid[2] - 0.3 * m))
        lo["film_lamp"] = 0.011
    add_object("Posts", posts.build("Posts", [wire]), col).visible_shadow = True
    add_object("LampTips", tips.build("LampTips", [lamp_tip]), col).visible_shadow = False
    add_object("MicroLamps", bulbs.build("MicroLamps", [micro]), col).visible_shadow = False

    # ---- the people
    fig = MB()
    people = cast()
    n_orange = 0
    for k, (x, y, turn, kind, scale) in enumerate(people):
        is_orange = r.random() < 0.38
        n_orange += is_orange
        px, py = x * SCALE, y * SCALE
        if kind == "sit":
            _figure(fig, r, px, py, PANEL_Z + 1.72 * m, turn, scale * 1.1, seated=True, orange=is_orange)
        elif kind == "rest":
            _figure(fig, r, px, py, PANEL_Z + 0.88 * m, turn, scale * 0.78, lean=r.normal(0, 0.05), orange=is_orange)
        else:
            _figure(fig, r, px, py, PANEL_Z, turn, scale * 1.1, lean=(0.1 if kind == "walk" else 0.0) + r.normal(0, 0.03), orange=is_orange)
    add_object("Figures", fig.build("Figures", [white, orange]), col)
    print(f"[square] {len(site.tree_sites())} trees, {len(lamps)} lamps, {count} micro lamps, {len(people)} figures ({n_orange} orange)")
    return col

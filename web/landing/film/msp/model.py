"""The model's city: a charcoal baseplate on a plinth, a pale plate for
every block, and on the plates the buildings, each cut from its own piece
of stock with its windows laser-etched. A share of the windows are cut
right through and backed with paper, with a lamp inside. The glass towers
are acrylic; the clock tower's cupola and the pyramid are turned brass.

The massing is the real ring of buildings round Madison Square, from
memory: the clock tower and its annex, the great stepped block north of it,
the dark slab, the tower with the gold pyramid, the lofts along the west,
the wedge at the south-west corner, the slim glass tower at the south.
"""

import math

import bpy
import numpy as np

from . import craft, site
from .common import MB, TAU, add_object, collection, revolve, rng
from .site import MM, SCALE, rect

PLATE = 0.8 * MM  # a block's plate stands this far above the street
KERB = 5.0  # city metres of pavement outside the building line
Z = np.array([0.0, 0.0, 1.0])

# window rhythms, in city metres: bay, floor, and the opening as fractions of them
LOFT = dict(bay=4.3, floor=4.1, ow=0.56, oh=0.56, sill=0.22, base=1.25)
OFFICE = dict(bay=3.4, floor=3.8, ow=0.42, oh=0.52, sill=0.26, base=1.4)
TOWER = dict(bay=3.7, floor=3.9, ow=0.38, oh=0.56, sill=0.24, base=1.5)
HOTEL = dict(bay=3.0, floor=3.5, ow=0.42, oh=0.5, sill=0.28, base=1.3)
GRAND = dict(bay=4.9, floor=4.5, ow=0.42, oh=0.6, sill=0.22, base=1.6)
FINE = dict(bay=2.6, floor=3.7, ow=0.4, oh=0.54, sill=0.26, base=1.4)

WOOD, POCKET = 0, 1


def to_model(poly):
    return [(x * SCALE, y * SCALE) for x, y in poly]


class Lamps:
    """The lamps inside one building: where they are decides which windows
    were cut through and how brightly each glows."""

    def __init__(self, r, poly, z0, z1, share, n=None):
        xs = [p[0] for p in poly]
        ys = [p[1] for p in poly]
        self.r = r
        self.share = share
        count = n or max(2, int(((max(xs) - min(xs)) * (max(ys) - min(ys)) * (z1 - z0)) ** (1 / 3) / 0.035))
        self.p = np.stack([r.uniform(min(xs), max(xs), count), r.uniform(min(ys), max(ys), count), z0 + (z1 - z0) * r.uniform(0.02, 0.95, count) ** 1.3], axis=-1)
        self.s = r.uniform(0.028, 0.06, count)
        self.gain = r.uniform(0.6, 1.3, count)

    def at(self, c, boost=0.0, gain=1.0):
        d2 = ((c[:, None, :] - self.p[None]) ** 2).sum(-1)
        g = (self.gain[None] * np.exp(-d2 / (2 * self.s[None] ** 2))).sum(-1)
        r = self.r
        n = len(c)
        cut = r.random(n) < np.clip(self.share * gain * (0.1 + 1.5 * g) + self.share * boost, 0.0, 0.9)
        glow = np.where(cut, (0.1 + 1.1 * np.clip(g, 0, 1.4) ** 1.3) * r.uniform(0.6, 1.4, n), 0.0)
        lit = np.clip(r.random(n) * (1.0 - 0.55 * np.clip(g, 0, 1)), 0.0, 1.0)
        return glow, lit


def prism(mb, poly, z0, z1, stock, win=None, lamps=None, r=None, cap=True, face_boost=None, crown=False):
    """One piece of stock: a prism over a convex footprint (model metres,
    counter-clockwise). With `win`, its sides are etched with a grid of
    window pockets."""
    n = len(poly)
    st = np.asarray(stock, np.float32)
    for i in range(n):
        a = np.array([poly[i][0], poly[i][1], 0.0])
        b = np.array([poly[(i + 1) % n][0], poly[(i + 1) % n][1], 0.0])
        L = float(np.linalg.norm(b - a))
        if L < 1e-6:
            continue
        U = (b - a) / L
        N = np.array([U[1], -U[0], 0.0])

        def world(uvd):
            uvd = np.asarray(uvd, float).reshape(-1, 3)
            return a[None] + uvd[:, 0:1] * U[None] + uvd[:, 1:2] * Z[None] + uvd[:, 2:3] * N[None]

        def plain(u0, u1, v0, v1):
            if u1 - u0 < 1e-6 or v1 - v0 < 1e-6:
                return
            mb.add(world([(u0, v0, 0), (u1, v0, 0), (u1, v1, 0), (u0, v1, 0)]), [[0, 1, 2, 3]], mat=WOOD, fattr={"stock": st[None], "cell": 0.0, "burn": 0.0}, vattr={"scorch": np.zeros(4)})

        grid = None
        if win is not None:
            bay = win["bay"] * SCALE
            fl = win["floor"] * SCALE
            base = win["base"] * fl
            mx_min = max(1.4 * MM, 0.22 * bay)
            nb = int((L - 2 * mx_min) / bay)
            nf = int((z1 - z0 - base - 0.35 * fl) / fl)
            if nb >= 1 and nf >= 1:
                grid = (bay, fl, base, nb, nf, (L - nb * bay) / 2)
        if grid is None:
            plain(0.0, L, z0, z1)
            continue
        bay, fl, base, nb, nf, mx = grid
        vt = z0 + base + nf * fl
        plain(0.0, mx, z0, z1)
        plain(L - mx, L, z0, z1)
        plain(mx, L - mx, vt, z1)
        depth = 0.55 * MM
        boost, gain = 0.0, 1.0
        if face_boost is not None:
            boost, gain = face_boost(a, b, N)

        def cells(u0, v0, cw, ch, ni, nj, ow, oh, sill, extra=0.0):
            ii, jj = np.meshgrid(np.arange(ni), np.arange(nj), indexing="ij")
            m = ni * nj
            off = np.stack([u0 + ii.ravel() * cw, v0 + jj.ravel() * ch, np.zeros(m)], axis=-1)
            a0 = (cw - ow) / 2
            a1 = a0 + ow
            b0 = sill
            b1 = sill + oh
            tv = np.array([(0, 0, 0), (cw, 0, 0), (cw, ch, 0), (0, ch, 0), (a0, b0, 0), (a1, b0, 0), (a1, b1, 0), (a0, b1, 0), (a0, b0, -depth), (a1, b0, -depth), (a1, b1, -depth), (a0, b1, -depth)], float)
            tuv = np.array([(0, 0), (1, 0), (1, 1), (0, 1), (a0 / cw, b0 / ch), (a1 / cw, b0 / ch), (a1 / cw, b1 / ch), (a0 / cw, b1 / ch), (0, 0), (1, 0), (1, 1), (0, 1)], float)
            vv = world((tv[None] + off[:, None, :]).reshape(-1, 3))
            base_i = (np.arange(m) * 12)[:, None, None]
            wall = np.array([[0, 1, 5, 4], [1, 2, 6, 5], [2, 3, 7, 6], [3, 0, 4, 7]])
            rev = np.array([[4, 5, 9, 8], [5, 6, 10, 9], [6, 7, 11, 10], [7, 4, 8, 11]])
            back = np.array([[8, 9, 10, 11]])
            scorch = np.tile(np.array([0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1], float), m)
            uv = np.tile(tuv, (m, 1))
            stock_f = np.tile(st[None], (m * 4, 1))
            mb.add(vv, (wall[None] + base_i).reshape(-1, 4), mat=WOOD, uv=uv, vattr={"scorch": scorch}, fattr={"stock": stock_f, "cell": 1.0, "burn": 0.0})
            mb.add(vv, (rev[None] + base_i).reshape(-1, 4), mat=WOOD, uv=uv, vattr={"scorch": scorch}, fattr={"stock": stock_f, "cell": 0.0, "burn": 1.0})
            centre = world(off + np.array([cw / 2, sill + oh / 2, 0.0]))
            if lamps is not None:
                glow, lit = lamps.at(centre, boost + extra * gain, gain)
            else:
                glow, lit = np.zeros(m), np.ones(m)
            mb.add(vv, (back[None] + base_i).reshape(-1, 4), mat=POCKET, uv=uv, fattr={"glow": glow, "lit": lit})

        # the street floor: wider, taller openings, more of them lit
        cells(mx, z0, bay, base, nb, 1, bay * 0.72, base * 0.6, base * 0.14, extra=0.35)
        cells(mx, z0 + base, bay, fl, nb, nf, bay * win["ow"], fl * win["oh"], fl * win["sill"])
    if cap:
        top = np.array([(p[0], p[1], z1) for p in poly])
        mb.add(top, [list(range(n))], mat=WOOD, fattr={"stock": st[None], "cell": 0.0, "burn": 0.0}, vattr={"scorch": np.zeros(n)})
    if crown and z1 - z0 > 3.0 * MM and abs(site.area(poly)) > (9 * MM) ** 2:
        # a thin plate laid on the block, a hair wider than it: the cornice. Its
        # middle is routed out, which leaves a parapet round a sunk roof.
        st2 = np.clip(st + (r.random(3) - 0.5) * 0.25 if r is not None else st, 0, 1).astype(np.float32)
        over = site.outset(poly, 0.32 * MM)
        inner = site.outset(poly, -0.95 * MM)
        zb, zt = z1 - 0.55 * MM, z1 + 0.42 * MM
        ring_ob = np.array([(p[0], p[1], zb) for p in over])
        ring_ot = np.array([(p[0], p[1], zt) for p in over])
        ring_it = np.array([(p[0], p[1], zt) for p in inner])
        ring_if = np.array([(p[0], p[1], z1 + 0.02 * MM) for p in inner])
        ring_bb = np.array([(p[0], p[1], zb) for p in poly])
        vv = np.concatenate([ring_ob, ring_ot, ring_it, ring_if, ring_bb])
        faces = []
        for i in range(n):
            j = (i + 1) % n
            faces.append([i, j, n + j, n + i])  # the cornice's face
            faces.append([n + i, n + j, 2 * n + j, 2 * n + i])  # the parapet's top
            faces.append([2 * n + i, 2 * n + j, 3 * n + j, 3 * n + i])  # its inner face
            faces.append([4 * n + i, 4 * n + j, j, i])  # the soffit
        m = len(faces)
        mb.add(vv, np.array(faces), mat=WOOD, fattr={"stock": np.tile(st2[None], (m, 1)), "cell": 0.0, "burn": 0.0}, vattr={"scorch": np.zeros(len(vv))})


def acrylic_prism(mb, poly, z0, z1, c=0.7 * MM, floor_h=0.0, mat=0):
    """A block of acrylic with real arrises: frosted faces, polished chamfers."""
    cut = site.chamfer(poly, c)
    n = len(cut)
    top_in = site.inset(cut, c)
    if len(top_in) != n:
        top_in = [((p[0] + q[0]) / 2, (p[1] + q[1]) / 2) for p, q in zip(cut, np.roll(np.array(cut), -1, axis=0))]
        top_in = cut
    ring0 = np.array([(p[0], p[1], z0) for p in cut])
    ring1 = np.array([(p[0], p[1], z1 - c) for p in cut])
    ring2 = np.array([(p[0], p[1], z1) for p in top_in])
    v = np.concatenate([ring0, ring1, ring2])
    faces = []
    frost = []
    for i in range(n):
        j = (i + 1) % n
        edge = math.hypot(cut[j][0] - cut[i][0], cut[j][1] - cut[i][1])
        faces.append([i, j, n + j, n + i])
        frost.append(0.0 if edge < 2.5 * c else 1.0)
        faces.append([n + i, n + j, 2 * n + j, 2 * n + i])
        frost.append(0.0)
    mb.add(v, np.array(faces), mat=mat, fattr={"frost": np.array(frost), "floor_h": floor_h})
    mb.add(ring2, [list(range(n))], mat=mat, fattr={"frost": 1.0, "floor_h": 0.0})
    mb.add(ring0, [list(range(n))[::-1]], mat=mat, fattr={"frost": 1.0, "floor_h": 0.0})


def _tank(mb, x, y, z, stock, r):
    """A water tank on a roof: a dowel on a short stand, a turned cap."""
    s = r.uniform(0.85, 1.1)
    prof = [(0.0001, z), (1.6 * MM * s, z), (1.6 * MM * s, z + 2.2 * MM * s), (2.4 * MM * s, z + 2.2 * MM * s), (2.4 * MM * s, z + 6.4 * MM * s), (2.6 * MM * s, z + 6.5 * MM * s), (0.0001, z + 7.6 * MM * s)]
    v, q, uv = revolve(prof, 20)
    v = v + np.array([x, y, 0], np.float32)
    mb.add(v, q, mat=WOOD, smooth=True, fattr={"stock": np.asarray(stock, np.float32)[None], "cell": 0.0, "burn": 0.0}, vattr={"scorch": np.zeros(len(v))})


def _inside(poly, margin, r, tries=40):
    xs = [p[0] for p in poly]
    ys = [p[1] for p in poly]
    inner = site.inset(poly, margin)
    if len(inner) < 3:
        return None
    for _ in range(tries):
        p = (r.uniform(min(xs), max(xs)), r.uniform(min(ys), max(ys)))
        if all((inner[(i + 1) % len(inner)][0] - inner[i][0]) * (p[1] - inner[i][1]) - (inner[(i + 1) % len(inner)][1] - inner[i][1]) * (p[0] - inner[i][0]) >= 0 for i in range(len(inner))):
            return p
    return None


# ------------------------------------------------------------------ the plan


def plan():
    """Every building on the baseplate: (name, kind, share of windows lit,
    tiers, extras). A tier is (footprint, from, to, window rhythm) in city
    metres. Kinds: maple and ash are wood; smoke and frost are acrylic."""
    B = []

    def add(name, kind, share, tiers, extras=()):
        B.append(dict(name=name, kind=kind, share=share, tiers=tiers, extras=list(extras)))

    def stepped(poly, steps, style):
        """Tiers stepping in: steps are (top, inset from the footprint)."""
        out = []
        z = 0.0
        for top, ins in steps:
            p = site.inset(poly, ins) if ins > 0 else poly
            if len(p) >= 3:
                out.append((p, z, top, style))
            z = top
        return out

    # ---- east of the park, south to north
    # the clock tower, at the corner of its block, and the annex that wraps it
    tower = rect(104, 0, 130, 26)
    add("clock_tower", "maple", 0.34, [
        (tower, 0, 128, FINE),
        (site.inset(tower, 1.2), 128, 142, dict(bay=4.4, floor=9.0, ow=0.5, oh=0.74, sill=0.12, base=0.08)),
    ], extras=[("clock",), ("spire",)])
    add("annex_east", "ash", 0.3, stepped(rect(132, -42, 226, 26), [(52, 0), (60, 5.0), (66, 11.0)], OFFICE), extras=[("bulk", 2)])
    add("annex_south", "maple", 0.28, stepped(rect(104, -42, 130, -2), [(52, 0), (58, 4.0)], OFFICE))
    # the great stepped block: it was meant to be a hundred storeys
    nb = rect(104, 44, 226, 112)
    add("north_building", "maple", 0.38, [
        (site.chamfer(nb, 7.0), 0, 60, GRAND),
        (site.chamfer(site.inset(nb, 8.0), 8.0), 60, 84, OFFICE),
        (site.chamfer(site.inset(nb, [14.0, 20.0, 14.0, 20.0]), 7.0), 84, 108, OFFICE),
        (site.chamfer(site.inset(nb, [20.0, 34.0, 20.0, 34.0]), 5.0), 108, 137, OFFICE),
    ], extras=[("bulk", 3)])
    # the courthouse, the dark slab and its neighbours
    add("courthouse", "maple", 0.5, [(rect(104, 130, 128, 156), 0, 17, dict(bay=3.4, floor=10.0, ow=0.36, oh=0.8, sill=0.1, base=0.2)), (site.inset(rect(104, 130, 128, 156), 2.5), 17, 20, None)])
    add("slab_podium", "ash", 0.4, [(rect(104, 160, 148, 198), 0, 9, None)])
    add("slab", "smoke", 0.0, [(site.inset(rect(104, 160, 148, 198), 2.0), 9, 170, None)])
    add("east_loft_a", "ash", 0.3, stepped(rect(132, 130, 226, 157), [(50, 0)], LOFT), extras=[("tank", 1), ("bulk", 2)])
    add("east_loft_b", "maple", 0.32, stepped(rect(152, 160, 226, 198), [(58, 0), (68, 4.5)], OFFICE), extras=[("bulk", 2)])
    # the tower with the gold pyramid
    nyl = rect(104, 220, 226, 281)
    add("pyramid_tower", "maple", 0.36, [
        (nyl, 0, 54, GRAND),
        (site.inset(nyl, [6.0, 10.0, 6.0, 10.0]), 54, 76, OFFICE),
        (site.chamfer(rect(138, 231, 192, 270), 4.0), 76, 132, TOWER),
        (site.chamfer(rect(142, 234, 188, 267), 4.0), 132, 158, TOWER),
    ], extras=[("pyramid",)])
    # ---- north of the park, west to east
    add("north_a", "ash", 0.3, stepped(rect(-75, 220, -28, 281), [(46, 0), (50, 3.0)], LOFT), extras=[("tank", 1), ("bulk", 1)])
    add("north_b", "maple", 0.34, stepped(rect(-26, 220, 8, 272), [(66, 0), (76, 3.5), (82, 7.0)], FINE), extras=[("bulk", 1)])
    add("north_c", "ash", 0.3, stepped(rect(10, 220, 36, 268), [(62, 0)], HOTEL), extras=[("tank", 1)])
    add("north_d", "maple", 0.32, stepped(rect(38, 220, 75, 281), [(40, 0), (46, 5.0)], LOFT), extras=[("bulk", 2)])
    # ---- west of the park: lofts, cut on the diagonal where Broadway passes
    def west(name, y0, y1, pieces):
        """Pieces across a block west of Fifth Avenue: (x0, x1, kind, share, steps, style, extras)."""
        for k, (x0, x1, kind, share, steps, style, extras) in enumerate(pieces):
            poly = rect(x0, y0, x1, y1)
            w, e = site.broadway_sides(poly)
            for side, pc in (("w", w), ("e", e)):
                if len(pc) >= 3 and site.area(pc) > 250:
                    add(f"{name}_{k}{side}", kind, share, stepped(pc, steps, style), extras)

    west("w22", -133, -72, [(-160, -104, "maple", 0.3, [(52, 0), (58, 4.0)], LOFT, [("tank", 1)]), (-200, -162, "ash", 0.28, [(40, 0)], LOFT, [("bulk", 1)]), (-238, -202, "maple", 0.3, [(88, 0), (96, 3.0)], HOTEL, [("bulk", 1)])])
    west("w23", -42, 26, [(-166, -104, "maple", 0.34, [(56, 0), (62, 4.0)], GRAND, [("bulk", 2)]), (-202, -168, "ash", 0.3, [(54, 0)], LOFT, [("tank", 1)]), (-238, -204, "maple", 0.28, [(42, 0)], LOFT, [("bulk", 1)])])
    west("w24", 44, 112, [(-190, -104, "ash", 0.34, [(58, 0), (64, 3.5)], LOFT, [("tank", 1), ("bulk", 1)]), (-238, -192, "maple", 0.28, [(46, 0)], LOFT, [("bulk", 1)])])
    west("w25", 130, 198, [(-136, -104, "maple", 0.32, [(44, 0)], LOFT, [("tank", 1)]), (-196, -138, "ash", 0.3, [(60, 0), (66, 4.0)], OFFICE, [("bulk", 2)]), (-238, -198, "maple", 0.3, [(50, 0)], LOFT, [("tank", 1)])])
    west("w26", 220, 281, [(-150, -104, "ash", 0.3, [(56, 0)], LOFT, [("bulk", 1)]), (-200, -152, "maple", 0.28, [(40, 0)], LOFT, [("tank", 1)]), (-238, -202, "ash", 0.3, [(52, 0)], OFFICE, [("bulk", 1)])])
    # ---- south of 23rd Street
    block = rect(-75, -133, 226, -72)
    wedge, rest = site.broadway_sides(block)
    if len(wedge) >= 3:
        add("wedge", "maple", 0.3, [(wedge, 0, 82, FINE), (site.inset(wedge, 0.8), 82, 87, None)])
    for k, (x0, x1, y0, y1, kind, share, steps, style, extras) in enumerate([
        # the row on the square's axis is kept low: the way in passes over it
        (-52, -8, -104, -72, "ash", 0.3, [(20, 0)], LOFT, [("tank", 1)]),
        (-6, 24, -104, -72, "maple", 0.3, [(16, 0)], LOFT, []),
        (26, 52, -104, -72, "ash", 0.28, [(24, 0)], LOFT, [("bulk", 1)]),
        (54, 80, -100, -72, "maple", 0.3, [(38, 0)], LOFT, [("tank", 1)]),
        (-30, 8, -133, -106, "ash", 0.34, [(20, 0)], HOTEL, []),
        (10, 50, -133, -106, "maple", 0.28, [(22, 0)], LOFT, [("bulk", 1)]),
        (52, 80, -133, -102, "ash", 0.3, [(42, 0)], LOFT, []),
        (84, 128, -133, -72, "maple", 0.3, [(46, 0), (52, 4.0)], OFFICE, [("tank", 1), ("bulk", 1)]),
        (130, 164, -133, -72, "ash", 0.32, [(58, 0), (66, 5.0)], OFFICE, [("bulk", 2)]),
        (190, 226, -133, -104, "maple", 0.3, [(30, 0)], LOFT, [("bulk", 1)]),
    ]):
        poly = rect(x0, y0, x1, y1)
        w, e = site.broadway_sides(poly)
        pc = e if x0 > -60 else w
        if len(pc) >= 3 and site.area(pc) > 200:
            add(f"s22_{k}", kind, share, stepped(pc, steps, style), extras)
    # the slim glass tower: a frosted shaft with boxes cantilevered off it
    add("slim_base", "ash", 0.4, [(rect(168, -102, 226, -72), 0, 7, None)])
    add("slim_tower", "frost", 0.0, [(rect(189, -95, 201, -81), 7, 188, None)], extras=[("pods",)])
    return B


def plates():
    """The plate under every block: the block and its pavement, as one piece."""
    out = []
    for poly in (rect(104, -42, 226, 26), rect(104, 44, 226, 112), rect(104, 130, 226, 198), rect(104, 220, 226, 281), rect(-75, 220, 75, 281)):
        out.append(poly)
    for y0, y1 in ((-133, -72), (-42, 26), (44, 112), (130, 198), (220, 281)):
        w, e = site.broadway_sides(rect(-238, y0, -104, y1))
        out += [p for p in (w, e) if len(p) >= 3]
    w, e = site.broadway_sides(rect(-75, -133, 226, -72))
    out += [p for p in (w, e) if len(p) >= 3]
    x0, y0, x1, y1 = site.EXTENT
    res = []
    for p in out:
        grown = site.outset(p, KERB)
        # nothing overhangs the baseplate
        for pt, nrm in (((x0 + 3, 0), (-1, 0)), ((x1 - 3, 0), (1, 0)), ((0, y0 + 3), (0, -1)), ((0, y1 - 3), (0, 1))):
            grown = site.clip(grown, pt, nrm)
        if len(grown) >= 3:
            res.append(grown)
    return res


# ------------------------------------------------------------------ building


def _crown_pieces(col, b, r, mats, to_model):
    """The brass: clock faces, the tower's roof and cupola, the pyramid.
    Turned parts are their own objects, so the lathe's rings run round each
    one's own axis."""
    brass = MB()
    wood = MB()
    name = b["name"]

    def turned(tag, prof, cx, cy, segs=40):
        mb = MB()
        v, q, uv = revolve(prof, segs)
        mb.add(v, q, smooth=True)
        add_object(f"{name}_{tag}", mb.build(f"{name}_{tag}", [mats["brass"]], smooth_angle=math.radians(40)), col, loc=(cx, cy, 0.0))

    for ex in b["extras"]:
        if ex[0] == "clock":
            poly = to_model(b["tiers"][0][0])
            zc = PLATE + 104.0 * SCALE
            rad = 4.4 * SCALE
            n = len(poly)
            for i in range(n):
                a = np.array([poly[i][0], poly[i][1], 0.0])
                bb = np.array([poly[(i + 1) % n][0], poly[(i + 1) % n][1], 0.0])
                U = (bb - a) / np.linalg.norm(bb - a)
                N = np.array([U[1], -U[0], 0.0])
                c = (a + bb) / 2 + Z * zc
                # a dial: a brass disc with a raised rim, proud of the wood
                prof = [(0.0001, 0.5 * MM), (rad * 0.86, 0.5 * MM), (rad * 0.9, 0.75 * MM), (rad, 0.75 * MM), (rad, 0.0)]
                v, q, uv = revolve(prof, 48)
                vv = c[None] + v[:, 0:1] * U[None] + v[:, 1:2] * Z[None] + v[:, 2:3] * N[None]
                brass.add(vv, q, mat=1, smooth=True)
        if ex[0] == "spire":
            poly = to_model(site.inset(b["tiers"][0][0], 1.2))
            z0 = PLATE + 142.0 * SCALE
            cx = sum(p[0] for p in poly) / 4
            cy = sum(p[1] for p in poly) / 4
            hw = abs(poly[1][0] - poly[0][0]) / 2
            hd = abs(poly[2][1] - poly[1][1]) / 2
            # a cornice slab, then the steep roof in wood, its faces a little hollow
            stock = r.random(3)
            prism(wood, [(cx - hw - 1.0 * MM, cy - hd - 1.0 * MM), (cx + hw + 1.0 * MM, cy - hd - 1.0 * MM), (cx + hw + 1.0 * MM, cy + hd + 1.0 * MM), (cx - hw - 1.0 * MM, cy + hd + 1.0 * MM)], z0, z0 + 1.6 * MM, stock)
            steps = 10
            rings = []
            top = 3.1 * SCALE
            zt = z0 + 1.6 * MM + 34.0 * SCALE
            for k in range(steps + 1):
                s = k / steps
                f = 1 - (1 - top / hw) * s ** 0.85
                rings.append([(cx - hw * f, cy - hd * f, z0 + 1.6 * MM + (zt - z0 - 1.6 * MM) * s), (cx + hw * f, cy - hd * f, z0 + 1.6 * MM + (zt - z0 - 1.6 * MM) * s), (cx + hw * f, cy + hd * f, z0 + 1.6 * MM + (zt - z0 - 1.6 * MM) * s), (cx - hw * f, cy + hd * f, z0 + 1.6 * MM + (zt - z0 - 1.6 * MM) * s)])
            rv = np.array(rings).reshape(-1, 3)
            rq = [[k * 4 + j, k * 4 + (j + 1) % 4, (k + 1) * 4 + (j + 1) % 4, (k + 1) * 4 + j] for k in range(steps) for j in range(4)]
            wood.add(rv, rq, mat=WOOD, fattr={"stock": np.tile(stock[None].astype(np.float32), (len(rq), 1)), "cell": 0.0, "burn": 0.0}, vattr={"scorch": np.zeros(len(rv))})
            wood.add(rv[-4:], [[0, 1, 2, 3]], mat=WOOD, fattr={"stock": stock[None].astype(np.float32), "cell": 0.0, "burn": 0.0}, vattr={"scorch": np.zeros(4)})
            # the cupola: a turned brass lantern, a dome, a finial
            m = MM
            prof = [(3.6 * m, zt), (3.6 * m, zt + 0.5 * m), (3.0 * m, zt + 0.7 * m), (2.7 * m, zt + 1.1 * m), (2.7 * m, zt + 4.6 * m), (3.1 * m, zt + 4.9 * m), (3.1 * m, zt + 5.3 * m), (2.8 * m, zt + 5.6 * m), (2.5 * m, zt + 6.9 * m), (1.7 * m, zt + 8.0 * m), (0.7 * m, zt + 8.7 * m), (0.45 * m, zt + 9.2 * m), (0.6 * m, zt + 9.8 * m), (0.3 * m, zt + 10.3 * m), (0.22 * m, zt + 14.5 * m), (0.0001, zt + 15.2 * m)]
            turned("cupola", prof, cx, cy)
        if ex[0] == "pyramid":
            poly = to_model(b["tiers"][-1][0])
            z0 = PLATE + b["tiers"][-1][2] * SCALE
            cx = sum(p[0] for p in poly) / len(poly)
            cy = sum(p[1] for p in poly) / len(poly)
            xs = [p[0] for p in poly]
            ys = [p[1] for p in poly]
            hw, hd = (max(xs) - min(xs)) / 2 - 0.4 * MM, (max(ys) - min(ys)) / 2 - 0.4 * MM
            zt = z0 + 27.0 * SCALE
            # a stepped foot, then the pyramid itself, then a lantern
            rings = []
            for f, zz in ((1.0, z0), (1.0, z0 + 0.8 * MM), (0.96, z0 + 0.8 * MM), (0.1, zt)):
                rings.append([(cx - hw * f, cy - hd * f, zz), (cx + hw * f, cy - hd * f, zz), (cx + hw * f, cy + hd * f, zz), (cx - hw * f, cy + hd * f, zz)])
            rv = np.array(rings).reshape(-1, 3)
            rq = [[k * 4 + j, k * 4 + (j + 1) % 4, (k + 1) * 4 + (j + 1) % 4, (k + 1) * 4 + j] for k in range(3) for j in range(4)]
            brass.add(rv, rq, mat=1)
            prof = [(1.3 * MM, zt - 0.4 * MM), (1.3 * MM, zt + 1.6 * MM), (1.6 * MM, zt + 1.8 * MM), (0.9 * MM, zt + 3.0 * MM), (0.25 * MM, zt + 3.6 * MM), (0.2 * MM, zt + 7.0 * MM), (0.0001, zt + 7.5 * MM)]
            turned("lantern", prof, cx, cy, 24)
    if brass.nv:
        add_object(f"{name}_brass", brass.build(f"{name}_brass", [mats["brass"], mats["brass_flat"]]), col)
    if wood.nv:
        add_object(f"{name}_roof", wood.build(f"{name}_roof", [mats["maple"], mats["pocket"]]), col)


def build(col=None):
    col = col or collection("Model")
    r = rng(400)
    mats = {
        "maple": craft.wood("wood_maple", "white_maple_veneer", value=0.84),
        "ash": craft.wood("wood_ash", "ash_veneer", value=1.12, sat=0.62),
        "plate": craft.wood("wood_plate", "white_maple_veneer", value=0.92, sat=0.8, etch=False),
        "pocket": craft.pocket(),
        "brass": craft.brass("brass_turned", turned=True),
        "brass_flat": craft.brass("brass_sheet", turned=False, rough=0.24),
        "smoke": craft.acrylic("acrylic_smoked", "smoked"),
        "frost": craft.acrylic("acrylic_frosted", "frosted"),
        "board": craft.board(),
        "plinth": craft.board("plinth_board", tone=(0.016, 0.016, 0.018), rough=0.42, dust=0.5),
    }
    x0, y0, x1, y1 = [v * SCALE for v in site.EXTENT]
    m = 6 * MM

    # ---- the baseplate and its plinth
    mb = MB()
    base_t = 12 * MM
    lo = np.array([x0 - m, y0 - m, -base_t])
    hi = np.array([x1 + m, y1 + m, 0.0])
    from .common import box

    v, q = box(lo, hi)
    mb.add(v, q, mat=0)
    # a shadow gap, then the plinth, a hand narrower all round
    gap = 4 * MM
    v, q = box((lo[0] + 9 * MM, lo[1] + 9 * MM, -base_t - gap), (hi[0] - 9 * MM, hi[1] - 9 * MM, -base_t))
    mb.add(v, q, mat=1)
    v, q = box((lo[0] + 3 * MM, lo[1] + 3 * MM, -base_t - gap - 46 * MM), (hi[0] - 3 * MM, hi[1] - 3 * MM, -base_t - gap))
    mb.add(v, q, mat=1)
    add_object("Baseplate", mb.build("Baseplate", [mats["board"], mats["plinth"]]), col)
    # a brass line let into the plate's edge, all the way round
    mb = MB()
    t = 1.2 * MM
    e = 2.6 * MM
    for a, b in (((lo[0] + e, lo[1] + e), (hi[0] - e, lo[1] + e + t)), ((lo[0] + e, hi[1] - e - t), (hi[0] - e, hi[1] - e)), ((lo[0] + e, lo[1] + e), (lo[0] + e + t, hi[1] - e)), ((hi[0] - e - t, lo[1] + e), (hi[0] - e, hi[1] - e))):
        v, q = box((a[0], a[1], -0.2 * MM), (b[0], b[1], 0.06 * MM))
        mb.add(v, q, mat=0)
    add_object("Inlay", mb.build("Inlay", [mats["brass_flat"]]), col)

    # ---- the plates
    mb = MB()
    for poly in plates():
        stock = r.random(3)
        prism(mb, to_model(poly), 0.0, PLATE, stock)
    # the park's own pavement: a frame round the sunk panel
    px0, py0, px1, py1 = site.PARK
    f = 4.0
    for poly in (rect(px0 - f, py0 - f, px1 + f, py0), rect(px0 - f, py1, px1 + f, py1 + f), rect(px0 - f, py0, px0, py1), rect(px1, py0, px1 + f, py1)):
        prism(mb, to_model(poly), 0.0, PLATE, r.random(3))
    add_object("Plates", mb.build("Plates", [mats["plate"], mats["pocket"]]), col)

    # ---- the buildings
    park_c = np.array([0.0, 78.0 * SCALE, 0.0])

    def boost(a, b, n):
        """The square is the lit room: the fronts that face it carry most of
        the light, the model's outer faces little."""
        mid = (a + b) / 2
        to = park_c - mid
        d = np.linalg.norm(to[:2])
        facing = np.dot(n[:2], to[:2]) / (d + 1e-9)
        if facing > 0.35 and d < 0.5:
            return 0.12, 1.15
        if facing < -0.2:
            return 0.0, 0.13
        return 0.0, 0.62

    count = 0
    for b in plan():
        kind = b["kind"]
        # a hand places every block: none is quite square to the grid
        allp = [p for tier in b["tiers"] for p in tier[0]]
        cx = sum(p[0] for p in allp) / len(allp) * SCALE
        cy = sum(p[1] for p in allp) / len(allp) * SCALE
        ang = r.normal(0, 0.0022)
        dx, dy = r.normal(0, 0.1 * MM, 2)

        def place(poly):
            out = []
            for x, y in to_model(poly):
                ox, oy = x - cx, y - cy
                out.append((cx + ox * math.cos(ang) - oy * math.sin(ang) + dx, cy + ox * math.sin(ang) + oy * math.cos(ang) + dy))
            return out

        if kind in ("smoke", "frost"):
            mb = MB()
            for poly, z0, z1, style in b["tiers"]:
                acrylic_prism(mb, place(poly), PLATE + z0 * SCALE, PLATE + z1 * SCALE, floor_h=3.75 * SCALE * (1.0 if kind == "smoke" else 3.0))
            for ex in b["extras"]:
                if ex[0] == "pods":
                    # glass rooms hung on the north and south faces at their own heights
                    for (px, py, w, d, za, zb) in ((187, -81.5, 16, 5.0, 52, 70), (187, -99.5, 16, 5.0, 84, 100), (187, -81.5, 16, 5.0, 114, 130), (187, -99.5, 16, 5.0, 142, 160), (190, -81.5, 10, 4.0, 24, 36)):
                        acrylic_prism(mb, place(rect(px, py, px + w, py + d)), PLATE + za * SCALE, PLATE + zb * SCALE, floor_h=0.0)
            add_object(b["name"], mb.build(b["name"], [mats[kind]]), col)
            count += 1
            continue
        mb = MB()
        first = b["tiers"][0]
        lamps = Lamps(r, place(first[0]), PLATE, PLATE + b["tiers"][-1][2] * SCALE, b["share"])
        tops = []
        for poly, z0, z1, style in b["tiers"]:
            pm = place(poly)
            prism(mb, pm, PLATE + z0 * SCALE, PLATE + z1 * SCALE, r.random(3), win=style, lamps=lamps, r=r, face_boost=boost, crown=True)
            tops.append((pm, PLATE + z1 * SCALE))
        # what stands on the roof
        pm, zt = tops[-1]
        for ex in b["extras"]:
            if ex[0] == "tank":
                p = _inside(pm, 4.5 * MM, r)
                if p:
                    _tank(mb, p[0], p[1], zt, r.random(3), r)
            if ex[0] == "bulk":
                for _ in range(ex[1]):
                    p = _inside(pm, 5.0 * MM, r)
                    if p:
                        w, d, h = r.uniform(5, 11) * MM, r.uniform(4, 9) * MM, r.uniform(3.0, 7.5) * MM
                        prism(mb, [(p[0] - w / 2, p[1] - d / 2), (p[0] + w / 2, p[1] - d / 2), (p[0] + w / 2, p[1] + d / 2), (p[0] - w / 2, p[1] + d / 2)], zt, zt + h, r.random(3))
        add_object(b["name"], mb.build(b["name"], [mats[kind], mats["pocket"]]), col)
        _crown_pieces(col, b, r, mats, place)
        count += 1
    # the obelisk on its triangle west of the park
    mb = MB()
    ox, oy = -111.0 * SCALE, 92.0 * SCALE
    prof = [(1.5 * MM, PLATE), (1.5 * MM, PLATE + 1.2 * MM), (0.9 * MM, PLATE + 1.5 * MM), (0.55 * MM, PLATE + 14.0 * MM), (0.0001, PLATE + 15.2 * MM)]
    v, q, uv = revolve(prof, 4)
    mb.add(v + np.array([ox, oy, 0], np.float32), q)
    add_object("Obelisk", mb.build("Obelisk", [mats["brass_flat"]]), col)
    print(f"[model] {count} buildings")
    return col

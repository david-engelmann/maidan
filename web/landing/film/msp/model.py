"""The model's city: a polished black base, a low plate with a brass kerb
for every block, and on the plates the buildings: satin black and graphite
blocks with chamfered arrises, their windows etched as hairlines and a few
of them lit. One tower is smoked glass over a lit core; the clock tower's
cupola and the pyramid are brass.

The ring round the square is the real ring of buildings round Madison
Square, from memory: the clock tower and its annex, the great stepped block
north of it, the dark slab, the tower with the gold pyramid, the lofts
along the west, the wedge at the south-west corner, the slim tower at the
south. Beyond the ring the grid runs on, so the model has no edge in frame.
"""

import math

import numpy as np

from . import craft, site
from .common import MB, add_object, box, collection, revolve, rng
from .site import MM, SCALE, rect

PLATE = 0.8 * MM  # a block's plate stands this far above the street
KERB = 5.0  # city metres of pavement outside the building line
CH = 0.55 * MM  # the width of a chamfer's face
Z = np.array([0.0, 0.0, 1.0])
CITY = (-585.0, -440.0, 550.0, 560.0)  # what the base covers, in city metres

# window rhythms, in city metres. st: 0 a grid, 1 ribbons, 2 piers
LOFT = dict(bay=2.3, floor=3.5, st=0, base=1.5)
OFFICE = dict(bay=1.6, floor=3.3, st=1, base=1.5)
TOWER = dict(bay=1.45, floor=3.1, st=2, base=1.7)
HOTEL = dict(bay=1.8, floor=2.9, st=0, base=1.6)
GRAND = dict(bay=2.1, floor=3.6, st=2, base=1.6)
FINE = dict(bay=1.35, floor=3.0, st=2, base=1.8)
GLASS = dict(bay=1.2, floor=3.0, st=1, base=1.2)

# finishes: value, roughness, metal
FINISH = {
    "obsidian": (0.013, 0.3, 0.0),
    "resin": (0.02, 0.21, 0.0),
    "piano": (0.008, 0.09, 0.0),
    "graphite": (0.042, 0.36, 1.0),
    "gunmetal": (0.07, 0.42, 1.0),
}
DARK = ["obsidian", "obsidian", "obsidian", "resin", "resin", "piano"]
GREY = ["graphite", "graphite", "graphite", "gunmetal"]

# the hero camera's bearing from the fountain and its height angle: the
# blocks it looks over are kept under its sight line
HERO_AZ, HERO_EL = math.radians(190.0), math.radians(18.0)


def to_model(poly):
    return [(x * SCALE, y * SCALE) for x, y in poly]


def chamfer_w(poly, w):
    """Cuts every corner of a convex polygon so that the cut face is w wide.
    Returns the polygon and, for each of its edges, whether it is a cut."""
    n = len(poly)
    P = [np.array(p, float) for p in poly]
    out, cut = [], []
    for i in range(n):
        p, a, b = P[i], P[i - 1], P[(i + 1) % n]
        da, db = a - p, b - p
        la, lb = np.linalg.norm(da), np.linalg.norm(db)
        da, db = da / (la + 1e-12), db / (lb + 1e-12)
        ang = math.acos(float(np.clip(np.dot(da, db), -1, 1)))
        if ang > math.radians(152) or min(la, lb) < 2.5 * w:
            out.append(tuple(p))
            cut.append(False)
            continue
        t = min(w / (2 * math.sin(ang / 2)), 0.3 * la, 0.3 * lb)
        out.append(tuple(p + da * t))
        cut.append(True)
        out.append(tuple(p + db * t))
        cut.append(False)
    return out, cut


def mitre(poly, d):
    """A convex polygon moved in by d, every corner along its bisector. None
    if a side would turn inside out."""
    n = len(poly)
    P = np.array(poly, float)
    out = []
    for i in range(n):
        p, a, b = P[i], P[i - 1], P[(i + 1) % n]
        e0, e1 = p - a, b - p
        e0, e1 = e0 / (np.linalg.norm(e0) + 1e-12), e1 / (np.linalg.norm(e1) + 1e-12)
        n0, n1 = np.array([-e0[1], e0[0]]), np.array([-e1[1], e1[0]])
        out.append(p + (n0 + n1) * (d / max(1.0 + float(np.dot(n0, n1)), 0.25)))
    out = np.array(out)
    for i in range(n):
        j = (i + 1) % n
        if np.dot(out[j] - out[i], P[j] - P[i]) <= 0 and np.linalg.norm(P[j] - P[i]) > 1e-7:
            return None
    return [tuple(q) for q in out]


def _attrs(m, tone, gridp=(0, 0, 0), dims=(0, 0, 0), litp=(0, 0, 0)):
    def f(t):
        return np.tile(np.asarray(t, np.float32)[None], (m, 1))

    return {"tone": f(tone), "gridp": f(gridp), "dims": f(dims), "litp": f(litp)}


def block(mb, poly, z0, z1, tone, style=None, share=0.0, seed=0.0, face_gain=None, roof=True, w=CH, ground=True):
    """One piece of the model: a prism over a convex footprint (model metres,
    counter-clockwise), its upright arrises and the edge of its roof
    chamfered and polished, a parapet round a sunk deck on top. With `style`
    its faces carry an etched grid of windows. Returns the roof's outline."""
    Q, cut = chamfer_w(poly, w)
    n = len(Q)
    ct = 0.45 * w
    T = mitre(Q, ct)
    if T is None:
        T, ct = Q, 0.0
    zc = z1 - ct
    H = zc - z0
    edge = (min(tone[0] * 1.7 + 0.014, 0.22), tone[1] * 0.45, tone[2])
    for k in range(n):
        a = np.array([Q[k][0], Q[k][1], 0.0])
        b = np.array([Q[(k + 1) % n][0], Q[(k + 1) % n][1], 0.0])
        L = float(np.linalg.norm(b - a))
        if L < 1e-7:
            continue
        v = np.array([[a[0], a[1], z0], [b[0], b[1], z0], [b[0], b[1], zc], [a[0], a[1], zc]])
        at = _attrs(1, edge if cut[k] else tone)
        if not cut[k] and style is not None:
            bay, fl = style["bay"] * SCALE, style["floor"] * SCALE
            base = style["base"] * fl if ground else 0.3 * fl
            mx = max(0.7 * MM, 0.3 * bay)
            nb = int((L - 2 * mx) / bay)
            nf = int((H - base - 0.4 * fl) / fl)
            if nb >= 2 and nf >= 1:
                sh, gain = share, 1.0
                if face_gain is not None:
                    U = (b - a) / L
                    sh, gain = face_gain((a + b) / 2, np.array([U[1], -U[0], 0.0]), share)
                at = _attrs(1, tone, (bay, fl, style["st"]), (nb * bay / 2, base, base + nf * fl), (sh, seed + k * 3.7, gain))
        mb.add(v, [[0, 1, 2, 3]], uv=[(-L / 2, 0.0), (L / 2, 0.0), (L / 2, H), (-L / 2, H)], fattr=at)
    if ct > 0:
        for k in range(n):
            j = (k + 1) % n
            mb.add([(*Q[k], zc), (*Q[j], zc), (*T[j], z1), (*T[k], z1)], [[0, 1, 2, 3]], fattr=_attrs(1, edge))
    deck = (tone[0] * 0.8, min(tone[1] * 1.3, 0.62), tone[2])
    pw, pd = 0.7 * MM, 0.5 * MM
    inner = mitre(T, pw) if roof else None
    if inner is not None and abs(site.area(inner)) > (5 * MM) ** 2:
        vv = np.array([(*p, z1) for p in T] + [(*p, z1) for p in inner] + [(*p, z1 - pd) for p in inner])
        faces = []
        for k in range(n):
            j = (k + 1) % n
            faces.append([k, j, n + j, n + k])
            faces.append([n + k, n + j, 2 * n + j, 2 * n + k])
        mb.add(vv, np.array(faces), fattr=_attrs(len(faces), tone))
        mb.add(vv[2 * n:], [list(range(n))], fattr=_attrs(1, deck))
    else:
        mb.add(np.array([(*p, z1) for p in T]), [list(range(n))], fattr=_attrs(1, deck))
    return T


def glass_block(mb, poly, z0, z1, floor_h=0.0, w=0.9 * MM):
    """A solid block of glass, closed all round, its arrises cut and polished."""
    Q, cut = chamfer_w(poly, w)
    n = len(Q)
    ct = 0.6 * w
    T = mitre(Q, ct) or Q
    zc = z1 - ct
    for k in range(n):
        j = (k + 1) % n
        mb.add([(*Q[k], z0), (*Q[j], z0), (*Q[j], zc), (*Q[k], zc)], [[0, 1, 2, 3]], fattr={"floor_h": np.array([0.0 if cut[k] else floor_h])})
        mb.add([(*Q[k], zc), (*Q[j], zc), (*T[j], z1), (*T[k], z1)], [[0, 1, 2, 3]], fattr={"floor_h": np.array([0.0])})
    mb.add(np.array([(*p, z1) for p in T]), [list(range(n))], fattr={"floor_h": np.array([0.0])})
    mb.add(np.array([(*p, z0) for p in Q])[::-1], [list(range(n))], fattr={"floor_h": np.array([0.0])})


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


def hcap(x, y):
    """How tall a block at (x, y), city metres, may stand without rising into
    the hero's view of the square."""
    cx, cy = math.sin(HERO_AZ), math.cos(HERO_AZ)
    s = x * cx + y * cy
    t = x * cy - y * cx
    if s < 30 or abs(t - 6) > 62 + 0.14 * s:
        return 1e9
    return max(9.0, s * math.tan(HERO_EL) * 0.78 - 8.0)


# ------------------------------------------------------------------ the plan

# the grid beyond the ring: streets as bands of y, avenues as bands of x
Y_RING = [(-133, -72), (-42, 26), (44, 112), (130, 198), (220, 281)]
Y_SOUTH = [(-224, -155), (-315, -246), (-406, -337)]
Y_NORTH = [(303, 364), (386, 447), (469, 530)]
X_WEST = [(-400, -266), (-562, -428)]
X_EAST = [(254, 376), (404, 526)]
X_RING = [(-238, -104), (-75, 75), (104, 226)]


def outer_blocks():
    out = []
    for yb in Y_RING:
        out += [(xb, yb) for xb in X_WEST + X_EAST]
    for yb in Y_SOUTH:
        out += [(xb, yb) for xb in X_WEST + [(-238, -104), (-75, 226)] + X_EAST]
    for yb in Y_NORTH:
        out += [(xb, yb) for xb in X_WEST + X_RING + X_EAST]
    return out


def plan():
    """Every building on the base: (name, kind, share of windows lit, tiers,
    extras). A tier is (footprint, from, to, window rhythm) in city metres.
    Kinds: dark and grey are the two families of finish; smoke is glass."""
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
    add("clock_tower", "dark", 0.1, [
        (tower, 0, 150, FINE),
        (site.inset(tower, 1.4), 150, 167, dict(bay=2.2, floor=6.5, st=2, base=0.3)),
    ], extras=[("clock",), ("spire",)])
    add("annex_east", "grey", 0.07, stepped(rect(132, -42, 226, 26), [(52, 0), (60, 5.0), (66, 11.0)], OFFICE), extras=[("bulk", 2)])
    add("annex_south", "dark", 0.07, stepped(rect(104, -42, 130, -2), [(52, 0), (58, 4.0)], OFFICE))
    # the great stepped block: it was meant to be a hundred storeys
    nb = rect(104, 44, 226, 112)
    add("north_building", "dark", 0.1, [
        (site.chamfer(nb, 7.0), 0, 60, GRAND),
        (site.chamfer(site.inset(nb, 8.0), 8.0), 60, 84, OFFICE),
        (site.chamfer(site.inset(nb, [14.0, 20.0, 14.0, 20.0]), 7.0), 84, 110, OFFICE),
        (site.chamfer(site.inset(nb, [20.0, 34.0, 20.0, 34.0]), 5.0), 110, 142, TOWER),
        (site.chamfer(site.inset(nb, [25.0, 42.0, 25.0, 42.0]), 3.0), 142, 150, None),
    ], extras=[("bulk", 2)])
    # the courthouse, the dark slab and its neighbours
    add("courthouse", "grey", 0.16, [(rect(104, 130, 128, 156), 0, 17, dict(bay=2.6, floor=5.0, st=2, base=0.25)), (site.inset(rect(104, 130, 128, 156), 2.5), 17, 20, None)])
    add("slab_podium", "grey", 0.12, [(rect(104, 160, 148, 198), 0, 9, None)])
    add("slab", "smoke", 0.22, [(site.inset(rect(104, 160, 148, 198), 3.0), 9, 196, None)])
    add("east_loft_a", "grey", 0.07, stepped(rect(132, 130, 226, 157), [(50, 0)], LOFT), extras=[("bulk", 2)])
    add("east_loft_b", "dark", 0.08, stepped(rect(152, 160, 226, 198), [(58, 0), (68, 4.5)], OFFICE), extras=[("bulk", 2)])
    # the tower with the gold pyramid
    nyl = rect(104, 220, 226, 281)
    add("pyramid_tower", "dark", 0.1, [
        (nyl, 0, 54, GRAND),
        (site.inset(nyl, [6.0, 10.0, 6.0, 10.0]), 54, 76, OFFICE),
        (site.chamfer(rect(140, 232, 190, 269), 4.0), 76, 142, TOWER),
        (site.chamfer(rect(144, 235, 186, 266), 4.0), 142, 168, TOWER),
    ], extras=[("pyramid",)])
    # ---- north of the park, west to east: the far wall of the room, and two tall, slender ones
    add("north_a", "grey", 0.07, stepped(rect(-75, 220, -28, 281), [(46, 0), (50, 3.0)], LOFT), extras=[("bulk", 1)])
    add("north_b", "dark", 0.11, stepped(rect(-26, 220, 8, 272), [(58, 0), (176, 5.0), (214, 8.0), (236, 10.5)], FINE), extras=[("needle",)])
    add("north_c", "grey", 0.08, stepped(rect(10, 220, 36, 268), [(74, 0), (98, 4.0)], HOTEL))
    add("north_d", "dark", 0.07, stepped(rect(38, 220, 75, 281), [(40, 0), (46, 5.0)], LOFT), extras=[("bulk", 2)])

    # ---- west of the park: lofts, cut on the diagonal where Broadway passes
    def west(name, y0, y1, pieces):
        """Pieces across a block west of Fifth Avenue: (x0, x1, kind, share, steps, style, extras)."""
        for k, (x0, x1, kind, share, steps, style, extras) in enumerate(pieces):
            poly = rect(x0, y0, x1, y1)
            w, e = site.broadway_sides(poly)
            for side, pc in (("w", w), ("e", e)):
                if len(pc) >= 3 and site.area(pc) > 250:
                    add(f"{name}_{k}{side}", kind, share, stepped(pc, steps, style), extras)

    west("w22", -133, -72, [(-160, -104, "dark", 0.06, [(52, 0), (58, 4.0)], LOFT, []), (-200, -162, "grey", 0.05, [(40, 0)], LOFT, [("bulk", 1)]), (-238, -202, "dark", 0.05, [(88, 0), (96, 3.0)], HOTEL, [("bulk", 1)])])
    west("w23", -42, 26, [(-166, -104, "dark", 0.07, [(56, 0), (62, 4.0)], GRAND, [("bulk", 2)]), (-202, -168, "grey", 0.05, [(54, 0)], LOFT, []), (-238, -204, "dark", 0.05, [(42, 0)], LOFT, [("bulk", 1)])])
    west("w24", 44, 112, [(-190, -104, "grey", 0.07, [(58, 0), (64, 3.5)], LOFT, [("bulk", 1)]), (-238, -192, "dark", 0.05, [(46, 0)], LOFT, [("bulk", 1)])])
    west("w25", 130, 198, [(-136, -104, "dark", 0.07, [(44, 0)], LOFT, []), (-196, -138, "grey", 0.05, [(60, 0), (66, 4.0)], OFFICE, [("bulk", 2)]), (-238, -198, "dark", 0.05, [(50, 0)], LOFT, [])])
    west("w26", 220, 281, [(-150, -104, "grey", 0.06, [(56, 0)], LOFT, [("bulk", 1)]), (-200, -152, "dark", 0.05, [(40, 0)], LOFT, []), (-238, -202, "grey", 0.05, [(52, 0)], OFFICE, [("bulk", 1)])])
    # ---- south of 23rd Street
    blk = rect(-75, -133, 226, -72)
    wedge, rest = site.broadway_sides(blk)
    if len(wedge) >= 3:
        add("wedge", "dark", 0.06, [(wedge, 0, 82, FINE), (site.inset(wedge, 0.8), 82, 87, None)])
    for k, (x0, x1, y0, y1, kind, share, steps, style, extras) in enumerate([
        # the row on the square's axis is kept low: the way in passes over it
        (-52, -8, -104, -72, "grey", 0.06, [(20, 0)], LOFT, []),
        (-6, 24, -104, -72, "dark", 0.06, [(16, 0)], LOFT, []),
        (26, 52, -104, -72, "grey", 0.06, [(24, 0)], LOFT, [("bulk", 1)]),
        (54, 80, -100, -72, "dark", 0.06, [(38, 0)], LOFT, []),
        (-30, 8, -133, -106, "grey", 0.06, [(20, 0)], HOTEL, []),
        (10, 50, -133, -106, "dark", 0.06, [(22, 0)], LOFT, [("bulk", 1)]),
        (52, 80, -133, -102, "grey", 0.06, [(42, 0)], LOFT, []),
        (84, 128, -133, -72, "dark", 0.07, [(46, 0), (52, 4.0)], OFFICE, [("bulk", 1)]),
        (130, 164, -133, -72, "grey", 0.07, [(58, 0), (66, 5.0)], OFFICE, [("bulk", 2)]),
        (190, 226, -133, -104, "dark", 0.06, [(30, 0)], LOFT, [("bulk", 1)]),
    ]):
        poly = rect(x0, y0, x1, y1)
        w, e = site.broadway_sides(poly)
        pc = e if x0 > -60 else w
        if len(pc) >= 3 and site.area(pc) > 200:
            add(f"s22_{k}", kind, share, stepped(pc, steps, style), extras)
    # the slim tower: a shaft with glass rooms cantilevered off it
    add("slim_base", "grey", 0.1, [(rect(168, -102, 226, -72), 0, 7, None)])
    add("slim_tower", "dark", 0.1, [(rect(189, -95, 201, -81), 7, 196, GLASS)], extras=[("pods",)])

    # ---- beyond the ring: the grid runs on
    r = rng(2026)
    for (bx0, bx1), (by0, by1) in outer_blocks():
        x = bx0
        k = 0
        while x < bx1 - 1:
            wd = float(r.choice([26, 30, 34, 40, 46, 54, 62]))
            x1 = bx1 if bx1 - (x + wd) < 22 else x + wd
            halves = [(by0, by1)] if r.random() < 0.45 else [(by0, (by0 + by1) / 2 + r.uniform(-6, 6)), None]
            if halves[-1] is None:
                halves[-1] = (halves[0][1], by1)
            for (y0, y1) in halves:
                lot = rect(x + 1.0, y0, x1 - 1.0, y1)
                for pc in site.broadway_sides(lot):
                    if len(pc) < 3 or site.area(pc) < 420:
                        continue
                    cx = sum(p[0] for p in pc) / len(pc)
                    cy = sum(p[1] for p in pc) / len(pc)
                    h = float(r.choice([22, 28, 34, 40, 46, 54, 62, 74, 90], p=[0.08, 0.12, 0.16, 0.18, 0.16, 0.12, 0.09, 0.06, 0.03])) * r.uniform(0.92, 1.08)
                    tall = r.random() < 0.07 and site.area(pc) > 900
                    if tall:
                        h = r.uniform(120, 200)
                    h = min(h, hcap(cx, cy))
                    style = [LOFT, LOFT, OFFICE, HOTEL, GRAND][int(r.integers(0, 5))] if h < 70 else [OFFICE, TOWER, FINE, GLASS][int(r.integers(0, 4))]
                    if tall:
                        steps = [(h * 0.3, 0), (h * 0.82, 5.0), (h, 8.5)]
                    elif h > 30 and r.random() < 0.45:
                        steps = [(h * r.uniform(0.82, 0.92), 0), (h, r.uniform(3.0, 5.0))]
                    else:
                        steps = [(h, 0)]
                    add(f"o_{int(bx0)}_{int(by0)}_{k}", "dark" if r.random() < 0.55 else "grey", r.uniform(0.03, 0.07), stepped(pc, steps, style), [("bulk", 1)] if r.random() < 0.5 else [])
                    k += 1
            x = x1
    return B


def plates():
    """The plate under every block: the block and its pavement, as one piece."""
    out = []
    for poly in (rect(104, -42, 226, 26), rect(104, 44, 226, 112), rect(104, 130, 226, 198), rect(104, 220, 226, 281), rect(-75, 220, 75, 281)):
        out.append(poly)
    for y0, y1 in Y_RING:
        w, e = site.broadway_sides(rect(-238, y0, -104, y1))
        out += [p for p in (w, e) if len(p) >= 3]
    w, e = site.broadway_sides(rect(-75, -133, 226, -72))
    out += [p for p in (w, e) if len(p) >= 3]
    for (bx0, bx1), (by0, by1) in outer_blocks():
        out += [p for p in site.broadway_sides(rect(bx0, by0, bx1, by1)) if len(p) >= 3]
    return [site.outset(p, KERB) for p in out]


# ------------------------------------------------------------------ building


def _crown_pieces(col, b, r, mats, to_model, tone):
    """The brass: clock faces, the tower's cupola, the pyramid, a needle.
    Turned parts are their own objects, so the lathe's rings run round each
    one's own axis."""
    brass = MB()
    roof = MB()
    name = b["name"]

    def turned(tag, prof, cx, cy, segs=40):
        mb = MB()
        v, q, uv = revolve(prof, segs)
        mb.add(v, q, smooth=True)
        add_object(f"{name}_{tag}", mb.build(f"{name}_{tag}", [mats["brass"]], smooth_angle=math.radians(40)), col, loc=(cx, cy, 0.0))

    for ex in b["extras"]:
        if ex[0] == "clock":
            poly = to_model(b["tiers"][0][0])
            zc = PLATE + 124.0 * SCALE
            rad = 4.2 * SCALE
            n = len(poly)
            for i in range(n):
                a = np.array([poly[i][0], poly[i][1], 0.0])
                bb = np.array([poly[(i + 1) % n][0], poly[(i + 1) % n][1], 0.0])
                U = (bb - a) / np.linalg.norm(bb - a)
                N = np.array([U[1], -U[0], 0.0])
                c = (a + bb) / 2 + Z * zc
                # a dial: a brass ring proud of the face, a dark disc inside it
                prof = [(rad * 0.8, 0.0), (rad * 0.8, 0.45 * MM), (rad, 0.45 * MM), (rad, 0.0)]
                v, q, uv = revolve(prof, 48)
                vv = c[None] + v[:, 0:1] * U[None] + v[:, 1:2] * Z[None] + v[:, 2:3] * N[None]
                brass.add(vv, q, mat=0, smooth=True)
        if ex[0] == "spire":
            poly = to_model(b["tiers"][-1][0])
            z0 = PLATE + b["tiers"][-1][2] * SCALE
            cx = sum(p[0] for p in poly) / 4
            cy = sum(p[1] for p in poly) / 4
            hw = abs(poly[1][0] - poly[0][0]) / 2
            hd = abs(poly[2][1] - poly[1][1]) / 2
            # a cornice slab, then the steep roof, its faces a little hollow
            e = 0.8 * MM
            block(roof, [(cx - hw - e, cy - hd - e), (cx + hw + e, cy - hd - e), (cx + hw + e, cy + hd + e), (cx - hw - e, cy + hd + e)], z0, z0 + 1.4 * MM, tone, roof=False, w=0.3 * MM)
            steps = 10
            rings = []
            top = 2.6 * SCALE
            zb = z0 + 1.4 * MM
            zt = zb + 36.0 * SCALE
            for k in range(steps + 1):
                s = k / steps
                f = 1 - (1 - top / hw) * s ** 0.85
                zz = zb + (zt - zb) * s
                rings.append([(cx - hw * f, cy - hd * f, zz), (cx + hw * f, cy - hd * f, zz), (cx + hw * f, cy + hd * f, zz), (cx - hw * f, cy + hd * f, zz)])
            rv = np.array(rings).reshape(-1, 3)
            rq = [[k * 4 + j, k * 4 + (j + 1) % 4, (k + 1) * 4 + (j + 1) % 4, (k + 1) * 4 + j] for k in range(steps) for j in range(4)]
            roof.add(rv, rq, fattr=_attrs(len(rq), (tone[0], tone[1] * 0.8, tone[2])))
            # the cupola: a turned brass lantern, a dome, a finial
            m = MM
            prof = [(3.0 * m, zt), (3.0 * m, zt + 0.45 * m), (2.5 * m, zt + 0.6 * m), (2.2 * m, zt + 1.0 * m), (2.2 * m, zt + 4.2 * m), (2.6 * m, zt + 4.5 * m), (2.6 * m, zt + 4.8 * m), (2.3 * m, zt + 5.1 * m), (2.0 * m, zt + 6.2 * m), (1.4 * m, zt + 7.2 * m), (0.6 * m, zt + 7.9 * m), (0.38 * m, zt + 8.4 * m), (0.5 * m, zt + 8.9 * m), (0.26 * m, zt + 9.4 * m), (0.16 * m, zt + 15.0 * m), (0.0001, zt + 15.6 * m)]
            turned("cupola", prof, cx, cy)
        if ex[0] == "pyramid":
            poly = to_model(b["tiers"][-1][0])
            z0 = PLATE + b["tiers"][-1][2] * SCALE
            cx = sum(p[0] for p in poly) / len(poly)
            cy = sum(p[1] for p in poly) / len(poly)
            xs = [p[0] for p in poly]
            ys = [p[1] for p in poly]
            hw, hd = (max(xs) - min(xs)) / 2 - 0.5 * MM, (max(ys) - min(ys)) / 2 - 0.5 * MM
            zt = z0 + 30.0 * SCALE
            # a stepped foot, then the pyramid itself, then a lantern
            rings = []
            for f, zz in ((1.0, z0), (1.0, z0 + 0.7 * MM), (0.965, z0 + 0.7 * MM), (0.07, zt)):
                rings.append([(cx - hw * f, cy - hd * f, zz), (cx + hw * f, cy - hd * f, zz), (cx + hw * f, cy + hd * f, zz), (cx - hw * f, cy + hd * f, zz)])
            rv = np.array(rings).reshape(-1, 3)
            rq = [[k * 4 + j, k * 4 + (j + 1) % 4, (k + 1) * 4 + (j + 1) % 4, (k + 1) * 4 + j] for k in range(3) for j in range(4)]
            brass.add(rv, rq, mat=1)
            prof = [(1.0 * MM, zt - 0.4 * MM), (1.0 * MM, zt + 1.3 * MM), (1.25 * MM, zt + 1.5 * MM), (0.7 * MM, zt + 2.5 * MM), (0.2 * MM, zt + 3.0 * MM), (0.14 * MM, zt + 7.5 * MM), (0.0001, zt + 8.0 * MM)]
            turned("lantern", prof, cx, cy, 24)
        if ex[0] == "needle":
            poly = to_model(b["tiers"][-1][0])
            z0 = PLATE + b["tiers"][-1][2] * SCALE
            cx = sum(p[0] for p in poly) / len(poly)
            cy = sum(p[1] for p in poly) / len(poly)
            prof = [(1.5 * MM, z0), (1.5 * MM, z0 + 0.5 * MM), (0.5 * MM, z0 + 0.9 * MM), (0.22 * MM, z0 + 9.0 * MM), (0.12 * MM, z0 + 26.0 * MM), (0.0001, z0 + 27.0 * MM)]
            turned("needle", prof, cx, cy, 20)
    if brass.nv:
        add_object(f"{name}_brass", brass.build(f"{name}_brass", [mats["brass"], mats["brass_flat"]]), col)
    if roof.nv:
        add_object(f"{name}_roof", roof.build(f"{name}_roof", [mats["obs"]]), col)


def build(col=None):
    col = col or collection("Model")
    r = rng(400)
    mats = {
        "obs": craft.facade(),
        "pave": craft.satin("pavement", (0.014, 0.014, 0.016), rough=0.34),
        "kerb": craft.brass("brass_kerb", turned=False, rough=0.24, bevel=False),
        "brass": craft.brass("brass_turned", turned=True, rough=0.16),
        "brass_flat": craft.brass("brass_sheet", turned=False, rough=0.2),
        "smoke": craft.smoked(),
        "base": craft.gloss(),
    }
    x0, y0, x1, y1 = [v * SCALE for v in CITY]

    # ---- the base: one sheet of polished black, far wider than any frame
    mb = MB()
    v, q = box((x0, y0, -12 * MM), (x1, y1, 0.0))
    mb.add(v, q, mat=0)
    add_object("Baseplate", mb.build("Baseplate", [mats["base"]]), col)

    # ---- the plates: satin tops, and a kerb of brass let in all the way round
    mb = MB()
    lip = 0.22 * MM

    def plate(poly, z1=PLATE):
        pm = to_model(poly)
        n = len(pm)
        inner = mitre(pm, lip) or pm
        ring0 = [(*p, 0.0) for p in pm]
        ring1 = [(*p, z1) for p in pm]
        ring2 = [(*p, z1) for p in inner]
        vv = np.array(ring0 + ring1 + ring2)
        faces = []
        for k in range(n):
            j = (k + 1) % n
            faces.append([k, j, n + j, n + k])
            faces.append([n + k, n + j, 2 * n + j, 2 * n + k])
        mb.add(vv, np.array(faces), mat=1)
        mb.add(vv[2 * n:], [list(range(n))], mat=0)

    for poly in plates():
        plate(poly)
    add_object("Plates", mb.build("Plates", [mats["pave"], mats["kerb"]]), col)

    # ---- the buildings
    park_c = np.array([0.0, 60.0 * SCALE, 0.0])

    def face_gain(mid, n, share):
        """The square is the lit room: the fronts that wall it carry the most
        light. The model's west, where the page's headline sits, is quiet."""
        to = park_c - mid
        d = float(np.linalg.norm(to[:2]))
        facing = float(np.dot(n[:2], to[:2])) / (d + 1e-9)
        quiet = 0.45 if mid[0] < -80 * SCALE else 1.0
        if facing > 0.35 and d < 0.62:
            return share * 1.7 * quiet, 1.2
        if facing < -0.2:
            return share * 0.5 * quiet, 0.75
        return share * quiet, 0.9

    count = 0
    for b in plan():
        kind = b["kind"]
        name = b["name"]
        # a hand places every block: none is quite square to the grid
        allp = [p for tier in b["tiers"] for p in tier[0]]
        cx = sum(p[0] for p in allp) / len(allp) * SCALE
        cy = sum(p[1] for p in allp) / len(allp) * SCALE
        ang = r.normal(0, 0.0016)
        dx, dy = r.normal(0, 0.06 * MM, 2)

        def place(poly):
            out = []
            for x, y in to_model(poly):
                ox, oy = x - cx, y - cy
                out.append((cx + ox * math.cos(ang) - oy * math.sin(ang) + dx, cy + ox * math.sin(ang) + oy * math.cos(ang) + dy))
            return out

        if kind == "smoke":
            mb = MB()
            core = MB()
            for poly, z0, z1, style in b["tiers"]:
                glass_block(mb, place(poly), PLATE + z0 * SCALE, PLATE + z1 * SCALE, floor_h=3.4 * SCALE)
                # the core the glass is cast round: graphite, with its own lit floors
                block(core, place(site.inset(poly, 9.5)), PLATE + z0 * SCALE + 0.2 * MM, PLATE + (z1 - 5.0) * SCALE, FINISH["graphite"], style=GLASS, share=b["share"], seed=float(r.uniform(0, 90)), roof=False, ground=False)
            add_object(name, mb.build(name, [mats["smoke"]]), col)
            add_object(name + "_core", core.build(name + "_core", [mats["obs"]]), col)
            count += 1
            continue
        fin = FINISH[(DARK if kind == "dark" else GREY)[int(r.integers(0, 6 if kind == "dark" else 4))]]
        tone = (fin[0] * r.uniform(0.85, 1.2), fin[1] * r.uniform(0.9, 1.12), fin[2])
        seed = float(r.uniform(0, 90))
        mb = MB()
        tops = []
        for i, (poly, z0, z1, style) in enumerate(b["tiers"]):
            pm = place(poly)
            T = block(mb, pm, PLATE + z0 * SCALE - (0.6 * MM if i else 0.0), PLATE + z1 * SCALE, tone, style=style, share=b["share"], seed=seed + i * 11.0, face_gain=face_gain, ground=(i == 0))
            tops.append((T, PLATE + z1 * SCALE))
        # what stands on the roof: small, and really small
        pm, zt = tops[-1]
        for ex in b["extras"]:
            if ex[0] == "bulk":
                for _ in range(ex[1]):
                    p = _inside(pm, 3.2 * MM, r)
                    if p:
                        w, d, h = r.uniform(2.6, 6.5) * MM, r.uniform(2.4, 5.0) * MM, r.uniform(1.4, 4.2) * MM
                        block(mb, [(p[0] - w / 2, p[1] - d / 2), (p[0] + w / 2, p[1] - d / 2), (p[0] + w / 2, p[1] + d / 2), (p[0] - w / 2, p[1] + d / 2)], zt - 0.5 * MM, zt + h, tone, roof=False, w=0.3 * MM)
            if ex[0] == "pods":
                # glass rooms hung on the north and south faces at their own heights
                for (px, py, w, d, za, zb) in ((187, -81.5, 16, 5.0, 52, 70), (187, -99.5, 16, 5.0, 84, 100), (187, -81.5, 16, 5.0, 114, 130), (187, -99.5, 16, 5.0, 142, 160), (190, -81.5, 10, 4.0, 24, 36)):
                    block(mb, place(rect(px, py, px + w, py + d)), PLATE + za * SCALE, PLATE + zb * SCALE, FINISH["piano"], style=GLASS, share=0.2, seed=seed + za, roof=False, ground=False)
        add_object(name, mb.build(name, [mats["obs"]]), col)
        _crown_pieces(col, b, r, mats, place, tone)
        count += 1
    # the obelisk on its triangle west of the park
    mb = MB()
    ox, oy = -111.0 * SCALE, 92.0 * SCALE
    prof = [(1.2 * MM, PLATE), (1.2 * MM, PLATE + 1.0 * MM), (0.7 * MM, PLATE + 1.2 * MM), (0.42 * MM, PLATE + 13.0 * MM), (0.0001, PLATE + 14.0 * MM)]
    v, q, uv = revolve(prof, 4)
    mb.add(v + np.array([ox, oy, 0], np.float32), q)
    add_object("Obelisk", mb.build("Obelisk", [mats["brass_flat"]]), col)
    print(f"[model] {count} buildings")
    return col

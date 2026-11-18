"""The plan of the place, in city metres. The fountain is the origin, +y is
north along the avenues and +x east along the streets. The model is built
from this plan at 1:400 (see SCALE).

Madison Square Park and the ring of blocks round it, from memory of the
real place rather than a survey: the park between 23rd and 26th Streets and
between Fifth and Madison Avenues, Broadway crossing Fifth on the diagonal
at its south-west corner, long blocks to the west and short ones to the
east.
"""

import math

import numpy as np

from .common import rng, smooth_path

SCALE = 1.0 / 400.0
MM = 0.001

PLAZA_R = 20.0
BASIN_R = 5.66
PARK = (-75.0, -42.0, 75.0, 198.0)  # x0, y0, x1, y1
LAWN_C = (2.0, 104.0)
LAWN_R = (31.0, 43.0)
# what the baseplate covers
EXTENT = (-238.0, -137.0, 230.0, 285.0)

# Broadway runs on the diagonal: its centre line, south to north. South of
# 23rd Street it lies east of Fifth Avenue, with the wedge block between
# them; it crosses Fifth between 23rd and 25th and goes on north-west.
BROADWAY = [(82.0, -420.0), (15.0, -260.0), (-22.0, -172.0), (-37.0, -136.0), (-62.0, -76.0), (-97.0, 35.0), (-148.6, 198.0), (-186.0, 316.0), (-257.0, 540.0)]
BROADWAY_HW = 11.5


# ---------------------------------------------------------------- polygons


def clip(poly, point, normal):
    """The part of a convex polygon behind a line (the side its normal points away from)."""
    out = []
    n = len(poly)
    px, py = point
    nx, ny = normal
    for i in range(n):
        a = poly[i]
        b = poly[(i + 1) % n]
        da = (a[0] - px) * nx + (a[1] - py) * ny
        db = (b[0] - px) * nx + (b[1] - py) * ny
        if da <= 0:
            out.append(a)
        if (da < 0) != (db < 0) and abs(da - db) > 1e-9:
            t = da / (da - db)
            out.append((a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t))
    return out


def area(poly):
    a = 0.0
    for i in range(len(poly)):
        x0, y0 = poly[i]
        x1, y1 = poly[(i + 1) % len(poly)]
        a += x0 * y1 - x1 * y0
    return a / 2


def inset(poly, d):
    """A convex polygon (counter-clockwise) moved in by d on every side. d may
    be one number or one per side."""
    out = poly
    n = len(poly)
    ds = [d] * n if np.isscalar(d) else list(d)
    for i in range(n):
        a = poly[i]
        b = poly[(i + 1) % n]
        ex, ey = b[0] - a[0], b[1] - a[1]
        ln = math.hypot(ex, ey)
        if ln < 1e-6:
            continue
        nx, ny = ey / ln, -ex / ln  # outward for a counter-clockwise polygon
        out = clip(out, (a[0] - nx * ds[i], a[1] - ny * ds[i]), (nx, ny))
        if len(out) < 3:
            return []
    return out


def outset(poly, d):
    """A convex polygon (counter-clockwise) grown by d on every side."""
    n = len(poly)
    lines = []
    for i in range(n):
        a = poly[i]
        b = poly[(i + 1) % n]
        ex, ey = b[0] - a[0], b[1] - a[1]
        ln = math.hypot(ex, ey)
        nx, ny = ey / ln, -ex / ln
        lines.append(((a[0] + nx * d, a[1] + ny * d), (ex / ln, ey / ln)))
    out = []
    for i in range(n):
        (p, e), (q, f) = lines[i - 1], lines[i]
        den = e[0] * f[1] - e[1] * f[0]
        if abs(den) < 1e-9:
            out.append(q)
            continue
        t = ((q[0] - p[0]) * f[1] - (q[1] - p[1]) * f[0]) / den
        out.append((p[0] + e[0] * t, p[1] + e[1] * t))
    return out


def rect(x0, y0, x1, y1):
    return [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]


def chamfer(poly, c):
    """Cuts every corner of a convex polygon back by c along its sides."""
    out = []
    n = len(poly)
    for i in range(n):
        p = np.array(poly[i])
        a = np.array(poly[i - 1])
        b = np.array(poly[(i + 1) % n])
        da = (a - p) / (np.linalg.norm(a - p) + 1e-9)
        db = (b - p) / (np.linalg.norm(b - p) + 1e-9)
        out.append(tuple(p + da * c))
        out.append(tuple(p + db * c))
    return out


def broadway_sides(poly):
    """A block cut by Broadway: the piece to its west and the piece to its
    east (either may be empty). Each block here is crossed by one straight
    stretch of the avenue, so both pieces stay convex."""
    ys = [p[1] for p in poly]
    ym = (min(ys) + max(ys)) / 2
    a, b = BROADWAY[0], BROADWAY[1]
    for i in range(len(BROADWAY) - 1):
        a, b = BROADWAY[i], BROADWAY[i + 1]
        if a[1] <= ym <= b[1]:
            break
    ex, ey = b[0] - a[0], b[1] - a[1]
    ln = math.hypot(ex, ey)
    nx, ny = ey / ln, -ex / ln  # toward the east of the line, going north
    west = clip(poly, (a[0] - nx * BROADWAY_HW, a[1] - ny * BROADWAY_HW), (nx, ny))
    east = clip(poly, (a[0] + nx * BROADWAY_HW, a[1] + ny * BROADWAY_HW), (-nx, -ny))
    return (west if len(west) >= 3 and area(west) > 40 else []), (east if len(east) >= 3 and area(east) > 40 else [])


# ---------------------------------------------------------------- the park
# Paths: (control points, half width). Left and right are close to mirrored,
# and deliberately not exactly.
PATHS = [
    ([(0, -19), (0, -30), (0, -42)], 4.6),
    ([(0, 19), (0.5, 34), (1.5, 50), (2, 58)], 3.4),
    ([(14.5, 14.0), (27, 28), (44, 40), (62, 47), (75, 50)], 2.7),
    ([(-14, 14.5), (-26, 29), (-43, 41), (-60, 49), (-75, 53)], 2.7),
    ([(20, 1.0), (34, -3), (52, -7), (75, -9)], 2.9),
    ([(-20, 0.5), (-36, -2), (-54, -5), (-75, -8)], 2.9),
    ([(14, -14.5), (30, -24), (48, -33), (62, -42)], 2.6),
    ([(-14.5, -14), (-31, -23), (-47, -34), (-60, -42)], 2.6),
    ([(34, 106), (50, 108), (75, 111)], 2.5),
    ([(-30, 104), (-50, 107), (-75, 108)], 2.5),
    ([(2, 149), (2.5, 170), (2, 198)], 3.4),
    ([(-75, 168), (-44, 163), (-18, 156), (2, 154), (24, 157), (48, 164), (75, 170)], 2.5),
    # the long curving walks through the trees either side of the lawn
    ([(-20, 26), (-46, 60), (-52, 104), (-44, 140), (-22, 158)], 2.2),
    ([(21, 25), (50, 62), (55, 106), (46, 142), (26, 160)], 2.2),
]
LAWN_WALK_HW = 2.6


def path_points(i, n=64):
    pts, hw = PATHS[i]
    return smooth_path(np.array(pts, float), n), hw


def lawn_walk(n=160):
    a = np.linspace(0, math.tau, n, endpoint=False)
    return np.stack([LAWN_C[0] + (LAWN_R[0] + LAWN_WALK_HW) * np.cos(a), LAWN_C[1] + (LAWN_R[1] + LAWN_WALK_HW) * np.sin(a)], axis=-1)


def seg_dist(p, a, b):
    """Distance from points p (n, 2) to the polyline a->b segments (m, 2)."""
    ab = b - a
    t = np.clip(((p[:, None, :] - a[None]) * ab[None]).sum(-1) / ((ab * ab).sum(-1)[None] + 1e-12), 0, 1)
    d = p[:, None, :] - (a[None] + t[..., None] * ab[None])
    return np.sqrt((d * d).sum(-1)).min(axis=1)


_PATH_CACHE = None


def all_paths():
    global _PATH_CACHE
    if _PATH_CACHE is None:
        _PATH_CACHE = [path_points(i) for i in range(len(PATHS))]
        w = lawn_walk()
        _PATH_CACHE.append((np.concatenate([w, w[:1]]), LAWN_WALK_HW))
    return _PATH_CACHE


def path_clearance(p):
    """How far each point is outside the nearest path's edge (negative: on it)."""
    p = np.asarray(p, float).reshape(-1, 2)
    best = np.full(len(p), 1e9)
    for pts, hw in all_paths():
        best = np.minimum(best, seg_dist(p, pts[:-1], pts[1:]) - hw)
    return best


def in_lawn(p, grow=0.0):
    p = np.asarray(p, float).reshape(-1, 2)
    return ((p[:, 0] - LAWN_C[0]) / (LAWN_R[0] + grow)) ** 2 + ((p[:, 1] - LAWN_C[1]) / (LAWN_R[1] + grow)) ** 2 < 1.0


def in_park(p, inset_=0.0):
    p = np.asarray(p, float).reshape(-1, 2)
    return (p[:, 0] > PARK[0] + inset_) & (p[:, 0] < PARK[2] - inset_) & (p[:, 1] > PARK[1] + inset_) & (p[:, 1] < PARK[3] - inset_)


def plantable(p, clear=2.2):
    """Where a tree may stand: in the park, off the paths, the plaza and the lawn."""
    p = np.asarray(p, float).reshape(-1, 2)
    r = np.hypot(p[:, 0], p[:, 1])
    return in_park(p, 4.0) & (r > PLAZA_R + 5.0) & ~in_lawn(p, LAWN_WALK_HW * 2 + 2.5) & (path_clearance(p) > clear)


def tree_sites(seed=7):
    """Where the model's trees stand: (x, y, crown diameter in city metres).
    A model-maker's planting: a ring round the plaza, a row inside each
    edge, a ring round the lawn, and a relaxed scatter between, thinner than
    the real park so the ground can be read."""
    r = rng(seed)
    out = []

    def near(p, d):
        return any(math.hypot(p[0] - q[0], p[1] - q[1]) < d + (q[2] + 0) * 0.25 for q in out)

    def put(x, y, size):
        out.append((x, y, size))

    for k in range(16):
        a = (k + 0.5) / 16 * math.tau + r.uniform(-0.06, 0.06)
        rad = PLAZA_R + 9.0 + r.uniform(-1.0, 1.5)
        p = (rad * math.cos(a), rad * math.sin(a))
        if plantable([p], 3.2)[0]:
            put(p[0], p[1], r.uniform(10.5, 13.0))
    for k in range(18):
        a = k / 18 * math.tau + r.uniform(-0.07, 0.07)
        p = (LAWN_C[0] + (LAWN_R[0] + 12.0) * math.cos(a), LAWN_C[1] + (LAWN_R[1] + 12.0) * math.sin(a))
        if plantable([p], 3.0)[0] and not near(p, 9.5):
            put(p[0], p[1], r.uniform(9.0, 12.0))
    for x in np.arange(PARK[0] + 8, PARK[2] - 6, 13.5):
        for y in (PARK[1] + 7.0, PARK[3] - 7.0):
            p = (x + r.uniform(-2, 2), y + r.uniform(-1, 1))
            if plantable([p], 2.6)[0] and not near(p, 9.0):
                put(p[0], p[1], r.uniform(7.5, 10.0))
    for y in np.arange(PARK[1] + 18, PARK[3] - 12, 13.5):
        for x in (PARK[0] + 7.0, PARK[2] - 7.0):
            p = (x + r.uniform(-1, 1), y + r.uniform(-2, 2))
            if plantable([p], 2.6)[0] and not near(p, 9.0):
                put(p[0], p[1], r.uniform(7.5, 10.0))
    tries = 0
    while tries < 9000:
        tries += 1
        p = (r.uniform(PARK[0] + 5, PARK[2] - 5), r.uniform(PARK[1] + 5, PARK[3] - 5))
        size = r.choice([5.5, 6.5, 7.5, 9.0, 10.5, 12.0], p=[0.16, 0.22, 0.24, 0.2, 0.12, 0.06]) * r.uniform(0.94, 1.06)
        if not plantable([p], 2.4 + size * 0.12)[0] or near(p, 6.2 + size * 0.42):
            continue
        put(p[0], p[1], size)
    return out


def bead_sites(seed=7):
    """Where the beads stand: (x, y, diameter in city metres). Never a grid:
    clusters at uneven stations either side of the curving walks, and groves
    between them."""
    r = rng(seed)
    out = []

    def free(p, d):
        if not plantable([p], d / 2 + 0.7)[0]:
            return False
        return all(math.hypot(p[0] - q[0], p[1] - q[1]) > (d + q[2]) * 0.5 + 0.35 for q in out)

    def size():
        return float(r.choice([2.2, 2.8, 3.4, 4.2, 5.2, 6.4], p=[0.18, 0.26, 0.24, 0.17, 0.1, 0.05])) * r.uniform(0.93, 1.07)

    def cluster(c, n, spread):
        for _ in range(n):
            for _try in range(8):
                p = (c[0] + r.normal(0, spread), c[1] + r.normal(0, spread))
                d = size()
                if free(p, d):
                    out.append((p[0], p[1], d))
                    break

    for pts, hw in all_paths():
        d = np.concatenate(([0], np.cumsum(np.linalg.norm(np.diff(pts, axis=0), axis=1))))
        s = r.uniform(2.0, 8.0)
        while s < d[-1] - 2.0:
            i = min(max(int(np.searchsorted(d, s)), 1), len(pts) - 2)
            t = pts[i + 1] - pts[i - 1]
            t = t / (np.linalg.norm(t) + 1e-9)
            nrm = np.array([-t[1], t[0]])
            side = 1 if r.random() < 0.5 else -1
            cluster(pts[i] + side * nrm * (hw + r.uniform(2.0, 6.5)), int(r.choice([1, 2, 2, 3, 4])), 2.6)
            if r.random() < 0.45:
                cluster(pts[i] - side * nrm * (hw + r.uniform(2.0, 7.0)), int(r.choice([1, 2, 3])), 2.4)
            s += r.uniform(5.0, 16.0)
    for _ in range(46):
        c = (r.uniform(PARK[0] + 8, PARK[2] - 8), r.uniform(PARK[1] + 8, PARK[3] - 8))
        cluster(c, int(r.integers(2, 7)), 4.2)
    return out


def lamp_sites():
    """Where the lamp posts stand: either side down every path, and round the plaza."""
    out = []
    for k in range(len(PATHS)):
        pts, hw = path_points(k, 48)
        d = np.concatenate(([0], np.cumsum(np.linalg.norm(np.diff(pts, axis=0), axis=1))))
        t = np.gradient(pts, axis=0)
        t /= np.linalg.norm(t, axis=1, keepdims=True)
        side = 1
        for s in np.arange(3.0, d[-1] - 2.0, 15.0):
            i = int(np.searchsorted(d, s))
            p = pts[i] + side * np.array([-t[i][1], t[i][0]]) * (hw + 0.9)
            if math.hypot(p[0], p[1]) > PLAZA_R + 1.0:
                out.append((p[0], p[1]))
            side = -side
    c = np.array(LAWN_C)
    for p in lawn_walk(10):
        q = c + (p - c) * 1.1
        out.append((q[0], q[1]))
    return out

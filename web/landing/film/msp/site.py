"""The plan of the place, in metres. The fountain is the origin, +y is
north and +x east. The camera's terrace is south of the park, looking north.

In the spirit of Madison Square Park, not a survey of it: a park of some
240 by 150 m with a fountain plaza at its south end, an oval lawn in its
middle, and streets and walls of buildings on every side.
"""

import math

import numpy as np

from .common import rng, smooth_path

PLAZA_R = 20.0
BASIN_R = 5.66  # the outer face of the coping
PARK = (-75.0, -42.0, 75.0, 198.0)  # x0, y0, x1, y1
SIDEWALK = 5.0
ROAD = 18.0
LOT = SIDEWALK + ROAD + 6.0  # from the park's edge to the building line across the street
LAWN_C = (2.0, 104.0)
LAWN_R = (31.0, 43.0)


def _mirror(pts):
    return [(-x, y) for x, y in pts]


# Paths: (control points, half width). Left and right are close to mirrored,
# and deliberately not exactly.
_NE = [(14.5, 14.0), (27, 28), (44, 40), (62, 47), (75, 50)]
_E = [(20, 1.0), (34, -3), (52, -7), (75, -9)]
_SE = [(14, -14.5), (30, -24), (48, -33), (62, -42)]
PATHS = [
    ([(0, -19), (0, -30), (0, -42)], 4.6),
    ([(0, 19), (0.5, 34), (1.5, 50), (2, 58)], 3.4),
    (_NE, 2.7),
    ([(-14, 14.5), (-26, 29), (-43, 41), (-60, 49), (-75, 53)], 2.7),
    (_E, 2.9),
    ([(-20, 0.5), (-36, -2), (-54, -5), (-75, -8)], 2.9),
    (_SE, 2.6),
    ([(-14.5, -14), (-31, -23), (-47, -34), (-60, -42)], 2.6),
    ([(34, 106), (50, 108), (75, 111)], 2.5),
    ([(-30, 104), (-50, 107), (-75, 108)], 2.5),
    ([(2, 149), (2.5, 170), (2, 198)], 3.4),
    ([(-75, 168), (-44, 163), (-18, 156), (2, 154), (24, 157), (48, 164), (75, 170)], 2.5),
]
# The walk round the oval lawn
LAWN_WALK_HW = 2.6


def path_points(i, n=64):
    pts, hw = PATHS[i]
    return smooth_path(np.array(pts, float), n), hw


def lawn_walk(n=160):
    a = np.linspace(0, math.tau, n, endpoint=False)
    return np.stack([LAWN_C[0] + (LAWN_R[0] + LAWN_WALK_HW) * np.cos(a), LAWN_C[1] + (LAWN_R[1] + LAWN_WALK_HW) * np.sin(a)], axis=-1)


def _seg_dist(p, a, b):
    """Distance from points p (n, 2) to the polyline a->b segments (m, 2)."""
    ab = b - a
    t = np.clip(((p[:, None, :] - a[None]) * ab[None]).sum(-1) / ((ab * ab).sum(-1)[None] + 1e-12), 0, 1)
    d = p[:, None, :] - (a[None] + t[..., None] * ab[None])
    return np.sqrt((d * d).sum(-1)).min(axis=1)


_PATH_CACHE = None


def path_clearance(p):
    """How far each point is outside the nearest path's edge (negative: on it)."""
    global _PATH_CACHE
    if _PATH_CACHE is None:
        _PATH_CACHE = [path_points(i) for i in range(len(PATHS))]
        w = lawn_walk()
        _PATH_CACHE.append((np.concatenate([w, w[:1]]), LAWN_WALK_HW))
    p = np.asarray(p, float).reshape(-1, 2)
    best = np.full(len(p), 1e9)
    for pts, hw in _PATH_CACHE:
        best = np.minimum(best, _seg_dist(p, pts[:-1], pts[1:]) - hw)
    return best


def in_lawn(p, grow=0.0):
    p = np.asarray(p, float).reshape(-1, 2)
    return ((p[:, 0] - LAWN_C[0]) / (LAWN_R[0] + grow)) ** 2 + ((p[:, 1] - LAWN_C[1]) / (LAWN_R[1] + grow)) ** 2 < 1.0


def in_park(p, inset=0.0):
    p = np.asarray(p, float).reshape(-1, 2)
    return (p[:, 0] > PARK[0] + inset) & (p[:, 0] < PARK[2] - inset) & (p[:, 1] > PARK[1] + inset) & (p[:, 1] < PARK[3] - inset)


def plantable(p, clear=2.2):
    """Where a tree may stand: in the park, off the paths, the plaza and the lawn."""
    p = np.asarray(p, float).reshape(-1, 2)
    r = np.hypot(p[:, 0], p[:, 1])
    # the terrace looks in over the park's south edge: nothing tall stands
    # between it and the plaza
    bearing = np.degrees(np.arctan2(p[:, 0], -p[:, 1]))  # 0 due south of the fountain, negative to the west
    corridor = (bearing > -84.0) & (bearing < 43.0) & (p[:, 1] < 4.0) & (r < 90.0)
    return in_park(p, 3.0) & (r > PLAZA_R + 4.5) & ~in_lawn(p, LAWN_WALK_HW * 2 + 2.0) & (path_clearance(p) > clear) & ~corridor


def tree_sites(seed=7):
    """Where the trees stand: (x, y, kind, size, turn). Kinds: 0 plane, 1 elm,
    2 locust, 3 oak, 4 young, 5 blossom."""
    r = rng(seed)
    out = []

    def near(p, d):
        return any(math.hypot(p[0] - q[0], p[1] - q[1]) < d for q in out)

    def put(x, y, kind, size=1.0):
        out.append((x, y, kind, size * r.uniform(0.9, 1.1), r.uniform(0, math.tau)))

    # the ring round the plaza: big planes between the paths
    for k in range(18):
        a = (k + 0.5) / 18 * math.tau + r.uniform(-0.05, 0.05)
        rad = PLAZA_R + 7.5 + r.uniform(-1.0, 1.5)
        p = (rad * math.cos(a), rad * math.sin(a))
        if plantable([p], 2.6)[0]:
            put(p[0], p[1], 0 if r.random() < 0.75 else 1, 1.05)
    # the perimeter: a row inside each edge
    for x in np.arange(PARK[0] + 6, PARK[2] - 5, 9.0):
        for y in (PARK[1] + 5.5, PARK[3] - 5.5):
            p = (x + r.uniform(-1.5, 1.5), y + r.uniform(-1, 1))
            if plantable([p], 2.0)[0] and not near(p, 7):
                put(p[0], p[1], int(r.choice([0, 0, 3, 1])), 0.95)
    for y in np.arange(PARK[1] + 14, PARK[3] - 10, 9.0):
        for x in (PARK[0] + 5.5, PARK[2] - 5.5):
            p = (x + r.uniform(-1, 1), y + r.uniform(-1.5, 1.5))
            if plantable([p], 2.0)[0] and not near(p, 7):
                put(p[0], p[1], int(r.choice([0, 0, 3, 1])), 0.95)
    # round the lawn: elms and planes that lean over its walk
    for k in range(22):
        a = k / 22 * math.tau + r.uniform(-0.06, 0.06)
        p = (LAWN_C[0] + (LAWN_R[0] + 10.5) * math.cos(a), LAWN_C[1] + (LAWN_R[1] + 10.5) * math.sin(a))
        if plantable([p], 2.2)[0] and not near(p, 7.5):
            put(p[0], p[1], int(r.choice([1, 1, 0, 2])), 1.0)
    # blossom at a few edges of lawn and plaza
    for a in (0.62, 2.45, 3.9, 5.55):
        p = (LAWN_C[0] + (LAWN_R[0] + 6.3) * math.cos(a), LAWN_C[1] + (LAWN_R[1] + 6.3) * math.sin(a))
        if plantable([p], 1.6)[0] and not near(p, 5):
            put(p[0], p[1], 5, 1.0)
    for p in ((27.5, -6.5), (-27, 8.5), (-9, 31), (23, 21.5)):
        if plantable([p], 1.2)[0] and not near(p, 4.5):
            put(p[0], p[1], 5, 0.9)
    # the south-west is kept low, so the evening sun reaches the plaza: blossom and young trees only
    for p, kind in (((-27.5, -17.0), 5), ((-34.0, -27.5), 5), ((-45.0, -17.5), 4), ((-25.0, -31.5), 5), ((-52.0, -31.0), 5), ((-63.0, -20.0), 4), ((-40.0, -37.0), 4), ((24.0, -30.0), 5), ((34.5, -36.5), 4), ((-17.0, -36.0), 5)):
        if path_clearance([p])[0] > 1.6 and not near(p, 5.0):
            put(p[0], p[1], kind, 0.95)
    # the rest: a relaxed scatter
    tries = 0
    while tries < 14000:
        tries += 1
        p = (r.uniform(PARK[0] + 4, PARK[2] - 4), r.uniform(PARK[1] + 4, PARK[3] - 4))
        if not plantable([p], 2.0)[0] or near(p, 7.4):
            continue
        u = r.random()
        kind = 0 if u < 0.42 else 1 if u < 0.62 else 2 if u < 0.76 else 3 if u < 0.88 else 4
        put(p[0], p[1], kind, 1.0)
    return out


# ------------------------------------------------------------------ the camera

HERO_CAM = (-9.0, -78.0, 52.0)


def project(cam, target, fov_tan, aspect, pitch, heading, p):
    """Screen position (0..1 across, 0..1 up) of world point p, for a quick
    composition check outside Blender."""
    d = np.asarray(p, float) - np.asarray(cam, float)
    fwd = np.array([math.sin(heading) * math.cos(pitch), math.cos(heading) * math.cos(pitch), math.sin(pitch)])
    right = np.cross(fwd, [0, 0, 1])
    right /= np.linalg.norm(right)
    up = np.cross(right, fwd)
    z = d @ fwd
    return 0.5 + 0.5 * (d @ right) / (z * fov_tan * aspect), 0.5 + 0.5 * (d @ up) / (z * fov_tan), z

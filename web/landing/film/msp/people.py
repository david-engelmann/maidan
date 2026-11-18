"""People: some two hundred small figures, walking, standing in pairs and
sitting, each built on its own skeleton in its own clothes. They are meant
to be read at a distance, as a crowd reads: posture and colour, not faces.
"""

import math
import os

import numpy as np

from . import site
from .common import MB, TAU, add_object, capsule, collection, ellipsoid, grade, material, rng
from .furniture import ORANGE, bench_sites

SKIN = [(0.62, 0.42, 0.3), (0.5, 0.3, 0.2), (0.33, 0.19, 0.12), (0.72, 0.52, 0.4), (0.2, 0.12, 0.08)]
TOPS = [(0.03, 0.05, 0.12), (0.04, 0.04, 0.045), (0.7, 0.69, 0.65), (0.42, 0.34, 0.22), (0.12, 0.14, 0.07), (0.22, 0.035, 0.045), (0.09, 0.16, 0.3), (0.5, 0.33, 0.05), (0.26, 0.26, 0.27), (0.03, 0.2, 0.2), (0.55, 0.25, 0.3), (0.5, 0.5, 0.52)]
BOTTOMS = [(0.05, 0.08, 0.16), (0.03, 0.03, 0.035), (0.33, 0.28, 0.19), (0.18, 0.18, 0.19), (0.09, 0.13, 0.22), (0.12, 0.1, 0.08)]
HAIR = [(0.02, 0.015, 0.012), (0.06, 0.035, 0.02), (0.2, 0.12, 0.05), (0.35, 0.33, 0.3), (0.45, 0.3, 0.12), (0.09, 0.03, 0.02)]


def mat_person(name="person"):
    mat, nt = material(name)
    g = grade(nt)
    c = nt.attr("col", out="Color")
    glow = nt.attr("glow")
    em = nt.shader("ShaderNodeEmission", Color=(0.75, 0.85, 1.0, 1.0), Strength=glow * (0.4 + g["night"] * 5.0))
    nt.out_surface(nt.add_shader(nt.principled(Base_Color=c, Roughness=0.75, Sheen_Weight=0.3), em))
    return mat


def _rot_x(v, a):
    c, s = math.cos(a), math.sin(a)
    return np.array([v[0], v[1] * c - v[2] * s, v[1] * s + v[2] * c])


def figure(r, pose, scale=1.0, orange=False):
    """One person, standing at the origin facing +y. Returns a mesh builder."""
    mb = MB()
    skin = np.array(SKIN[int(r.integers(len(SKIN)))])
    top = np.array(ORANGE if orange else TOPS[int(r.integers(len(TOPS)))]) * r.uniform(0.8, 1.15)
    bottom = np.array(BOTTOMS[int(r.integers(len(BOTTOMS)))]) * r.uniform(0.8, 1.2)
    hair = np.array(HAIR[int(r.integers(len(HAIR)))])
    shoe = np.array((0.03, 0.03, 0.03) if r.random() < 0.7 else (0.6, 0.6, 0.58))
    build_w = r.uniform(0.9, 1.14)  # slight to broad
    coat = r.random() < 0.3
    skirt = (not coat) and r.random() < 0.14
    long_hair = r.random() < 0.4
    s = scale
    kid = s < 0.8
    head_r = 0.105 * (1.18 if kid else 1.0)

    def part(p0, p1, r0, r1, col, squash=(1.0, 1.0), sides=7, glow=0.0):
        v, q, uv = capsule(np.array(p0) * s, np.array(p1) * s, r0 * s, r1 * s, sides=sides, rings=2, squash=squash)
        mb.add(v, q, smooth=True, vattr={"col": np.tile(col, (len(v), 1)), "glow": np.full(len(v), glow)})

    # ---- the skeleton, by pose
    hip_h = 0.93
    seat = None
    if pose.startswith("sit"):
        seat = 0.5 if pose == "sit_coping" else 0.45
        hip_h = seat / s + 0.09
    lean = {"walk": 0.06, "stand": r.normal(0, 0.03), "phone": 0.1}.get(pose, r.uniform(-0.12, 0.16) if seat else 0.0)
    pelvis = np.array([0.0, 0.0, hip_h])
    if pose == "stand" or pose == "phone":
        pelvis[0] += r.normal(0, 0.02)
    spine = _rot_x(np.array([0.0, 0.0, 0.5]), -lean)
    neck = pelvis + spine
    head = neck + _rot_x(np.array([0.0, 0.0, 0.17 + head_r * 0.2]), -lean - (0.35 if pose == "phone" else r.normal(0, 0.08)))
    sh_w = 0.19 * build_w
    hip_w = 0.1 * build_w
    phase = r.uniform(0, TAU)
    joints = {}
    for side, sx in (("l", -1.0), ("r", 1.0)):
        hip = pelvis + np.array([sx * hip_w, 0, 0])
        if pose == "walk":
            a = 0.46 * math.sin(phase) * sx
            kf = 0.2 + 0.6 * max(0.0, -math.sin(phase) * sx)
            knee = hip + _rot_x(np.array([0, 0, -0.44]), -a)
            ankle = knee + _rot_x(np.array([0, 0, -0.43]), -a + kf)
        elif seat:
            ext = r.uniform(-0.25, 0.45)
            knee = hip + np.array([sx * r.uniform(0.0, 0.07), 0.44, 0.02])
            ankle = knee + _rot_x(np.array([0, 0, -0.43]), ext)
            ankle[2] = max(ankle[2], 0.07 / s) if ankle[2] * s > 0.0 else ankle[2]
        else:
            a = r.normal(0, 0.04)
            knee = hip + _rot_x(np.array([sx * 0.02, 0, -0.44]), -a)
            ankle = knee + np.array([sx * 0.01, 0, -0.43])
        toe = ankle + np.array([0, 0.19, -0.03])
        joints[side] = (hip, knee, ankle, toe)
        sh = neck + np.array([sx * sh_w, 0, -0.05])
        if pose == "walk":
            a = -0.42 * math.sin(phase) * sx
            elbow = sh + _rot_x(np.array([sx * 0.03, 0, -0.29]), -a)
            wrist = elbow + _rot_x(np.array([0, 0, -0.26]), -a - 0.35)
        elif pose == "phone" and side == "r":
            elbow = sh + np.array([0.02, 0.07, -0.28])
            wrist = elbow + np.array([-0.07, 0.2, 0.14])
        elif seat:
            elbow = sh + np.array([sx * 0.05, 0.1, -0.27])
            wrist = elbow + np.array([-sx * 0.06, 0.23, -0.08])
        else:
            a = r.normal(0, 0.1)
            elbow = sh + _rot_x(np.array([sx * 0.04, 0, -0.29]), -a)
            wrist = elbow + _rot_x(np.array([0, 0.03, -0.26]), -a - r.uniform(0.0, 0.5))
        joints[side + "a"] = (sh, elbow, wrist)

    # ---- flesh and cloth
    for side in ("l", "r"):
        hip, knee, ankle, toe = joints[side]
        leg_col = skin if skirt else bottom
        part(hip, knee, 0.082 * build_w, 0.06, bottom if not skirt else skin)
        part(knee, ankle, 0.058, 0.042, leg_col)
        part(ankle + np.array([0, -0.03, 0]), toe, 0.045, 0.036, shoe, sides=6)
        sh, elbow, wrist = joints[side + "a"]
        part(sh, elbow, 0.052 * build_w, 0.042, top)
        part(elbow, wrist, 0.04, 0.034, top if r.random() < 0.75 else skin)
        hand = wrist + (wrist - elbow) * 0.22
        part(wrist, hand, 0.034, 0.03, skin, sides=6)
        if pose == "phone" and side == "r":
            # the screen in the hand
            pv, pq, _ = capsule((hand + np.array([-0.02, 0.0, 0.05])) * s, (hand + np.array([-0.02, 0.0, 0.11])) * s, 0.02 * s, 0.02 * s, sides=4, rings=1)
            mb.add(pv, pq, vattr={"col": np.tile((0.6, 0.7, 0.9), (len(pv), 1)), "glow": np.full(len(pv), 1.0)})
    chest = pelvis + spine * 0.62
    part(pelvis + np.array([0, 0, -0.02]), chest, 0.125 * build_w, 0.15 * build_w, top, squash=(0.68, 1.0), sides=9)
    part(chest, neck + np.array([0, 0, -0.07]), 0.15 * build_w, 0.13 * build_w, top, squash=(0.66, 1.0), sides=9)
    if coat:
        part(pelvis + np.array([0, 0, 0.05]), pelvis + np.array([0, 0.0 if not seat else 0.2, -0.3 if not seat else -0.02]), 0.14 * build_w, 0.16 * build_w, top, squash=(0.72, 1.0), sides=9)
    elif skirt:
        part(pelvis + np.array([0, 0, 0.02]), pelvis + np.array([0, 0.0 if not seat else 0.3, -0.36 if not seat else 0.0]), 0.13 * build_w, 0.2 * build_w, top * 0.8, squash=(0.8, 1.0), sides=9)
    else:
        part(pelvis + np.array([0, 0, -0.03]), pelvis + np.array([0, 0, 0.1]), 0.125 * build_w, 0.125 * build_w, bottom, squash=(0.7, 1.0), sides=9)
    part(neck + np.array([0, 0, -0.06]), neck + np.array([0, 0, 0.07]), 0.048, 0.045, skin, sides=6)
    v, q, uv = ellipsoid(head * s, (head_r * 0.86 * s, head_r * 0.96 * s, head_r * 1.08 * s), sides=10, rings=7)
    mb.add(v, q, smooth=True, vattr={"col": np.tile(skin, (len(v), 1)), "glow": np.zeros(len(v))})
    # hair: a cap over the crown and the back of the head
    hc = head + np.array([0, -0.022, 0.028])
    v, q, uv = ellipsoid(hc * s, (head_r * 0.9 * s, head_r * 0.98 * s, head_r * 1.02 * s), sides=10, rings=7)
    mb.add(v, q, smooth=True, vattr={"col": np.tile(hair, (len(v), 1)), "glow": np.zeros(len(v))})
    if long_hair:
        part(head + np.array([0, -0.07, 0.0]), neck + np.array([0, -0.085, -0.12]), 0.085, 0.07, hair, squash=(0.6, 1.0))
    if r.random() < 0.16 and not kid:
        # a bag on the shoulder
        b = joints["la"][0] + np.array([-0.07, 0.0, -0.42])
        part(b, b + np.array([0, 0.0, -0.2]), 0.09, 0.1, np.array(BOTTOMS[int(r.integers(len(BOTTOMS)))]), squash=(1.0, 0.45), sides=6)
    return mb


def _place(mb, x, y, turn, z=0.0):
    c, s = math.cos(turn), math.sin(turn)
    m = np.array([[c, -s, 0, x], [s, c, 0, y], [0, 0, 1, z], [0, 0, 0, 1]], np.float32)
    out = MB()
    out.extend(mb, matrix=m)
    return out


def cast(seed=5):
    """Who is where: (x, y, turn, z, pose, scale, orange). The turn is the way
    they face, measured from +y toward -x."""
    r = rng(seed)
    q = float(os.environ.get("FILM_QUALITY", "1"))
    out = []

    def face(dx, dy):
        return math.atan2(-dx, dy)

    def add(x, y, turn, pose, z=0.0, scale=None, orange=None):
        sc = scale if scale is not None else r.uniform(0.92, 1.1)
        out.append((x, y, turn, z, pose, sc, (r.random() < 0.055) if orange is None else orange))

    # sitting on the basin's coping, most looking out across the plaza
    n = int(26 * q)
    used = []
    while len(used) < n:
        a = r.uniform(0, TAU)
        if any(abs((a - b + math.pi) % TAU - math.pi) < 0.17 for b in used):
            continue
        used.append(a)
        if len(used) % 3 == 0:  # a neighbour, close by
            used.append(a + 0.12)
    for a in used:
        inward = r.random() < 0.18
        rad = 5.36 if not inward else 5.3
        d = (-math.cos(a), -math.sin(a)) if inward else (math.cos(a), math.sin(a))
        add(rad * math.cos(a) - d[0] * 0.13, rad * math.sin(a) - d[1] * 0.13, face(*d) + r.normal(0, 0.25), "sit_coping")
    # standing round the fountain, in twos and threes
    for _ in range(int(12 * q)):
        a = r.uniform(0, TAU)
        rad = r.uniform(6.9, 11.5)
        c = np.array([rad * math.cos(a), rad * math.sin(a)])
        k = int(r.choice([2, 2, 3, 1]))
        for j in range(k):
            off = np.array([math.cos(j * TAU / max(k, 2) + a), math.sin(j * TAU / max(k, 2) + a)]) * (0.42 if k > 1 else 0.0)
            p = c + off
            turn = face(-off[0], -off[1]) if k > 1 else face(-c[0], -c[1])
            add(p[0], p[1], turn + r.normal(0, 0.3), "phone" if r.random() < 0.22 else "stand", scale=0.62 if (k == 3 and j == 2 and r.random() < 0.5) else None)
    # on the benches
    benches = bench_sites(rng(17))
    for x, y, turn in benches:
        if r.random() > 0.42 * q:
            continue
        for off in ([-0.5, 0.45] if r.random() < 0.4 else [r.uniform(-0.6, 0.6)]):
            c, s = math.cos(turn), math.sin(turn)
            add(x + off * c - 0.12 * s, y + off * s + 0.12 * c, turn + r.normal(0, 0.15), "sit_bench")
    # crossing the plaza: chords that keep clear of the basin
    n = 0
    while n < int(30 * q):
        a = r.uniform(0, TAU)
        rad = math.sqrt(r.uniform(8.0 ** 2, (site.PLAZA_R - 1.5) ** 2))
        p = (rad * math.cos(a), rad * math.sin(a))
        if math.hypot(p[0] - 11.5, p[1] + 11.5) < 5.5:
            continue  # the cafe
        heading = a + math.pi / 2 * (1 if r.random() < 0.5 else -1) + r.normal(0, 0.5)
        add(p[0], p[1], face(math.cos(heading), math.sin(heading)), "walk")
        if r.random() < 0.35:
            add(p[0] + 0.55 * math.cos(heading + math.pi / 2), p[1] + 0.55 * math.sin(heading + math.pi / 2), face(math.cos(heading), math.sin(heading)), "walk", scale=0.62 if r.random() < 0.3 else None)
        n += 1
    # strolling the paths and the lawn's walk
    import numpy as _np

    for k in range(len(site.PATHS)):
        pts, hw = site.path_points(k, 60)
        d = _np.concatenate(([0], _np.cumsum(_np.linalg.norm(_np.diff(pts, axis=0), axis=1))))
        count = int(d[-1] / 9.0 * q) + 1
        for _ in range(count):
            s0 = r.uniform(2.0, d[-1] - 1.0)
            i = int(_np.searchsorted(d, s0))
            i = min(max(i, 1), len(pts) - 2)
            t = pts[i + 1] - pts[i - 1]
            t /= _np.linalg.norm(t)
            sgn = 1 if r.random() < 0.5 else -1
            p = pts[i] + _np.array([-t[1], t[0]]) * r.uniform(-hw * 0.6, hw * 0.6)
            if math.hypot(p[0], p[1]) < site.PLAZA_R:
                continue
            add(p[0], p[1], face(t[0] * sgn, t[1] * sgn), "walk", z=-0.01)
            if r.random() < 0.4:
                add(p[0] - t[1] * 0.55, p[1] + t[0] * 0.55, face(t[0] * sgn, t[1] * sgn), "walk", z=-0.01)
    walk = site.lawn_walk(int(26 * q))
    c = _np.array(site.LAWN_C)
    for i, p in enumerate(walk):
        nxt = walk[(i + 1) % len(walk)]
        t = nxt - p
        sgn = 1 if r.random() < 0.6 else -1
        add(p[0] + r.normal(0, 0.6), p[1] + r.normal(0, 0.6), face(t[0] * sgn, t[1] * sgn), "walk", z=-0.01)
    # on the lawn: sitting on the grass in little groups
    for _ in range(int(9 * q)):
        a = r.uniform(0, TAU)
        rad = r.uniform(0.1, 0.8)
        g = c + _np.array([site.LAWN_R[0] * rad * math.cos(a), site.LAWN_R[1] * rad * math.sin(a)])
        for j in range(int(r.integers(1, 4))):
            off = r.normal(0, 0.7, 2)
            add(g[0] + off[0], g[1] + off[1], r.uniform(0, TAU), "sit_grass", z=0.12)
    # the pavements round the park
    x0, y0, x1, y1 = site.PARK
    for _ in range(int(44 * q)):
        if r.random() < 0.5:
            x = r.uniform(x0, x1)
            y = y0 - r.uniform(0.8, 4.2) if r.random() < 0.5 else y1 + r.uniform(0.8, 4.2)
            d = 1 if r.random() < 0.5 else -1
            add(x, y, face(d, 0), "walk", z=-0.01)
        else:
            y = r.uniform(y0, y1)
            x = x0 - r.uniform(0.8, 4.2) if r.random() < 0.5 else x1 + r.uniform(0.8, 4.2)
            d = 1 if r.random() < 0.5 else -1
            add(x, y, face(0, d), "walk", z=-0.01)
    return out


def build(col=None):
    col = col or collection("People")
    r = rng(23)
    mat = mat_person()
    people = cast()
    # a handful of merged meshes rather than one object each: Cycles has no
    # use for two hundred separate people
    groups = [MB() for _ in range(8)]
    for k, (x, y, turn, z, pose, scale, orange) in enumerate(people):
        seat = pose.startswith("sit")
        fig = figure(r, "sit" + pose[3:] if seat else pose, scale=scale, orange=orange)
        if pose == "sit_grass":
            z -= 0.45 * scale - 0.06
        groups[k % len(groups)].extend(_place(fig, x, y, turn, z))
    for k, mb in enumerate(groups):
        add_object(f"People{k}", mb.build(f"People{k}", [mat]), col)
    print(f"[people] {len(people)} figures")
    return col

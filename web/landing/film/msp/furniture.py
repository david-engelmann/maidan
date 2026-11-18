"""What stands in the park: benches, globe lamp posts, the festoon lights
round the plaza, flower beds and their hedges, cafe tables under umbrellas,
and a flagpole."""

import math

import bpy
import numpy as np

from . import site
from .common import MB, TAU, add_object, boxes, capsule, collection, disc, grade, material, revolve, rng, tube

ORANGE = (1.0, 0.24, 0.035)  # the Maidan orange, in scene-linear


# ------------------------------------------------------------------ materials


def mat_paint(name, tone, rough=0.45, metallic=0.0):
    mat, nt = material(name)
    p = nt.coord("Object")
    wear = nt.noise(p, scale=9.0, detail=3.0)
    col = nt.mixc(nt.rgb(tone), nt.rgb((tone[0] * 0.55, tone[1] * 0.55, tone[2] * 0.55)), nt.smoothstep(0.45, 0.8, wear) * 0.5)
    nt.out_surface(nt.principled(Base_Color=col, Roughness=rough + wear * 0.15, Metallic=metallic))
    return mat


def mat_wood(name="bench_wood"):
    mat, nt = material(name)
    p = nt.coord("Object")
    x, y, z = nt.xyz(p)
    grain = nt.noise(nt.vec(x * 3.0, y * 40.0, z * 40.0), scale=1.0, detail=3.0)
    col = nt.ramp(grain, [(0.3, (0.07, 0.045, 0.025)), (0.6, (0.13, 0.085, 0.045)), (0.85, (0.2, 0.14, 0.08))])
    nt.out_surface(nt.principled(Base_Color=col, Roughness=0.6, Normal=nt.bump(grain, strength=0.2, distance=0.003)))
    return mat


def mat_globe(name="lamp_globe", power=1.0, tint=(1.0, 0.72, 0.42)):
    """A lit lamp: strong to the camera and in reflections; the light it
    throws comes from the point lamp inside it."""
    mat, nt = material(name)
    g = grade(nt)
    seen = nt.max(nt.lightpath("Is Camera Ray"), nt.lightpath("Is Glossy Ray"))
    em = nt.shader("ShaderNodeEmission", Color=(*tint, 1.0), Strength=g["lamps"] * power * (seen * 0.96 + 0.04))
    glass = nt.principled(Base_Color=(0.85, 0.82, 0.74, 1.0), Roughness=0.25, Transmission_Weight=0.6)
    nt.out_surface(nt.add_shader(glass, em))
    return mat


def mat_petals(name="petals"):
    mat, nt = material(name)
    c = nt.attr("col", out="Color")
    thru = nt.shader("ShaderNodeBsdfTranslucent", Color=c)
    nt.out_surface(nt.mix_shader(nt.principled(Base_Color=c, Roughness=0.5), thru, 0.35))
    return mat


def mat_hedge(name="hedge"):
    mat, nt = material(name)
    p = nt.coord("Object")
    leaf = nt.voronoi(p, scale=38.0, out="Distance")
    tone = nt.voronoi(p, scale=38.0, out="Color")
    patch = nt.noise(p, scale=1.6, detail=3.0)
    col = nt.ramp(nt.xyz(tone)[0] * 0.6 + patch * 0.4, [(0.2, (0.012, 0.035, 0.008)), (0.5, (0.03, 0.075, 0.014)), (0.8, (0.07, 0.14, 0.024))])
    n = nt.bump(leaf, strength=1.0, distance=0.03)
    body = nt.principled(Base_Color=col, Roughness=0.5, Normal=n, Sheen_Weight=0.4)
    nt.out_surface(nt.mix_shader(body, nt.shader("ShaderNodeBsdfTranslucent", Color=nt.mixc(col, nt.rgb((0.2, 0.3, 0.03)), 0.5), Normal=n), 0.2))
    return mat


def mat_cloth(name, tone):
    mat, nt = material(name)
    weave = nt.noise(nt.coord("Object"), scale=60.0, detail=2.0)
    col = nt.mixc(nt.rgb(tone), nt.rgb((tone[0] * 0.8, tone[1] * 0.8, tone[2] * 0.8)), weave)
    thru = nt.shader("ShaderNodeBsdfTranslucent", Color=col)
    nt.out_surface(nt.mix_shader(nt.principled(Base_Color=col, Roughness=0.8, Sheen_Weight=0.4), thru, 0.3))
    return mat


# -------------------------------------------------------------------- pieces


def bench_mesh(mats):
    """A park bench: wooden slats on three cast-iron frames."""
    mb = MB()
    L = 2.2
    lo = []
    hi = []
    for k in range(5):  # seat
        y = -0.02 + k * 0.095
        lo.append((-L / 2, y, 0.425))
        hi.append((L / 2, y + 0.075, 0.455))
    for k in range(4):  # back, leaning
        z = 0.56 + k * 0.105
        y = -0.07 - k * 0.028
        lo.append((-L / 2, y - 0.03, z))
        hi.append((L / 2, y, z + 0.08))
    v, q = boxes(np.array(lo), np.array(hi))
    mb.add(v, q, mat=0)
    lo = []
    hi = []
    for x in (-L / 2 + 0.12, 0.0, L / 2 - 0.12):
        lo += [(x - 0.025, -0.03, 0.0), (x - 0.025, 0.4, 0.0), (x - 0.025, -0.03, 0.39), (x - 0.025, -0.17, 0.42), (x - 0.035, -0.05, 0.6)]
        hi += [(x + 0.025, 0.02, 0.43), (x + 0.025, 0.45, 0.43), (x + 0.025, 0.45, 0.43), (x + 0.025, -0.03, 0.98), (x + 0.035, 0.44, 0.63)]
    v, q = boxes(np.array(lo), np.array(hi))
    mb.add(v, q, mat=1)
    return mb.build("Bench", mats)


def lamp_mesh(mats):
    """A park lamp post: a fluted iron shaft on a bell base, a lantern-shaped
    globe under a cap and finial."""
    mb = MB()
    prof = [(0.2, 0.0), (0.2, 0.08), (0.16, 0.12), (0.17, 0.3), (0.13, 0.42), (0.1, 0.5), (0.075, 0.62), (0.085, 0.66), (0.07, 0.72), (0.058, 1.4), (0.048, 2.75), (0.07, 2.8), (0.075, 2.86), (0.05, 2.9), (0.09, 3.0), (0.13, 3.04), (0.13, 3.08)]
    v, q, uv = revolve(prof, 12)
    mb.add(v, q, mat=0, smooth=True)
    glass = [(0.12, 3.08), (0.2, 3.2), (0.235, 3.36), (0.22, 3.52), (0.16, 3.64), (0.1, 3.7)]
    v, q, uv = revolve(glass, 14)
    mb.add(v, q, mat=1, smooth=True)
    cap = [(0.1, 3.7), (0.19, 3.72), (0.15, 3.78), (0.06, 3.86), (0.03, 3.9), (0.045, 3.94), (0.02, 4.02), (0.002, 4.08)]
    v, q, uv = revolve(cap, 12)
    mb.add(v, q, mat=0, smooth=True)
    return mb.build("Lamp", mats)


def _lamp_sites(r):
    out = []
    # either side of each path where it meets the plaza, and on down it
    for k in range(len(site.PATHS)):
        pts, hw = site.path_points(k, 48)
        d = np.concatenate(([0], np.cumsum(np.linalg.norm(np.diff(pts, axis=0), axis=1))))
        t = np.gradient(pts, axis=0)
        t /= np.linalg.norm(t, axis=1, keepdims=True)
        side = 1
        for s in np.arange(1.5, d[-1] - 2.0, 15.0):
            i = int(np.searchsorted(d, s))
            p = pts[i] + side * np.array([-t[i][1], t[i][0]]) * (hw + 0.7)
            if math.hypot(p[0], p[1]) > site.PLAZA_R + 0.4:
                out.append((p[0], p[1]))
            side = -side
    walk = site.lawn_walk(12)
    c = np.array(site.LAWN_C)
    for p in walk:
        q = c + (p - c) * 1.09
        out.append((q[0], q[1]))
    # the plaza's own ring, between the paths
    for k in range(16):
        a = (k + 0.5) / 16 * TAU
        p = np.array([(site.PLAZA_R - 1.0) * math.cos(a), (site.PLAZA_R - 1.0) * math.sin(a)])
        if site.path_clearance([p * 1.08])[0] > 1.0:
            out.append((p[0], p[1]))
    # the pavements round the park
    x0, y0, x1, y1 = site.PARK
    for x in np.arange(x0 + 8, x1 - 4, 27.0):
        out += [(x, y0 - 3.4), (x, y1 + 3.4)]
    for y in np.arange(y0 + 12, y1 - 4, 27.0):
        out += [(x0 - 3.4, y), (x1 + 3.4, y)]
    del r
    return out


def _festoon(col, mats, r):
    """Strings of bulbs from post to post round the plaza, each sagging in
    its own catenary."""
    mb = MB()
    n = 14
    rad = site.PLAZA_R - 1.9
    h = 5.4
    posts = [(rad * math.cos((k + 0.5) / n * TAU), rad * math.sin((k + 0.5) / n * TAU)) for k in range(n)]
    for x, y in posts:
        v, q, uv = revolve([(0.075, 0.0), (0.075, 0.15), (0.05, 0.2), (0.04, h), (0.06, h + 0.04), (0.02, h + 0.12)], 8)
        mb.add(v + np.array([x, y, 0], np.float32), q, mat=0, smooth=True)
    bulbs = []
    for k in range(n):
        a = np.array(posts[k])
        b = np.array(posts[(k + 1) % n])
        span = np.linalg.norm(b - a)
        sag = 0.085 * span * r.uniform(0.85, 1.15)
        m = int(span / 0.42)
        s = np.linspace(0, 1, m + 1)
        # a shallow catenary is close enough to a parabola to draw as one
        z = h - sag * 4 * s * (1 - s)
        pts = np.stack([a[0] + (b[0] - a[0]) * s, a[1] + (b[1] - a[1]) * s, z], axis=-1)
        v, q, uv = tube(pts, 0.009, 4)
        mb.add(v, q, mat=0, smooth=True)
        for p in pts[1:-1]:
            bulbs.append(p - np.array([0, 0, 0.07]))
    bulbs = np.array(bulbs)
    for p in bulbs:
        v, q, uv = revolve([(0.001, 0.07), (0.014, 0.05), (0.03, 0.015), (0.036, -0.02), (0.026, -0.05), (0.001, -0.062)], 8)
        mb.add(v + p.astype(np.float32), q, mat=1, smooth=True)
    add_object("Festoon", mb.build("Festoon", mats), col).visible_shadow = False
    # the light they throw: one soft lamp for every few bulbs
    for k, p in enumerate(bulbs[::5]):
        ld = bpy.data.lights.new(f"FestoonLight{k}", "POINT")
        ld.energy = 240.0
        ld.color = (1.0, 0.66, 0.36)
        ld.shadow_soft_size = 0.5
        ob = add_object(f"FestoonLight{k}", ld, col, loc=tuple(p))
        ob["film_lamp"] = 240.0
        ob.visible_camera = False
    return len(bulbs)


def _beds(col, r):
    """Crescent flower beds just outside the plaza's kerb, between the paths:
    a clipped hedge round each, and tulips inside, most of them orange."""
    soil, nt = material("bed_soil")
    p = nt.coord("Object")
    nt.out_surface(nt.principled(Base_Color=nt.mixc(nt.rgb((0.03, 0.022, 0.016)), nt.rgb((0.06, 0.045, 0.03)), nt.noise(p, scale=14.0, detail=3.0)), Roughness=0.9))
    hedge = mat_hedge()
    petals = mat_petals()
    mb = MB()
    fl = MB()
    r0, r1 = site.PLAZA_R + 0.5, site.PLAZA_R + 3.9
    a = np.linspace(0, TAU, 720, endpoint=False)
    mid = np.stack([(r0 + r1) / 2 * np.cos(a), (r0 + r1) / 2 * np.sin(a)], axis=-1)
    free = site.path_clearance(mid) > 1.5
    # runs of free angle are the beds
    runs = []
    start = None
    for i in range(len(a) * 2):
        k = i % len(a)
        if free[k] and start is None:
            start = i
        if not free[k] and start is not None:
            if i - start > 30 and start < len(a):
                runs.append((start, i))
            start = None
    for i0, i1 in runs:
        a0, a1 = a[i0 % len(a)], a[i0 % len(a)] + (i1 - i0) / len(a) * TAU
        segs = max(8, int((a1 - a0) * 22))
        v, q, uv = disc(r0, r1, segs, rings=2, z=0.07, a0=a0, a1=a1)
        mb.add(v, q, mat=0)
        # the hedge: a low rounded bar along the inner and outer arcs and across the ends
        for rr in (r0 + 0.25, r1 - 0.25):
            aa = np.linspace(a0, a1, segs * 2)
            pts = np.stack([rr * np.cos(aa), rr * np.sin(aa), np.full_like(aa, 0.3) + 0.03 * np.sin(aa * 50)], axis=-1)
            v, q, uv = tube(pts, 0.27, 8, squash=np.tile([1.15, 0.95], (len(pts), 1)))
            mb.add(v, q, mat=1, smooth=True)
        for ae in (a0, a1):
            rr = np.linspace(r0 + 0.25, r1 - 0.25, 6)
            pts = np.stack([rr * math.cos(ae), rr * math.sin(ae), np.full_like(rr, 0.3)], axis=-1)
            v, q, uv = tube(pts, 0.27, 8, squash=np.tile([0.95, 1.15], (len(pts), 1)))
            mb.add(v, q, mat=1, smooth=True)
        # tulips
        area = (a1 - a0) * (r1 * r1 - r0 * r0) / 2
        n = int(area * 26)
        ta = r.uniform(a0 + 0.02, a1 - 0.02, n)
        tr = np.sqrt(r.uniform((r0 + 0.6) ** 2, (r1 - 0.6) ** 2, n))
        base = np.stack([tr * np.cos(ta), tr * np.sin(ta), np.full(n, 0.07)], axis=-1)
        hgt = r.uniform(0.32, 0.5, n)
        lean = r.normal(0, 0.05, (n, 2))
        top = base + np.concatenate([lean * hgt[:, None], hgt[:, None]], axis=1)
        # the drift of colour through a bed: mostly orange, with islands of cream and yellow
        drift = np.sin(ta * 9.0 + tr * 1.3) + r.normal(0, 0.5, n)
        colr = np.where(drift[:, None] > 0.9, np.array([[0.95, 0.9, 0.7]]), np.where(drift[:, None] < -1.1, np.array([[1.0, 0.62, 0.05]]), np.array([ORANGE])))
        colr = colr * r.uniform(0.75, 1.1, (n, 1))
        # stem: a thin three-sided prism
        ang = np.arange(3) * TAU / 3
        ring = np.stack([np.cos(ang), np.sin(ang), np.zeros(3)], axis=-1) * 0.006
        sv = np.concatenate([base[:, None, :] + ring[None], top[:, None, :] + ring[None]], axis=1).reshape(-1, 3)
        sq = np.array([[0, 1, 4, 3], [1, 2, 5, 4], [2, 0, 3, 5]])
        fl.add(sv, (sq[None] + (np.arange(n) * 6)[:, None, None]).reshape(-1, 4), mat=0, vattr={"col": np.tile([0.05, 0.14, 0.03], (n * 6, 1))})
        # two leaves
        for side in (-1, 1):
            d = np.stack([np.cos(ta + side * 1.2), np.sin(ta + side * 1.2), np.zeros(n)], axis=-1)
            lv = np.stack([base, base + d * 0.035 + np.array([0, 0, 0.12]), base + d * 0.09 + np.array([0, 0, 0.26]), base - d * 0.0 + np.array([0, 0, 0.14])], axis=1)
            lv[:, 3] += np.cross(d, [0, 0, 1]) * 0.03
            fl.add(lv.reshape(-1, 3), np.arange(4)[None, :] + (np.arange(n) * 4)[:, None], mat=0, smooth=True, vattr={"col": np.tile([0.06, 0.16, 0.035], (n * 4, 1))})
        # the cup: six petals closing to a point
        ang = np.arange(6) * TAU / 6
        low = np.stack([np.cos(ang) * 0.012, np.sin(ang) * 0.012, np.zeros(6)], axis=-1)
        midr = np.stack([np.cos(ang) * 0.03, np.sin(ang) * 0.03, np.full(6, 0.035)], axis=-1)
        tip = np.stack([np.cos(ang) * 0.022, np.sin(ang) * 0.022, np.full(6, 0.075)], axis=-1)
        cv = np.concatenate([top[:, None, :] + low[None], top[:, None, :] + midr[None], top[:, None, :] + tip[None]], axis=1).reshape(-1, 3)
        cq = []
        for ring_k in range(2):
            for j in range(6):
                cq.append([ring_k * 6 + j, ring_k * 6 + (j + 1) % 6, (ring_k + 1) * 6 + (j + 1) % 6, (ring_k + 1) * 6 + j])
        cq = np.array(cq)
        fl.add(cv, (cq[None] + (np.arange(n) * 18)[:, None, None]).reshape(-1, 4), mat=0, smooth=True, vattr={"col": np.repeat(colr, 18, axis=0)})
    add_object("Beds", mb.build("Beds", [soil, hedge]), col)
    add_object("Tulips", fl.build("Tulips", [petals]), col)


def _cafe(col, r, iron, cloth_orange, cloth_cream):
    """A few cafe tables at the plaza's edge, some under umbrellas."""
    mb = MB()
    centre = np.array([11.5, -11.5])
    spots = []
    tries = 0
    while len(spots) < 7 and tries < 400:
        tries += 1
        p = centre + r.uniform(-4.2, 4.2, 2)
        rr = math.hypot(p[0], p[1])
        if rr < 10.5 or rr > site.PLAZA_R - 3.2:
            continue
        if any(np.linalg.norm(p - q) < 2.5 for q in spots):
            continue
        spots.append(p)
    for k, p in enumerate(spots):
        x, y = p
        v, q, uv = revolve([(0.2, 0.0), (0.2, 0.02), (0.025, 0.04), (0.02, 0.7), (0.36, 0.72), (0.36, 0.74), (0.001, 0.74)], 16)
        mb.add(v + np.array([x, y, 0], np.float32), q, mat=0, smooth=True)
        for j in range(int(r.integers(2, 4))):
            a = r.uniform(0, TAU)
            cx, cy = x + 0.62 * math.cos(a), y + 0.62 * math.sin(a)
            lo = [(cx - 0.19, cy - 0.19, 0.42), (cx - 0.19, cy - 0.19, 0.0), (cx + 0.16, cy - 0.19, 0.0), (cx - 0.19, cy + 0.16, 0.0), (cx + 0.16, cy + 0.16, 0.0)]
            hi = [(cx + 0.19, cy + 0.19, 0.45), (cx - 0.16, cy - 0.16, 0.42), (cx + 0.19, cy - 0.16, 0.42), (cx - 0.16, cy + 0.19, 0.42), (cx + 0.19, cy + 0.19, 0.42)]
            # a back, on the side away from the table
            bx, by = cx + 0.19 * math.cos(a), cy + 0.19 * math.sin(a)
            lo.append((bx - 0.17, by - 0.17, 0.45))
            hi.append((bx + 0.17, by + 0.17, 0.82))
            v, q = boxes(np.array(lo), np.array(hi))
            # a chair back is a thin slab: squash the box toward the table's far side
            mb.add(v, q, mat=0)
        if k < 5:
            v, q, uv = revolve([(0.02, 0.74), (0.02, 2.5)], 6)
            mb.add(v + np.array([x, y, 0], np.float32), q, mat=0)
            prof = [(1.45, 2.22), (1.0, 2.38), (0.5, 2.52), (0.04, 2.6), (0.001, 2.66)]
            v, q, uv = revolve(prof, 8)
            mb.add(v + np.array([x, y, 0], np.float32), q, mat=1 if k % 2 == 0 else 2)
    add_object("Cafe", mb.build("Cafe", [iron, cloth_orange, cloth_cream]), col)
    return spots


def _flag(col, iron, cloth):
    mb = MB()
    x, y, h = 17.0, 27.5, 21.0
    v, q, uv = revolve([(0.9, 0.0), (0.9, 0.5), (0.6, 0.6), (0.6, 1.1), (0.3, 1.25), (0.13, 1.4), (0.075, h), (0.11, h + 0.05), (0.11, h + 0.25), (0.02, h + 0.4)], 16)
    mb.add(v + np.array([x, y, 0], np.float32), q, mat=0, smooth=True)
    # the flag: a sheet with a wave travelling down it, sagging from its hoist
    nu, nv = 40, 16
    u = np.linspace(0, 1, nu)[:, None]
    w = np.linspace(0, 1, nv)[None, :]
    L, H = 4.2, 2.4
    wave = 0.22 * u * np.sin(u * 9.0 + w * 1.5) + 0.07 * u * np.sin(u * 21.0 - w * 3.0)
    fx = x + 0.08 + u * L * 0.93 + 0 * w
    fy = y + wave + 0.25 * u
    fz = h - 0.3 - w * H - 0.55 * u ** 1.6 + 0 * u
    fv = np.stack([fx, fy, fz], axis=-1).reshape(-1, 3)
    i = np.arange(nu - 1)[:, None]
    j = np.arange(nv - 1)[None, :]
    fq = np.stack([i * nv + j, (i + 1) * nv + j, (i + 1) * nv + j + 1, i * nv + j + 1], axis=-1).reshape(-1, 4)
    mb.add(fv, fq, mat=1, smooth=True)
    add_object("Flagpole", mb.build("Flagpole", [iron, cloth]), col)


def bench_sites(r):
    """Where benches stand, and which way each faces: (x, y, turn)."""
    out = []
    # round the plaza, backs to the beds, facing the fountain
    rad = site.PLAZA_R - 0.75
    for k in range(44):
        a = k / 44 * TAU
        p = (rad * math.cos(a), rad * math.sin(a))
        if site.path_clearance([(p[0] * 1.1, p[1] * 1.1)])[0] > 1.6:
            out.append((p[0], p[1], a + math.pi / 2))
    # down the paths, both sides, facing across
    for k in range(len(site.PATHS)):
        pts, hw = site.path_points(k, 60)
        d = np.concatenate(([0], np.cumsum(np.linalg.norm(np.diff(pts, axis=0), axis=1))))
        t = np.gradient(pts, axis=0)
        t /= np.linalg.norm(t, axis=1, keepdims=True)
        for s in np.arange(5.0, d[-1] - 3.0, 6.5):
            i = int(np.searchsorted(d, s))
            for side in (1, -1):
                if r.random() < 0.3:
                    continue
                nrm = side * np.array([-t[i][1], t[i][0]])
                p = pts[i] + nrm * (hw - 0.3)
                if math.hypot(p[0], p[1]) < site.PLAZA_R + 1.0:
                    continue
                out.append((p[0], p[1], math.atan2(-nrm[1], -nrm[0]) - math.pi / 2))
    return out


def build(col=None):
    col = col or collection("Furniture")
    r = rng(17)
    iron = mat_paint("park_iron", (0.02, 0.028, 0.022), 0.4, metallic=0.6)
    wood = mat_wood()
    globe = mat_globe("lamp_globe", power=26.0)
    bulb = mat_globe("festoon_bulb", power=90.0, tint=(1.0, 0.62, 0.3))
    bench = bench_mesh([wood, iron])
    for k, (x, y, turn) in enumerate(bench_sites(r)):
        add_object(f"Bench{k:03d}", bench, col, loc=(x, y, 0.0 if math.hypot(x, y) < site.PLAZA_R else -0.01), rot=(0, 0, turn))
    lamp = lamp_mesh([iron, globe])
    for k, (x, y) in enumerate(_lamp_sites(r)):
        ob = add_object(f"Lamp{k:03d}", lamp, col, loc=(x, y, -0.01), rot=(0, 0, r.uniform(0, TAU)))
        ob.visible_shadow = False  # or the globe would shadow its own lamp
        ld = bpy.data.lights.new(f"LampLight{k:03d}", "POINT")
        ld.energy = 900.0
        ld.color = (1.0, 0.66, 0.36)
        ld.shadow_soft_size = 0.2
        lo = add_object(f"LampLight{k:03d}", ld, col, loc=(x, y, 3.4))
        lo["film_lamp"] = 900.0
        del ob
    # uplights at the feet of the trees that edge the plaza, the lawn and the
    # walks: the crowns glow from inside, and the leaves pass the light on up
    ups = 0
    for x, y, kind, size, turn in site.tree_sites():
        p = np.array([x, y])
        rr = math.hypot(x, y)
        near_plaza = rr < site.PLAZA_R + 16
        c = np.array(site.LAWN_C)
        e = math.hypot((x - c[0]) / (site.LAWN_R[0] + 16), (y - c[1]) / (site.LAWN_R[1] + 16))
        near_lawn = e < 1.0
        near_walk = site.path_clearance([p])[0] < 5.5
        if not (near_plaza or near_lawn or near_walk) or kind == 4:
            continue
        for j in range(2 if (near_plaza or near_lawn) else 1):
            a = r.uniform(0, TAU)
            off = 1.3 * size
            ld = bpy.data.lights.new(f"TreeUp{ups:03d}", "SPOT")
            power = (2600.0 if near_plaza else 1700.0) * (0.45 if kind == 5 else 1.0)
            ld.energy = power
            ld.color = (1.0, 0.72, 0.42)
            ld.spot_size = 1.75
            ld.spot_blend = 0.8
            ld.shadow_soft_size = 0.25
            lo = add_object(f"TreeUp{ups:03d}", ld, col, loc=(x + off * math.cos(a), y + off * math.sin(a), 0.12))
            # up, leaning in toward the trunk
            lo.rotation_euler = (math.pi + 0.12 * math.sin(a), -0.12 * math.cos(a), 0.0)
            lo["film_lamp"] = power
            ups += 1
    print(f"[furniture] {ups} tree uplights")
    n = _festoon(col, [iron, bulb], r)
    _beds(col, r)
    _cafe(col, r, iron, mat_cloth("umbrella_orange", ORANGE), mat_cloth("umbrella_cream", (0.72, 0.68, 0.58)))
    _flag(col, iron, mat_cloth("flag", (0.8, 0.77, 0.7)))
    print(f"[furniture] {len(col.objects)} objects, {n} bulbs")
    return col

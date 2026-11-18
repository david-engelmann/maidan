"""Trees, grown rather than modelled.

Each variant starts as a trunk and a few scaffold limbs, and the rest is
space colonisation: points scattered through the crown's envelope pull the
nearest twig toward them and are used up as growth reaches them. That gives
limbs that fork where a real limb would, and a crown with holes in it. Limb
radii follow the pipe model (a limb is as thick as the limbs it carries).
Leaves are folded cards on the outermost twigs, each with its own tone.

A dozen variants are grown and then instanced across the park.
"""

import math
import os

import numpy as np

from . import site
from .common import MB, TAU, add_object, collection, frames_along, material, rng, smooth_path

# height, crown base, crown width, trunk radius, attractors, step, kill, influence
SPECIES = {
    "plane": dict(h=23.0, base=7.5, w=16.0, trunk=0.46, na=3400, step=0.5, kill=1.0, infl=6.0, scaffold=(4, 0.62), leaf=0.21, leaves=46000, shape="ovoid"),
    "elm": dict(h=22.0, base=7.0, w=17.0, trunk=0.42, na=3200, step=0.5, kill=1.0, infl=6.0, scaffold=(5, 0.42), leaf=0.12, leaves=52000, shape="vase"),
    "locust": dict(h=17.0, base=5.5, w=13.0, trunk=0.3, na=1700, step=0.5, kill=1.15, infl=6.0, scaffold=(3, 0.7), leaf=0.24, leaves=9000, shape="open"),
    "oak": dict(h=20.0, base=5.0, w=11.5, trunk=0.36, na=2800, step=0.45, kill=0.95, infl=5.0, scaffold=(1, 0.0), leaf=0.13, leaves=44000, shape="spire"),
    "young": dict(h=10.5, base=3.2, w=5.6, trunk=0.13, na=900, step=0.35, kill=0.7, infl=3.5, scaffold=(1, 0.0), leaf=0.15, leaves=11000, shape="ovoid"),
    "blossom": dict(h=6.8, base=1.9, w=7.2, trunk=0.14, na=1300, step=0.3, kill=0.55, infl=3.0, scaffold=(4, 0.85), leaf=0.05, leaves=30000, shape="dome"),
}
# variant name: (species, seed, size)
VARIANTS = {
    "plane_a": ("plane", 11, 1.0), "plane_b": ("plane", 12, 1.08), "plane_c": ("plane", 13, 0.9),
    "elm_a": ("elm", 21, 1.0), "elm_b": ("elm", 22, 0.92),
    "locust_a": ("locust", 31, 1.0), "oak_a": ("oak", 41, 1.0),
    "young_a": ("young", 51, 1.0), "young_b": ("young", 52, 0.85),
    "blossom_pink": ("blossom", 61, 1.0), "blossom_white": ("blossom", 62, 0.9),
}
KIND_VARIANTS = {0: ["plane_a", "plane_b", "plane_c"], 1: ["elm_a", "elm_b"], 2: ["locust_a"], 3: ["oak_a"], 4: ["young_a", "young_b"], 5: ["blossom_pink", "blossom_white"]}


# ------------------------------------------------------------------- growing


def _envelope(r, sp, size):
    """Attraction points filling the crown."""
    h, base, w = sp["h"] * size, sp["base"] * size, sp["w"] * size
    n = sp["na"]
    ch = h - base
    out = []
    lobes = r.uniform(0, TAU, 4)
    while sum(len(o) for o in out) < n:
        p = r.uniform(-1, 1, (n * 3, 3))
        rad = np.hypot(p[:, 0], p[:, 1])
        ang = np.arctan2(p[:, 1], p[:, 0])
        s = (p[:, 2] + 1) / 2  # 0 at the crown's base, 1 at its top
        # the crown is not a solid of revolution: it bulges and hollows
        bulge = 1.0 + 0.16 * np.sin(ang * 2 + lobes[0]) + 0.1 * np.sin(ang * 3 + lobes[1] + s * 3.0) + 0.07 * np.sin(s * 9 + lobes[2])
        shape = sp["shape"]
        if shape == "ovoid":
            lim = np.sqrt(np.clip(1 - (2 * s - 0.9) ** 2 / 1.25, 0, 1)) * (0.35 + 0.65 * np.clip(s / 0.3, 0, 1))
        elif shape == "vase":
            lim = np.where(s < 0.72, 0.18 + 0.82 * (s / 0.72) ** 0.8, np.sqrt(np.clip(1 - ((s - 0.72) / 0.3) ** 2, 0, 1)))
        elif shape == "open":
            lim = np.sqrt(np.clip(1 - (2 * s - 1.05) ** 2, 0, 1)) * (0.5 + 0.5 * np.clip(s / 0.25, 0, 1))
        elif shape == "spire":
            lim = np.clip(1.05 - s, 0, 1) ** 0.75 * (0.45 + 0.55 * np.clip(s / 0.12, 0, 1))
        else:  # a low dome
            lim = np.sqrt(np.clip(1 - s ** 2.2, 0, 1)) * (0.4 + 0.6 * np.clip(s / 0.2, 0, 1))
        ok = rad < lim * bulge
        # more of them toward the outside, where the leaves are
        ok &= r.random(len(p)) < 0.3 + 0.7 * (rad / (lim * bulge + 1e-6)) ** 1.5
        q = np.stack([p[:, 0] * w / 2, p[:, 1] * w / 2, base + s * ch], axis=-1)[ok]
        out.append(q)
    att = np.concatenate(out)[:n]
    if sp["shape"] == "vase":
        # an elm's outer branches arch over and hang
        k = n // 6
        a = r.uniform(0, TAU, k)
        rr = w / 2 * r.uniform(0.86, 1.06, k)
        att = np.concatenate([att, np.stack([rr * np.cos(a), rr * np.sin(a), base + ch * r.uniform(0.3, 0.72, k)], axis=-1)])
    return att


def _grow(r, sp, size):
    """The skeleton: node positions and each node's parent."""
    h, base = sp["h"] * size, sp["base"] * size
    step = sp["step"] * size ** 0.5
    pos = []
    par = []

    def add(p, parent):
        pos.append(np.asarray(p, float))
        par.append(parent)
        return len(pos) - 1

    # the trunk: never quite straight
    lean = r.normal(0, 0.035, 2)
    sway = r.uniform(0, TAU, 2)
    last = add((0, 0, 0), -1)
    nz = max(3, int(base / step))
    for k in range(1, nz + 1):
        z = base * k / nz
        x = lean[0] * z + 0.16 * size * math.sin(z * 0.5 + sway[0])
        y = lean[1] * z + 0.16 * size * math.sin(z * 0.43 + sway[1])
        last = add((x, y, z), last)
    top = last
    count, spread = sp["scaffold"]
    if count > 1:
        a0 = r.uniform(0, TAU)
        for k in range(count):
            a = a0 + k * TAU / count + r.normal(0, 0.22)
            tilt = spread * r.uniform(0.75, 1.2)
            d = np.array([math.cos(a) * math.sin(tilt), math.sin(a) * math.sin(tilt), math.cos(tilt)])
            node = top
            # the limb leaves the trunk a little below its top, and turns up as it goes
            for j in range(int(2.6 * size / step) + 1):
                d = d + np.array([0, 0, 0.07]) + r.normal(0, 0.04, 3)
                d /= np.linalg.norm(d)
                node = add(pos[node] + d * step, node)
    else:
        node = top
        for j in range(int(1.5 * size / step) + 1):
            node = add(pos[node] + np.array([r.normal(0, 0.03), r.normal(0, 0.03), 1.0]) * step, node)

    att = _envelope(r, sp, size)
    pos = np.array(pos)
    par = np.array(par)
    kill = sp["kill"] * size ** 0.5
    infl = sp["infl"] * size ** 0.5
    alive = np.ones(len(att), bool)
    near_i = np.full(len(att), -1)
    near_d = np.full(len(att), np.inf)
    kids = np.zeros(len(pos), int)
    new0 = 0
    up = np.array([0.0, 0.0, 0.06 if sp["shape"] != "vase" else 0.1])
    for it in range(400):
        fresh = pos[new0:]
        if len(fresh):
            idx = np.nonzero(alive)[0]
            for c0 in range(0, len(idx), 4000):
                ii = idx[c0:c0 + 4000]
                d = np.linalg.norm(att[ii][:, None, :] - fresh[None, :, :], axis=-1)
                j = d.argmin(axis=1)
                dm = d[np.arange(len(ii)), j]
                better = dm < near_d[ii]
                near_d[ii[better]] = dm[better]
                near_i[ii[better]] = new0 + j[better]
        alive &= near_d > kill
        act = alive & (near_d < infl)
        if not act.any():
            break
        ni = near_i[act]
        v = att[act] - pos[ni]
        v /= np.linalg.norm(v, axis=1, keepdims=True) + 1e-9
        acc = np.zeros_like(pos)
        np.add.at(acc, ni, v)
        grow = np.unique(ni)
        grow = grow[kids[grow] < 3]
        if len(grow) == 0:
            break
        # a twig keeps something of the way it was going
        back = np.where(par[grow] >= 0, par[grow], grow)
        inertia = pos[grow] - pos[back]
        inertia /= np.linalg.norm(inertia, axis=1, keepdims=True) + 1e-9
        d = acc[grow] / (np.linalg.norm(acc[grow], axis=1, keepdims=True) + 1e-9) + 0.45 * inertia + up + r.normal(0, 0.07, (len(grow), 3))
        ln = np.linalg.norm(d, axis=1)
        ok = ln > 0.25
        grow, d = grow[ok], d[ok] / ln[ok, None]
        if len(grow) == 0:
            break
        new0 = len(pos)
        kids[grow] += 1
        pos = np.concatenate([pos, pos[grow] + d * step])
        par = np.concatenate([par, grow])
        kids = np.concatenate([kids, np.zeros(len(grow), int)])
    return pos, par


def _radii(pos, par, trunk, tip=0.011):
    n = len(pos)
    nkids = np.bincount(par[par >= 0], minlength=n)
    tips = int((nkids == 0).sum())
    # the pipe model's exponent, chosen so this many tips add up to this trunk
    p = math.log(max(tips, 2)) / math.log(trunk / tip)
    p = min(max(p, 1.75), 3.0)
    acc = np.zeros(n)
    acc[nkids == 0] = tip ** p
    for i in range(n - 1, 0, -1):
        acc[par[i]] += acc[i]
    rad = acc ** (1 / p)
    rad *= trunk / rad[0]
    rad = np.maximum(rad, tip * 0.6)
    # the root flare
    rad *= 1.0 + 0.55 * np.exp(-pos[:, 2] / (trunk * 1.6))
    return rad, nkids


def _chains(par, rad, nkids):
    """Splits the skeleton into unbranched runs, each continuing along its
    thickest child."""
    n = len(par)
    children = [[] for _ in range(n)]
    for i in range(1, n):
        children[par[i]].append(i)
    chains = []
    stack = [(0, None)]
    while stack:
        start, origin = stack.pop()
        run = [origin] if origin is not None else []
        node = start
        while True:
            run.append(node)
            ch = children[node]
            if not ch:
                break
            ch = sorted(ch, key=lambda c: -rad[c])
            for other in ch[1:]:
                stack.append((other, node))
            node = ch[0]
        chains.append(run)
    return chains


def _leaf_shape(kind):
    """The outline of one leaf, base at the origin, tip at +y, unit length."""
    if kind == "plane":
        return np.array([(0, 0), (-0.3, 0.1), (-0.56, 0.42), (-0.3, 0.6), (-0.2, 0.86), (0, 1.0), (0.2, 0.86), (0.3, 0.6), (0.56, 0.42), (0.3, 0.1)], float)
    if kind == "oak":
        return np.array([(0, 0), (-0.2, 0.2), (-0.36, 0.5), (-0.16, 0.72), (0, 1.0), (0.16, 0.72), (0.36, 0.5), (0.2, 0.2)], float)
    if kind == "petal":
        return np.array([(0, 0), (-0.42, 0.3), (-0.5, 0.66), (-0.22, 0.96), (0.22, 0.96), (0.5, 0.66), (0.42, 0.3)], float)
    return np.array([(0, 0), (-0.26, 0.3), (-0.24, 0.66), (0, 1.0), (0.24, 0.66), (0.26, 0.3)], float)


def _leaves(mb, r, at, tangent, n, size, outline, mat, crown_c, crown_r, droop=0.5, cup=0.16):
    """n leaves hung on the twig points `at` (with the twig's direction there)."""
    k = r.integers(0, len(at), n)
    p = at[k]
    t = tangent[k]
    # each leaf faces somewhere near up, tipped its own way, and hangs a little
    nrm = np.array([0.0, 0.0, 1.0])[None] + r.normal(0, 0.5, (n, 3))
    nrm /= np.linalg.norm(nrm, axis=1, keepdims=True)
    side = np.cross(t, r.normal(0, 1, (n, 3)))
    side /= np.linalg.norm(side, axis=1, keepdims=True) + 1e-9
    axis = side + t * r.uniform(-0.2, 0.7, (n, 1)) - np.array([0, 0, 1.0])[None] * r.uniform(0.0, droop, (n, 1))
    axis -= nrm * (axis * nrm).sum(1, keepdims=True)
    axis /= np.linalg.norm(axis, axis=1, keepdims=True) + 1e-9
    wide = np.cross(nrm, axis)
    ln = size * r.uniform(0.7, 1.25, n)
    base = p + side * r.uniform(0.02, 0.09, (n, 1)) + r.normal(0, 0.03, (n, 3))
    m = len(outline)
    ox = outline[:, 0][None, :, None]
    oy = outline[:, 1][None, :, None]
    fold = -cup * np.abs(outline[:, 0])[None, :, None] + 0.1 * (outline[:, 1] ** 2)[None, :, None] * r.uniform(-1, 1, (n, 1, 1))
    v = base[:, None, :] + (ox * wide[:, None, :] + oy * axis[:, None, :] + fold * nrm[:, None, :]) * ln[:, None, None]
    faces = np.arange(m)[None, :] + (np.arange(n) * m)[:, None]
    tone = r.random(n)
    depth = np.clip(np.linalg.norm((p - crown_c) / crown_r, axis=1), 0, 1.2) / 1.2
    mb.add(v.reshape(-1, 3), faces, mat=mat, smooth=True, vattr={"v": np.repeat(tone, m), "d": np.repeat(depth, m)})


def grow_tree(name):
    species, seed, size = VARIANTS[name]
    sp = SPECIES[species]
    q = float(os.environ.get("FILM_QUALITY", "1"))
    r = rng(seed)
    pos, par = _grow(r, sp, size)
    rad, nkids = _radii(pos, par, sp["trunk"] * size)
    mb = MB()
    twig_p = []
    twig_t = []
    for run in _chains(par, rad, nkids):
        pts = pos[run]
        rr = rad[run].copy()
        if len(run) < 2:
            continue
        # a side limb starts inside its parent, at its own thickness
        if par[run[0]] >= 0 or run[0] != 0:
            rr[0] = rr[1]
        if len(pts) >= 3:
            m = max(3, len(pts) * 2 - 1)
            d = np.concatenate(([0], np.cumsum(np.linalg.norm(np.diff(pts, axis=0), axis=1))))
            sm = smooth_path(pts, m)
            ds = np.concatenate(([0], np.cumsum(np.linalg.norm(np.diff(sm, axis=0), axis=1))))
            rr = np.interp(ds / ds[-1], d / d[-1], rr)
            pts = sm
        rmax = rr.max()
        if rmax < 0.045:
            # the leaf-bearing wood
            t = np.gradient(pts, axis=0)
            t /= np.linalg.norm(t, axis=1, keepdims=True) + 1e-9
            twig_p.append(pts)
            twig_t.append(t)
        sides = 12 if rmax > 0.2 else 8 if rmax > 0.09 else 6 if rmax > 0.035 else 4 if rmax > 0.016 else 3
        t, n, b = frames_along(pts)
        ang = np.arange(sides) * TAU / sides
        ring = np.cos(ang)[None, :, None] * n[:, None, :] + np.sin(ang)[None, :, None] * b[:, None, :]
        v = pts[:, None, :] + ring * rr[:, None, None]
        i = np.arange(len(pts) - 1)[:, None]
        j = np.arange(sides)[None, :]
        j2 = (j + 1) % sides
        quads = np.stack([i * sides + j, i * sides + j2, (i + 1) * sides + j2, (i + 1) * sides + j], axis=-1).reshape(-1, 4)
        mb.add(v.reshape(-1, 3), quads, mat=0, smooth=True)
    twig_p = np.concatenate(twig_p)
    twig_t = np.concatenate(twig_t)
    crown_c = np.array([0, 0, (sp["base"] + (sp["h"] - sp["base"]) * 0.5) * size])
    crown_r = np.array([sp["w"] / 2, sp["w"] / 2, (sp["h"] - sp["base"]) / 2]) * size
    n_leaves = int(sp["leaves"] * size * size * q)
    if species == "locust":
        # a compound leaf: a row of small leaflets either side of a stalk
        base_n = n_leaves
        k = r.integers(0, len(twig_p), base_n)
        stalk_p = []
        stalk_t = []
        for j in range(9):
            d = twig_t[k] * 0.3 + np.cross(twig_t[k], r.normal(0, 1, (base_n, 3)))
            d[:, 2] -= 0.35
            d /= np.linalg.norm(d, axis=1, keepdims=True) + 1e-9
            if j == 0:
                dirs = d
                origin = twig_p[k]
            stalk_p.append(origin + dirs * (0.03 + 0.026 * j))
            stalk_t.append(dirs)
        _leaves(mb, r, np.concatenate(stalk_p), np.concatenate(stalk_t), base_n * 16, 0.038, _leaf_shape("elm"), 1, crown_c, crown_r, droop=0.2)
    elif species == "blossom":
        _leaves(mb, r, twig_p, twig_t, n_leaves, 0.03, _leaf_shape("petal"), 1, crown_c, crown_r, droop=0.1, cup=-0.2)
        _leaves(mb, r, twig_p, twig_t, n_leaves // 9, 0.07, _leaf_shape("elm"), 2, crown_c, crown_r)
    else:
        _leaves(mb, r, twig_p, twig_t, n_leaves, sp["leaf"] * size ** 0.3, _leaf_shape(species if species in ("plane", "oak") else "elm"), 1, crown_c, crown_r)
    return mb, species


# ------------------------------------------------------------------ materials


def mat_bark(name, kind):
    mat, nt = material(name)
    p = nt.coord("Object")
    x, y, z = nt.xyz(p)
    if kind == "plane":
        # a plane tree sheds its bark in plates: olive over cream over grey
        plates = nt.voronoi(nt.vec(x * 5.0, y * 5.0, z * 2.2), scale=1.0, out="Color")
        a = nt.xyz(plates)[0]
        b = nt.noise(nt.vec(x * 3.0, y * 3.0, z * 1.4), scale=1.0, detail=3.0)
        col = nt.ramp(a * 0.6 + b * 0.4, [(0.25, (0.17, 0.15, 0.11)), (0.42, (0.3, 0.29, 0.2)), (0.55, (0.52, 0.49, 0.38)), (0.72, (0.42, 0.4, 0.33)), (0.9, (0.22, 0.2, 0.16))], interp="CONSTANT")
        edge = nt.voronoi(nt.vec(x * 5.0, y * 5.0, z * 2.2), scale=1.0, out="Distance", feature="DISTANCE_TO_EDGE")
        height = nt.smoothstep(0.0, 0.08, edge) * 0.5 + a * 0.5
        strength = 0.35
    elif kind == "smooth":
        band = nt.noise(nt.vec(x * 2.0, y * 2.0, z * 26.0), scale=1.0, detail=2.0)
        col = nt.ramp(band, [(0.3, (0.1, 0.075, 0.065)), (0.6, (0.17, 0.13, 0.11)), (0.8, (0.23, 0.19, 0.16))])
        height = band
        strength = 0.3
    else:
        # deep furrows running up the trunk
        furrow = nt.noise(nt.vec(x * 34.0, y * 34.0, z * 3.0), scale=1.0, detail=4.0, rough=0.6, dist=0.4)
        col = nt.ramp(furrow, [(0.3, (0.035, 0.03, 0.026)), (0.5, (0.1, 0.085, 0.07)), (0.7, (0.17, 0.15, 0.125))])
        height = furrow
        strength = 0.8
    moss = nt.noise(p, scale=0.9, detail=3.0)
    col = nt.mixc(col, nt.rgb((0.1, 0.13, 0.05)), nt.smoothstep(0.6, 0.8, moss) * 0.3)
    n = nt.bump(height, strength=strength, distance=0.03)
    nt.out_surface(nt.principled(Base_Color=col, Roughness=0.8, Normal=n))
    return mat


def mat_leaf(name, dark, mid, light, through, gloss=0.12):
    """A leaf: its own tone, darker deep in the crown, and the light that
    comes through it from behind."""
    mat, nt = material(name)
    v = nt.attr("v")
    d = nt.attr("d")
    tree = nt.objinfo("Random")
    col = nt.ramp(v, [(0.0, dark), (0.5, mid), (0.88, light), (1.0, (light[0] * 1.15, light[1] * 1.05, light[2] * 0.8))])
    # each tree is its own green
    col = nt.hsv(col, h=0.5 + (tree - 0.5) * 0.075, s=0.82 + tree * 0.3, v=0.8 + nt.fract(tree * 7.31) * 0.42)
    # new growth at the outside of the crown is paler
    col = nt.mixc(nt.mixc(col, nt.rgb(dark), 0.45), col, nt.smoothstep(0.3, 0.95, d))
    thru = nt.mixc(col, nt.rgb(through), 0.6)
    body = nt.mix_shader(nt.shader("ShaderNodeBsdfDiffuse", Color=col), nt.shader("ShaderNodeBsdfTranslucent", Color=thru), 0.4)
    shine = nt.shader("ShaderNodeBsdfGlossy", Color=(1.0, 1.0, 1.0, 1.0), Roughness=0.32)
    nt.out_surface(nt.mix_shader(body, shine, nt.layer(0.3, "Fresnel") * gloss * 3.0 + gloss * 0.2))
    return mat


def _materials():
    return {
        "plane": [mat_bark("bark_plane", "plane"), mat_leaf("leaf_plane", (0.03, 0.075, 0.012), (0.075, 0.17, 0.024), (0.18, 0.3, 0.05), (0.42, 0.55, 0.07))],
        "elm": [mat_bark("bark_elm", "furrow"), mat_leaf("leaf_elm", (0.022, 0.065, 0.014), (0.055, 0.14, 0.026), (0.14, 0.25, 0.045), (0.36, 0.5, 0.06))],
        "locust": [mat_bark("bark_locust", "furrow"), mat_leaf("leaf_locust", (0.06, 0.13, 0.015), (0.13, 0.24, 0.03), (0.3, 0.4, 0.06), (0.55, 0.62, 0.08), gloss=0.06)],
        "oak": [mat_bark("bark_oak", "furrow"), mat_leaf("leaf_oak", (0.018, 0.055, 0.012), (0.04, 0.11, 0.02), (0.1, 0.2, 0.035), (0.3, 0.45, 0.05), gloss=0.2)],
        "young": [mat_bark("bark_young", "smooth"), mat_leaf("leaf_young", (0.05, 0.12, 0.014), (0.11, 0.22, 0.03), (0.25, 0.37, 0.055), (0.5, 0.6, 0.08))],
        "blossom_pink": [mat_bark("bark_cherry", "smooth"), mat_leaf("petal_pink", (0.62, 0.3, 0.38), (0.85, 0.52, 0.6), (0.96, 0.78, 0.82), (1.0, 0.75, 0.8), gloss=0.03), mat_leaf("leaf_cherry", (0.08, 0.1, 0.02), (0.16, 0.19, 0.04), (0.3, 0.3, 0.07), (0.5, 0.5, 0.08))],
        "blossom_white": [mat_bark("bark_pear", "smooth"), mat_leaf("petal_white", (0.72, 0.7, 0.62), (0.9, 0.89, 0.84), (0.98, 0.97, 0.93), (1.0, 0.98, 0.9), gloss=0.03), mat_leaf("leaf_pear", (0.05, 0.11, 0.02), (0.1, 0.2, 0.04), (0.22, 0.33, 0.06), (0.45, 0.58, 0.08))],
    }


def build(col=None):
    col = col or collection("Trees")
    mats = _materials()
    meshes = {}
    for name in VARIANTS:
        mb, species = grow_tree(name)
        key = name if name.startswith("blossom") else species
        meshes[name] = mb.build(f"Tree_{name}", mats[key])
        print(f"[trees] {name}: {len(meshes[name].polygons)} polygons")
    r = rng(99)
    for k, (x, y, kind, size, turn) in enumerate(site.tree_sites()):
        name = KIND_VARIANTS[kind][int(r.integers(0, len(KIND_VARIANTS[kind])))]
        s = size * r.uniform(0.92, 1.08)
        add_object(f"Tree{k:03d}", meshes[name], col, loc=(x, y, -0.03), rot=(0.0, 0.0, turn), scale=(s, s, s * r.uniform(0.94, 1.06)))
    print(f"[trees] {len(col.objects)} trees")
    return col

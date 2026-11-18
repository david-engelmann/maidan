"""The walls of the room: masonry buildings with real depth in their
fronts (windows set back in their reveals, sills, piers, string courses,
cornices, shopfronts, and rooms behind the glass that light up at dusk), a
dark glass slab, a cream clock tower with a gilded cupola, water tanks on the
roofs, and the layered skyline beyond with one stepped spire.

Nothing here carries a sign or a name.
"""

import math

import numpy as np

from . import site
from .common import MB, TAU, add_object, box, boxes, collection, grade, material, revolve, rng

Z = np.array([0.0, 0.0, 1.0])
# material slots, the same order in every building mesh
WALL, TRIM, FRAME, GLASS, ROOM, BLIND, ROOF, METAL, SHOP, AWNING, GOLD, DIAL = range(12)


# ------------------------------------------------------------------ materials


def mat_masonry(name, tone, kind="stone", stain=0.5):
    """A wall: limestone ashlar, brick or terracotta, weathered where water
    runs and soot settles."""
    mat, nt = material(name)
    p = nt.coord("Object")
    x, y, z = nt.xyz(p)
    broad = nt.noise(p, scale=0.06, detail=4.0, rough=0.6)
    mid = nt.noise(p, scale=0.9, detail=3.0, rough=0.6)
    # streaks: noise stretched down the wall
    streak = nt.noise(nt.vec(x * 2.2, y * 2.2, z * 0.11), scale=1.0, detail=4.0, rough=0.65)
    base = nt.rgb(tone)
    dark = nt.rgb((tone[0] * 0.5, tone[1] * 0.48, tone[2] * 0.46))
    pale = nt.rgb((min(1, tone[0] * 1.22), min(1, tone[1] * 1.2), min(1, tone[2] * 1.16)))
    col = nt.mixc(base, pale, nt.smoothstep(0.45, 0.8, broad) * 0.6)
    col = nt.mixc(col, dark, nt.smoothstep(0.52, 0.8, streak) * stain)
    if kind == "brick":
        course = nt.fract(z / 0.075)
        row = nt.floor(z / 0.075)
        along = (x + y) / 0.215 + row * 0.5
        tone_b = nt.white(nt.vec(nt.floor(along), row, 0.0))
        mortar = 1.0 - nt.smoothstep(0.02, 0.12, nt.min(nt.min(course, 1.0 - course), nt.min(nt.fract(along), 1.0 - nt.fract(along)) * 2.8))
        col = nt.mixc(col, nt.mixc(col, nt.rgb((0.0, 0.0, 0.0)), 0.55), tone_b * 0.6)
        col = nt.mixc(col, nt.rgb((0.42, 0.39, 0.34)), mortar * 0.6)
        height = -mortar
        rough = 0.82
    else:
        course_h = 0.62 if kind == "stone" else 0.38
        course = nt.fract(z / course_h)
        row = nt.floor(z / course_h)
        along = (x + y) / (course_h * 2.3) + row * 0.5
        block = nt.white(nt.vec(nt.floor(along), row, 0.0))
        joint = 1.0 - nt.smoothstep(0.006, 0.03, nt.min(nt.min(course, 1.0 - course), nt.min(nt.fract(along), 1.0 - nt.fract(along)) * 2.3))
        col = nt.mixc(col, nt.mixc(col, dark, 0.5), (block - 0.5) * 0.5 + 0.2)
        col = nt.mixc(col, dark, joint * 0.6)
        height = -joint
        rough = 0.7 if kind == "stone" else 0.4
    # hollows gather dirt: under sills and cornices, in the reveals
    ao = nt.node("ShaderNodeAmbientOcclusion", samples=4)
    ao.inputs["Distance"].default_value = 1.2
    col = nt.mixc(nt.mixc(col, dark, 0.75), col, nt.pow(nt.v(ao.outputs["AO"]), 1.4))
    n = nt.bump(height + mid * 0.3, strength=0.35, distance=0.02)
    nt.out_surface(nt.principled(Base_Color=col, Roughness=rough + mid * 0.1, Normal=n, Specular_IOR_Level=0.35))
    return mat


def mat_flat(name, tone, rough=0.5, metallic=0.0):
    mat, nt = material(name)
    p = nt.coord("Object")
    v = nt.noise(p, scale=1.4, detail=3.0)
    col = nt.mixc(nt.rgb(tone), nt.rgb((tone[0] * 0.6, tone[1] * 0.6, tone[2] * 0.6)), nt.smoothstep(0.4, 0.75, v) * 0.5)
    nt.out_surface(nt.principled(Base_Color=col, Roughness=rough, Metallic=metallic))
    return mat


def mat_glass(name="window_glass", tint=(0.72, 0.82, 0.88), mirror=0.14):
    """Window glass: it shows the room a little and the sky a lot, more of
    the sky the more obliquely it is seen."""
    mat, nt = material(name)
    wob = nt.noise(nt.coord("Object"), scale=0.35, detail=2.0)
    n = nt.bump(wob, strength=0.06, distance=0.2)
    fres = nt.layer(0.22, "Fresnel", normal=n)
    refl = nt.shader("ShaderNodeBsdfGlossy", Color=(*tint, 1.0), Roughness=0.03, Normal=n)
    through = nt.shader("ShaderNodeBsdfTransparent", Color=(0.85, 0.88, 0.9, 1.0))
    fac = nt.sat(fres * (1.0 - mirror) + mirror)
    # shadow rays go through: a lit room lights the sill outside it, and daylight gets in
    fac = fac * (1.0 - nt.lightpath("Is Shadow Ray"))
    nt.out_surface(nt.mix_shader(through, refl, fac))
    return mat


def _room_light(nt, g):
    lit = nt.attr("lit")
    tint = nt.attr("tint")
    on = nt.smoothstep(0.0, 0.012, g["windows"] - lit)
    col = nt.ramp(tint, [(0.0, (1.0, 0.56, 0.24)), (0.45, (1.0, 0.7, 0.4)), (0.75, (1.0, 0.84, 0.62)), (0.9, (0.95, 0.93, 0.88)), (1.0, (0.62, 0.8, 1.0))])
    # by day a lit room barely shows; at dusk it is the brightest thing in the wall
    power = on * (0.5 + 3.4 * nt.fract(lit * 37.7 + tint * 11.3)) * nt.mix(0.06, 1.0, g["night"])
    return col, power, on


def mat_room(name="room"):
    """Behind the glass: a room, dark by day, lit at dusk in its own colour."""
    mat, nt = material(name)
    g = grade(nt)
    col, power, on = _room_light(nt, g)
    p = nt.coord("Object")
    x, y, z = nt.xyz(p)
    # furniture, curtains, a lamp: enough unevenness that no window is a flat card
    stuff = nt.noise(nt.vec(x * 1.3, y * 1.3, z * 0.9), scale=1.0, detail=3.0, rough=0.6)
    fold = nt.noise(nt.vec(x * 9.0, y * 9.0, z * 0.2), scale=1.0, detail=1.0)
    shade = 0.35 + stuff * 1.1 + fold * 0.25
    em = nt.shader("ShaderNodeEmission", Color=col, Strength=power * shade * 1.15)
    wall = nt.principled(Base_Color=nt.mixc(nt.rgb((0.1, 0.09, 0.08)), nt.rgb((0.3, 0.27, 0.22)), stuff), Roughness=0.8)
    nt.out_surface(nt.add_shader(wall, em))
    return mat


def mat_blind(name="blind"):
    mat, nt = material(name)
    g = grade(nt)
    col, power, on = _room_light(nt, g)
    p = nt.coord("Object")
    slat = nt.fract(nt.xyz(p)[2] / 0.06)
    tone = nt.mixc(nt.rgb((0.62, 0.58, 0.5)), nt.rgb((0.45, 0.42, 0.37)), nt.smoothstep(0.6, 0.9, slat))
    em = nt.shader("ShaderNodeEmission", Color=nt.mixc(col, nt.rgb((1.0, 0.9, 0.75)), 0.4), Strength=power * 0.9)
    nt.out_surface(nt.add_shader(nt.principled(Base_Color=tone, Roughness=0.7), em))
    return mat


def mat_shop(name="shopfront"):
    """Street-level glass with a lit interior behind it."""
    mat, nt = material(name)
    g = grade(nt)
    p = nt.coord("Object")
    x, y, z = nt.xyz(p)
    stuff = nt.noise(nt.vec(x * 0.7, y * 0.7, z * 1.2), scale=1.0, detail=4.0, rough=0.7)
    warm = nt.ramp(nt.attr("tint"), [(0.0, (1.0, 0.6, 0.28)), (0.6, (1.0, 0.78, 0.5)), (1.0, (0.9, 0.92, 1.0))])
    em = nt.shader("ShaderNodeEmission", Color=warm, Strength=(0.2 + g["night"] * 1.9) * (0.15 + stuff * stuff * 2.6) * nt.smoothstep(0.0, 0.2, nt.attr("lit") + 0.35))
    refl = nt.shader("ShaderNodeBsdfGlossy", Color=(0.8, 0.85, 0.9, 1.0), Roughness=0.03)
    dark = nt.principled(Base_Color=(0.03, 0.03, 0.035, 1.0), Roughness=0.3)
    body = nt.add_shader(dark, em)
    nt.out_surface(nt.mix_shader(body, refl, nt.layer(0.2, "Fresnel") * 0.8 + 0.08))
    return mat


def mat_gold(name="gilding"):
    mat, nt = material(name)
    p = nt.coord("Object")
    leaf = nt.noise(p, scale=6.0, detail=3.0)
    col = nt.mixc(nt.rgb((1.0, 0.7, 0.22)), nt.rgb((0.95, 0.56, 0.14)), leaf)
    nt.out_surface(nt.principled(Base_Color=col, Metallic=1.0, Roughness=0.2 + leaf * 0.16))
    return mat


def mat_dial(name="clock_dial"):
    mat, nt = material(name)
    g = grade(nt)
    em = nt.shader("ShaderNodeEmission", Color=(1.0, 0.86, 0.62, 1.0), Strength=g["lamps"] * 2.6)
    nt.out_surface(nt.add_shader(nt.principled(Base_Color=(0.8, 0.78, 0.7, 1.0), Roughness=0.4), em))
    return mat


def mat_far(name, tone, glass=(0.1, 0.13, 0.17), bay=3.2, floor=3.7, seed=0.0, curtain=False):
    """A distant front, drawn in the shader: a grid of windows over a wall,
    some of them lit. The UVs are in metres along and up the wall."""
    mat, nt = material(name)
    g = grade(nt)
    uv = nt.coord("UV")
    u, v, _ = nt.xyz(uv)
    cu = nt.floor(u / bay)
    cv = nt.floor(v / floor)
    fu = nt.fract(u / bay)
    fv = nt.fract(v / floor)
    if curtain:
        win = nt.gt(fu, 0.05) * nt.lt(fu, 0.95) * nt.gt(fv, 0.26)
    else:
        win = nt.gt(fu, 0.24) * nt.lt(fu, 0.76) * nt.gt(fv, 0.24) * nt.lt(fv, 0.78)
    cell = nt.vec(cu, cv, seed)
    rnd = nt.white(cell)
    rnd2 = nt.white(nt.vec(cv, cu, seed + 5.0))
    on = win * nt.smoothstep(0.0, 0.02, g["windows"] * 0.55 - rnd)
    lamp = nt.ramp(rnd2, [(0.0, (1.0, 0.58, 0.26)), (0.5, (1.0, 0.74, 0.46)), (0.85, (1.0, 0.9, 0.74)), (1.0, (0.7, 0.84, 1.0))])
    p = nt.coord("Object")
    weather = nt.noise(p, scale=0.05, detail=3.0)
    wall = nt.mixc(nt.rgb(tone), nt.rgb((tone[0] * 0.62, tone[1] * 0.6, tone[2] * 0.6)), nt.smoothstep(0.4, 0.75, weather) * 0.6)
    # floors show as faint bands, as spandrels do from far off
    wall = nt.mixc(wall, nt.mixc(wall, nt.rgb((0.0, 0.0, 0.0)), 0.25), nt.lt(fv, 0.08))
    col = nt.mixc(wall, nt.rgb(glass), win)
    rough = nt.mix(0.75, 0.06, win)
    em = nt.shader("ShaderNodeEmission", Color=lamp, Strength=on * (0.2 + rnd2 * rnd2 * 1.6))
    nt.out_surface(nt.add_shader(nt.principled(Base_Color=col, Roughness=rough, Specular_IOR_Level=nt.mix(0.3, 1.0, win)), em))
    return mat


# ------------------------------------------------------------- facade pieces


class Wall:
    """One plane of a building: a corner, the direction along it, and its
    outward normal. Pieces are given in (along, up, out) and placed in the
    world."""

    def __init__(self, mb, origin, normal, r):
        self.mb = mb
        self.o = np.asarray(origin, float)
        self.n = np.asarray(normal, float)
        self.u = np.cross(Z, self.n)
        self.r = r

    def world(self, uvd):
        uvd = np.asarray(uvd, float)
        return self.o[None] + uvd[:, 0:1] * self.u[None] + uvd[:, 1:2] * Z[None] + uvd[:, 2:3] * self.n[None]

    def quads(self, v, q, mat, fattr=None, smooth=False):
        self.mb.add(self.world(np.asarray(v).reshape(-1, 3)), q, mat=mat, fattr=fattr, smooth=smooth)

    def bars(self, lo, hi, mat):
        """Boxes given by their low and high corners in (along, up, out)."""
        lo = np.asarray(lo, float).reshape(-1, 3)
        hi = np.asarray(hi, float).reshape(-1, 3)
        if len(lo) == 0:
            return
        v, q = boxes(np.minimum(lo, hi), np.maximum(lo, hi))
        self.mb.add(self.world(v), q, mat=mat)

    def plain(self, u0, u1, v0, v1, mat=WALL, d=0.0):
        self.quads([(u0, v0, d), (u1, v0, d), (u1, v1, d), (u0, v1, d)], [[0, 1, 2, 3]], mat)

    def evening(self, ni, nj, profile):
        """Which windows light up, and in what colour, the way a real evening
        goes: offices lit a floor at a time and mostly cool, flats lit here
        and there and warm, whole floors dark. Returns, per window, the point
        of dusk at which it comes on (0 early, 1 never) and its tint."""
        r = self.r
        u = r.random((ni, nj))
        if profile == "office":
            kind = r.random(nj)
            floor = np.where(kind < 0.5, 0.06, np.where(kind < 0.86, 1.1, 7.0))
            # part of a busy floor is still dark: a run of bays, not a scatter
            run0 = r.integers(0, max(1, ni), nj)
            runl = r.integers(max(1, ni // 4), ni + 1, nj)
            ii = np.arange(ni)[:, None]
            inrun = ((ii - run0[None, :]) % ni) < runl[None, :]
            gain = floor[None, :] * np.where(inrun, 1.0, 0.1)
            tint = np.clip(0.8 + 0.12 * r.normal(0, 1, (ni, nj)) + 0.1 * (r.random(nj)[None, :] - 0.5), 0, 1)
            tint = np.where(r.random((ni, nj)) < 0.18, r.uniform(0.25, 0.6, (ni, nj)), tint)
        elif profile == "hotel":
            gain = np.full((ni, nj), 1.5) * np.where(r.random(nj) < 0.12, 0.05, 1.0)[None, :]
            tint = np.clip(0.4 + 0.08 * r.normal(0, 1, (ni, nj)), 0, 1)
        else:  # flats: a stack of the same room up the building, some of them home
            stack = r.uniform(0.3, 1.7, ni) ** 1.5
            floor = np.where(r.random(nj) < 0.2, 0.15, 1.0)
            gain = stack[:, None] * floor[None, :]
            # a lit neighbour makes a lit window likelier: flats are more than one window wide
            u = np.minimum(u, np.roll(u, 1, axis=0) + r.uniform(0.0, 0.5, (ni, nj)))
            tint = np.clip(r.beta(2.0, 3.5, (ni, nj)) * 0.8, 0, 1)
            tint = np.where(r.random((ni, nj)) < 0.05, 1.0, tint)
        lit = np.clip(u / np.maximum(gain, 1e-3), 0, 1)
        return lit.ravel(), tint.ravel()

    def windows(self, u0, v0, cw, ch, ni, nj, ow, oh, sill, reveal=0.32, lintel=0.0, mullion=False, rooms=True, lit_bias=0.0, frame=0.06, blinds=0.55, profile="flats"):
        """A grid of ni by nj windows, each in a cell cw by ch: the wall round
        the opening, the reveal, the frame and glass, a sill, a blind, and a
        room behind."""
        r = self.r
        n = ni * nj
        if n <= 0:
            return
        ii, jj = np.meshgrid(np.arange(ni), np.arange(nj), indexing="ij")
        off = np.stack([u0 + ii.ravel() * cw, v0 + jj.ravel() * ch, np.zeros(n)], axis=-1)
        a0 = (cw - ow) / 2
        a1 = a0 + ow
        b0 = sill
        b1 = sill + oh
        lit, tint = self.evening(ni, nj, profile)
        lit = np.clip(lit + lit_bias, 0, 1)

        def tile(v, q, mat, attrs=True):
            v = np.asarray(v, float)
            q = np.asarray(q)
            vv = (v[None] + off[:, None, :]).reshape(-1, 3)
            qq = (q[None] + (np.arange(n) * len(v))[:, None, None]).reshape(-1, q.shape[1])
            fa = {"lit": np.repeat(lit, len(q)), "tint": np.repeat(tint, len(q))} if attrs else None
            self.quads(vv, qq, mat, fattr=fa)

        # the wall round the opening, and the reveal going back from it
        v = [(0, 0, 0), (cw, 0, 0), (cw, ch, 0), (0, ch, 0), (a0, b0, 0), (a1, b0, 0), (a1, b1, 0), (a0, b1, 0), (a0, b0, -reveal), (a1, b0, -reveal), (a1, b1, -reveal), (a0, b1, -reveal)]
        tile(v, [[0, 1, 5, 4], [1, 2, 6, 5], [2, 3, 7, 6], [3, 0, 4, 7], [4, 5, 9, 8], [5, 6, 10, 9], [6, 7, 11, 10], [7, 4, 8, 11]], WALL, attrs=False)
        tile([(a0, b0, -reveal), (a1, b0, -reveal), (a1, b1, -reveal), (a0, b1, -reveal)], [[0, 1, 2, 3]], GLASS)
        # the frame: a border, a meeting rail, and a mullion in a wide window
        f = frame
        d = -reveal + 0.035
        fv = []
        fq = []

        def strip(x0, y0, x1, y1):
            k = len(fv)
            fv.extend([(x0, y0, d), (x1, y0, d), (x1, y1, d), (x0, y1, d)])
            fq.append([k, k + 1, k + 2, k + 3])

        strip(a0, b0, a1, b0 + f)
        strip(a0, b1 - f, a1, b1)
        strip(a0, b0, a0 + f, b1)
        strip(a1 - f, b0, a1, b1)
        strip(a0, b0 + oh * 0.52 - f / 2, a1, b0 + oh * 0.52 + f / 2)
        if mullion:
            strip((a0 + a1) / 2 - f / 2, b0, (a0 + a1) / 2 + f / 2, b1)
        tile(fv, fq, FRAME, attrs=False)
        if sill > 0.2:
            lo = off + np.array([a0 - 0.07, b0 - 0.1, 0.0])
            hi = off + np.array([a1 + 0.07, b0, 0.09])
            self.bars(lo, hi, TRIM)
        if lintel > 0:
            lo = off + np.array([a0 - 0.09, b1, 0.0])
            hi = off + np.array([a1 + 0.09, b1 + lintel, 0.06])
            self.bars(lo, hi, TRIM)
        if rooms:
            dr = -reveal - 0.05
            db = dr - 3.4
            v = [(a0 - 0.3, b0 - 0.5, dr), (a1 + 0.3, b0 - 0.5, dr), (a1 + 0.3, b1 + 0.3, dr), (a0 - 0.3, b1 + 0.3, dr), (a0 - 0.3, b0 - 0.5, db), (a1 + 0.3, b0 - 0.5, db), (a1 + 0.3, b1 + 0.3, db), (a0 - 0.3, b1 + 0.3, db)]
            tile(v, [[4, 5, 6, 7], [0, 1, 5, 4], [3, 7, 6, 2], [0, 4, 7, 3], [1, 2, 6, 5]], ROOM)
            # a blind, drawn down a different way in every window
            has = r.random(n) < blinds
            drop = r.uniform(0.12, 0.85, n) ** 1.3 * oh
            sel = np.nonzero(has)[0]
            if len(sel):
                o = off[sel]
                dd = -reveal - 0.02
                bv = np.stack([o + np.array([a0, b1, dd]), o + np.array([a1, b1, dd]), o + np.array([a1, b1, dd]), o + np.array([a0, b1, dd])], axis=1)
                bv[:, 0, 1] -= drop[sel]
                bv[:, 1, 1] -= drop[sel]
                bq = np.arange(4)[None, :] + (np.arange(len(sel)) * 4)[:, None]
                self.quads(bv.reshape(-1, 3), bq, BLIND, fattr={"lit": lit[sel], "tint": tint[sel]})

    def shops(self, u0, v0, cw, ch, ni, awnings=0.4):
        """Street level: wide glass between piers, a fascia over it, awnings."""
        r = self.r
        n = ni
        off = np.stack([u0 + np.arange(n) * cw, np.full(n, v0), np.zeros(n)], axis=-1)
        pier = 0.5
        a0, a1 = pier, cw - pier
        b0, b1 = 0.45, ch - 0.9
        lit = r.random(n) * 0.5
        tint = r.random(n)
        v = np.array([(0, 0, 0), (cw, 0, 0), (cw, ch, 0), (0, ch, 0), (a0, b0, 0), (a1, b0, 0), (a1, b1, 0), (a0, b1, 0), (a0, b0, -0.45), (a1, b0, -0.45), (a1, b1, -0.45), (a0, b1, -0.45)], float)
        q = np.array([[0, 1, 5, 4], [1, 2, 6, 5], [2, 3, 7, 6], [3, 0, 4, 7], [4, 5, 9, 8], [5, 6, 10, 9], [6, 7, 11, 10], [7, 4, 8, 11]])
        vv = (v[None] + off[:, None, :]).reshape(-1, 3)
        self.quads(vv, (q[None] + (np.arange(n) * 12)[:, None, None]).reshape(-1, 4), TRIM)
        g = np.array([(a0, b0, -0.45), (a1, b0, -0.45), (a1, b1, -0.45), (a0, b1, -0.45)], float)
        self.quads((g[None] + off[:, None, :]).reshape(-1, 3), np.arange(4)[None, :] + (np.arange(n) * 4)[:, None], SHOP, fattr={"lit": lit, "tint": tint})
        # glazing bars
        lo = []
        hi = []
        for k in (1, 2):
            x = a0 + (a1 - a0) * k / 3
            lo.append(off + np.array([x - 0.03, b0, -0.45]))
            hi.append(off + np.array([x + 0.03, b1, -0.4]))
        lo.append(off + np.array([a0, b1 - 0.75, -0.45]))
        hi.append(off + np.array([a1, b1 - 0.69, -0.4]))
        self.bars(np.concatenate(lo), np.concatenate(hi), FRAME)
        has = np.nonzero(r.random(n) < awnings)[0]
        if len(has):
            o = off[has]
            # an awning: a sloped slab out over the pavement
            av = np.stack([o + np.array([a0 - 0.1, b1 + 0.25, 0.0]), o + np.array([a1 + 0.1, b1 + 0.25, 0.0]), o + np.array([a1 + 0.1, b1 - 0.45, 1.5]), o + np.array([a0 - 0.1, b1 - 0.45, 1.5])], axis=1)
            aq = np.arange(4)[None, :] + (np.arange(len(has)) * 4)[:, None]
            self.quads(av.reshape(-1, 3), aq, AWNING, fattr={"tint": r.random(len(has))})

    def cornice(self, u0, u1, v_top, size=0.7, dentils=True):
        steps = [(0.0, 0.18, 0.45), (0.45, 0.42, 0.75), (0.75, 0.66, 0.9), (0.9, 1.0, 1.0)]
        lo = []
        hi = []
        h = size * 1.35
        for a, out, b in steps:
            lo.append((u0 - out * size, v_top - h * (1 - a), 0.0))
            hi.append((u1 + out * size, v_top - h * (1 - b), out * size))
        self.bars(lo, hi, TRIM)
        if dentils:
            xs = np.arange(u0 + 0.2, u1 - 0.2, size * 0.75)
            lo = np.stack([xs, np.full_like(xs, v_top - h * 0.62), np.zeros_like(xs)], axis=-1)
            hi = np.stack([xs + size * 0.36, np.full_like(xs, v_top - h * 0.45), np.full_like(xs, size * 0.36)], axis=-1)
            self.bars(lo, hi, TRIM)

    def course(self, u0, u1, v, h=0.3, out=0.13):
        self.bars([(u0 - out, v, 0.0)], [(u1 + out, v + h, out)], TRIM)

    def piers(self, us, v0, v1, w=0.55, out=0.16):
        us = np.asarray(us, float)
        lo = np.stack([us - w / 2, np.full_like(us, v0), np.zeros_like(us)], axis=-1)
        hi = np.stack([us + w / 2, np.full_like(us, v1), np.full_like(us, out)], axis=-1)
        self.bars(lo, hi, WALL)


def _sides(x0, y0, x1, y1):
    return {
        "S": ((x0, y0, 0.0), (0.0, -1.0, 0.0), x1 - x0),
        "N": ((x1, y1, 0.0), (0.0, 1.0, 0.0), x1 - x0),
        "E": ((x1, y0, 0.0), (1.0, 0.0, 0.0), y1 - y0),
        "W": ((x0, y1, 0.0), (-1.0, 0.0, 0.0), y1 - y0),
    }


def _tank(mb, x, y, z, r, s=1.0):
    """A rooftop water tank: wooden staves, a conical cap, a steel stand."""
    legs_lo = []
    legs_hi = []
    for dx in (-1, 1):
        for dy in (-1, 1):
            legs_lo.append((x + dx * 1.3 * s - 0.08, y + dy * 1.3 * s - 0.08, z))
            legs_hi.append((x + dx * 1.3 * s + 0.08, y + dy * 1.3 * s + 0.08, z + 2.6 * s))
    legs_lo.append((x - 1.7 * s, y - 1.7 * s, z + 2.6 * s))
    legs_hi.append((x + 1.7 * s, y + 1.7 * s, z + 2.78 * s))
    v, q = boxes(np.array(legs_lo), np.array(legs_hi))
    mb.add(v, q, mat=METAL)
    prof = [(0.01, 2.78 * s), (1.75 * s, 2.78 * s), (1.68 * s, 6.1 * s), (1.82 * s, 6.1 * s), (0.02, 7.2 * s)]
    v, q, uv = revolve(prof, 16)
    v = v + np.array([x, y, z], np.float32)
    mb.add(v, q, mat=AWNING + 100, smooth=True)
    del r


def _roof(mb, r, x0, y0, x1, y1, z, tank=True, clutter=True):
    v, q = box((x0, y0, z - 0.4), (x1, y1, z))
    mb.add(v, q, mat=ROOF)
    # a parapet
    t = 0.3
    lo = [(x0, y0, z), (x0, y1 - t, z), (x0, y0, z), (x1 - t, y0, z)]
    hi = [(x1, y0 + t, z + 1.0), (x1, y1, z + 1.0), (x0 + t, y1, z + 1.0), (x1, y1, z + 1.0)]
    v, q = boxes(np.array(lo), np.array(hi))
    mb.add(v, q, mat=WALL)
    w, d = x1 - x0, y1 - y0
    if clutter and w > 9 and d > 9:
        lo = []
        hi = []
        for _ in range(int(r.integers(1, 4))):
            bw, bd, bh = r.uniform(3, 7), r.uniform(3, 7), r.uniform(2.6, 5.0)
            bx, by = r.uniform(x0 + 1.5, x1 - bw - 1.5), r.uniform(y0 + 1.5, y1 - bd - 1.5)
            lo.append((bx, by, z))
            hi.append((bx + bw, by + bd, z + bh))
        for _ in range(int(r.integers(2, 7))):
            bx, by = r.uniform(x0 + 1.2, x1 - 3), r.uniform(y0 + 1.2, y1 - 3)
            lo.append((bx, by, z))
            hi.append((bx + r.uniform(0.8, 2.0), by + r.uniform(0.8, 2.0), z + r.uniform(0.6, 1.6)))
        v, q = boxes(np.array(lo), np.array(hi))
        mb.add(v, q, mat=METAL)
        if tank and r.random() < 0.6:
            _tank(mb, r.uniform(x0 + 3.5, x1 - 3.5), r.uniform(y0 + 3.5, y1 - 3.5), z, r, s=r.uniform(0.85, 1.1))


def masonry(mb, r, rect, tiers, st, sides="NSEW", profile="flats"):
    """A masonry building: a shopfront base, floors of windows between piers,
    string courses, and a cornice on each tier, which may step back."""
    x0, y0, x1, y1 = rect
    z0 = 0.0
    for ti, (z1, inset) in enumerate(tiers):
        rx0, ry0, rx1, ry1 = x0 + inset, y0 + inset, x1 - inset, y1 - inset
        for name, (o, n, width) in _sides(rx0, ry0, rx1, ry1).items():
            w = Wall(mb, (o[0], o[1], 0.0), n, r)
            if name not in sides:
                w.plain(0, width, z0, z1)
                continue
            bay = st["bay"]
            nb = max(1, int(round((width - 2 * st["corner"]) / bay)))
            bay = (width - 2 * st["corner"]) / nb
            v = z0
            if ti == 0:
                # the base: shopfronts two bays wide
                bh = st["base_h"]
                ns = max(1, nb // 2)
                w.plain(0, st["corner"], v, v + bh, TRIM)
                w.plain(width - st["corner"], width, v, v + bh, TRIM)
                w.shops(st["corner"], v, (width - 2 * st["corner"]) / ns, bh, ns, awnings=st.get("awnings", 0.4))
                w.course(0, width, v + bh - 0.1, h=0.4, out=0.2)
                v += bh
            fh = st["floor"]
            nf = max(1, int((z1 - v - st["attic"]) / fh))
            fh = (z1 - v - st["attic"]) / nf
            w.plain(0, st["corner"], v, z1)
            w.plain(width - st["corner"], width, v, z1)
            w.windows(st["corner"], v, bay, fh, nb, nf, st["ow"] * bay, st["oh"] * fh, st["sill"] * fh, reveal=st["reveal"], lintel=st.get("lintel", 0.0), mullion=st["ow"] * bay > 1.7, blinds=st.get("blinds", 0.55), profile=profile)
            w.plain(st["corner"], width - st["corner"], z1 - st["attic"], z1)
            if st.get("piers"):
                us = st["corner"] + np.arange(0, nb + 1, st["piers"]) * bay
                w.piers(us, v, z1 - st["attic"] * 0.5)
            for k in st.get("courses", []):
                if 0 < k < nf:
                    w.course(0, width, v + k * fh - 0.15)
            w.cornice(0, width, z1, size=st.get("cornice", 0.7) * (1.0 if ti == len(tiers) - 1 else 0.6))
        last = ti == len(tiers) - 1
        _roof(mb, r, rx0, ry0, rx1, ry1, z1, tank=last, clutter=last)
        z0 = z1


def slab(mb, r, rect, h, sides="NSEW"):
    """The dark glass slab: storey-high bronze glass between thin fins, a band
    of spandrel at every floor, a blank crown."""
    x0, y0, x1, y1 = rect
    fh = 3.75
    bay = 1.55
    for name, (o, n, width) in _sides(x0, y0, x1, y1).items():
        w = Wall(mb, (o[0], o[1], 0.0), n, r)
        if name not in sides:
            w.plain(0, width, 0, h, METAL)
            continue
        nb = int(width / bay)
        bay_w = width / nb
        base = 7.5
        crown = 5.0
        nf = int((h - base - crown) / fh)
        w.plain(0, width, 0, base, METAL)
        w.shops(0, 0.0, width / max(1, nb // 5), base - 0.6, max(1, nb // 5), awnings=0.0)
        w.plain(0, width, h - crown, h, METAL)
        w.windows(0, base, bay_w, fh, nb, nf, bay_w - 0.09, fh - 0.95, 0.95, reveal=0.1, rooms=True, frame=0.03, blinds=0.12, lit_bias=0.05, profile="office")
        us = np.arange(nb + 1) * bay_w
        lo = np.stack([us - 0.045, np.full_like(us, base), np.zeros_like(us)], axis=-1)
        hi = np.stack([us + 0.045, np.full_like(us, h - crown), np.full_like(us, 0.16)], axis=-1)
        w.bars(lo, hi, FRAME)
    _roof(mb, r, x0, y0, x1, y1, h, tank=False)
    v, q = box((x0 + 5, y0 + 5, h), (x1 - 5, y1 - 5, h + 5.5))
    mb.add(v, q, mat=METAL)


def clock_tower(mb, r, cx, cy, half, z0, st):
    """The cream tower: a shaft of narrow windows, a clock on every face, an
    open arcade, a steep roof and a gilded cupola."""
    x0, y0, x1, y1 = cx - half, cy - half, cx + half, cy + half
    shaft_top = 84.0
    clock_top = 96.0
    loggia_top = 107.0
    for name, (o, n, width) in _sides(x0, y0, x1, y1).items():
        w = Wall(mb, (o[0], o[1], 0.0), n, r)
        corner = 3.2
        nb = 7
        bay = (width - 2 * corner) / nb
        nf = int((shaft_top - z0) / 3.7)
        fh = (shaft_top - z0) / nf
        w.plain(0, corner, z0, loggia_top)
        w.plain(width - corner, width, z0, loggia_top)
        w.windows(corner, z0, bay, fh, nb, nf, bay * 0.5, fh * 0.6, fh * 0.24, reveal=0.4, lintel=0.14, blinds=0.5, profile="hotel", lit_bias=0.1)
        w.piers(corner + np.arange(nb + 1) * bay, z0, shaft_top, w=0.5, out=0.2)
        w.piers([corner / 2, width - corner / 2], z0, loggia_top, w=corner * 0.72, out=0.3)
        w.course(0, width, shaft_top - 0.3, h=0.7, out=0.5)
        # the clock stage
        w.plain(corner, width - corner, shaft_top, clock_top)
        rad = 4.5
        c = np.array([width / 2, (shaft_top + clock_top) / 2 + 0.2])
        a = np.arange(48) * TAU / 48
        ring = np.stack([c[0] + rad * np.cos(a), c[1] + rad * np.sin(a), np.full(48, 0.12)], axis=-1)
        w.quads(np.concatenate([[(c[0], c[1], 0.12)], ring]), [[0, 1 + k, 1 + (k + 1) % 48] for k in range(48)], DIAL)
        outer = np.stack([c[0] + (rad + 0.45) * np.cos(a), c[1] + (rad + 0.45) * np.sin(a), np.full(48, 0.22)], axis=-1)
        inner = np.stack([c[0] + rad * np.cos(a), c[1] + rad * np.sin(a), np.full(48, 0.22)], axis=-1)
        w.quads(np.concatenate([inner, outer]), [[k, (k + 1) % 48, 48 + (k + 1) % 48, 48 + k] for k in range(48)], TRIM)
        for k in range(12):
            aa = k * TAU / 12
            p = c + (rad - 0.55) * np.array([math.cos(aa), math.sin(aa)])
            w.bars([(p[0] - 0.13, p[1] - 0.13, 0.12)], [(p[0] + 0.13, p[1] + 0.13, 0.18)], METAL)
        for aa, ln, wd in ((1.0, 2.6, 0.2), (2.75, 3.7, 0.13)):
            d = np.array([math.cos(aa), math.sin(aa)])
            nrm = np.array([-d[1], d[0]])
            pts = [c - d * 0.5 - nrm * wd, c - d * 0.5 + nrm * wd, c + d * ln + nrm * wd * 0.4, c + d * ln - nrm * wd * 0.4]
            w.quads([(q[0], q[1], 0.2) for q in pts], [[1, 0, 3, 2]], METAL)
        w.course(0, width, clock_top - 0.2, h=0.6, out=0.45)
        # the arcade: tall openings with nothing behind them but a lit chamber
        nb2 = 5
        bay2 = (width - 2 * corner) / nb2
        w.windows(corner, clock_top, bay2, loggia_top - clock_top, nb2, 1, bay2 * 0.56, (loggia_top - clock_top) * 0.74, (loggia_top - clock_top) * 0.1, reveal=1.1, lit_bias=-1.0, blinds=0.0, frame=0.0)
        w.cornice(0, width, loggia_top + 1.0, size=1.25)
    v, q = box((x0, y0, loggia_top), (x1, y1, loggia_top + 1.0))
    mb.add(v, q, mat=TRIM)
    # the roof: a steep pyramid in four curved faces
    base = half + 0.6
    top = 3.4
    zb, zt = loggia_top + 1.0, loggia_top + 15.0
    steps = 8
    rings = []
    for k in range(steps + 1):
        s = k / steps
        hw = base + (top - base) * (s ** 0.82)
        zz = zb + (zt - zb) * s
        rings.append([(cx - hw, cy - hw, zz), (cx + hw, cy - hw, zz), (cx + hw, cy + hw, zz), (cx - hw, cy + hw, zz)])
    rv = np.array(rings).reshape(-1, 3)
    rq = [[k * 4 + j, k * 4 + (j + 1) % 4, (k + 1) * 4 + (j + 1) % 4, (k + 1) * 4 + j] for k in range(steps) for j in range(4)]
    mb.add(rv, rq, mat=ROOF + 100)
    # the cupola: a ring of columns, a gilded dome, a finial
    zc = zt
    v, q = box((cx - top - 0.3, cy - top - 0.3, zc), (cx + top + 0.3, cy + top + 0.3, zc + 0.5))
    mb.add(v, q, mat=TRIM)
    lo = []
    hi = []
    for k in range(8):
        a = k * TAU / 8 + TAU / 16
        px, py = cx + 2.5 * math.cos(a), cy + 2.5 * math.sin(a)
        lo.append((px - 0.22, py - 0.22, zc + 0.5))
        hi.append((px + 0.22, py + 0.22, zc + 4.4))
    v, q = boxes(np.array(lo), np.array(hi))
    mb.add(v, q, mat=TRIM)
    v, q, uv = revolve([(1.7, zc + 0.5), (1.7, zc + 4.4)], 8)
    mb.add(v + np.array([cx, cy, 0], np.float32), q, mat=ROOM, fattr={"lit": 0.0, "tint": 0.3})
    prof = [(3.1, zc + 4.4), (3.1, zc + 4.9), (2.9, zc + 5.0)]
    for k in range(1, 9):
        a = k / 8 * math.pi / 2
        prof.append((2.9 * math.cos(a) + 0.12, zc + 5.0 + 3.3 * math.sin(a)))
    prof += [(0.3, zc + 8.6), (0.34, zc + 9.2), (0.12, zc + 9.5), (0.1, zc + 11.8), (0.02, zc + 12.4)]
    v, q, uv = revolve(prof, 24)
    mb.add(v + np.array([cx, cy, 0], np.float32), q, mat=GOLD, smooth=True)


def far_box(mb, rect, z0, z1, mat, flat_roof=ROOF):
    """A distant building: four walls with metre UVs, and a roof."""
    x0, y0, x1, y1 = rect
    for name, (o, n, width) in _sides(x0, y0, x1, y1).items():
        u = np.cross(Z, np.asarray(n))
        o = np.asarray(o)
        v = np.array([o + Z * z0, o + u * width + Z * z0, o + u * width + Z * z1, o + Z * z1])
        mb.add(v, [[0, 1, 2, 3]], mat=mat, uv=[(0, z0), (width, z0), (width, z1), (0, z1)])
    v, q = box((x0, y0, z1 - 0.3), (x1, y1, z1))
    mb.add(v, q, mat=flat_roof)


# ------------------------------------------------------------------ the walls

STYLES = {
    "limestone": dict(bay=3.3, floor=3.9, corner=2.2, base_h=6.4, attic=1.6, ow=0.46, oh=0.56, sill=0.24, reveal=0.36, lintel=0.14, piers=1, courses=[2], cornice=0.9, awnings=0.3),
    "limestone_grand": dict(bay=3.6, floor=4.0, corner=2.8, base_h=8.2, attic=2.0, ow=0.5, oh=0.6, sill=0.22, reveal=0.42, lintel=0.16, piers=1, courses=[3], cornice=1.1, awnings=0.15),
    "brick": dict(bay=3.0, floor=3.6, corner=1.6, base_h=5.6, attic=1.3, ow=0.44, oh=0.52, sill=0.27, reveal=0.26, lintel=0.2, piers=0, courses=[1], cornice=0.7, awnings=0.5),
    "terracotta": dict(bay=2.9, floor=3.7, corner=1.9, base_h=6.0, attic=1.6, ow=0.5, oh=0.6, sill=0.22, reveal=0.32, lintel=0.12, piers=2, courses=[2], cornice=0.85, awnings=0.4),
    "loft": dict(bay=3.9, floor=4.0, corner=1.5, base_h=6.2, attic=1.2, ow=0.62, oh=0.6, sill=0.22, reveal=0.26, lintel=0.0, piers=1, courses=[], cornice=0.6, awnings=0.4),
    "pairs": dict(bay=1.85, floor=3.5, corner=1.4, base_h=5.4, attic=1.3, ow=0.56, oh=0.55, sill=0.26, reveal=0.24, lintel=0.16, piers=2, courses=[1], cornice=0.75, awnings=0.5),
}
TONES = {
    "cream": ((0.6, 0.54, 0.43), "stone"), "pale": ((0.66, 0.62, 0.54), "stone"), "buff": ((0.5, 0.41, 0.29), "stone"),
    "red": ((0.3, 0.085, 0.055), "brick"), "brown": ((0.2, 0.11, 0.075), "brick"), "tan": ((0.43, 0.32, 0.2), "brick"),
    "terracotta": ((0.46, 0.2, 0.1), "terra"), "grey": ((0.36, 0.35, 0.33), "stone"), "white": ((0.7, 0.68, 0.63), "terra"),
}

# The buildings that stand round the park: (name, kind, rect, heights/tiers, style, tone, sides worth detailing)
def _plan():
    x0, y0, x1, y1 = site.PARK
    e = x1 + site.LOT  # the east building line
    w = x0 - site.LOT
    n = y1 + site.LOT
    out = []
    # east wall, south to north: a mid block, the cream block, the clock tower on its podium, the glass slab
    out.append(("E0", "masonry", (e, -64, e + 44, 40), [(46.0, 0.0), (54.0, 3.5)], "terracotta", "buff", "WSN", "flats"))
    out.append(("E1", "masonry", (e, 52, e + 48, 128), [(52.0, 0.0), (61.0, 4.0), (68.0, 8.0)], "limestone_grand", "cream", "WSN", "office"))
    out.append(("E2", "podium", (e, 139, e + 46, 181), [(36.0, 0.0)], "limestone", "pale", "WSN", "office"))
    out.append(("E3", "slab", (e + 3, 193, e + 46, 237), 104.0, None, None, "WSN", "office"))
    out.append(("E4", "masonry", (e, 250, e + 44, 330), [(58.0, 0.0), (70.0, 4.0)], "limestone", "grey", "WS", "office"))
    # west wall. The south-west corner is left open, as it is where the two
    # avenues cross: that is the way the evening sun comes in to the plaza
    out.append(("W0", "masonry", (w - 42, -4, w, 28), [(24.0, 0.0)], "loft", "tan", "ESN", "flats"))
    out.append(("W1", "masonry", (w - 44, 40, w, 106), [(47.0, 0.0), (54.0, 3.0)], "brick", "red", "ESN", "flats"))
    out.append(("W2", "masonry", (w - 46, 118, w, 190), [(60.0, 0.0), (71.0, 4.0)], "limestone", "cream", "ESN", "office"))
    out.append(("W3", "masonry", (w - 44, 202, w, 262), [(41.0, 0.0)], "terracotta", "white", "ES", "flats"))
    # north wall, west to east
    out.append(("N0", "masonry", (x0 - 4, n, -26, n + 40), [(45.0, 0.0)], "brick", "red", "SEW", "flats"))
    out.append(("N1", "masonry", (-16, n, 32, n + 44), [(60.0, 0.0), (72.0, 3.5), (79.0, 7.0)], "limestone", "cream", "SEW", "office"))
    out.append(("N2", "masonry", (42, n, x1 + 4, n + 40), [(35.0, 0.0)], "pairs", "tan", "SEW", "flats"))
    return out


def _skyline(mb, r, mats):
    """Blocks beyond the walls, row behind row, taller and hazier with distance."""
    x0, y0, x1, y1 = site.PARK
    taken = []
    count = 0
    n_m = len(mats)
    # a street grid north of the park: avenues every ~125 m, streets every ~62 m
    for row in range(26):
        ya = y1 + site.LOT + 52 + row * 62
        for colm in range(-9, 10):
            xa = -62 + colm * 125 + (row % 2) * 6
            # leave the two avenues that pass the park open
            w = r.uniform(70, 104)
            d = r.uniform(34, 46)
            lot_x = xa + r.uniform(0, 8)
            # two or three buildings to a block
            xx = lot_x
            while xx < lot_x + w - 14:
                bw = min(r.uniform(18, 44), lot_x + w - xx)
                dist = math.hypot(xx + site.HERO_CAM[0], ya - site.HERO_CAM[1])
                # low and mid-rise in the foreground of the skyline, towers further off
                tall = r.random() ** 3.2
                h = 16 + 34 * r.random() + tall * min(190.0, 30 + 0.16 * max(dist - 380, 0))
                if dist > 700 and r.random() < 0.02:
                    h += 90
                rect = (xx, ya, xx + bw - 2.5, ya + d)
                m = int(r.integers(0, n_m))
                if h > 70 and r.random() < 0.6:
                    # a setback tower
                    h1 = h * r.uniform(0.45, 0.7)
                    far_box(mb, rect, 0, h1, 20 + m)
                    ins = r.uniform(3, 7)
                    rect2 = (rect[0] + ins, rect[1] + ins, rect[2] - ins, rect[3] - ins)
                    if rect2[2] - rect2[0] > 8:
                        h2 = h * r.uniform(0.8, 0.94)
                        far_box(mb, rect2, h1, h2, 20 + m)
                        ins2 = r.uniform(2, 4)
                        rect3 = (rect2[0] + ins2, rect2[1] + ins2, rect2[2] - ins2, rect2[3] - ins2)
                        if rect3[2] - rect3[0] > 6:
                            far_box(mb, rect3, h2, h, 20 + m)
                else:
                    far_box(mb, rect, 0, h, 20 + m)
                if dist < 700 and h < 75 and r.random() < 0.45:
                    _tank(mb, (rect[0] + rect[2]) / 2 + r.uniform(-3, 3), (rect[1] + rect[3]) / 2 + r.uniform(-3, 3), h, r, s=r.uniform(0.85, 1.15))
                taken.append(rect)
                count += 1
                xx += bw
    return count


def _spire(mb, x, y, h, mat):
    """The far stepped tower: a broad base, a shaft that steps in as it
    climbs, a crown and a mast."""
    steps = [(0.0, 0.07, 46.0), (0.07, 0.2, 36.0), (0.2, 0.68, 27.0), (0.68, 0.76, 23.0), (0.76, 0.82, 19.0), (0.82, 0.86, 14.0), (0.86, 0.9, 9.0)]
    for a, b, half in steps:
        hw = half * h / 380.0
        far_box(mb, (x - hw, y - hw * 0.78, x + hw, y + hw * 0.78), a * h, b * h, mat)
    v, q, uv = revolve([(5.5 * h / 380, 0.9 * h), (4.0 * h / 380, 0.95 * h), (1.6 * h / 380, 0.97 * h), (0.9 * h / 380, 1.12 * h), (0.1, 1.17 * h)], 12)
    mb.add(v + np.array([x, y, 0], np.float32), q, mat=METAL, smooth=True)


def build(col=None):
    col = col or collection("Buildings")
    r = rng(2024)
    frame = mat_flat("window_frame", (0.035, 0.035, 0.04), 0.4)
    frame_b = mat_flat("slab_fin", (0.05, 0.04, 0.03), 0.3, metallic=0.8)
    glass = mat_glass()
    glass_b = mat_glass("slab_glass", tint=(0.42, 0.38, 0.32), mirror=0.34)
    room = mat_room()
    blind = mat_blind()
    roof = mat_flat("roof", (0.09, 0.09, 0.095), 0.85)
    metal = mat_flat("roof_metal", (0.14, 0.14, 0.15), 0.5, metallic=0.4)
    shop = mat_shop()
    gold = mat_gold()
    dial = mat_dial()
    wood = mat_flat("tank_wood", (0.16, 0.1, 0.06), 0.8)
    slate = mat_flat("tower_roof", (0.36, 0.38, 0.36), 0.5)
    awn, nt = material("awning")
    tint = nt.attr("tint")
    acol = nt.ramp(tint, [(0.0, (0.06, 0.09, 0.07)), (0.3, (0.1, 0.1, 0.13)), (0.55, (0.3, 0.27, 0.22)), (0.8, (0.2, 0.05, 0.04)), (0.86, (0.9, 0.2, 0.03)), (1.0, (0.9, 0.2, 0.03))], interp="CONSTANT")
    nt.out_surface(nt.principled(Base_Color=acol, Roughness=0.7))
    walls = {k: mat_masonry(f"wall_{k}", tone, kind) for k, (tone, kind) in TONES.items()}
    trims = {k: mat_masonry(f"trim_{k}", (min(1, tone[0] * 1.12 + 0.05), min(1, tone[1] * 1.12 + 0.05), min(1, tone[2] * 1.1 + 0.05)) if kind != "brick" else (0.5, 0.46, 0.38), "stone", stain=0.6) for k, (tone, kind) in TONES.items()}

    def slots(tone, slab_=False):
        m = [walls[tone], trims[tone], frame_b if slab_ else frame, glass_b if slab_ else glass, room, blind, roof, metal, shop, awn, gold, dial]
        return m

    def fix(mb):
        """Slots above 100 are extras shared by every building: tank wood, tower roof."""
        for k, (f, mat, sm) in enumerate(mb.chunks):
            if np.isscalar(mat) and mat >= 100:
                mb.chunks[k] = (f, {AWNING + 100: 12, ROOF + 100: 13}[mat], sm)

    for name, kind, rect, tiers, style, tone, sides, profile in _plan():
        mb = MB()
        if kind == "slab":
            slab(mb, r, rect, tiers, sides)
            fix(mb)
            add_object(name, mb.build(name, slots("grey", True) + [wood, slate]), col)
            continue
        masonry(mb, r, rect, tiers, STYLES[style], sides, profile)
        if kind == "podium":
            cx, cy = rect[0] + 16.0, (rect[1] + rect[3]) / 2
            clock_tower(mb, r, cx, cy, 12.0, tiers[-1][0], STYLES[style])
        fix(mb)
        add_object(name, mb.build(name, slots(tone) + [wood, slate]), col)

    # ---- the skyline: shader fronts
    far_mats = [
        mat_far("far_cream", (0.5, 0.45, 0.36), seed=1.0), mat_far("far_red", (0.27, 0.1, 0.07), seed=2.0), mat_far("far_tan", (0.4, 0.31, 0.2), seed=3.0),
        mat_far("far_grey", (0.3, 0.3, 0.3), seed=4.0), mat_far("far_pale", (0.6, 0.57, 0.5), seed=5.0), mat_far("far_brown", (0.2, 0.13, 0.09), seed=6.0),
        mat_far("far_glass", (0.1, 0.12, 0.14), glass=(0.06, 0.09, 0.12), bay=1.6, seed=7.0, curtain=True),
        mat_far("far_glass_blue", (0.08, 0.1, 0.13), glass=(0.07, 0.12, 0.17), bay=1.6, seed=8.0, curtain=True),
    ]
    spire_mat = mat_far("far_spire", (0.5, 0.47, 0.4), bay=3.0, floor=3.8, seed=9.0)
    mb = MB()
    count = _skyline(mb, r, far_mats)
    _spire(mb, 46.0, 1120.0, 300.0, 20 + len(far_mats))
    for k, (f, mat, sm) in enumerate(mb.chunks):
        if np.isscalar(mat) and mat >= 100:
            mb.chunks[k] = (f, {AWNING + 100: 12, ROOF + 100: 13}[mat], sm)
    filler = [roof] * (20 - 14)
    add_object("Skyline", mb.build("Skyline", slots("grey") + [wood, slate] + filler + far_mats + [spire_mat]), col)
    print(f"[buildings] {count} skyline blocks")
    return col

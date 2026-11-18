"""The fountain: a granite basin with a sitting-height coping, a carved
pedestal carrying a gadrooned bowl, a central jet, water sheeting over the
bowl's lip, a ring of small arcing jets, foam where water lands, ripples
across the pool, underwater lights and drifting mist.

The water is drawn as a long exposure would hold it: every drop is a strand
along its own ballistic path (p = p0 + v0 t + g t^2 / 2, with a little wind),
so a single frame is soft and neighbouring frames agree.
"""

import math

import bpy
import numpy as np

from .common import MB, TAU, add_object, collection, disc, grade, material, revolve, rng

G = 9.81
WATER_Z = 0.34
LIP_R = 1.935
LIP_Z = 2.555
NOZZLE_Z = 2.82
WIND = np.array([0.95, 0.35, 0.0])  # a light air from the south-west
ARC_N = 20
ARC_R0 = 4.25
ARC_R1 = 2.8
ARC_H = 1.28

BASIN = [
    (5.70, 0.00), (5.70, 0.09), (5.64, 0.13), (5.56, 0.13), (5.54, 0.33), (5.57, 0.355), (5.64, 0.385), (5.66, 0.41),
    (5.675, 0.44), (5.66, 0.475), (5.62, 0.495), (5.56, 0.50), (5.10, 0.50), (5.05, 0.495), (5.02, 0.475), (5.015, 0.44),
    (5.03, 0.41), (5.06, 0.39), (5.06, -0.22),
]
PEDESTAL = [
    (1.05, -0.22), (1.05, 0.40), (1.02, 0.44), (0.90, 0.46), (0.90, 0.52), (0.94, 0.55), (0.96, 0.60), (0.94, 0.65),
    (0.88, 0.68), (0.78, 0.71), (0.72, 0.76), (0.70, 0.82), (0.72, 0.88), (0.78, 0.91), (0.80, 0.94), (0.78, 0.97),
    (0.70, 0.99), (0.60, 1.02), (0.62, 1.10), (0.66, 1.22), (0.67, 1.34), (0.64, 1.48), (0.56, 1.62), (0.47, 1.74),
    (0.42, 1.84), (0.40, 1.92), (0.45, 1.95), (0.47, 1.98), (0.45, 2.01), (0.40, 2.03), (0.42, 2.07), (0.50, 2.12),
    (0.62, 2.16),
]


# ------------------------------------------------------------------ materials


def mat_granite(name, tone=(0.42, 0.41, 0.40), joints=0, joint_axis="angle", wet_line=None):
    """Flecked granite. `joints` cuts it into that many blocks round the axis."""
    mat, nt = material(name)
    g = grade(nt)
    p = nt.coord("Object")
    fleck = nt.noise(p, scale=260.0, detail=2.0, rough=0.7)
    grain = nt.noise(p, scale=38.0, detail=3.0, rough=0.6)
    cloud = nt.noise(p, scale=1.3, detail=3.0, rough=0.55)
    base = nt.rgb(tone)
    col = nt.mixc(base, nt.rgb((tone[0] * 0.45, tone[1] * 0.45, tone[2] * 0.47)), nt.smoothstep(0.56, 0.7, fleck))
    col = nt.mixc(col, nt.rgb((min(1, tone[0] * 1.5), min(1, tone[1] * 1.45), min(1, tone[2] * 1.4))), nt.smoothstep(0.6, 0.78, grain) * 0.5)
    col = nt.mixc(col, nt.rgb((tone[0] * 0.7, tone[1] * 0.7, tone[2] * 0.72)), nt.smoothstep(0.42, 0.7, cloud) * 0.45)
    height = fleck * 0.2 + grain * 0.5
    x, y, z = nt.xyz(p)
    joint = None
    if joints:
        ang = nt.atan2(y, x) / TAU + 0.5
        rr = nt.sqrt(x * x + y * y)
        a = nt.fract(ang * joints)
        edge = nt.min(a, 1.0 - a) * (rr * (TAU / joints))  # metres from the joint
        joint = 1.0 - nt.smoothstep(0.003, 0.009, edge)
        block = nt.white(nt.vec(nt.floor(ang * joints), 3.7, 0.0))
        col = nt.mixc(col, nt.mixc(col, nt.rgb((0.0, 0.0, 0.0)), 0.5), (block - 0.5) * 0.36 + 0.18)
        col = nt.mixc(col, nt.rgb((0.05, 0.05, 0.05)), joint * 0.85)
        height = height - joint * 6.0
    rough = 0.42 + grain * 0.2
    if wet_line is not None:
        # stone below the splash line stays dark and glossy
        wet = 1.0 - nt.smoothstep(wet_line - 0.12, wet_line + 0.1, z + nt.noise(p, scale=7.0, detail=2.0) * 0.12)
        col = nt.mixc(col, nt.mixc(col, nt.rgb((0.02, 0.025, 0.03)), 0.6), wet)
        rough = nt.mix(rough, 0.08, wet)
    # at dusk the day's rain is still on the stone
    rough = nt.mix(rough, rough * 0.45, g["wet"])
    col = nt.mixc(col, nt.mixc(col, nt.rgb((0.0, 0.0, 0.0)), 0.3), g["wet"])
    n = nt.bump(height, strength=0.25, distance=0.004)
    nt.out_surface(nt.principled(Base_Color=col, Roughness=rough, Normal=n, Specular_IOR_Level=0.5))
    return mat


def mat_stone(name):
    """The pedestal and bowl: pale carved stone, streaked where water runs."""
    mat, nt = material(name)
    p = nt.coord("Object")
    x, y, z = nt.xyz(p)
    rr = nt.sqrt(x * x + y * y)
    ang = nt.atan2(y, x)
    fine = nt.noise(p, scale=90.0, detail=3.0, rough=0.65)
    cloud = nt.noise(p, scale=2.4, detail=4.0, rough=0.6)
    # runs of algae and lime down from the lip, and a tide line at the water
    streak = nt.noise(nt.vec(ang * 9.0, z * 0.7, rr * 1.5), scale=3.0, detail=3.0, rough=0.6)
    runs = nt.smoothstep(0.5, 0.72, streak) * nt.smoothstep(1.2, 2.2, z)
    tide = 1.0 - nt.smoothstep(WATER_Z + 0.05, WATER_Z + 0.5, z + cloud * 0.25)
    col = nt.rgb((0.66, 0.62, 0.55))
    col = nt.mixc(col, nt.rgb((0.78, 0.75, 0.69)), nt.smoothstep(0.4, 0.75, cloud))
    col = nt.mixc(col, nt.rgb((0.34, 0.35, 0.3)), runs * 0.55)
    col = nt.mixc(col, nt.rgb((0.2, 0.22, 0.2)), tide * 0.7)
    col = nt.mixc(col, nt.rgb((0.5, 0.47, 0.42)), nt.smoothstep(0.55, 0.8, fine) * 0.3)
    # every hollow is a little darker: the carving reads without hard light
    ao = nt.node("ShaderNodeAmbientOcclusion", samples=4)
    ao.inputs["Distance"].default_value = 0.25
    col = nt.mixc(nt.mixc(col, nt.rgb((0.18, 0.16, 0.14)), 0.7), col, nt.v(ao.outputs["AO"]))
    rough = nt.mix(0.55, 0.12, nt.max(tide, runs * 0.6))
    n = nt.bump(fine * 0.6 + cloud * 0.4, strength=0.3, distance=0.004)
    nt.out_surface(nt.principled(Base_Color=col, Roughness=rough, Normal=n))
    return mat


def mat_bronze(name="bronze"):
    mat, nt = material(name)
    p = nt.coord("Object")
    pat = nt.noise(p, scale=30.0, detail=3.0)
    col = nt.mixc(nt.rgb((0.16, 0.11, 0.06)), nt.rgb((0.12, 0.2, 0.16)), nt.smoothstep(0.45, 0.7, pat))
    nt.out_surface(nt.principled(Base_Color=col, Metallic=0.85, Roughness=0.42 + pat * 0.2))
    return mat


def mat_pool_floor(name="pool_floor"):
    """Small blue-grey tiles under the water."""
    mat, nt = material(name)
    p = nt.coord("Object")
    x, y, z = nt.xyz(p)
    s = 1.0 / 0.11
    cell = nt.vec(nt.floor(x * s), nt.floor(y * s), 0.0)
    tone = nt.white(cell)
    fx = nt.fract(x * s)
    fy = nt.fract(y * s)
    grout = 1.0 - nt.smoothstep(0.03, 0.09, nt.min(nt.min(fx, 1.0 - fx), nt.min(fy, 1.0 - fy)))
    col = nt.ramp(tone, [(0.0, (0.07, 0.11, 0.13)), (0.5, (0.11, 0.17, 0.19)), (1.0, (0.2, 0.27, 0.28))])
    col = nt.mixc(col, nt.rgb((0.05, 0.06, 0.06)), grout)
    nt.out_surface(nt.principled(Base_Color=col, Roughness=0.35))
    return mat


def mat_water(name="water"):
    """The pool: clear water that lets light through to and from the lamps
    under it. Shadow rays pass straight through, so the uplights reach the
    bowl and the spray without caustics."""
    mat, nt = material(name)
    p = nt.coord("Object")
    chop = nt.noise(p, scale=9.0, detail=3.0, rough=0.6)
    fine = nt.noise(p, scale=42.0, detail=2.0, rough=0.5)
    n = nt.bump(chop * 0.7 + fine * 0.3, strength=0.22, distance=0.03)
    glass = nt.shader("ShaderNodeBsdfGlass", Color=(0.86, 0.95, 0.96, 1.0), Roughness=0.015, IOR=1.33, Normal=n)
    clear = nt.shader("ShaderNodeBsdfTransparent", Color=(0.9, 0.97, 0.97, 1.0))
    nt.out_surface(nt.mix_shader(glass, clear, nt.lightpath("Is Shadow Ray")))
    return mat


def _veil_bsdf(nt, tint=(0.93, 0.97, 1.0)):
    """Aerated water in a long exposure: white, soft, and lit through."""
    body = nt.mix_shader(nt.shader("ShaderNodeBsdfDiffuse", Color=(*tint, 1.0)), nt.shader("ShaderNodeBsdfTranslucent", Color=(*tint, 1.0)), 0.55)
    gloss = nt.shader("ShaderNodeBsdfGlossy", Color=(1.0, 1.0, 1.0, 1.0), Roughness=0.18)
    return nt.mix_shader(body, gloss, nt.layer(0.35, "Fresnel") * 0.5 + 0.06)


def mat_streak(name="water_streak"):
    mat, nt = material(name)
    a = nt.attr("a")
    alpha = nt.sat(a * (1.0 - nt.lightpath("Is Shadow Ray") * 0.6))
    nt.out_surface(nt.mix_shader(nt.shader("ShaderNodeBsdfTransparent"), _veil_bsdf(nt), alpha))
    return mat


def mat_veil(name="water_veil"):
    """The sheet off the bowl's lip: glassy where it leaves the stone, torn
    into strands and then drops as it falls."""
    mat, nt = material(name)
    uv = nt.coord("UV")
    u, v, _ = nt.xyz(uv)  # u round the bowl (0..1), v the fall (0 at the lip, 1 at the pool)
    strands = nt.noise(nt.vec(u * 260.0, v * 1.3, 0.0), scale=1.0, detail=4.0, rough=0.66, dims="2D")
    ropes = nt.noise(nt.vec(u * 70.0, v * 0.4, 9.0), scale=1.0, detail=2.0, dims="2D")
    broad = nt.noise(nt.vec(u * 17.0, v * 0.25, 3.0), scale=1.0, detail=2.0, dims="2D")
    tear = nt.smoothstep(0.0, 0.8, v)
    # the lip sheds water in sheets with gaps between; each sheet tears into
    # ropes and the ropes into strings before they reach the pool
    body = strands * 0.42 + ropes * 0.3 + broad * 0.28
    alpha = nt.smoothstep(nt.mix(0.4, 0.54, tear), nt.mix(0.5, 0.6, tear), body)
    alpha = alpha * nt.mix(0.62, 0.42, tear)
    alpha = alpha * (1.0 - nt.lightpath("Is Shadow Ray") * 0.55)
    glassy = nt.shader("ShaderNodeBsdfGlass", Color=(0.92, 0.97, 0.98, 1.0), Roughness=0.08, IOR=1.33)
    body = nt.mix_shader(glassy, _veil_bsdf(nt), nt.smoothstep(0.0, 0.16, v))
    nt.out_surface(nt.mix_shader(nt.shader("ShaderNodeBsdfTransparent"), body, alpha))
    return mat


def mat_foam(name="foam"):
    mat, nt = material(name)
    p = nt.coord("Object")
    a = nt.attr("a")
    cells = nt.voronoi(p, scale=16.0, out="Distance")
    lace = nt.noise(p, scale=5.5, detail=4.0, rough=0.7)
    alpha = nt.sat(nt.smoothstep(0.36, 0.56, lace + a * 0.5 - 0.25) * a * 1.9)
    n = nt.bump(cells, strength=0.6, distance=0.02)
    foam = nt.principled(Base_Color=(0.97, 0.98, 1.0, 1.0), Roughness=0.6, Subsurface_Weight=0.6, Subsurface_Radius=(0.06, 0.07, 0.08), Subsurface_Scale=0.4, Normal=n)
    nt.out_surface(nt.mix_shader(nt.shader("ShaderNodeBsdfTransparent"), foam, alpha * (1.0 - nt.lightpath("Is Shadow Ray") * 0.7)))
    return mat


def mat_mist(name="mist"):
    """Spray hanging in the air: over the crown, round the foot of the falls,
    low over the pool, all leaning downwind."""
    mat, nt = material(name)
    p = nt.geom("Position")
    x, y, z = nt.xyz(p)
    lean = nt.max(z - 1.0, 0.0)
    xd = x - lean * lean * 0.035 * float(WIND[0]) - lean * 0.12 * float(WIND[0])
    yd = y - lean * lean * 0.035 * float(WIND[1]) - lean * 0.12 * float(WIND[1])
    r2 = xd * xd + yd * yd
    rr = nt.sqrt(r2)
    wide = 0.75 + nt.max(z - 2.8, 0.0) * 0.34
    crown = nt.exp(-(r2 / (wide * wide))) * nt.smoothstep(2.7, 4.2, z) * (1.0 - nt.smoothstep(6.2, 9.0, z))
    ring = nt.exp(-nt.pow(nt.abs(rr - 2.25) / 0.7, 2.0)) * nt.exp(-nt.max(z - WATER_Z, 0.0) / 0.9)
    low = nt.exp(-(r2 / 30.0)) * nt.exp(-nt.max(z - WATER_Z, 0.0) / 0.55) * 0.35
    drift = nt.exp(-(nt.pow(x - 5.5, 2.0) + nt.pow(y - 2.0, 2.0)) / 34.0) * nt.exp(-nt.pow((z - 3.4) / 2.6, 2.0)) * 0.12
    wisp = nt.noise(nt.vec(x, y, z * 0.7), scale=0.75, detail=4.0, rough=0.62)
    dens = (crown * 0.5 + ring * 0.85 + low + drift) * nt.smoothstep(0.28, 0.78, wisp) * nt.attr("mist_gain", kind="OBJECT")
    vol = nt.shader("ShaderNodeVolumeScatter", Color=(0.96, 0.98, 1.0, 1.0), Density=dens, Anisotropy=0.45)
    nt.out_surface(volume=vol)
    return mat


def mat_lens(name="fountain_lens"):
    mat, nt = material(name)
    g = grade(nt)
    em = nt.shader("ShaderNodeEmission", Color=(1.0, 0.86, 0.66, 1.0), Strength=g["lamps"] * 60.0)
    nt.out_surface(nt.add_shader(em, nt.principled(Base_Color=(0.05, 0.05, 0.05, 1.0), Roughness=0.2)))
    return mat


# ------------------------------------------------------------------- geometry


def _lathe_mod(profile, segs, fn):
    """A lathe whose radius and height are nudged per vertex by fn(theta, k)."""
    v, q, uv = revolve(profile, segs)
    k = len(profile)
    theta = np.repeat(np.arange(segs) * TAU / segs, k)
    idx = np.tile(np.arange(k), segs)
    dr, dz = fn(theta, idx)
    r = np.hypot(v[:, 0], v[:, 1]) + 1e-9
    v[:, 0] *= (r + dr) / r
    v[:, 1] *= (r + dr) / r
    v[:, 2] += dz
    return v, q, uv


def _bowl_profiles():
    under = []
    for i in range(22):
        s = i / 21
        under.append((0.62 + (1.86 - 0.62) * s, 2.16 + 0.4 * s ** 1.7))
    rim = [(1.875, 2.545), (1.9, 2.535), (1.925, 2.54), (1.935, 2.56), (1.932, 2.64), (1.92, 2.665), (1.89, 2.676), (1.84, 2.672), (1.78, 2.648)]
    inner = []
    for i in range(1, 14):
        r = 1.74 * (1 - i / 13)
        inner.append((max(r, 0.17), 2.37 + 0.27 * (r / 1.74) ** 2.2))
    return under, rim, inner


def _streaks(mb, p0, v0, t0, dur, t_end, radius, alpha, drag, steps=7, sides=3, mat=0):
    """Strands: each from time t0 to t0 + dur along its path, cut where it lands."""
    n = len(p0)
    t1 = np.minimum(t0 + dur, t_end)
    keep = t1 - t0 > 0.012
    p0, v0, t0, t1, radius, alpha, drag = p0[keep], v0[keep], t0[keep], t1[keep], radius[keep], alpha[keep], drag[keep]
    n = len(p0)
    if n == 0:
        return
    s = np.linspace(0, 1, steps)[None, :]
    t = t0[:, None] + (t1 - t0)[:, None] * s  # (n, steps)
    acc = np.array([0.0, 0.0, -G])[None, None, :] + WIND[None, None, :] * drag[:, None, None]
    P = p0[:, None, :] + v0[:, None, :] * t[..., None] + 0.5 * acc * (t ** 2)[..., None]
    Vv = v0[:, None, :] + acc * t[..., None]
    T = Vv / (np.linalg.norm(Vv, axis=-1, keepdims=True) + 1e-9)
    ref = np.array([0.31, 0.83, 0.46])
    N = np.cross(T, ref[None, None, :])
    N /= np.linalg.norm(N, axis=-1, keepdims=True) + 1e-9
    B = np.cross(T, N)
    # a strand is thin at its tail and its head
    taper = np.sin(np.pi * np.clip(s * 0.86 + 0.07, 0, 1)) ** 0.7
    R = radius[:, None] * taper
    ang = np.arange(sides) * TAU / sides
    ring = (np.cos(ang)[None, None, :, None] * N[:, :, None, :] + np.sin(ang)[None, None, :, None] * B[:, :, None, :]) * R[:, :, None, None]
    verts = (P[:, :, None, :] + ring).reshape(-1, 3)
    i = np.arange(steps - 1)[:, None]
    j = np.arange(sides)[None, :]
    j2 = (j + 1) % sides
    q = np.stack([i * sides + j, i * sides + j2, (i + 1) * sides + j2, (i + 1) * sides + j], axis=-1).reshape(-1, 4)
    quads = (q[None] + (np.arange(n) * steps * sides)[:, None, None]).reshape(-1, 4)
    mb.add(verts, quads, mat=mat, smooth=True, vattr={"a": np.repeat(alpha, steps * sides)})


def _water_particles(seed=3):
    """Every strand of the fountain's water: jet, fall-back, curtain drops,
    arcing jets and splash."""
    r = rng(seed)
    mb = MB()

    # -- the central jet: up, spreading at the crown, and back into the bowl
    n = 11000
    ang = r.uniform(0, TAU, n)
    spread = np.abs(r.normal(0, 0.27, n)) + 0.015
    rad0 = r.uniform(0, 0.055, n) ** 0.5 * 0.2
    p0 = np.stack([rad0 * np.cos(ang), rad0 * np.sin(ang), np.full(n, NOZZLE_Z)], axis=-1)
    vz = 7.95 * (1 + r.normal(0, 0.035, n))
    v0 = np.stack([spread * np.cos(ang), spread * np.sin(ang), vz], axis=-1)
    drag = r.uniform(0, 1, n) ** 3 * 1.6
    # until it is back in the bowl
    t_end = (vz + np.sqrt(vz * vz + 2 * G * (NOZZLE_Z - 2.67))) / G
    t0 = r.uniform(0, 1, n) * t_end
    rising = t0 < vz / G
    # rising water is still rope; past the crown it has broken into beads
    dur = np.where(rising, r.uniform(0.05, 0.11, n), r.uniform(0.02, 0.055, n))
    radius = np.where(rising, r.uniform(0.004, 0.011, n), r.uniform(0.006, 0.014, n))
    _streaks(mb, p0, v0, t0, dur, t_end, radius, r.uniform(0.25, 0.65, n), drag)
    # the solid core of the jet, where it is still one rope of water
    n = 1500
    ang = r.uniform(0, TAU, n)
    rc = r.uniform(0, 1, n) ** 0.5 * 0.075
    p0 = np.stack([rc * np.cos(ang), rc * np.sin(ang), np.full(n, NOZZLE_Z - 0.03)], axis=-1)
    vz = 7.95 * (1 + r.normal(0, 0.015, n))
    v0 = np.stack([0.07 * np.cos(ang), 0.07 * np.sin(ang), vz], axis=-1)
    t0 = r.uniform(0, 0.5, n) ** 1.3
    _streaks(mb, p0, v0, t0, r.uniform(0.12, 0.2, n), np.full(n, 0.78), r.uniform(0.016, 0.034, n), r.uniform(0.45, 0.8, n), np.zeros(n))

    # -- drops shed by the sheet off the lip, most of them low down
    n = 5200
    ang = r.uniform(0, TAU, n)
    vr = 0.45 + 0.16 * np.sin(ang * 7 + 1.3) * np.sin(ang * 3) + r.normal(0, 0.07, n)
    p0 = np.stack([LIP_R * np.cos(ang), LIP_R * np.sin(ang), np.full(n, LIP_Z)], axis=-1)
    v0 = np.stack([vr * np.cos(ang), vr * np.sin(ang), r.normal(0, 0.05, n)], axis=-1)
    t_end = np.sqrt(2 * (LIP_Z - WATER_Z) / G) * np.ones(n)
    t0 = (r.uniform(0, 1, n) ** 0.55) * t_end
    _streaks(mb, p0, v0, t0, r.uniform(0.05, 0.1, n), t_end, r.uniform(0.006, 0.013, n), r.uniform(0.3, 0.75, n), r.uniform(0, 1, n) ** 4 * 1.2)

    # -- the ring of small jets, each a true parabola into the pool
    vz_a = math.sqrt(2 * G * ARC_H)
    t_arc = 2 * vz_a / G + 0.012
    vr_a = (ARC_R0 - ARC_R1) / t_arc
    for k in range(ARC_N):
        # no two nozzles are set quite alike
        a = (k + 0.5) / ARC_N * TAU + r.normal(0, 0.012)
        push = 1 + r.normal(0, 0.035)
        n = 420
        c, s = math.cos(a), math.sin(a)
        j = r.normal(0, 1, (n, 3))
        p0 = np.stack([ARC_R0 * c + j[:, 0] * 0.004, ARC_R0 * s + j[:, 1] * 0.004, np.full(n, WATER_Z + 0.08)], axis=-1)
        t0 = r.uniform(0, 1, n) ** 0.8 * t_arc
        late = np.clip((t0 / t_arc - 0.42) / 0.58, 0, 1)
        # a thin rope on the way up, which wavers, then breaks into drops past
        # the apex: the later a strand is seen, the shorter and the more astray
        loose = 0.006 + 0.11 * late ** 1.6
        v0 = np.stack([-vr_a * c * push + j[:, 0] * loose, -vr_a * s * push + j[:, 1] * loose, vz_a * push * (1 + j[:, 2] * (0.004 + 0.02 * late))], axis=-1)
        dur = np.where(late > 0.15, r.uniform(0.018, 0.05, n), r.uniform(0.06, 0.12, n))
        rad = np.where(late > 0.15, r.uniform(0.004, 0.0085, n), r.uniform(0.0028, 0.0055, n))
        _streaks(mb, p0, v0, t0, dur, np.full(n, t_arc * 1.06), rad, r.uniform(0.35, 0.8, n), r.uniform(0, 1, n) ** 6 * 0.1)

    # -- splash: short hops where the sheet and the arcs land, and in the bowl
    def splash(cx, cy, n, spread, up, z=WATER_Z):
        ang = r.uniform(0, TAU, n)
        vr = np.abs(r.normal(0, spread, n))
        vz = np.abs(r.normal(up * 0.6, up * 0.5, n)) + 0.3
        p0 = np.stack([cx + r.normal(0, 0.09, n), cy + r.normal(0, 0.09, n), np.full(n, z)], axis=-1)
        v0 = np.stack([vr * np.cos(ang), vr * np.sin(ang), vz], axis=-1)
        t_end = 2 * vz / G
        t0 = r.uniform(0, 1, n) * t_end
        _streaks(mb, p0, v0, t0, r.uniform(0.03, 0.07, n), t_end, r.uniform(0.004, 0.009, n), r.uniform(0.3, 0.7, n), np.zeros(n), steps=5)

    n = 5200
    ang = r.uniform(0, TAU, n)
    land = LIP_R + 0.45 * math.sqrt(2 * (LIP_Z - WATER_Z) / G) + r.normal(0, 0.1, n)
    vr = r.normal(0.15, 0.7, n)
    vz = np.abs(r.normal(0.9, 0.8, n)) + 0.25
    p0 = np.stack([land * np.cos(ang), land * np.sin(ang), np.full(n, WATER_Z)], axis=-1)
    v0 = np.stack([vr * np.cos(ang), vr * np.sin(ang), vz], axis=-1)
    t_end = 2 * vz / G
    _streaks(mb, p0, v0, r.uniform(0, 1, n) * t_end, r.uniform(0.03, 0.07, n), t_end, r.uniform(0.004, 0.009, n), r.uniform(0.3, 0.7, n), np.zeros(n), steps=5)
    for k in range(ARC_N):
        a = (k + 0.5) / ARC_N * TAU
        splash(ARC_R1 * math.cos(a), ARC_R1 * math.sin(a), 110, 0.5, 1.1)
    splash(0.0, 0.0, 900, 0.9, 1.0, z=2.668)
    return mb


def _pool_surface(seed=5):
    """The pool's surface with its rings: out from where the sheet lands,
    round each arc's landing, and a light chop over all of it."""
    r = rng(seed)
    rings = 150
    segs = 384
    radii = np.linspace(1.02, 5.07, rings + 1)
    ang = np.arange(segs) * TAU / segs
    R, A = np.meshgrid(radii, ang, indexing="ij")
    X = R * np.cos(A)
    Y = R * np.sin(A)
    land = LIP_R + 0.31
    out = np.maximum(R - land, 0)
    h = 0.016 * np.sin(out * 17.0 + 1.0) * np.exp(-out / 1.5) * (R > land)
    h += 0.01 * np.sin((land - R) * 22.0) * np.exp(-np.maximum(land - R, 0) / 0.5) * (R <= land)
    for k in range(ARC_N):
        a = (k + 0.5) / ARC_N * TAU
        d = np.hypot(X - ARC_R1 * math.cos(a), Y - ARC_R1 * math.sin(a))
        h += 0.011 * np.cos(d * 21.0 - 0.6 + k) * np.exp(-d / 0.55)
    # turbulence under the falls
    h += 0.012 * np.exp(-((R - land) / 0.3) ** 2) * np.sin(A * 61 + R * 40) * np.cos(A * 23 - R * 9)
    h += 0.003 * np.sin(X * 6.3 + Y * 2.1 + 0.4) + 0.003 * np.sin(Y * 7.7 - X * 1.4)
    h *= np.clip((5.07 - R) / 0.15, 0, 1) ** 0.5 * 0.9 + 0.1
    v = np.stack([X, Y, WATER_Z + h], axis=-1).reshape(-1, 3)
    i = np.arange(rings)[:, None]
    j = np.arange(segs)[None, :]
    j2 = (j + 1) % segs
    q = np.stack([i * segs + j, (i + 1) * segs + j, (i + 1) * segs + j2, i * segs + j2], axis=-1).reshape(-1, 4)
    del r
    return v, q


def _curtain(seed=9):
    """The sheet that leaves the lip all the way round and falls to the pool."""
    r = rng(seed)
    segs = 720
    steps = 30
    # one more column than segments: the seam has its own vertices, so the
    # UVs run 0..1 without wrapping backwards
    cols = segs + 1
    ang = np.arange(cols) * TAU / segs
    # the lip is not level to the millimetre: the sheet is fuller in places
    vr = 0.45 + 0.16 * np.sin(ang * 7 + 1.3) * np.sin(ang * 3) + 0.05 * np.sin(ang * 31 + r.uniform(0, 6))
    T = math.sqrt(2 * (LIP_Z - WATER_Z) / G)
    t = (np.linspace(0, 1, steps) ** 0.8) * T
    wob = 0.012 * np.sin(ang[None, :] * 47 + t[:, None] * 31) * (t[:, None] / T)
    rad = LIP_R + vr[None, :] * t[:, None] + wob
    z = LIP_Z - 0.5 * G * t[:, None] ** 2 + 0 * ang[None, :]
    drift = 0.5 * 0.22 * t[:, None] ** 2
    v = np.stack([rad * np.cos(ang)[None, :] + drift * WIND[0], rad * np.sin(ang)[None, :] + drift * WIND[1], z], axis=-1).reshape(-1, 3)
    uv = np.stack([np.broadcast_to((ang / TAU)[None, :], (steps, cols)), np.broadcast_to((t / T)[:, None], (steps, cols))], axis=-1).reshape(-1, 2)
    i = np.arange(steps - 1)[:, None]
    j = np.arange(segs)[None, :]
    q = np.stack([i * cols + j, i * cols + j + 1, (i + 1) * cols + j + 1, (i + 1) * cols + j], axis=-1).reshape(-1, 4)
    return v, q, uv


def build(col=None):
    col = col or collection("Fountain")
    granite = mat_granite("fountain_granite", (0.36, 0.35, 0.345), joints=32, wet_line=None)
    stone = mat_stone("fountain_stone")
    bronze = mat_bronze()
    floor = mat_pool_floor()
    water = mat_water()
    veil = mat_veil()
    streak = mat_streak()
    foam = mat_foam()
    lens = mat_lens()

    # ---- stone
    mb = MB()
    v, q, uv = revolve(BASIN, 256)
    mb.add(v, q, mat=0, smooth=True, uv=uv)
    v, q, uv = disc(0.0, 5.06, 96, rings=8, z=-0.22)
    mb.add(v, q, mat=1, smooth=True, uv=uv)
    add_object("Basin", mb.build("Basin", [granite, floor], smooth_angle=math.radians(50)), col)

    mb = MB()

    def flute(theta, k):
        z = np.array([PEDESTAL[i][1] for i in range(len(PEDESTAL))])[k]
        on = np.clip((z - 1.02) / 0.1, 0, 1) * np.clip((1.92 - z) / 0.12, 0, 1)
        return 0.028 * on * (0.5 + 0.5 * np.cos(theta * 16)) ** 0.7, np.zeros_like(theta)

    v, q, uv = _lathe_mod(PEDESTAL, 192, flute)
    mb.add(v, q, mat=0, smooth=True, uv=uv)
    under, rim, inner = _bowl_profiles()
    nu = len(under)

    def gadroon(theta, k):
        s = np.clip(k / (nu - 1), 0, 1)
        on = np.sin(np.pi * np.clip(s, 0, 1) ** 0.8) * (k < nu)
        lobe = (0.5 + 0.5 * np.cos(theta * 24)) ** 0.6
        return 0.05 * on * lobe, -0.035 * on * lobe

    v, q, uv = _lathe_mod(under + rim + inner, 384, gadroon)
    mb.add(v, q, mat=0, smooth=True, uv=uv)
    # the nozzle: a short bronze stem with a collar
    v, q, uv = revolve([(0.17, 2.36), (0.16, 2.6), (0.21, 2.64), (0.21, 2.7), (0.13, 2.75), (0.1, NOZZLE_Z), (0.06, NOZZLE_Z), (0.05, 2.7)], 32)
    mb.add(v, q, mat=1, smooth=True, uv=uv)
    add_object("Pedestal", mb.build("Pedestal", [stone, bronze], smooth_angle=math.radians(40)), col)

    # ---- fittings: the arc nozzles and the lamps under the water
    mb = MB()
    lights = []
    for k in range(ARC_N):
        a = (k + 0.5) / ARC_N * TAU
        v, q, uv = revolve([(0.05, -0.22), (0.05, WATER_Z + 0.02), (0.035, WATER_Z + 0.09), (0.02, WATER_Z + 0.09)], 10)
        v[:, 0] += ARC_R0 * math.cos(a)
        v[:, 1] += ARC_R0 * math.sin(a)
        mb.add(v, q, mat=0, smooth=True)
    spots = []
    for k in range(12):
        a = k / 12 * TAU
        spots.append((3.25 * math.cos(a), 3.25 * math.sin(a), 0.02, -0.5, 520.0, 1.15))
    for k in range(6):
        a = (k + 0.5) / 6 * TAU
        spots.append((1.5 * math.cos(a), 1.5 * math.sin(a), 0.02, 0.18, 300.0, 1.3))
    for k in range(ARC_N):
        a = (k + 0.5) / ARC_N * TAU
        spots.append(((ARC_R0 + 0.16) * math.cos(a), (ARC_R0 + 0.16) * math.sin(a), 0.2, -0.3, 70.0, 0.95))
    for x, y, z, lean, power, cone in spots:
        v, q, uv = revolve([(0.11, -0.22), (0.11, z - 0.02), (0.09, z)], 14)
        v[:, 0] += x
        v[:, 1] += y
        mb.add(v, q, mat=0, smooth=True)
        v, q, uv = disc(0.0, 0.09, 14, z=z + 0.002)
        v[:, 0] += x
        v[:, 1] += y
        mb.add(v, q, mat=1)
        lights.append((x, y, z + 0.05, lean, power, cone))
    add_object("Fittings", mb.build("Fittings", [bronze, lens]), col)
    for k, (x, y, z, lean, power, cone) in enumerate(lights):
        ld = bpy.data.lights.new(f"FountainSpot{k}", "SPOT")
        ld.energy = power
        ld.color = (1.0, 0.8, 0.56)
        ld.spot_size = cone
        ld.spot_blend = 0.6
        ld.shadow_soft_size = 0.06
        ob = add_object(f"FountainSpot{k}", ld, col, loc=(x, y, z))
        # lean in toward (or out from) the axis; a spot shines down its -Z
        rr = math.hypot(x, y)
        ob.rotation_euler = (math.pi + lean * (-y / rr), lean * (x / rr), 0.0)
        ob["film_lamp"] = power
    # three more in the bowl, close round the jet
    for k in range(4):
        a = k / 4 * TAU + 0.4
        ld = bpy.data.lights.new(f"BowlSpot{k}", "SPOT")
        ld.energy = 420.0
        ld.color = (1.0, 0.84, 0.62)
        ld.spot_size = 0.85
        ld.spot_blend = 0.7
        ld.shadow_soft_size = 0.04
        ob = add_object(f"BowlSpot{k}", ld, col, loc=(0.55 * math.cos(a), 0.55 * math.sin(a), 2.5))
        ob.rotation_euler = (math.pi + 0.16 * math.sin(a), -0.16 * math.cos(a), 0.0)
        ob["film_lamp"] = 420.0

    # ---- water
    v, q = _pool_surface()
    mb = MB()
    mb.add(v, q, smooth=True)
    v, q, uv = disc(0.17, 1.9, 96, rings=10, z=2.668)
    v[:, 2] += 0.004 * np.sin(np.hypot(v[:, 0], v[:, 1]) * 30.0)
    mb.add(v, q, smooth=True)
    pool = add_object("Pool", mb.build("Pool", [water]), col)
    pool.visible_shadow = False

    v, q, uv = _curtain()
    mb = MB()
    mb.add(v, q, smooth=True, uv=uv)
    add_object("Curtain", mb.build("Curtain", [veil]), col)

    add_object("Spray", _water_particles().build("Spray", [streak]), col)

    # foam: a band where the sheet lands, and a patch under each arc
    mb = MB()
    land = LIP_R + 0.31
    v, q, uv = disc(land - 0.42, land + 1.15, 256, rings=12, z=WATER_Z + 0.012)
    rr = np.hypot(v[:, 0], v[:, 1])
    a = np.exp(-((rr - land) / 0.33) ** 2) + 0.35 * np.exp(-np.maximum(rr - land, 0) / 0.5)
    v[:, 2] += 0.03 * np.exp(-((rr - land) / 0.25) ** 2)
    mb.add(v, q, smooth=True, vattr={"a": np.clip(a, 0, 1)})
    for k in range(ARC_N):
        ang = (k + 0.5) / ARC_N * TAU
        v, q, uv = disc(0.0, 0.62, 20, rings=4, z=WATER_Z + 0.014)
        rr = np.hypot(v[:, 0], v[:, 1])
        v[:, 2] += 0.02 * np.exp(-(rr / 0.2) ** 2)
        v[:, 0] += ARC_R1 * math.cos(ang)
        v[:, 1] += ARC_R1 * math.sin(ang)
        mb.add(v, q, smooth=True, vattr={"a": np.exp(-(rr / 0.3) ** 2)})
    v, q, uv = disc(0.2, 0.95, 48, rings=5, z=2.675)
    rr = np.hypot(v[:, 0], v[:, 1])
    mb.add(v, q, smooth=True, vattr={"a": np.exp(-((rr - 0.42) / 0.3) ** 2)})
    foam_ob = add_object("Foam", mb.build("Foam", [foam]), col)
    foam_ob.visible_shadow = False

    # ---- mist: one volume round the whole fountain
    mb = MB()
    v, q, uv = revolve([(0.01, 0.3), (12.5, 0.3), (12.5, 10.5), (0.01, 10.5)], 24)
    v[:, 0] += 2.5
    v[:, 1] += 0.9
    mb.add(v, q)
    mist = add_object("Mist", mb.build("Mist", [mat_mist()]), col)
    mist["mist_gain"] = 0.9
    mist.visible_shadow = False
    return col

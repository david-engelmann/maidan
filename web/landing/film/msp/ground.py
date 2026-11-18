"""The ground: the plaza's ring-and-radial paving, the paths with their
kerbs, the lawn, the planting under the trees, and the streets round the
park."""

import math

import numpy as np

from . import site
from .common import MB, TAU, add_object, boxes, collection, disc, grade, material, revolve

PLAZA_Z = 0.0
BED_Z = -0.03
ROAD_Z = -0.15
WALK_Z = -0.01


# ------------------------------------------------------------------ materials


def _wet(nt, g, p, rough, col, puddle_scale=0.16):
    """After rain: darker stone, and water lying where the paving dips."""
    lie = nt.noise(p, scale=puddle_scale, detail=3.0, rough=0.55)
    lie2 = nt.noise(p, scale=puddle_scale * 4.3, detail=2.0)
    puddle = nt.smoothstep(0.5, 0.64, lie * 0.75 + lie2 * 0.25)
    damp = nt.mix(rough * 0.5, 0.035, puddle)
    rough_out = nt.mix(rough, damp, g["wet"])
    col_out = nt.mixc(col, nt.mixc(col, nt.rgb((0.0, 0.0, 0.0)), nt.mix(0.32, 0.55, puddle)), g["wet"])
    return rough_out, col_out, puddle * g["wet"]


def mat_paving(name="plaza_paving"):
    """Granite laid in rings round the basin, with a dark collar, a dark ring
    and eight dark spokes. Every stone has its own tone and its own slight
    tilt, which is what breaks a reflection the way real paving does."""
    mat, nt = material(name)
    g = grade(nt)
    p = nt.coord("Object")
    x, y, z = nt.xyz(p)
    r = nt.sqrt(x * x + y * y)
    ang = nt.atan2(y, x) / TAU + 0.5
    course = 0.62
    ri = nt.floor(r / course)
    rf = nt.fract(r / course)
    count = nt.max(nt.floor((ri + 0.5) * (course * TAU / 0.98)), 8.0)
    aj = ang * count + ri * 0.37
    ai = nt.floor(aj)
    af = nt.fract(aj)
    edge_r = nt.min(rf, 1.0 - rf) * course
    edge_a = nt.min(af, 1.0 - af) * (r * TAU / count)
    edge = nt.min(edge_r, edge_a)
    joint = 1.0 - nt.smoothstep(0.003, 0.011, edge)
    cell = nt.vec(ri, ai, 0.0)
    tone = nt.white(cell)
    tone2 = nt.white(nt.vec(ai, ri, 7.0))
    # which stone: the collar, the ring, the spokes, the field
    collar = nt.lt(r, 7.52)
    ring = nt.gt(r, 13.02) * nt.lt(r, 14.26)
    sp = nt.fract(ang * 8.0 + 0.5)
    spoke = nt.lt(nt.abs(sp - 0.5) * (r * TAU / 8.0), 0.62) * nt.gt(r, 7.52)
    dark = nt.max(nt.max(collar, ring), spoke)
    field = nt.ramp(tone, [(0.0, (0.3, 0.285, 0.265)), (0.45, (0.4, 0.385, 0.36)), (0.8, (0.47, 0.45, 0.42)), (1.0, (0.5, 0.44, 0.4))])
    slate = nt.ramp(tone, [(0.0, (0.085, 0.088, 0.095)), (0.6, (0.13, 0.132, 0.14)), (1.0, (0.18, 0.176, 0.176))])
    col = nt.mixc(field, slate, dark)
    fleck = nt.noise(p, scale=180.0, detail=2.0, rough=0.7)
    col = nt.mixc(col, nt.mixc(col, nt.rgb((0.0, 0.0, 0.0)), 0.5), nt.smoothstep(0.55, 0.72, fleck) * 0.45)
    # wear: a paler track where people walk round the basin and down the spokes
    stain = nt.noise(p, scale=0.5, detail=4.0, rough=0.6)
    col = nt.mixc(col, nt.mixc(col, nt.rgb((0.0, 0.0, 0.0)), 0.35), nt.smoothstep(0.5, 0.75, stain) * 0.5)
    col = nt.mixc(col, nt.rgb((0.035, 0.034, 0.032)), joint * 0.9)
    rough = 0.5 + tone2 * 0.16 + joint * 0.25
    rough, col, puddle = _wet(nt, g, p, rough, col)
    height = fleck * 0.15 - joint * 5.0
    n = nt.bump(height * (1.0 - puddle), strength=0.3, distance=0.004)
    # the lie of each slab
    tilt = nt.vec((tone - 0.5) * 0.022, (tone2 - 0.5) * 0.022, 0.0)
    n = nt.vmath("NORMALIZE", nt.vmath("ADD", n, nt.vmath("SCALE", tilt, scale=1.0 - puddle * 0.85)))
    nt.out_surface(nt.principled(Base_Color=col, Roughness=rough, Normal=n))
    return mat


def mat_pavers(name="path_pavers"):
    """The park's paths: small hexagonal asphalt pavers, worn and patched."""
    mat, nt = material(name)
    g = grade(nt)
    p = nt.coord("Object")
    cells = nt.node("ShaderNodeTexVoronoi", feature="DISTANCE_TO_EDGE", voronoi_dimensions="2D")
    nt.put(cells.inputs["Vector"], p)
    cells.inputs["Scale"].default_value = 4.6
    cells.inputs["Randomness"].default_value = 0.28
    edge = nt.v(cells.outputs["Distance"])
    idn = nt.voronoi(p, scale=4.6, out="Color", rand=0.28, dims="2D")
    tone = nt.xyz(idn)[0]
    joint = 1.0 - nt.smoothstep(0.01, 0.05, edge)
    patch = nt.noise(p, scale=0.35, detail=4.0, rough=0.6)
    col = nt.ramp(tone, [(0.0, (0.15, 0.143, 0.135)), (0.6, (0.21, 0.2, 0.185)), (1.0, (0.27, 0.25, 0.23))])
    col = nt.mixc(col, nt.rgb((0.3, 0.28, 0.25)), nt.smoothstep(0.55, 0.75, patch) * 0.5)
    col = nt.mixc(col, nt.rgb((0.05, 0.048, 0.045)), joint * 0.8)
    rough, col, puddle = _wet(nt, g, p, 0.62 + tone * 0.15, col, puddle_scale=0.22)
    n = nt.bump(joint * -1.0 + nt.noise(p, scale=140.0) * 0.1, strength=0.35, distance=0.004)
    nt.out_surface(nt.principled(Base_Color=col, Roughness=rough, Normal=n))
    return mat


def mat_lawn(name="lawn"):
    """Grass no mower has evened out: lush and thin places, clover, a little
    wear at the edges, and the long light catching its blades."""
    mat, nt = material(name)
    p = nt.coord("Object")
    broad = nt.noise(p, scale=0.07, detail=4.0, rough=0.6)
    mid = nt.noise(p, scale=0.45, detail=4.0, rough=0.65)
    fine = nt.noise(p, scale=14.0, detail=3.0, rough=0.7)
    blades = nt.noise(p, scale=95.0, detail=2.0, rough=0.6)
    col = nt.ramp(broad * 0.55 + mid * 0.45, [(0.25, (0.035, 0.085, 0.018)), (0.45, (0.06, 0.14, 0.026)), (0.6, (0.1, 0.19, 0.035)), (0.78, (0.17, 0.24, 0.05))])
    col = nt.mixc(col, nt.rgb((0.2, 0.2, 0.07)), nt.smoothstep(0.62, 0.82, fine) * 0.35)
    col = nt.mixc(col, nt.mixc(col, nt.rgb((0.0, 0.0, 0.0)), 0.5), nt.smoothstep(0.35, 0.65, blades) * 0.6)
    n = nt.bump(blades * 0.6 + fine * 0.4, strength=0.9, distance=0.03)
    base = nt.principled(Base_Color=col, Roughness=0.65, Normal=n, Sheen_Weight=0.5, Sheen_Roughness=0.45, Sheen_Tint=(0.75, 0.9, 0.4, 1.0), Specular_IOR_Level=0.25)
    # blades let the low sun through
    trans = nt.shader("ShaderNodeBsdfTranslucent", Color=nt.mixc(col, nt.rgb((0.35, 0.5, 0.05)), 0.5), Normal=n)
    nt.out_surface(nt.mix_shader(base, trans, 0.22))
    return mat


def mat_bed(name="planting_bed"):
    """Under the trees: mulch, ivy and shade plants seen as one dark floor."""
    mat, nt = material(name)
    p = nt.coord("Object")
    broad = nt.noise(p, scale=0.2, detail=4.0, rough=0.65)
    leafy = nt.voronoi(p, scale=5.5, out="Distance", dims="3D")
    fine = nt.noise(p, scale=30.0, detail=3.0, rough=0.7)
    col = nt.ramp(broad, [(0.3, (0.03, 0.026, 0.02)), (0.5, (0.025, 0.055, 0.02)), (0.7, (0.045, 0.095, 0.03))])
    col = nt.mixc(col, nt.rgb((0.07, 0.13, 0.04)), nt.smoothstep(0.2, 0.5, leafy) * nt.smoothstep(0.45, 0.7, broad) * 0.7)
    n = nt.bump(leafy * 0.7 + fine * 0.3, strength=1.0, distance=0.08)
    nt.out_surface(nt.principled(Base_Color=col, Roughness=0.7, Normal=n, Sheen_Weight=0.3))
    return mat


def mat_asphalt(name="asphalt"):
    """The streets: dark, patched, lane-marked asphalt; glossy when wet."""
    mat, nt = material(name)
    g = grade(nt)
    p = nt.coord("Object")
    x, y, z = nt.xyz(p)
    patch = nt.noise(p, scale=0.09, detail=4.0, rough=0.6)
    grit = nt.noise(p, scale=160.0, detail=2.0)
    track = nt.noise(nt.vec(x * 1.4, y * 0.05, 0.0), scale=1.0, detail=2.0)
    col = nt.ramp(patch, [(0.3, (0.028, 0.028, 0.03)), (0.6, (0.045, 0.045, 0.047)), (0.8, (0.07, 0.068, 0.066))])
    col = nt.mixc(col, nt.rgb((0.02, 0.02, 0.022)), nt.smoothstep(0.5, 0.7, track) * 0.4)
    # markings come in as a mask painted per face
    mark = nt.attr("mark")
    worn = nt.smoothstep(0.3, 0.6, nt.noise(p, scale=3.0, detail=3.0))
    col = nt.mixc(col, nt.rgb((0.62, 0.6, 0.55)), mark * (0.55 + worn * 0.35))
    rough, col, puddle = _wet(nt, g, p, 0.72, col, puddle_scale=0.1)
    n = nt.bump(grit, strength=0.3 * 1.0, distance=0.004)
    nt.out_surface(nt.principled(Base_Color=col, Roughness=rough, Normal=n))
    return mat


def mat_sidewalk(name="sidewalk"):
    mat, nt = material(name)
    g = grade(nt)
    p = nt.coord("Object")
    x, y, z = nt.xyz(p)
    s = 1.0 / 1.5
    fx = nt.fract(x * s)
    fy = nt.fract(y * s)
    joint = 1.0 - nt.smoothstep(0.004, 0.012, nt.min(nt.min(fx, 1.0 - fx), nt.min(fy, 1.0 - fy)))
    tone = nt.white(nt.vec(nt.floor(x * s), nt.floor(y * s), 0.0))
    stain = nt.noise(p, scale=0.6, detail=4.0, rough=0.65)
    col = nt.ramp(tone * 0.5 + stain * 0.5, [(0.2, (0.2, 0.195, 0.185)), (0.6, (0.3, 0.29, 0.275)), (0.9, (0.36, 0.35, 0.33))])
    col = nt.mixc(col, nt.rgb((0.06, 0.06, 0.058)), joint * 0.8)
    rough, col, puddle = _wet(nt, g, p, 0.68, col, puddle_scale=0.2)
    n = nt.bump(joint * -1.0 + nt.noise(p, scale=120.0) * 0.08, strength=0.3, distance=0.004)
    nt.out_surface(nt.principled(Base_Color=col, Roughness=rough, Normal=n))
    return mat


# ------------------------------------------------------------------- geometry


def _ribbon(mb, pts, hw, z, mat, kerb_mat=None, kerb=0.11, closed=False):
    """A path along a polyline, with a low stone kerb down each side."""
    p = np.asarray(pts, float)
    t = np.gradient(p, axis=0)
    if closed:
        t = np.roll(p, -1, axis=0) - np.roll(p, 1, axis=0)
    t /= np.linalg.norm(t, axis=1, keepdims=True) + 1e-9
    nrm = np.stack([-t[:, 1], t[:, 0]], axis=-1)
    n = len(p)
    idx = np.arange(n if closed else n - 1)
    nxt = (idx + 1) % n

    def strip(off0, off1, z0, z1, m):
        a = np.concatenate([p + nrm * off0, np.full((n, 1), z0)], axis=1)
        b = np.concatenate([p + nrm * off1, np.full((n, 1), z1)], axis=1)
        v = np.concatenate([a, b])
        q = np.stack([idx, nxt, nxt + n, idx + n], axis=-1)
        mb.add(v, q, mat=m, smooth=False)

    strip(hw, -hw, z, z, mat)
    if kerb_mat is not None:
        for side in (1, -1):
            o0, o1 = side * hw, side * (hw + 0.16)
            top = z + kerb
            lo, hi = (o0, o1) if side > 0 else (o1, o0)
            strip(hi, lo, top, top, kerb_mat)
            strip(lo, lo, top, z - 0.05, kerb_mat)
            strip(hi, hi, z - 0.05, top, kerb_mat)


def _rect(mb, x0, y0, x1, y1, z, mat, fattr=None, nx=1, ny=1, hole=0.0):
    """A flat grid. `hole` leaves out the faces within that radius of the
    origin: the basin's floor lies below the park's."""
    xs = np.linspace(x0, x1, nx + 1)
    ys = np.linspace(y0, y1, ny + 1)
    X, Y = np.meshgrid(xs, ys, indexing="ij")
    v = np.stack([X, Y, np.full_like(X, z)], axis=-1).reshape(-1, 3)
    i = np.arange(nx)[:, None]
    j = np.arange(ny)[None, :]
    q = np.stack([i * (ny + 1) + j, (i + 1) * (ny + 1) + j, (i + 1) * (ny + 1) + j + 1, i * (ny + 1) + j + 1], axis=-1).reshape(-1, 4)
    if hole > 0:
        c = v[q].mean(axis=1)
        q = q[np.hypot(c[:, 0], c[:, 1]) > hole]
    mb.add(v, q, mat=mat, fattr=fattr)


def _marks(mb, mat):
    """Lane lines and crossings, as thin slabs just over the road."""
    x0, y0, x1, y1 = site.PARK
    sw, rd = site.SIDEWALK, site.ROAD
    lo = []
    hi = []
    z = ROAD_Z + 0.006

    def bar(ax, ay, bx, by):
        lo.append((min(ax, bx), min(ay, by), z))
        hi.append((max(ax, bx), max(ay, by), z + 0.004))

    # the avenues run north-south, the streets east-west: dashed lane lines
    for side in (-1, 1):
        edge = x1 + sw if side > 0 else x0 - sw
        for lane in (1, 2, 3):
            xc = edge + side * rd * lane / 4
            for y in np.arange(-330, 620, 9.0):
                if lane == 2 or True:
                    bar(xc - 0.07, y, xc + 0.07, y + 3.0)
    for edge, side in ((y0 - sw, -1), (y1 + sw, 1)):
        for lane in (1, 2, 3):
            yc = edge + side * rd * lane / 4
            for x in np.arange(-330, 330, 9.0):
                bar(x, yc - 0.07, x + 3.0, yc + 0.07)
    # crossings at the park's four corners, across each street
    for cx in (x0 - sw - rd / 2, x1 + sw + rd / 2):
        for cy in (y0 - sw - rd - 4.5, y1 + sw + rd + 1.5, y0 + 2.0, y1 - 5.0):
            for k in range(int(rd / 1.2)):
                xx = cx - rd / 2 + 0.5 + k * 1.2
                bar(xx, cy, xx + 0.6, cy + 3.0)
    for cy in (y0 - sw - rd / 2, y1 + sw + rd / 2):
        for cx in (x0 - sw - rd - 4.5, x1 + sw + rd + 1.5, -3.5, 0.5):
            for k in range(int(rd / 1.2)):
                yy = cy - rd / 2 + 0.5 + k * 1.2
                bar(cx, yy, cx + 3.0, yy + 0.6)
    v, q = boxes(np.array(lo), np.array(hi))
    mb.add(v, q, mat=mat, fattr={"mark": 1.0})


def build(col=None):
    col = col or collection("Ground")
    paving = mat_paving()
    pavers = mat_pavers()
    lawn = mat_lawn()
    bed = mat_bed()
    asphalt = mat_asphalt()
    sidewalk = mat_sidewalk()
    from .fountain import mat_granite

    kerb = mat_granite("kerb_granite", (0.3, 0.295, 0.29))

    # ---- the plaza: a polar grid, so the paving's rings follow the mesh
    mb = MB()
    v, q, uv = disc(5.62, site.PLAZA_R, 256, rings=24, z=PLAZA_Z)
    mb.add(v, q, mat=0, smooth=True, uv=uv)
    # its edge: a raised granite kerb, broken where the paths arrive
    v, q, uv = revolve([(site.PLAZA_R, PLAZA_Z), (site.PLAZA_R, PLAZA_Z + 0.14), (site.PLAZA_R + 0.3, PLAZA_Z + 0.14), (site.PLAZA_R + 0.3, BED_Z)], 512)
    ang = np.arctan2(v[:, 1], v[:, 0])
    pts = np.stack([(site.PLAZA_R + 0.6) * np.cos(ang), (site.PLAZA_R + 0.6) * np.sin(ang)], axis=-1)
    on_path = site.path_clearance(pts) < 0.2
    keep = ~on_path[q].any(axis=1)
    mb.add(v, q[keep], mat=1, smooth=False, uv=uv)
    add_object("Plaza", mb.build("Plaza", [paving, kerb]), col)

    # ---- the park floor, the lawn and the paths
    x0, y0, x1, y1 = site.PARK
    mb = MB()
    _rect(mb, x0, y0, x1, y1, BED_Z, 0, nx=30, ny=48, hole=12.0)
    a = np.linspace(0, TAU, 160, endpoint=False)
    rings = 14
    lv = []
    for k in range(rings + 1):
        s = k / rings
        # the lawn is crowned a hand's breadth, as lawns are
        lv.append(np.stack([site.LAWN_C[0] + site.LAWN_R[0] * s * np.cos(a), site.LAWN_C[1] + site.LAWN_R[1] * s * np.sin(a), np.full_like(a, BED_Z + 0.03 + 0.22 * (1 - s * s))], axis=-1))
    lv = np.concatenate(lv)
    i = np.arange(rings)[:, None]
    j = np.arange(160)[None, :]
    j2 = (j + 1) % 160
    lq = np.stack([i * 160 + j, (i + 1) * 160 + j, (i + 1) * 160 + j2, i * 160 + j2], axis=-1).reshape(-1, 4)
    mb.add(lv, lq, mat=1, smooth=True)
    for k in range(len(site.PATHS)):
        pts, hw = site.path_points(k, 72)
        _ribbon(mb, pts, hw, WALK_Z, 2, kerb_mat=3)
    _ribbon(mb, site.lawn_walk(200), site.LAWN_WALK_HW, WALK_Z + 0.002, 2, kerb_mat=3, closed=True)
    add_object("Park", mb.build("Park", [bed, lawn, pavers, kerb]), col)

    # ---- streets and sidewalks
    sw, rd = site.SIDEWALK, site.ROAD
    mb = MB()
    # sidewalks: a band round the park, and one along each far building line
    for a0, b0, a1, b1 in (
        (x0 - sw, y0 - sw, x1 + sw, y0), (x0 - sw, y1, x1 + sw, y1 + sw), (x0 - sw, y0, x0, y1), (x1, y0, x1 + sw, y1),
    ):
        _rect(mb, a0, b0, a1, b1, WALK_Z, 0, nx=4, ny=4)
    far = sw + rd
    for a0, b0, a1, b1 in (
        (-330, y0 - far - 6, 330, y0 - far), (-330, y1 + far, 330, y1 + far + 6), (x0 - far - 6, -330, x0 - far, 620), (x1 + far, -330, x1 + far + 6, 620),
    ):
        _rect(mb, a0, b0, a1, b1, WALK_Z, 0, nx=6, ny=6)
    # kerb faces
    lo = [(x0 - sw - 0.15, y0 - sw - 0.15, ROAD_Z), (x0 - sw - 0.15, y1 + sw, ROAD_Z), (x0 - sw - 0.15, y0 - sw, ROAD_Z), (x1 + sw, y0 - sw, ROAD_Z)]
    hi = [(x1 + sw + 0.15, y0 - sw, WALK_Z), (x1 + sw + 0.15, y1 + sw + 0.15, WALK_Z), (x0 - sw, y1 + sw, WALK_Z), (x1 + sw + 0.15, y1 + sw, WALK_Z)]
    v, q = boxes(np.array(lo), np.array(hi))
    mb.add(v, q, mat=2)
    add_object("Sidewalks", mb.build("Sidewalks", [sidewalk, asphalt, kerb]), col)

    mb = MB()
    _rect(mb, -900, -500, 900, 2600, ROAD_Z, 0, fattr={"mark": 0.0}, nx=180, ny=310, hole=12.0)
    _marks(mb, 0)
    add_object("Streets", mb.build("Streets", [asphalt]), col)
    return col

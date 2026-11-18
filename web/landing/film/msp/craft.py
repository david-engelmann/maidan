"""The model's materials: obsidian, brass and light. Satin black and graphite
blocks with their windows etched as hairlines and a few of them lit; brass
for the crowns, the kerbs and the court; smoked glass; frosted beads that
carry light; a polished black base that mirrors all of it.

Everything is at true model size (metres): a hairline is a sixth of a
millimetre wide.
"""

import os

import bpy

from .common import grade, material

ORANGE = (1.0, 0.144, 0.024)  # #ff6a2b in scene-linear
WARM = (1.0, 0.56, 0.24)  # a warm lamp
PEARL = (1.0, 0.7, 0.42)  # the beads' light
HAIRLINE = 0.00017  # an etched line's width


def assets():
    return os.environ.get("FILM_ASSETS", os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "assets"))


def _tex(nt, img, vec, interp="Cubic"):
    n = nt.node("ShaderNodeTexImage", interpolation=interp)
    n.image = img
    nt.put(n.inputs["Vector"], vec)
    return nt.v(n.outputs["Color"])


def _bevel(nt, radius=0.00012, samples=4):
    n = nt.node("ShaderNodeBevel", samples=samples)
    n.inputs["Radius"].default_value = radius
    return nt.v(n.outputs["Normal"])


def _pick(nt, s1, s2, a, b, c):
    """a, b or c by style: grid, ribbon or pier."""
    return nt.mix(nt.mix(a, b, s1), c, s2)


def facade(name="obsidian"):
    """Every block is this one material; the mesh says which finish a piece
    has and what is etched on each face.

    `tone` is the finish (value, roughness, metal). `gridp` is the etched
    grid (bay, floor, style), `dims` where it sits on the face (half its
    width, where it starts above the foot, where it stops), `litp` how many
    of its windows are lit (share, seed, how brightly). A face's UV is in
    metres: across from its middle, and up from its foot."""
    mat, nt = material(name)
    g = grade(nt)
    u, v, _ = nt.xyz(nt.coord("UV"))
    val, rough0, metal = nt.xyz(nt.attr("tone", out="Color"))
    bay, fl, style = nt.xyz(nt.attr("gridp", out="Color"))
    half, base, vtop = nt.xyz(nt.attr("dims", out="Color"))
    share, seed, gain = nt.xyz(nt.attr("litp", out="Color"))
    p = nt.geom("Position")
    bay = nt.max(bay, 1e-5)
    fl = nt.max(fl, 1e-5)
    cu = (u + half) / bay
    cv = (v - base) / fl
    inside = nt.gt(half, 0.0) * nt.gt(cu, 0.0) * nt.lt(u, half) * nt.gt(cv, 0.0) * nt.lt(v, vtop)
    fu, fv = nt.fract(cu), nt.fract(cv)
    iu, iv = nt.floor(cu), nt.floor(cv)
    s1 = nt.gt(style, 0.5) * nt.lt(style, 1.5)
    s2 = nt.gt(style, 1.5)
    # the etching: floor lines and mullions, a hair wide
    dv = nt.min(fv, 1.0 - fv) * fl
    du = nt.min(fu, 1.0 - fu) * bay
    lw = HAIRLINE / 2
    line_h = (1.0 - nt.smoothstep(lw * 0.6, lw * 1.3, dv)) * _pick(nt, s1, s2, 0.85, 1.0, 0.3)
    line_v = (1.0 - nt.smoothstep(lw * 0.5, lw * 1.1, du)) * _pick(nt, s1, s2, 0.7, 0.16, 1.0)
    # a belt course where the street floor ends
    belt = (1.0 - nt.smoothstep(lw * 0.8, lw * 1.8, nt.abs(v - base))) * nt.gt(half, 0.0) * nt.gt(base, 0.0008)
    lines = nt.max(line_h, line_v) * inside
    # the pane inside each cell
    mu = _pick(nt, s1, s2, 0.2, 0.06, 0.27)
    v0 = _pick(nt, s1, s2, 0.24, 0.3, 0.14)
    v1 = _pick(nt, s1, s2, 0.76, 0.7, 0.86)
    pane = nt.gt(fu, mu) * nt.lt(fu, 1.0 - mu) * nt.gt(fv, v0) * nt.lt(fv, v1) * inside
    # which are lit: few, in clusters, and now and then a run along a floor
    rnd = nt.white(nt.vec(iu, iv, seed))
    rnd2 = nt.white(nt.vec(iu + 31.7, iv + 11.3, seed))
    rnd3 = nt.white(nt.vec(iu + 5.1, iv + 77.7, seed))
    cluster = nt.noise(p, scale=17.0, detail=1.0, rough=0.5)
    run = nt.noise(nt.vec(iu * 0.11, iv * 1.9, seed * 9.0), scale=1.0, detail=0.0)
    dens = share * (0.2 + 3.4 * nt.smoothstep(0.52, 0.74, cluster) + 2.6 * nt.smoothstep(0.64, 0.8, run))
    on = nt.lt(rnd, dens * g["windows"]) * pane
    warm = nt.mixc(nt.rgb((1.0, 0.47, 0.17)), nt.rgb((1.0, 0.7, 0.4)), rnd2)
    tint = nt.mixc(warm, nt.rgb((0.72, 0.84, 1.0)), nt.gt(rnd3, 0.93))
    # a blind half drawn, a lamp at one side: no window glows evenly
    shade = 0.55 + 0.45 * nt.smoothstep(0.0, 1.0, nt.fract(fu * 0.5 + rnd3))
    strength = on * (0.18 + 1.5 * rnd2 * rnd2) * shade * gain * g["lamps"] * 14.0
    # the finish: this piece's tone, a hand's faint clouding, the grain of the satin
    cloud = nt.noise(p, scale=31.0, detail=2.0, rough=0.55)
    fine = nt.noise(p, scale=5200.0, detail=1.0)
    rough = rough0 * (0.86 + 0.3 * nt.smoothstep(0.35, 0.75, cloud)) + (fine - 0.5) * 0.05
    col = nt.vec(val * 0.95, val, val * 1.1)
    col = nt.mixc(col, nt.rgb((0.004, 0.004, 0.005)), pane * 0.8)
    rough = nt.mix(rough, 0.07, pane * 0.9)
    metal = nt.mix(metal, 0.0, pane)
    etched = nt.sat(lines + belt)
    col = nt.mixc(col, nt.rgb((0.2, 0.185, 0.165)), etched * 0.9)
    rough = nt.mix(rough, 0.42, etched)
    metal = nt.mix(metal, 1.0, etched)
    em = nt.shader("ShaderNodeEmission", Color=tint, Strength=strength)
    nt.out_surface(nt.add_shader(nt.principled(Base_Color=col, Roughness=rough, Metallic=metal, Specular_IOR_Level=0.5), em))
    # the windows are for the eye; the square's own lamps light the model
    mat.cycles.emission_sampling = "NONE"
    return mat


def satin(name, tone=(0.016, 0.016, 0.018), rough=0.4, metal=0.0):
    """A plain dark finish: pavements, frames."""
    mat, nt = material(name)
    p = nt.geom("Position")
    cloud = nt.noise(p, scale=24.0, detail=2.0)
    fine = nt.noise(p, scale=4200.0, detail=1.0)
    nt.out_surface(nt.principled(Base_Color=(*tone, 1.0), Metallic=metal, Roughness=rough + nt.smoothstep(0.35, 0.8, cloud) * 0.08 + (fine - 0.5) * 0.05, Specular_IOR_Level=0.5))
    return mat


def gloss(name="gloss_black", tone=(0.0035, 0.0035, 0.004), rough=0.045):
    """The base: polished black, a mirror for the glow. A cloth has been over
    it: the polish is not quite even."""
    mat, nt = material(name)
    p = nt.geom("Position")
    x, y, z = nt.xyz(p)
    wipe = nt.noise(nt.vec(x * 3.0, y * 14.0, 0.0), scale=1.0, detail=3.0, rough=0.6)
    fine = nt.noise(p, scale=2400.0, detail=1.0)
    nt.out_surface(nt.principled(Base_Color=(*tone, 1.0), Roughness=rough + nt.smoothstep(0.4, 0.85, wipe) * 0.03 + fine * 0.012, Specular_IOR_Level=0.5))
    return mat


def brass(name="brass", turned=True, rough=0.2, tone=(0.86, 0.56, 0.2), bevel=True):
    """Brushed brass: the lathe's fine rings on turned parts, and the faint
    clouding a hand leaves."""
    mat, nt = material(name)
    p = nt.coord("Object")
    smudge = nt.noise(p, scale=38.0, detail=3.0, rough=0.6)
    fine = nt.noise(p, scale=5000.0, detail=1.0)
    col = nt.mixc(nt.rgb(tone), nt.rgb((tone[0] * 0.88, tone[1] * 0.8, tone[2] * 0.7)), nt.smoothstep(0.45, 0.75, smudge) * 0.5)
    args = dict(Base_Color=col, Metallic=1.0, Roughness=rough + nt.smoothstep(0.4, 0.8, smudge) * 0.1 + fine * 0.04, Anisotropic=0.7)
    if bevel:
        args["Normal"] = _bevel(nt)
    if turned:
        tan = nt.node("ShaderNodeTangent", direction_type="RADIAL", axis="Z")
        args["Tangent"] = nt.v(tan.outputs[0])
    nt.out_surface(nt.principled(**args))
    return mat


def court(engraving, name="court_brass"):
    """The square's floor: a plate of brass, blasted satin so that it glows
    under the beads, with the walks and the plaza polished into it and their
    edges cut as hairlines."""
    mat, nt = material(name)
    p = nt.coord("Object")
    x, y, z = nt.xyz(p)
    eng = _tex(nt, engraving, nt.coord("UV"))
    er, eg, eb = nt.xyz(eng)
    brush = nt.noise(nt.vec(x * 30.0, y * 5200.0, 0.0), scale=1.0, detail=2.0, rough=0.6)
    cloud = nt.noise(p, scale=14.0, detail=3.0, rough=0.6)
    field = nt.mixc(nt.rgb((0.74, 0.47, 0.17)), nt.rgb((0.82, 0.55, 0.22)), brush)
    walk = nt.rgb((0.93, 0.7, 0.36))
    col = nt.mixc(field, walk, er)
    col = nt.mixc(col, nt.rgb((0.05, 0.03, 0.012)), nt.sat(eg * 0.92))
    rough = nt.mix(0.5 + (brush - 0.5) * 0.12 + nt.smoothstep(0.4, 0.8, cloud) * 0.05, 0.3, er)
    rough = nt.mix(rough, 0.6, nt.sat(eg))
    nt.out_surface(nt.principled(Base_Color=col, Metallic=1.0, Roughness=rough, Normal=nt.bump(-eg - er * 0.3, strength=0.4, distance=0.00008)))
    return mat


def smoked(name="glass_smoked"):
    """Cast glass with real thickness: a bronze smoke that deepens with the
    depth looked through, and its floors etched round it."""
    mat, nt = material(name)
    g = grade(nt)
    p = nt.coord("Object")
    x, y, z = nt.xyz(p)
    floor_h = nt.attr("floor_h")
    line = (1.0 - nt.smoothstep(0.03, 0.06, nt.fract(z / nt.max(floor_h, 0.001)))) * nt.gt(floor_h, 0.0005)
    rough = nt.mix(0.035, 0.5, line)
    glass = nt.principled(Base_Color=(1.0, 1.0, 1.0, 1.0), Roughness=rough, Transmission_Weight=1.0, IOR=1.5)
    glow = nt.shader("ShaderNodeEmission", Color=(1.0, 0.7, 0.4, 1.0), Strength=line * g["lamps"] * 0.12)
    surf = nt.add_shader(glass, glow)
    clear = nt.shader("ShaderNodeBsdfTransparent", Color=(0.3, 0.26, 0.22, 1.0))
    surf = nt.mix_shader(surf, clear, nt.lightpath("Is Shadow Ray"))
    vol = nt.shader("ShaderNodeVolumeAbsorption", Color=(0.5, 0.42, 0.34, 1.0), Density=30.0)
    nt.out_surface(surf, volume=vol)
    mat.cycles.emission_sampling = "NONE"
    return mat


def pearl(name="pearl"):
    """A frosted glass bead on a pin that carries light up into it: brightest
    where the pin enters, and soft at its edge. `glow` is how bright this
    bead is, `h` runs from its foot to its top."""
    mat, nt = material(name)
    g = grade(nt)
    glow = nt.attr("glow")
    h = nt.attr("h")
    facing = 1.0 - nt.layer(0.5, "Facing")
    tint = nt.mixc(nt.rgb((1.0, 0.62, 0.33)), nt.rgb((1.0, 0.78, 0.55)), nt.attr("hue"))
    body = (0.42 + 0.58 * nt.pow(facing, 1.4)) * nt.mix(1.9, 0.5, nt.smoothstep(0.0, 0.9, h))
    em = nt.shader("ShaderNodeEmission", Color=tint, Strength=glow * body * g["lamps"] * 5.2)
    fine = nt.noise(nt.coord("Object"), scale=3000.0, detail=1.0)
    bead = nt.principled(Base_Color=(0.82, 0.8, 0.77, 1.0), Roughness=0.3 + fine * 0.08, Coat_Weight=0.35, Coat_Roughness=0.12, Specular_IOR_Level=0.5)
    nt.out_surface(nt.add_shader(bead, em))
    return mat


def black_glass(name="black_glass", tone=(0.004, 0.006, 0.005), rough=0.03, ripple=0.0):
    """An inlay of black glass: the lawn, the fountain's pool."""
    mat, nt = material(name)
    args = dict(Base_Color=(*tone, 1.0), Roughness=rough, Specular_IOR_Level=0.6, Coat_Weight=0.4, Coat_Roughness=0.02)
    if ripple > 0:
        x, y, z = nt.xyz(nt.coord("Object"))
        r = nt.sqrt(x * x + y * y)
        args["Normal"] = nt.bump(nt.sin(r * 5200.0), strength=ripple, distance=0.00005)
    nt.out_surface(nt.principled(**args))
    return mat


def light(name="ring_light", power=30.0, tint=(1.0, 0.8, 0.6)):
    """Light itself: the fountain's ring, the tip of a lamp."""
    mat, nt = material(name)
    g = grade(nt)
    nt.out_surface(nt.shader("ShaderNodeEmission", Color=(*tint, 1.0), Strength=g["lamps"] * power))
    return mat


def plume(name="plume", power=26.0):
    """The fountain's jet: a rod of frosted glass with light piped up it,
    fading as it rises and soft at its edge. `h` runs up its length."""
    mat, nt = material(name)
    g = grade(nt)
    h = nt.attr("h")
    facing = 1.0 - nt.layer(0.5, "Facing")
    fade = nt.pow(1.0 - nt.sat(h), 1.6)
    alpha = nt.sat(nt.pow(facing, 1.5) * (0.12 + 0.88 * fade))
    em = nt.shader("ShaderNodeEmission", Color=(1.0, 0.82, 0.62, 1.0), Strength=g["lamps"] * power * (0.1 + fade))
    clear = nt.shader("ShaderNodeBsdfTransparent", Color=(1.0, 1.0, 1.0, 1.0))
    nt.out_surface(nt.mix_shader(clear, em, alpha))
    return mat


def paint(name, tone, rough=0.5, glow=0.0):
    """Sprayed resin: the figures."""
    mat, nt = material(name)
    p = nt.coord("Object")
    peel = nt.noise(p, scale=4200.0, detail=1.0)
    args = dict(Base_Color=(*tone, 1.0), Roughness=rough + peel * 0.08, Specular_IOR_Level=0.4)
    if glow > 0:
        args.update(Emission_Color=(*tone, 1.0), Emission_Strength=glow)
    nt.out_surface(nt.principled(**args))
    mat.cycles.emission_sampling = "NONE"
    return mat

"""The model's materials: what a model-maker's bench holds. Pale wood with
its grain, end grain and laser etching; brushed brass; smoked, frosted and
opal acrylic; sage felt; charcoal board; resin; paint.

Everything is at true model size (metres), so a millimetre of grain is a
millimetre of grain.
"""

import os

import bpy

from .common import grade, material

ORANGE = (1.0, 0.144, 0.024)  # #ff6a2b in scene-linear
SCORCH = (0.13, 0.068, 0.03)
WARM = (1.0, 0.52, 0.2)  # a warm white LED behind paper


def assets():
    return os.environ.get("FILM_ASSETS", os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "assets"))


def image(name, colour=False):
    path = os.path.join(assets(), "tex", name)
    img = bpy.data.images.load(path, check_existing=True)
    img.colorspace_settings.name = "sRGB" if colour else "Non-Color"
    return img


def _tex(nt, img, vec, interp="Cubic"):
    n = nt.node("ShaderNodeTexImage", interpolation=interp)
    n.image = img
    nt.put(n.inputs["Vector"], vec)
    return nt.v(n.outputs["Color"])


def _bevel(nt, radius=0.00028, samples=6):
    """Every cut edge is eased a quarter of a millimetre, as sanding does."""
    n = nt.node("ShaderNodeBevel", samples=samples)
    n.inputs["Radius"].default_value = radius
    return nt.v(n.outputs["Normal"])


def wood(name, tex="white_maple_veneer", value=0.86, sat=0.92, etch=True, engraving=None):
    """Solid pale wood. Up the sides, the scanned long grain; on the cut
    tops, end grain drawn from the same log: growth rings and pores. Each
    piece of stock sits in a different part of the board, a shade apart.
    Etched lines are scorched brown, with a faint halo."""
    mat, nt = material(name)
    p = nt.coord("Object")
    n = nt.geom("True Normal")
    x, y, z = nt.xyz(p)
    nx, ny, nz = nt.xyz(n)
    stock = nt.attr("stock", out="Color")
    sx, sy, sz = nt.xyz(stock)
    top = nt.smoothstep(0.55, 0.8, nt.abs(nz))
    # along the wall, and up it: the grain runs up
    h = y * nx - x * ny
    tile = 0.46
    vec = nt.vec((z + sx * 0.41) / tile, (h + sy * 0.37) / tile, 0.0)
    flat = nt.vec((x + sx * 0.41) / tile, (y + sy * 0.37) / tile, 0.0)
    col_long = _tex(nt, image(f"{tex}_diff.jpg", True), vec)
    col_flat = _tex(nt, image(f"{tex}_diff.jpg", True), flat)
    # the scan is a finished veneer; raw sanded stock shows its figure more
    col_long = nt.mixc(col_long, nt.mixc(col_long, col_long, 1.0, blend="MULTIPLY"), 0.55)
    col_flat = nt.mixc(col_flat, nt.mixc(col_flat, col_flat, 1.0, blend="MULTIPLY"), 0.55)
    fibre = nt.noise(nt.vec(z * 60.0, h * 2600.0, (x + y) * 1300.0), scale=1.0, detail=2.0, rough=0.6)
    col_long = nt.mixc(col_long, nt.mixc(col_long, nt.rgb((0.3, 0.19, 0.1)), 0.5), nt.smoothstep(0.52, 0.8, fibre) * 0.28)
    rough_long = _tex(nt, image(f"{tex}_rough.jpg"), vec)
    disp_long = _tex(nt, image(f"{tex}_disp.jpg"), vec)
    # end grain: rings about a pith well off to one side, and the pores between
    cx = x + (sx - 0.5) * 0.5 + 0.21
    cy = y + (sy - 0.5) * 0.5 - 0.17
    rr = nt.sqrt(cx * cx + cy * cy)
    wav = nt.noise(p, scale=55.0, detail=2.0)
    ring = nt.fract(rr / 0.0017 + wav * 0.9)
    late = nt.smoothstep(0.62, 0.97, ring) * (1.0 - nt.smoothstep(0.97, 1.0, ring))
    pores = nt.noise(p, scale=7000.0, detail=1.0)
    col_end = nt.mixc(col_flat, nt.mixc(col_flat, nt.rgb((0.32, 0.2, 0.1)), 0.42), late * 0.55 + nt.smoothstep(0.62, 0.8, pores) * 0.3)
    col_end = nt.hsv(col_end, v=0.9, s=1.08)
    col = nt.mixc(col_long, col_end, top)
    # this piece's own tone
    col = nt.hsv(col, h=0.5 + (sz - 0.5) * 0.014, s=sat * (0.92 + sy * 0.2), v=value * (0.9 + sz * 0.2))
    height = disp_long * 0.4 + pores * 0.15 * top + fibre * 0.5 * (1.0 - top)
    rough = nt.mix(nt.remap(rough_long, 0.0, 1.0, 0.46, 0.68), 0.72, top)
    if etch:
        uv = nt.coord("UV")
        cu, cv, _ = nt.xyz(uv)
        cell = nt.attr("cell")
        line_v = 1.0 - nt.smoothstep(0.009, 0.02, nt.min(cv, 1.0 - cv))
        line_u = 1.0 - nt.smoothstep(0.01, 0.024, nt.min(cu, 1.0 - cu))
        lines = nt.max(line_v, line_u * 0.6) * cell
        halo = nt.pow(nt.sat(nt.attr("scorch")), 2.6)
        burn = nt.attr("burn")
        # a laser's line is not even: it bites deeper where the grain is soft
        bite = 0.7 + 0.5 * nt.noise(p, scale=900.0, detail=1.0)
        mark = nt.sat(lines * 0.82 * bite + halo * 0.4 + burn)
        col = nt.mixc(col, nt.rgb(SCORCH), mark * 0.86)
        rough = nt.mix(rough, 0.82, mark)
        height = height - lines * 1.6
    if engraving is not None:
        uv = nt.coord("UV")
        eng = _tex(nt, engraving, uv, interp="Cubic")
        er, eg, eb = nt.xyz(eng)
        raster = nt.fract((x + y) / 0.00022)
        tone = nt.mixc(col, nt.rgb((0.36, 0.23, 0.12)), er * (0.4 + 0.1 * nt.smoothstep(0.3, 0.7, raster)))
        col = nt.mixc(tone, nt.rgb(SCORCH), nt.sat(eg * 0.9 + eb * 0.25))
        rough = nt.mix(rough, 0.8, nt.sat(er + eg))
        height = height - er * 1.2 - eg * 2.2
    nrm = nt.bump(height, strength=0.35, distance=0.00012, normal=_bevel(nt))
    nt.out_surface(nt.principled(Base_Color=col, Roughness=rough, Normal=nrm, Specular_IOR_Level=0.35, Sheen_Weight=0.08))
    return mat


def pocket(name="window_pocket"):
    """The floor of an etched window. Most are only engraved: scorched, with
    the raster of the laser in them. Some are cut through and backed with
    paper, and those have light behind."""
    mat, nt = material(name)
    g = grade(nt)
    p = nt.coord("Object")
    x, y, z = nt.xyz(p)
    glow = nt.attr("glow")
    lit = nt.attr("lit")
    through = nt.gt(glow, 0.001)
    raster = nt.smoothstep(0.25, 0.75, nt.fract(z / 0.00016))
    burnt = nt.mixc(nt.rgb((0.2, 0.115, 0.055)), nt.rgb((0.1, 0.055, 0.026)), raster)
    fibre = nt.noise(p, scale=2600.0, detail=2.0, rough=0.7)
    # the paper is a dark amber tracing film: unlit, it sits back with the engraved ones
    paper = nt.mixc(nt.rgb((0.23, 0.14, 0.075)), nt.rgb((0.17, 0.1, 0.05)), fibre)
    col = nt.mixc(burnt, paper, through)
    on = nt.smoothstep(0.0, 0.03, g["windows"] - lit) * through
    # the lamp is somewhere behind the paper: no window glows evenly
    blotch = nt.noise(p, scale=140.0, detail=2.0)
    uv = nt.coord("UV")
    pu, pv, _ = nt.xyz(uv)
    du = pu - 0.5
    dv = pv - 0.42
    pane = nt.sat(1.0 - (du * du + dv * dv) * 2.6)
    strength = on * glow * g["lamps"] * (0.5 + 0.9 * blotch) * (0.75 + 0.5 * fibre) * (0.35 + 0.9 * pane) * 7.0
    tint = nt.mixc(nt.rgb(WARM), nt.rgb((1.0, 0.66, 0.36)), nt.fract(lit * 13.7))
    em = nt.shader("ShaderNodeEmission", Color=tint, Strength=strength)
    nt.out_surface(nt.add_shader(nt.principled(Base_Color=col, Roughness=0.85, Normal=_bevel(nt, 0.0002, 4)), em))
    return mat


def brass(name="brass", turned=True, rough=0.2):
    """Brushed brass: the lathe's fine rings on turned parts, and the faint
    clouding a hand leaves."""
    mat, nt = material(name)
    p = nt.coord("Object")
    smudge = nt.noise(p, scale=38.0, detail=3.0, rough=0.6)
    fine = nt.noise(p, scale=5000.0, detail=1.0)
    col = nt.mixc(nt.rgb((0.86, 0.54, 0.16)), nt.rgb((0.74, 0.43, 0.12)), nt.smoothstep(0.45, 0.75, smudge) * 0.5)
    args = dict(Base_Color=col, Metallic=1.0, Roughness=rough + nt.smoothstep(0.4, 0.8, smudge) * 0.12 + fine * 0.04, Anisotropic=0.72, Normal=_bevel(nt, 0.00012, 4))
    if turned:
        tan = nt.node("ShaderNodeTangent", direction_type="RADIAL", axis="Z")
        args["Tangent"] = nt.v(tan.outputs[0])
    nt.out_surface(nt.principled(**args))
    return mat


def acrylic(name, kind="smoked"):
    """Cast acrylic with real thickness. Smoked: a bronze that deepens with
    the depth looked through, satin faces and polished arrises that catch the
    light. Frosted: a clear block sanded matt. Both carry etched floor lines
    that light up when the block is lit from its foot."""
    mat, nt = material(name)
    g = grade(nt)
    p = nt.coord("Object")
    x, y, z = nt.xyz(p)
    frost = nt.attr("frost")
    floor_h = nt.attr("floor_h")
    line = (1.0 - nt.smoothstep(0.018, 0.04, nt.fract(z / nt.max(floor_h, 0.001)))) * nt.gt(floor_h, 0.0005) * frost
    if kind == "smoked":
        rough = nt.mix(0.015, 0.2, frost)
        col = (1.0, 1.0, 1.0, 1.0)
    else:
        rough = nt.mix(0.02, 0.16, frost)
        col = (0.97, 0.985, 1.0, 1.0)
    rough = nt.mix(rough, 0.85, line)
    smear = nt.noise(p, scale=60.0, detail=2.0)
    rough = rough + nt.smoothstep(0.5, 0.8, smear) * 0.03
    glass = nt.principled(Base_Color=col, Roughness=rough, Transmission_Weight=1.0, IOR=1.49, Normal=_bevel(nt, 0.00015, 4))
    # etched lines scatter the light piped up the block
    glow = nt.shader("ShaderNodeEmission", Color=(1.0, 0.72, 0.42, 1.0), Strength=line * g["lamps"] * (0.7 if kind == "smoked" else 0.5))
    surf = nt.add_shader(glass, glow)
    # a shadow through acrylic is tinted, not black
    clear = nt.shader("ShaderNodeBsdfTransparent", Color=(0.45, 0.4, 0.36, 1.0) if kind == "smoked" else (0.8, 0.82, 0.84, 1.0))
    surf = nt.mix_shader(surf, clear, nt.lightpath("Is Shadow Ray"))
    if kind == "smoked":
        vol = nt.shader("ShaderNodeVolumeAbsorption", Color=(0.3, 0.25, 0.21, 1.0), Density=70.0)
    else:
        vol = nt.shader("ShaderNodeVolumeAbsorption", Color=(0.86, 0.93, 0.95, 1.0), Density=6.0)
    nt.out_surface(surf, volume=vol)
    return mat


def opal(name="opal"):
    """Frosted acrylic beads, for the trees' crowns: clear inside, sanded
    matt outside, so light goes into them and comes out soft."""
    mat, nt = material(name)
    p = nt.coord("Object")
    fine = nt.noise(p, scale=2500.0, detail=1.0)
    bead = nt.principled(Base_Color=(0.96, 0.975, 0.98, 1.0), Roughness=0.34 + fine * 0.08, Transmission_Weight=1.0, IOR=1.49, Specular_IOR_Level=0.5)
    # its shadow is a soft lens of light, not a black disc
    clear = nt.shader("ShaderNodeBsdfTransparent", Color=(0.72, 0.74, 0.76, 1.0))
    surf = nt.mix_shader(bead, clear, nt.lightpath("Is Shadow Ray"))
    vol = nt.shader("ShaderNodeVolumeScatter", Color=(0.97, 0.98, 1.0, 1.0), Density=38.0, Anisotropy=0.55)
    nt.out_surface(surf, volume=vol)
    return mat


def resin(name="resin_water"):
    """The fountain's pool: a disc of frosted resin over a lamp."""
    mat, nt = material(name)
    g = grade(nt)
    p = nt.coord("Object")
    x, y, z = nt.xyz(p)
    r = nt.sqrt(x * x + y * y)
    hot = nt.exp(-(r * r) / (0.0085 * 0.0085))
    ripple = nt.sin(r * 3400.0) * 0.5 + 0.5
    em = nt.shader("ShaderNodeEmission", Color=(1.0, 0.74, 0.46, 1.0), Strength=g["lamps"] * (0.7 + 3.2 * hot) * (0.7 + 0.5 * ripple))
    body = nt.principled(Base_Color=(0.86, 0.9, 0.9, 1.0), Roughness=0.35, Subsurface_Weight=0.8, Subsurface_Radius=(0.004, 0.004, 0.004), Subsurface_Scale=1.0, Normal=nt.bump(ripple, strength=0.25, distance=0.00008))
    nt.out_surface(nt.add_shader(body, em))
    return mat


def rod(name="acrylic_rod"):
    """The jet: a frosted rod that carries the lamp's light up its length."""
    mat, nt = material(name)
    g = grade(nt)
    z = nt.xyz(nt.coord("Object"))[2]
    up = nt.sat(z / 0.016)
    em = nt.shader("ShaderNodeEmission", Color=(1.0, 0.8, 0.56, 1.0), Strength=g["lamps"] * (2.4 * nt.pow(1.0 - up, 2.0) + 1.0 * nt.smoothstep(0.86, 1.0, up) + 0.25))
    body = nt.principled(Base_Color=(0.95, 0.97, 1.0, 1.0), Roughness=0.4, Transmission_Weight=0.85, IOR=1.49, Subsurface_Weight=0.2, Subsurface_Radius=(0.002, 0.002, 0.002))
    nt.out_surface(nt.add_shader(body, em))
    return mat


def felt(name="felt_sage", tone=(0.27, 0.33, 0.23)):
    """Wool felt: matted fibres, a soft sheen at grazing angles."""
    mat, nt = material(name)
    p = nt.coord("Object")
    fibre = nt.noise(p, scale=5200.0, detail=3.0, rough=0.7, dist=1.6)
    clump = nt.noise(p, scale=420.0, detail=2.0)
    col = nt.mixc(nt.rgb(tone), nt.rgb((tone[0] * 1.35, tone[1] * 1.3, tone[2] * 1.3)), nt.smoothstep(0.35, 0.75, fibre) * 0.6)
    col = nt.mixc(col, nt.rgb((tone[0] * 0.72, tone[1] * 0.74, tone[2] * 0.74)), nt.smoothstep(0.5, 0.8, clump) * 0.5)
    nrm = nt.bump(fibre * 0.7 + clump * 0.3, strength=0.9, distance=0.00025)
    nt.out_surface(nt.principled(Base_Color=col, Roughness=0.95, Sheen_Weight=1.0, Sheen_Roughness=0.42, Sheen_Tint=(0.8, 0.9, 0.75, 1.0), Specular_IOR_Level=0.1, Normal=nrm))
    return mat


def board(name="charcoal_board", tone=(0.022, 0.022, 0.024), rough=0.58, dust=1.0):
    """The baseplate: through-dyed charcoal fibreboard, with the dust any
    dark model collects."""
    mat, nt = material(name)
    p = nt.coord("Object")
    fibre = nt.noise(p, scale=3200.0, detail=2.0, rough=0.7)
    cloud = nt.noise(p, scale=26.0, detail=3.0)
    col = nt.mixc(nt.rgb(tone), nt.rgb((tone[0] * 2.2, tone[1] * 2.2, tone[2] * 2.2)), nt.smoothstep(0.6, 0.85, fibre) * 0.5)
    specks = nt.voronoi(p, scale=900.0, out="Distance")
    speck = (1.0 - nt.smoothstep(0.02, 0.05, specks)) * nt.smoothstep(0.5, 0.75, nt.noise(p, scale=12.0, detail=2.0)) * dust
    col = nt.mixc(col, nt.rgb((0.35, 0.34, 0.32)), speck * 0.5)
    nrm = nt.bump(fibre, strength=0.12, distance=0.0001, normal=_bevel(nt, 0.0004, 4))
    nt.out_surface(nt.principled(Base_Color=col, Roughness=rough + nt.smoothstep(0.4, 0.8, cloud) * 0.12, Normal=nrm, Specular_IOR_Level=0.4))
    return mat


def paint(name, tone, rough=0.5, sss=0.0):
    """Sprayed resin: the figures."""
    mat, nt = material(name)
    p = nt.coord("Object")
    peel = nt.noise(p, scale=4200.0, detail=1.0)
    args = dict(Base_Color=(*tone, 1.0), Roughness=rough + peel * 0.08, Normal=nt.bump(peel, strength=0.05, distance=0.00005), Specular_IOR_Level=0.4)
    if sss > 0:
        args.update(Subsurface_Weight=sss, Subsurface_Radius=(0.0012, 0.0011, 0.001), Subsurface_Scale=1.0)
    nt.out_surface(nt.principled(**args))
    return mat


def led(name="led", power=1.0, tint=(1.0, 0.6, 0.28)):
    """A point of light: a fibre's end, a micro lamp."""
    mat, nt = material(name)
    g = grade(nt)
    em = nt.shader("ShaderNodeEmission", Color=(*tint, 1.0), Strength=g["lamps"] * power)
    nt.out_surface(nt.add_shader(nt.principled(Base_Color=(0.8, 0.8, 0.78, 1.0), Roughness=0.3, Transmission_Weight=0.5), em))
    return mat


def table(name="table", tone=(0.012, 0.012, 0.014), rough=0.16):
    """The studio table: satin, with the faint wipe marks of a cloth."""
    mat, nt = material(name)
    p = nt.coord("Object")
    x, y, z = nt.xyz(p)
    wipe = nt.noise(nt.vec(x * 2.0, y * 9.0, 0.0), scale=1.0, detail=4.0, rough=0.6)
    fine = nt.noise(p, scale=900.0, detail=2.0)
    specks = nt.voronoi(p, scale=420.0, out="Distance")
    speck = (1.0 - nt.smoothstep(0.015, 0.04, specks)) * nt.smoothstep(0.55, 0.8, nt.noise(p, scale=5.0, detail=2.0))
    col = nt.mixc(nt.rgb(tone), nt.rgb((0.3, 0.3, 0.29)), speck * 0.35)
    nt.out_surface(nt.principled(Base_Color=col, Roughness=rough + nt.smoothstep(0.35, 0.8, wipe) * 0.14 + fine * 0.03 + speck * 0.4, Specular_IOR_Level=0.5, Coat_Weight=0.0))
    return mat


def paper(name="sweep", tone=(0.03, 0.032, 0.036)):
    """The backdrop: a roll of seamless paper."""
    mat, nt = material(name)
    p = nt.coord("Object")
    grain = nt.noise(p, scale=1400.0, detail=2.0)
    col = nt.mixc(nt.rgb(tone), nt.rgb((tone[0] * 0.86, tone[1] * 0.86, tone[2] * 0.86)), grain)
    nt.out_surface(nt.principled(Base_Color=col, Roughness=0.92, Specular_IOR_Level=0.2))
    return mat

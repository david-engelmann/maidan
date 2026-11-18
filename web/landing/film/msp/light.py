"""Light: the sky, the sun, and the grade of each time of day.

Four grades of one scene. The dark theme arrives at blue hour and closes at
night; the light theme arrives in the late afternoon and closes at golden
hour.
"""

import math

import bpy

from .common import NT, TAU, add_object, collection, set_grade

# The sun's azimuth is measured from north, clockwise; the camera looks north,
# so a sun in the west (270) rakes across the frame from the left.
GRADES = {
    # blue hour: the sun is on the horizon behind the west wall. The park is in
    # shadow with its lamps on; only the tallest tops still have the gold.
    "blue": dict(
        sun_az=283.0, sun_el=2.2, sun_strength=5.0, sun_color=(1.0, 0.42, 0.14), sun_angle=1.0,
        sky="dusk", sky_strength=6.0, zenith=(0.008, 0.022, 0.1), mid=(0.03, 0.065, 0.2), horizon=(0.5, 0.25, 0.14), ember=(0.62, 0.15, 0.04),
        glow_az=318.0, cloud=0.55, night=0.75, lamps=1.0, windows=0.2, wet=1.0, mist=0.9, exposure=0.0, look="AgX - Punchy",
        haze=(0.22, 0.2, 0.3), haze_amt=0.55, bloom=0.75, lift=(0.985, 0.995, 1.03), gain=(1.0, 0.99, 0.98),
    ),
    "night": dict(
        sun_az=283.0, sun_el=-8.0, sun_strength=0.0, sun_color=(1.0, 0.5, 0.2), sun_angle=1.0,
        sky="dusk", sky_strength=1.6, zenith=(0.006, 0.012, 0.05), mid=(0.02, 0.035, 0.1), horizon=(0.2, 0.11, 0.11), ember=(0.3, 0.08, 0.04),
        glow_az=318.0, cloud=0.4, night=1.0, lamps=1.0, windows=0.3, wet=1.0, mist=1.0, exposure=0.35, look="AgX - Punchy",
        haze=(0.07, 0.07, 0.13), haze_amt=0.5, bloom=0.9, lift=(0.98, 0.99, 1.04), gain=(1.0, 0.99, 0.97),
    ),
    # the light theme arrives late in the afternoon: a low warm sun out of the
    # south-west, raking across the plaza and through the crowns
    "gold": dict(
        sun_az=238.0, sun_el=15.0, sun_strength=5.2, sun_color=(1.0, 0.76, 0.5), sun_angle=1.4,
        sky="day", sky_strength=1.0, air=1.0, dust=2.6, ozone=1.2,
        cloud=0.52, night=0.0, lamps=0.0, windows=0.1, wet=0.0, mist=0.22, exposure=0.0, look="AgX - Medium High Contrast",
        haze=(0.95, 0.8, 0.62), haze_amt=0.42, bloom=0.3, lift=(1.0, 0.995, 1.03), gain=(1.03, 1.0, 0.95),
    ),
    # and closes at golden hour proper, the sun nearly down
    "sunset": dict(
        sun_az=242.0, sun_el=7.5, sun_strength=4.6, sun_color=(1.0, 0.6, 0.3), sun_angle=1.8,
        sky="day", sky_strength=1.0, air=1.0, dust=3.6, ozone=1.4,
        cloud=0.58, night=0.12, lamps=0.35, windows=0.14, wet=0.0, mist=0.3, exposure=0.2, look="AgX - Medium High Contrast",
        haze=(1.0, 0.72, 0.48), haze_amt=0.5, bloom=0.4, lift=(1.0, 0.99, 1.04), gain=(1.04, 1.0, 0.93),
    ),
}


def _dir(az_deg, el_deg):
    """The unit vector toward a point of the sky."""
    az = math.radians(az_deg)
    el = math.radians(el_deg)
    return (math.sin(az) * math.cos(el), math.cos(az) * math.cos(el), math.sin(el))


def _clouds(nt, d, amount, seed=0.0):
    """A deck of broken cloud, seen in true perspective: the view ray is
    carried out to a flat layer overhead, so the cloud closes up and flattens
    toward the horizon as real cloud does. Returns its cover (0..1), and how
    thin it is there (the thin edges are what light up)."""
    x, y, z = nt.xyz(d)
    zc = nt.max(z, 0.012)
    u = x / (zc + 0.11)
    v = y / (zc + 0.11)
    p = nt.vec(u * 0.34 + seed, v * 0.34 + 0.6 * seed, 0.0)
    big = nt.noise(p, scale=1.0, detail=5.0, rough=0.58, dist=0.6, dims="2D")
    fine = nt.noise(nt.vec(u * 1.7, v * 1.7 + 3.0, 0.0), scale=1.0, detail=4.0, rough=0.6, dims="2D")
    dens = big * 0.8 + fine * 0.2
    lo = 0.62 - 0.22 * amount
    cover = nt.smoothstep(lo, lo + 0.22, dens)
    core = nt.smoothstep(lo + 0.1, lo + 0.34, dens)
    # no cloud right at the horizon line: haze takes it
    cover = cover * nt.smoothstep(0.0, 0.06, z)
    return cover, core


def world(g):
    w = bpy.data.worlds.get("FilmWorld") or bpy.data.worlds.new("FilmWorld")
    bpy.context.scene.world = w
    nt = NT(w.node_tree)
    d = nt.vmath("NORMALIZE", nt.coord("Generated"))
    x, y, z = nt.xyz(d)
    if g["sky"] == "day":
        sky = nt.node("ShaderNodeTexSky", sky_type="MULTIPLE_SCATTERING")
        sky.sun_elevation = math.radians(g["sun_el"])
        # the sky texture turns its sun from +y toward -x
        sky.sun_rotation = math.radians(-g["sun_az"]) % TAU
        sky.sun_disc = False
        sky.air_density = g.get("air", 1.0)
        sky.aerosol_density = g.get("dust", 1.0)
        sky.ozone_density = g.get("ozone", 1.0)
        base = nt.v(sky.outputs[0])
        cover, core = _clouds(nt, d, g["cloud"])
        sx, sy, sz = _dir(g["sun_az"], g["sun_el"])
        toward = nt.sat((x * sx + y * sy + z * sz) * 0.5 + 0.5)
        warm = g["sun_color"]
        lit = nt.mixc(nt.rgb((0.9, 0.88, 0.86)), nt.rgb((1.0, warm[1] * 1.02, warm[2] * 1.05)), nt.pow(toward, 2.0))
        shade = nt.mixc(nt.rgb((0.36, 0.38, 0.47)), nt.rgb((0.5, 0.42, 0.42)), nt.pow(toward, 3.0))
        cloud_col = nt.mixc(lit, shade, core)
        # the sky's own brightness sets the cloud's, so the two always sit together
        bw = nt.node("ShaderNodeRGBToBW")
        nt.put(bw.inputs[0], base)
        lum = nt.v(bw.outputs[0])
        cloud_rgb = nt.vmath("SCALE", cloud_col, scale=nt.max(lum, 0.02) * 1.9 + 0.25 * g["sky_strength"])
        col = nt.mixc(base, cloud_rgb, cover * 0.92)
        strength = g["sky_strength"] * 0.06
    else:
        # dusk: a gradient we own. Deep blue overhead, peach then ember toward
        # the horizon, strongest where the sun went down.
        gx, gy, _ = _dir(g["glow_az"], 0.0)
        toward = nt.sat((x * gx + y * gy) / nt.sqrt(nt.max(x * x + y * y, 1e-4)) * 0.5 + 0.5)
        near = nt.pow(toward, 2.2)
        el = nt.math("ARCSINE", nt.sat(nt.abs(z))) / (math.pi / 2)
        up = nt.sat(el)
        band = nt.exp(-up / nt.mix(0.018, 0.05, near))
        peach = nt.exp(-up / nt.mix(0.04, 0.12, near))
        col = nt.mixc(nt.rgb(g["mid"]), nt.rgb(g["zenith"]), nt.smoothstep(0.08, 0.7, up))
        col = nt.mixc(col, nt.rgb(g["horizon"]), nt.sat(peach * nt.mix(0.3, 1.0, near)))
        col = nt.mixc(col, nt.rgb(g["ember"]), nt.sat(band * nt.mix(0.12, 0.85, near)))
        cover, core = _clouds(nt, d, g["cloud"], seed=2.3)
        # cloud at dusk: slate bodies, their thin edges and undersides catching the last light
        edge = cover * (1.0 - core)
        slate = nt.mixc(nt.rgb((0.03, 0.04, 0.09)), nt.rgb((0.12, 0.07, 0.1)), near)
        fire = nt.mixc(nt.rgb((0.3, 0.2, 0.3)), nt.rgb((1.0, 0.45, 0.25)), near)
        low = nt.exp(-up / 0.22)
        col = nt.mixc(col, slate, cover * 0.8)
        col = nt.mixc(col, fire, nt.sat(edge * nt.mix(0.25, 1.0, low) * nt.mix(0.4, 1.0, near)))
        # below the horizon: the same glow, sinking to dark, so nothing reflects a black void
        col = nt.mixc(col, nt.vmath("SCALE", col, scale=0.3), nt.smoothstep(0.0, -0.2, z))
        strength = g["sky_strength"] * 1.0
    bg = nt.shader("ShaderNodeBackground", Color=col, Strength=strength)
    out = nt.node("ShaderNodeOutputWorld")
    nt.put(out.inputs["Surface"], bg)
    w.mist_settings.start = 40.0
    w.mist_settings.depth = 1500.0
    w.mist_settings.falloff = "LINEAR"
    return w


def sun(g, col=None):
    col = col or bpy.data.collections.get("Light") or collection("Light")
    ob = bpy.data.objects.get("Sun")
    if ob is None:
        ob = add_object("Sun", bpy.data.lights.new("Sun", "SUN"), col)
    ld = ob.data
    ld.energy = g["sun_strength"]
    ld.color = g["sun_color"]
    ld.angle = math.radians(g["sun_angle"])
    # a sun lamp shines down its own -Z: turn that to point away from the sun
    from mathutils import Vector

    to_sun = Vector(_dir(g["sun_az"], g["sun_el"]))
    ob.rotation_mode = "QUATERNION"
    ob.rotation_quaternion = (-to_sun).to_track_quat("-Z", "Y")
    ob.hide_render = g["sun_strength"] <= 0
    return ob


def apply(name):
    g = GRADES[name]
    world(g)
    sun(g)
    set_grade(night=g["night"], lamps=g["lamps"], windows=g["windows"], wet=g["wet"], sun=1.0 if g["sun_strength"] > 0 else 0.0)
    # lamps that are light objects, not materials
    for ob in bpy.data.objects:
        if ob.type == "LIGHT" and "film_lamp" in ob:
            ob.data.energy = ob["film_lamp"] * g["lamps"]
            ob.hide_render = g["lamps"] <= 0
        if "mist_gain" in ob:
            ob["mist_gain"] = g.get("mist", 0.5)
    sc = bpy.context.scene
    sc.view_settings.view_transform = "AgX"
    sc.view_settings.look = g["look"]
    sc.view_settings.exposure = g["exposure"]
    return g

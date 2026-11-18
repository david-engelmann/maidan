"""The studio: a table that runs back into a paper sweep, and the lights.

Two set-ups of the one model. In the dark one the studio is dimmed: a cool
low key skims the roofs, a rim picks out the far edges, and the model's own
lamps do the rest. In the light one it is a daylight studio: one big soft
window, and a small warm light low at the back that rakes long shadows
across the square.
"""

import math
import os

import bpy
import numpy as np
from mathutils import Vector

from . import craft
from .common import MB, NT, TAU, add_object, collection, grade, material, revolve, set_grade

FLOOR_Z = -0.062  # where the plinth stands
CENTRE = Vector((0.0, 0.17, 0.02))  # the middle of the model, near enough
# the direction the hero camera looks, on the plan: lights are placed by it
VIEW_AZ = math.radians(192.0)

SETUPS = {
    "dark": dict(
        floor="dark", lamps=1.0, windows=0.7, hdri="studio_small_03", hdri_strength=0.035, hdri_turn=1.9,
        lights=[
            # (name, right, forward, up, width, height, power, colour, spread)
            ("key", 1.9, 2.0, 0.5, 1.8, 0.5, 105.0, (0.74, 0.85, 1.0), 1.0, True),
            ("rim", -1.9, 1.4, 0.7, 0.16, 1.4, 34.0, (0.76, 0.87, 1.0), 0.6, True),
            ("fill", 0.3, -2.2, 2.4, 2.2, 2.2, 6.5, (0.74, 0.84, 1.0), 1.0, True),
            # a small warm lamp for the brass: crowns should read as brass, not steel
            ("brass", 1.1, -1.9, 1.5, 0.5, 0.5, 7.5, (1.0, 0.86, 0.68), 0.3, True, (0.3, 0.2, 0.42)),
            # the sweep's own wash: a low glow behind the model, falling off to the left
            ("wash", 1.3, 2.6, 0.25, 2.6, 0.4, 7.0, (0.55, 0.68, 1.0), 0.8),
            # and a breath of light on the table in front, so the plinth has an edge
            ("table", -0.2, -1.6, 0.5, 1.6, 0.3, 2.6, (0.6, 0.72, 1.0), 0.7),
        ],
        exposure=0.0, look="AgX - Punchy", bloom=0.34, bloom_threshold=3.2, vignette=0.42, grain=0.018,
        lift=(0.99, 0.995, 1.02), gain=(1.0, 0.995, 0.985),
    ),
    # the close: the same studio, every lamp in the model on
    "dark_full": dict(
        floor="dark", lamps=1.18, windows=1.01, hdri="studio_small_03", hdri_strength=0.03, hdri_turn=1.9,
        lights=[
            ("key", 1.9, 2.0, 0.5, 1.8, 0.5, 70.0, (0.74, 0.85, 1.0), 1.0, True),
            ("rim", -1.9, 1.4, 0.7, 0.16, 1.4, 26.0, (0.76, 0.87, 1.0), 0.6, True),
            ("fill", 0.3, -2.2, 2.4, 2.2, 2.2, 4.5, (0.74, 0.84, 1.0), 1.0, True),
            ("brass", 1.1, -1.9, 1.5, 0.5, 0.5, 6.5, (1.0, 0.86, 0.68), 0.3, True, (0.3, 0.2, 0.42)),
            ("wash", 1.3, 2.6, 0.25, 2.6, 0.4, 6.0, (0.55, 0.68, 1.0), 0.8),
        ],
        exposure=0.0, look="AgX - Punchy", bloom=0.4, bloom_threshold=3.2, vignette=0.42, grain=0.018,
        lift=(0.99, 0.995, 1.02), gain=(1.0, 0.995, 0.985),
    ),
    "light": dict(
        floor="light", lamps=0.0, windows=0.0, hdri="studio_kontrast_04", hdri_strength=0.14, hdri_turn=0.4,
        lights=[
            ("key", -2.4, 0.4, 2.3, 3.0, 2.4, 150.0, (0.9, 0.95, 1.0), 1.0),
            ("kicker", 2.5, 2.5, 0.85, 0.22, 0.22, 260.0, (1.0, 0.7, 0.4), 0.45),
            ("fill", 1.0, -2.6, 1.4, 2.4, 1.6, 14.0, (1.0, 0.98, 0.96), 1.0),
        ],
        exposure=0.0, look="AgX - Medium High Contrast", bloom=0.12, bloom_threshold=3.0, vignette=0.22, grain=0.014,
        lift=(1.0, 1.0, 1.01), gain=(1.01, 1.0, 0.985),
    ),
}


def mat_floor(name, kind):
    """One surface from table to sweep: satin near the model, where it shows
    the plinth and the lights, and matt paper where it curves up."""
    mat, nt = material(name)
    p = nt.coord("Object")
    x, y, z = nt.xyz(p)
    r = nt.sqrt(x * x + y * y)
    far = nt.smoothstep(1.2, 2.6, r)
    wipe = nt.noise(nt.vec(x * 1.5, y * 6.0, 0.0), scale=1.0, detail=4.0, rough=0.6)
    fine = nt.noise(p, scale=1100.0, detail=2.0)
    specks = nt.voronoi(p, scale=380.0, out="Distance")
    speck = (1.0 - nt.smoothstep(0.012, 0.035, specks)) * nt.smoothstep(0.56, 0.8, nt.noise(p, scale=4.0, detail=2.0)) * (1.0 - far)
    if kind == "dark":
        tone = nt.mixc(nt.rgb((0.009, 0.0095, 0.011)), nt.rgb((0.012, 0.013, 0.016)), far)
        tone = nt.mixc(tone, nt.rgb((0.3, 0.3, 0.3)), speck * 0.3)
        rough = nt.mix(0.13 + nt.smoothstep(0.35, 0.8, wipe) * 0.03 + fine * 0.015, 0.9, far) + speck * 0.4
        spec = 0.5
    else:
        tone = nt.mixc(nt.rgb((0.6, 0.595, 0.58)), nt.rgb((0.63, 0.625, 0.61)), fine)
        rough = nt.mix(0.62, 0.9, far)
        spec = 0.3
    nt.out_surface(nt.principled(Base_Color=tone, Roughness=rough, Specular_IOR_Level=spec))
    return mat


def _cyc(mat, name, col):
    """A cyclorama: a floor that sweeps up into a wall all the way round."""
    mb = MB()
    prof = [(0.001, FLOOR_Z), (2.0, FLOOR_Z), (3.4, FLOOR_Z)]
    rad = 1.4
    for k in range(1, 17):
        a = k / 16 * math.pi / 2
        prof.append((3.4 + rad * math.sin(a), FLOOR_Z + rad * (1 - math.cos(a))))
    prof.append((4.8, FLOOR_Z + 5.0))
    v, q, uv = revolve(prof, 96, flip=True)
    mb.add(v + np.array([CENTRE.x, CENTRE.y, 0], np.float32), q, smooth=True)
    return add_object(name, mb.build(name, [mat]), col)


def build(col=None):
    col = col or collection("Studio")
    dark = _cyc(mat_floor("floor_dark", "dark"), "CycDark", col)
    light = _cyc(mat_floor("floor_light", "light"), "CycLight", col)
    dark["film_floor"] = "dark"
    light["film_floor"] = "light"
    return col


def _frame():
    f = Vector((-math.sin(VIEW_AZ), -math.cos(VIEW_AZ), 0.0))  # from the camera toward the model
    r = Vector((f.y, -f.x, 0.0))
    return r, f


def world(s):
    w = bpy.data.worlds.get("FilmWorld") or bpy.data.worlds.new("FilmWorld")
    bpy.context.scene.world = w
    nt = NT(w.node_tree)
    env = nt.node("ShaderNodeTexEnvironment")
    path = os.path.join(craft.assets(), "hdri", s["hdri"] + "_2k.hdr")
    env.image = bpy.data.images.load(path, check_existing=True)
    vec = nt.mapping(nt.coord("Generated"), rot=(0.0, 0.0, s["hdri_turn"]))
    nt.put(env.inputs["Vector"], vec)
    # the environment is for reflections and a breath of fill; the camera sees the sweep
    # drained of colour: the studio's own tint is no part of the picture
    grey = nt.hsv(nt.v(env.outputs["Color"]), s=0.15)
    bg = nt.shader("ShaderNodeBackground", Color=grey, Strength=s["hdri_strength"])
    out = nt.node("ShaderNodeOutputWorld")
    nt.put(out.inputs["Surface"], bg)
    return w


def apply(name):
    s = SETUPS[name]
    sc = bpy.context.scene
    col = bpy.data.collections.get("StudioLights") or collection("StudioLights")
    for ob in list(col.objects):
        bpy.data.objects.remove(ob, do_unlink=True)
    r, f = _frame()
    # the model's lights light the model: the table would only throw their
    # glare back at the lens. The floor has a wash of its own.
    lit = bpy.data.collections.get("LitByStudio")
    if lit is None:
        lit = bpy.data.collections.new("LitByStudio")
        for name_ in ("Model", "Square"):
            c = bpy.data.collections.get(name_)
            if c is not None:
                lit.children.link(c)
    for spec in s["lights"]:
        (lname, lr, lf, up, w, h, power, colour, spread) = spec[:9]
        only_model = len(spec) > 9 and spec[9]
        ld = bpy.data.lights.new("Studio_" + lname, "AREA")
        ld.shape = "RECTANGLE"
        ld.size = w
        ld.size_y = h
        ld.energy = power
        ld.color = colour
        ld.spread = math.radians(180 * spread)
        pos = CENTRE + r * lr + f * lf + Vector((0, 0, up))
        ob = add_object("Studio_" + lname, ld, col, loc=tuple(pos))
        ob.rotation_mode = "QUATERNION"
        aim_at = CENTRE if len(spec) < 11 else Vector(spec[10])
        ob.rotation_quaternion = (aim_at - pos).to_track_quat("-Z", "Y")
        if only_model:
            ob.light_linking.receiver_collection = lit
    for ob in bpy.data.objects:
        if "film_floor" in ob:
            ob.hide_render = ob["film_floor"] != s["floor"]
        if ob.type == "LIGHT" and "film_lamp" in ob:
            ob.data.energy = ob["film_lamp"] * s["lamps"]
            ob.hide_render = s["lamps"] <= 0
    world(s)
    set_grade(night=1.0 if s["floor"] == "dark" else 0.0, lamps=s["lamps"], windows=s["windows"], wet=0.0, sun=0.0)
    sc.view_settings.view_transform = "AgX"
    sc.view_settings.look = s["look"]
    sc.view_settings.exposure = s["exposure"]
    return s

"""The camera: the terrace view, the glide down into the plaza, and the rise
back out. Every shot is a position, a point to hold at a place in the frame,
and a lens."""

import math

import bpy
from mathutils import Vector

from . import site
from .common import add_object, collection

SENSOR = 36.0

# name: (position, target, where the target sits in frame (across, up), lens mm,
#        f-stop, focus distance or None for the target)
SHOTS = {
    # the terrace: the park as the city's room. The camera is held level and
    # the lens shifted down, as an architectural photographer would, so the
    # walls of the room stand upright.
    "hero": dict(pos=site.HERO_CAM, target=(0.0, 0.0, 1.0), at=(0.58, 0.2), lens=16.0, fstop=0.14, tilt=0.0),
    "hero_tall": dict(pos=(-2.0, -78.0, 52.0), target=(0.0, 0.0, 1.0), at=(0.5, 0.16), lens=23.5, fstop=0.14, tilt=0.0),
    # the close: the same terrace, a little wider, the plaza on the axis
    "close": dict(pos=(-2.0, -84.0, 56.0), target=(0.0, 0.0, 1.0), at=(0.5, 0.2), lens=15.0, fstop=0.14, tilt=0.0),
    "close_tall": dict(pos=(-2.0, -84.0, 56.0), target=(0.0, 0.0, 1.0), at=(0.5, 0.16), lens=22.0, fstop=0.14, tilt=0.0),
    # at plaza level, for looking hard at the fountain
    "plaza": dict(pos=(-6.5, -17.5, 2.3), target=(0.0, 0.0, 2.3), at=(0.56, 0.42), lens=32.0, fstop=2.0, tilt=1.0),
    "fountain": dict(pos=(-3.5, -11.5, 2.0), target=(0.0, 0.0, 2.5), at=(0.5, 0.45), lens=35.0, fstop=2.8, tilt=1.0),
    "trees": dict(pos=(-30.0, -34.0, 9.0), target=(-27.0, 18.0, 13.0), at=(0.5, 0.5), lens=35.0, fstop=4.0, tilt=1.0),
    "facade": dict(pos=(40.0, 60.0, 30.0), target=(104.0, 96.0, 34.0), at=(0.5, 0.5), lens=50.0, fstop=8.0, tilt=1.0),
}


def _tan(lens, aspect):
    """Half-angle tangents across and up for a lens on the 36 mm sensor."""
    if aspect >= 1:
        th = SENSOR / 2 / lens
        return th, th / aspect
    tv = SENSOR / 2 / lens
    return tv * aspect, tv


def aim(pos, target, at, lens, aspect, tilt=1.0):
    """Heading, pitch and lens shift that put `target` at frame position
    `at`. With tilt 1 the camera pitches to do it; with tilt 0 it stays level
    and the lens shifts instead; between, some of each."""
    th, tv = _tan(lens, aspect)
    d = Vector(target) - Vector(pos)
    heading = math.atan2(d.x, d.y) - math.atan((2 * at[0] - 1) * th)
    full = math.atan2(d.z, math.hypot(d.x, d.y)) - math.atan((2 * at[1] - 1) * tv)
    pitch = full * tilt
    # where the target falls with this pitch and no shift, against where it should
    below = math.atan2(d.z, math.hypot(d.x, d.y)) - pitch
    ndc = math.tan(below) / tv
    shift = (ndc - (2 * at[1] - 1)) / (2 * aspect if aspect >= 1 else 2.0)
    return heading, pitch, shift


def orient(ob, heading, pitch):
    fwd = Vector((math.sin(heading) * math.cos(pitch), math.cos(heading) * math.cos(pitch), math.sin(pitch)))
    ob.rotation_mode = "QUATERNION"
    ob.rotation_quaternion = fwd.to_track_quat("-Z", "Y")


def camera_object():
    ob = bpy.data.objects.get("FilmCamera")
    if ob is None:
        cam = bpy.data.cameras.new("FilmCamera")
        ob = add_object("FilmCamera", cam, collection("Camera"))
    cam = ob.data
    cam.sensor_width = SENSOR
    cam.sensor_fit = "AUTO"
    cam.clip_start = 0.3
    cam.clip_end = 6000.0
    bpy.context.scene.camera = ob
    return ob


def set_shot(name, aspect, t=None):
    """Places the camera for a named shot (or, for the moves, at time t in 0..1)."""
    ob = camera_object()
    cam = ob.data
    if name in ("enter", "leave"):
        s = journey(name, t or 0.0, aspect)
    else:
        key = name + "_tall" if aspect < 1 and name + "_tall" in SHOTS else name
        s = dict(SHOTS[key])
        s["heading"], s["pitch"], s["shift"] = aim(s["pos"], s["target"], s["at"], s["lens"], aspect, s.get("tilt", 1.0))
        s["focus"] = (Vector(s["target"]) - Vector(s["pos"])).length
    ob.location = s["pos"]
    orient(ob, s["heading"], s["pitch"])
    cam.lens = s["lens"]
    cam.shift_x = 0.0
    cam.shift_y = s["shift"]
    cam.dof.use_dof = True
    cam.dof.focus_distance = s["focus"]
    cam.dof.aperture_fstop = s["fstop"]
    cam.dof.aperture_blades = 9
    return s


def _ease(x):
    x = max(0.0, min(1.0, x))
    return x * x * (3 - 2 * x)


def _lerp(a, b, k):
    return a + (b - a) * k


def journey(name, t, aspect):
    """The scroll moves. `enter` glides from the terrace down between the
    crowns, across the plaza and into the fountain's spray; `leave` comes out
    of the mist at plaza level and rises back to the wide view."""
    tall = aspect < 1
    a = dict(SHOTS["hero_tall" if tall else "hero"])
    z = dict(SHOTS["close_tall" if tall else "close"])
    # where the move ends (or the leave begins): close on the falls, in the mist
    mist_pos = Vector((1.2, -5.2, 2.75))
    mist_target = Vector((0.0, 0.0, 3.3))
    if name == "enter":
        k = _ease(t)
        p0 = Vector(a["pos"])
        # a drone's line: forward first, the descent steepening, flattening out over the plaza
        along = k
        drop = _ease(min(1.0, t * 1.08)) ** 1.15
        pos = Vector((_lerp(p0.x, mist_pos.x, along), _lerp(p0.y, mist_pos.y, along ** 0.9), _lerp(p0.z, mist_pos.z, drop)))
        target = Vector(a["target"]).lerp(mist_target, _ease(t * 1.1))
        at = (_lerp(a["at"][0], 0.5, k), _lerp(a["at"][1], 0.5, _ease(t * 1.3)))
        lens = _lerp(a["lens"], 30.0, k)
        fstop = _lerp(a["fstop"], 1.6, _ease((t - 0.45) / 0.5)) if t > 0.45 else a["fstop"]
    else:
        k = _ease(t)
        p1 = Vector(z["pos"])
        rise = _ease(t * 1.05) ** 0.85
        back = k ** 1.1
        pos = Vector((_lerp(mist_pos.x, p1.x, back), _lerp(mist_pos.y, p1.y, back), _lerp(mist_pos.z, p1.z, rise)))
        target = mist_target.lerp(Vector(z["target"]), _ease(t * 1.2))
        at = (_lerp(0.5, z["at"][0], k), _lerp(0.5, z["at"][1], _ease((t - 0.2) / 0.8)))
        lens = _lerp(30.0, z["lens"], k)
        fstop = _lerp(1.6, z["fstop"], _ease(t / 0.5)) if t < 0.5 else z["fstop"]
    # the lens shift gives way to a true tilt as the camera comes down
    tilt = _ease(t / 0.55) if name == "enter" else 1.0 - _ease((t - 0.45) / 0.55)
    heading, pitch, shift = aim(pos, target, at, lens, aspect, tilt)
    return dict(pos=tuple(pos), heading=heading, pitch=pitch, shift=shift, lens=lens, fstop=fstop, focus=(target - pos).length)

"""The camera: a macro lens on a real model. Every shot is where the
camera stands (a bearing, a height angle and a distance from what it looks
at), where that point sits in the frame, and a lens and stop. Distances are
metres at the model's own size.
"""

import math

import bpy
from mathutils import Vector

from .common import add_object, collection

SENSOR = 36.0
FOUNTAIN = Vector((0.0, 0.0, 0.004))

# look-at point, bearing of the camera from it (degrees, clockwise from
# north), height angle, distance, frame position of the point, lens, stop
SHOTS = {
    # arrive: the whole model, three-quarter, the square right of centre
    "hero": dict(at=FOUNTAIN, az=192.0, el=35.0, dist=3.05, frame=(0.605, 0.38), lens=50.0, fstop=4.0, tilt=0.5),
    "hero_tall": dict(at=FOUNTAIN, az=188.0, el=40.0, dist=3.5, frame=(0.5, 0.6), lens=60.0, fstop=4.0, tilt=0.5),
    # the close: a little further round and a little wider, every lamp on
    "close": dict(at=FOUNTAIN, az=202.0, el=38.0, dist=3.05, frame=(0.5, 0.4), lens=50.0, fstop=4.0, tilt=0.5),
    "close_tall": dict(at=FOUNTAIN, az=196.0, el=42.0, dist=3.7, frame=(0.5, 0.6), lens=58.0, fstop=4.0, tilt=0.5),
    # where the glide into the square ends: low over the plaza, on the fountain
    "plaza": dict(at=Vector((0.0, 0.0, 0.008)), az=179.0, el=19.0, dist=0.195, frame=(0.5, 0.46), lens=90.0, fstop=8.0, tilt=1.0),
    # for looking hard at the craft
    "fountain": dict(at=Vector((0.0, 0.0, 0.006)), az=178.0, el=24.0, dist=0.24, frame=(0.5, 0.45), lens=100.0, fstop=8.0, tilt=1.0),
    "trees": dict(at=Vector((-0.085, 0.235, 0.028)), az=112.0, el=13.0, dist=0.26, frame=(0.5, 0.5), lens=100.0, fstop=5.6, tilt=1.0),
    "facade": dict(at=Vector((0.262, 0.195, 0.075)), az=258.0, el=9.0, dist=0.42, frame=(0.5, 0.5), lens=100.0, fstop=5.6, tilt=1.0),
    "crown": dict(at=Vector((0.2925, 0.0325, 0.438)), az=205.0, el=6.0, dist=0.34, frame=(0.5, 0.42), lens=100.0, fstop=5.6, tilt=1.0),
}


def _tan(lens, aspect):
    """Half-angle tangents across and up for a lens on the 36 mm sensor."""
    if aspect >= 1:
        th = SENSOR / 2 / lens
        return th, th / aspect
    tv = SENSOR / 2 / lens
    return tv * aspect, tv


def place(at, az, el, dist):
    a = math.radians(az)
    e = math.radians(el)
    return Vector(at) + Vector((math.sin(a) * math.cos(e), math.cos(a) * math.cos(e), math.sin(e))) * dist


def aim(pos, target, frame, lens, aspect, tilt=1.0):
    """Heading, pitch and lens shift that put `target` at `frame`. With tilt
    1 the camera pitches to do it; below 1 it pitches less and the lens
    shifts for the rest, as a view camera does, which keeps the towers
    upright."""
    th, tv = _tan(lens, aspect)
    d = Vector(target) - Vector(pos)
    heading = math.atan2(d.x, d.y) - math.atan((2 * frame[0] - 1) * th)
    down = math.atan2(d.z, math.hypot(d.x, d.y))
    pitch = (down - math.atan((2 * frame[1] - 1) * tv)) * tilt
    ndc = math.tan(down - pitch) / tv
    shift = (ndc - (2 * frame[1] - 1)) / (2 * aspect if aspect >= 1 else 2.0)
    return heading, pitch, shift


def camera_object():
    ob = bpy.data.objects.get("FilmCamera")
    if ob is None:
        cam = bpy.data.cameras.new("FilmCamera")
        ob = add_object("FilmCamera", cam, collection("Camera"))
    cam = ob.data
    cam.sensor_width = SENSOR
    cam.sensor_fit = "AUTO"
    cam.clip_start = 0.01
    cam.clip_end = 40.0
    bpy.context.scene.camera = ob
    return ob


def _resolve(name, aspect):
    key = name + "_tall" if aspect < 1 and name + "_tall" in SHOTS else name
    s = dict(SHOTS[key])
    s["pos"] = place(s["at"], s["az"], s["el"], s["dist"])
    s["focus_at"] = Vector(s["at"])
    return s


def _ease(x):
    x = max(0.0, min(1.0, x))
    return x * x * (3 - 2 * x)


def _lerp(a, b, k):
    return a + (b - a) * k


def journey(name, t, aspect):
    """The scroll moves. `enter` glides from the three-quarter view forward
    and down across the roofs into the square, the focus coming onto the
    figures and the fountain. `leave` is the way back out, to the close."""
    a = _resolve("hero" if name == "enter" else "close", aspect)
    b = _resolve("plaza", aspect)
    k = _ease(t) if name == "enter" else _ease(1.0 - t)
    # the path is drawn in bearing, height angle and distance, so it sweeps
    # round and down rather than cutting straight through the model
    e = k ** 0.9
    az = _lerp(a["az"], b["az"], _ease(k))
    el = _lerp(a["el"], b["el"], e ** 1.25)
    dist = math.exp(_lerp(math.log(a["dist"]), math.log(b["dist"]), e))
    at = Vector(a["at"]).lerp(Vector(b["at"]), e)
    return dict(
        at=at, pos=place(at, az, el, dist), focus_at=at,
        frame=(_lerp(a["frame"][0], b["frame"][0], e), _lerp(a["frame"][1], b["frame"][1], e)),
        lens=_lerp(a["lens"], b["lens"], e), fstop=_lerp(a["fstop"], b["fstop"], e), tilt=_lerp(a["tilt"], b["tilt"], _ease(k * 1.6)),
    )


def set_shot(name, aspect, t=0.0):
    ob = camera_object()
    cam = ob.data
    s = journey(name, t, aspect) if name in ("enter", "leave") else _resolve(name, aspect)
    heading, pitch, shift = aim(s["pos"], s["at"], s["frame"], s["lens"], aspect, s.get("tilt", 1.0))
    ob.location = s["pos"]
    fwd = Vector((math.sin(heading) * math.cos(pitch), math.cos(heading) * math.cos(pitch), math.sin(pitch)))
    ob.rotation_mode = "QUATERNION"
    ob.rotation_quaternion = fwd.to_track_quat("-Z", "Y")
    cam.lens = s["lens"]
    cam.shift_x = 0.0
    cam.shift_y = shift
    cam.dof.use_dof = True
    # focus is the distance along the lens axis to the plane the point lies in
    cam.dof.focus_distance = max(0.02, (Vector(s["focus_at"]) - Vector(s["pos"])).dot(fwd))
    cam.dof.aperture_fstop = s["fstop"]
    cam.dof.aperture_blades = 9
    cam.dof.aperture_rotation = 0.3
    return s

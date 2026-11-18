"""Traffic: cars, yellow cabs and a bus on the streets round the park, and
cars parked along the kerbs."""

import math
import os

import bpy
import numpy as np

from . import site
from .common import MB, TAU, add_object, boxes, collection, grade, material, revolve, rng
from .ground import ROAD_Z

CAB = (0.9, 0.47, 0.012)
PAINTS = [(0.012, 0.012, 0.014), (0.02, 0.02, 0.024), (0.45, 0.46, 0.47), (0.62, 0.62, 0.6), (0.16, 0.17, 0.18), (0.03, 0.05, 0.11), (0.2, 0.02, 0.02), (0.07, 0.09, 0.08), (0.3, 0.27, 0.22)]


def mat_paint(name="car_paint"):
    mat, nt = material(name)
    col = nt.objinfo("Color")
    nt.out_surface(nt.principled(Base_Color=col, Metallic=0.35, Roughness=0.32, Coat_Weight=0.8, Coat_Roughness=0.06))
    return mat


def mat_carglass(name="car_glass"):
    mat, nt = material(name)
    nt.out_surface(nt.principled(Base_Color=(0.012, 0.014, 0.016, 1.0), Roughness=0.04, Specular_IOR_Level=1.0, Coat_Weight=1.0, Coat_Roughness=0.02))
    return mat


def mat_light(name, tone, power):
    mat, nt = material(name)
    g = grade(nt)
    em = nt.shader("ShaderNodeEmission", Color=(*tone, 1.0), Strength=0.3 + g["lamps"] * power)
    nt.out_surface(nt.add_shader(nt.principled(Base_Color=(*tone, 1.0), Roughness=0.2), em))
    return mat


def car_mesh(name, mats, L=4.6, W=1.82, H=1.44, hood=0.24, cabin=(0.27, 0.74), belt=0.78, ride=0.2, roof_sign=False, boxy=0.0):
    """A car lofted from sections along its length: sills, a belt line, and a
    cabin of glass narrowing to the roof. It faces +y."""
    mb = MB()
    n = 22
    ys = np.linspace(-L / 2, L / 2, n)
    t = (ys + L / 2) / L
    # plan: the body narrows a little to the nose and tail
    half = W / 2 * (1 - (0.14 - 0.1 * boxy) * np.abs(2 * t - 1) ** 3.0)
    # the belt drops toward the nose
    belt_z = belt * (1 - 0.1 * np.clip((t - 0.7) / 0.3, 0, 1) ** 2) * np.ones(n)
    nose = np.clip(np.minimum(t, 1 - t) / 0.06, 0, 1) ** 0.5
    belt_z = ride + (belt_z - ride) * (0.72 + 0.28 * nose)
    c0, c1 = cabin
    rise = np.clip(np.minimum((t - c0) / (0.16 - 0.1 * boxy), (c1 - t) / (0.2 - 0.14 * boxy)), 0, 1)
    rise = rise * rise * (3 - 2 * rise)
    roof_z = belt_z + (H - belt) * rise
    roof_half = half * (1 - (0.2 - 0.12 * boxy) * rise)
    rows = []
    for k in range(n):
        h, b, rz, rh = half[k], belt_z[k], roof_z[k], roof_half[k]
        rows.append([(-h * 0.92, ys[k], ride), (-h, ys[k], ride + 0.16), (-h, ys[k], b), (-rh, ys[k], rz), (0.0, ys[k], rz + 0.035 * rise[k]), (rh, ys[k], rz), (h, ys[k], b), (h, ys[k], ride + 0.16), (h * 0.92, ys[k], ride)])
    v = np.array(rows).reshape(-1, 3)
    m = 9
    quads = []
    mat = []
    for k in range(n - 1):
        for j in range(m - 1):
            quads.append([k * m + j, (k + 1) * m + j, (k + 1) * m + j + 1, k * m + j + 1])
            glassy = j in (2, 5) and rise[k] > 0.02 and rise[k + 1] > 0.02
            # the windscreen and the rear window are the cabin's sloping ends
            ends = j in (3, 4) and 0.02 < (rise[k] + rise[k + 1]) / 2 < 0.98
            mat.append(1 if glassy or ends else 0)
    mb.add(v, np.array(quads), mat=np.array(mat), smooth=True)
    for k in (0, n - 1):
        mb.add(v, [[k * m + j for j in range(m)][:: (1 if k else -1)]], mat=0)
    # wheels
    for wy in (-L * 0.3, L * 0.31):
        for wx in (-1, 1):
            prof = [(0.001, 0.0), (0.22, 0.0), (0.33, 0.03), (0.33, 0.2), (0.22, 0.23), (0.001, 0.23)]
            wv, wq, _ = revolve(prof, 14)
            wv = wv[:, [2, 1, 0]].copy()  # the axle along x
            wv[:, 0] = wx * (W / 2 - 0.23) + (wv[:, 0] if wx > 0 else -wv[:, 0])
            wv[:, 1] += wy
            wv[:, 2] += 0.33
            mb.add(wv, wq, mat=2, smooth=True)
    # lamps
    lo = [(-W / 2 + 0.12, L / 2 - 0.05, belt_z[-1] - 0.16), (W / 2 - 0.42, L / 2 - 0.05, belt_z[-1] - 0.16)]
    hi = [(-W / 2 + 0.42, L / 2 + 0.012, belt_z[-1] - 0.05), (W / 2 - 0.12, L / 2 + 0.012, belt_z[-1] - 0.05)]
    hv, hq = boxes(np.array(lo), np.array(hi))
    mb.add(hv, hq, mat=3)
    lo = [(-W / 2 + 0.1, -L / 2 - 0.012, belt_z[0] - 0.15), (W / 2 - 0.4, -L / 2 - 0.012, belt_z[0] - 0.15)]
    hi = [(-W / 2 + 0.4, -L / 2 + 0.05, belt_z[0] - 0.05), (W / 2 - 0.1, -L / 2 + 0.05, belt_z[0] - 0.05)]
    tv, tq = boxes(np.array(lo), np.array(hi))
    mb.add(tv, tq, mat=4)
    if roof_sign:
        sv, sq = boxes(np.array([(-0.32, -0.1, H + 0.0)]), np.array([(0.32, 0.1, H + 0.14)]))
        mb.add(sv, sq, mat=5)
    return mb.build(name, mats)


def bus_mesh(name, mats):
    mb = MB()
    L, W, H = 12.2, 2.55, 3.15
    lo = [(-W / 2, -L / 2, 0.32), (-W / 2 + 0.25, -L / 2 + 1.0, H), (-W / 2 - 0.01, -L / 2 + 0.4, 1.25), (-W / 2 + 0.3, L / 2 - 0.01, 1.2)]
    hi = [(W / 2, L / 2, H), (W / 2 - 0.25, L / 2 - 3.0, H + 0.28), (W / 2 + 0.01, L / 2 - 0.5, 2.35), (W / 2 - 0.3, L / 2 + 0.012, 2.5)]
    v, q = boxes(np.array(lo[:2]), np.array(hi[:2]))
    mb.add(v, q, mat=0)
    v, q = boxes(np.array(lo[2:]), np.array(hi[2:]))
    mb.add(v, q, mat=1)
    for wy in (-L * 0.3, L * 0.33):
        for wx in (-1, 1):
            prof = [(0.001, 0.0), (0.3, 0.0), (0.48, 0.03), (0.48, 0.28), (0.3, 0.3), (0.001, 0.3)]
            wv, wq, _ = revolve(prof, 14)
            wv = wv[:, [2, 1, 0]].copy()
            wv[:, 0] = wx * (W / 2 - 0.3) + (wv[:, 0] if wx > 0 else -wv[:, 0])
            wv[:, 1] += wy
            wv[:, 2] += 0.48
            mb.add(wv, wq, mat=2, smooth=True)
    hv, hq = boxes(np.array([(-W / 2 + 0.2, L / 2 - 0.02, 0.6), (W / 2 - 0.55, L / 2 - 0.02, 0.6)]), np.array([(-W / 2 + 0.55, L / 2 + 0.02, 0.8), (W / 2 - 0.2, L / 2 + 0.02, 0.8)]))
    mb.add(hv, hq, mat=3)
    tv, tq = boxes(np.array([(-W / 2 + 0.15, -L / 2 - 0.02, 0.7), (W / 2 - 0.45, -L / 2 - 0.02, 0.7)]), np.array([(-W / 2 + 0.45, -L / 2 + 0.02, 1.0), (W / 2 - 0.15, -L / 2 + 0.02, 1.0)]))
    mb.add(tv, tq, mat=4)
    return mb.build(name, mats)


def lanes():
    """The lanes cars drive in: (start, end) of each, in the direction of travel."""
    x0, y0, x1, y1 = site.PARK
    sw, rd = site.SIDEWALK, site.ROAD
    out = []
    # the east avenue runs north, the west avenue south; the streets run both ways
    for k in (0, 1, 2):
        x = x1 + sw + rd * (k + 1.5) / 5
        out.append(((x, -260.0), (x, 620.0)))
        x = x0 - sw - rd * (k + 1.5) / 5
        out.append(((x, 620.0), (x, -260.0)))
    for edge, sgn in ((y0 - sw, -1), (y1 + sw, 1)):
        ya = edge + sgn * rd * 0.3
        yb = edge + sgn * rd * 0.7
        out.append(((-300.0, ya) if sgn < 0 else (300.0, ya), (300.0, ya) if sgn < 0 else (-300.0, ya)))
        out.append(((300.0, yb) if sgn < 0 else (-300.0, yb), (-300.0, yb) if sgn < 0 else (300.0, yb)))
    return out


def build(col=None):
    col = col or collection("Traffic")
    r = rng(31)
    q = float(os.environ.get("FILM_QUALITY", "1"))
    paint = mat_paint()
    glass = mat_carglass()
    tyre, nt = material("tyre")
    nt.out_surface(nt.principled(Base_Color=(0.015, 0.015, 0.016, 1.0), Roughness=0.8))
    head = mat_light("headlamp", (1.0, 0.93, 0.8), 60.0)
    tail = mat_light("taillamp", (1.0, 0.03, 0.01), 14.0)
    sign = mat_light("cab_sign", (1.0, 0.8, 0.4), 8.0)
    mats = [paint, glass, tyre, head, tail, sign]
    kinds = {
        "sedan": car_mesh("Car_sedan", mats),
        "cab": car_mesh("Car_cab", mats, roof_sign=True),
        "suv": car_mesh("Car_suv", mats, L=4.8, W=1.92, H=1.72, cabin=(0.2, 0.9), belt=0.95, boxy=0.7),
        "van": car_mesh("Car_van", mats, L=5.4, W=2.0, H=2.2, cabin=(0.04, 0.97), belt=1.2, boxy=1.0),
        "bus": bus_mesh("Bus", mats),
    }
    count = 0

    def put(kind, x, y, heading, colour, moving=True):
        nonlocal count
        ob = add_object(f"Car{count:03d}", kinds[kind], col, loc=(x, y, ROAD_Z), rot=(0, 0, heading))
        ob.color = (*colour, 1.0)
        ob["film_moving"] = 1 if moving else 0
        count += 1
        if moving and -90 < y < 330 and abs(x) < 140:
            ld = bpy.data.lights.new(f"CarBeam{count:03d}", "SPOT")
            ld.energy = 900.0
            ld.color = (1.0, 0.93, 0.8)
            ld.spot_size = 1.1
            ld.spot_blend = 0.7
            ld.shadow_soft_size = 0.1
            lo = add_object(f"CarBeam{count:03d}", ld, col, parent=ob, loc=(0.0, 2.2, 0.7))
            lo.rotation_euler = (math.radians(82), 0, 0)
            lo["film_lamp"] = 900.0

    for (ax, ay), (bx, by) in lanes():
        d = np.array([bx - ax, by - ay])
        length = np.linalg.norm(d)
        d /= length
        heading = math.atan2(-d[0], d[1])
        s = r.uniform(0, 20)
        while s < length - 10:
            u = r.random()
            if u < 0.03:
                kind, colour, gap = "bus", (0.55, 0.57, 0.6), 13.0
            elif u < 0.3:
                kind, colour, gap = "cab", CAB, 5.0
            elif u < 0.5:
                kind, colour, gap = "suv", PAINTS[int(r.integers(len(PAINTS)))], 5.2
            elif u < 0.56:
                kind, colour, gap = "van", (0.6, 0.6, 0.58), 6.0
            else:
                kind, colour, gap = "sedan", PAINTS[int(r.integers(len(PAINTS)))], 5.0
            p = np.array([ax, ay]) + d * s
            put(kind, p[0], p[1], heading, colour)
            # traffic bunches: a few close together, then a gap
            s += gap + (r.uniform(2.5, 7.0) if r.random() < 0.55 else r.uniform(18.0, 60.0) / max(q, 0.3))
    # parked along the kerb, park side and far side
    x0, y0, x1, y1 = site.PARK
    sw, rd = site.SIDEWALK, site.ROAD
    for x, heading in ((x1 + sw + 1.15, 0.0), (x1 + sw + rd - 1.15, 0.0), (x0 - sw - 1.15, math.pi), (x0 - sw - rd + 1.15, math.pi)):
        y = -250.0
        while y < 600:
            if r.random() < 0.72 and not (y0 - 30 < y < y0 - 2) and not (y1 + 2 < y < y1 + 30):
                kind = "suv" if r.random() < 0.3 else "sedan"
                put(kind, x, y, heading, PAINTS[int(r.integers(len(PAINTS)))], moving=False)
            y += r.uniform(5.6, 6.6)
    print(f"[traffic] {count} vehicles")
    return col

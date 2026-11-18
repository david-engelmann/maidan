"""Shared helpers: a numpy mesh accumulator, primitive shapes, a small
expression layer over shader nodes, and collection plumbing.

Everything in the film is generated from fixed seeds, so a rebuild gives the
same scene.
"""

import math

import bpy
import numpy as np

TAU = math.tau


def rng(seed):
    return np.random.default_rng(seed)


# ---------------------------------------------------------------- collections


def collection(name, parent=None):
    """The named collection, emptied if it already exists."""
    col = bpy.data.collections.get(name)
    if col is None:
        col = bpy.data.collections.new(name)
        (parent or bpy.context.scene.collection).children.link(col)
    else:
        for ob in list(col.objects):
            bpy.data.objects.remove(ob, do_unlink=True)
        for child in list(col.children):
            for ob in list(child.objects):
                bpy.data.objects.remove(ob, do_unlink=True)
            bpy.data.collections.remove(child)
    return col


def purge():
    """Drops meshes, materials and node groups nothing uses any more."""
    for _ in range(3):
        for block in (bpy.data.meshes, bpy.data.curves, bpy.data.lights, bpy.data.cameras):
            for item in list(block):
                if item.users == 0:
                    block.remove(item)


def add_object(name, data, col, loc=(0, 0, 0), rot=(0, 0, 0), scale=(1, 1, 1), parent=None):
    ob = bpy.data.objects.new(name, data)
    ob.location = loc
    ob.rotation_euler = rot
    ob.scale = scale
    if parent is not None:
        ob.parent = parent
    col.objects.link(ob)
    return ob


# ---------------------------------------------------------------------- meshes


class MB:
    """Accumulates polygons, then makes one mesh of them.

    Each `add` takes its own vertices and faces (indices into those
    vertices), a material slot, and optional per-vertex UVs and per-vertex or
    per-face attributes.
    """

    def __init__(self):
        self.vs = []
        self.nv = 0
        self.chunks = []  # (faces, mat, smooth)
        self.uvs = []
        self.vattr = {}
        self.fattr = {}
        self.nf = 0

    def add(self, v, f, mat=0, smooth=False, uv=None, vattr=None, fattr=None):
        v = np.asarray(v, np.float32).reshape(-1, 3)
        f = np.asarray(f, np.int64)
        if f.ndim == 1:
            f = f.reshape(1, -1)
        if len(v) == 0 or len(f) == 0:
            return
        n = len(v)
        m = len(f)
        self.chunks.append((f + self.nv, mat, smooth))
        self.vs.append(v)
        self.uvs.append(np.zeros((n, 2), np.float32) if uv is None else np.asarray(uv, np.float32).reshape(n, 2))
        for store, given, count, start in ((self.vattr, vattr, n, self.nv), (self.fattr, fattr, m, self.nf)):
            for key, val in (given or {}).items():
                val = np.asarray(val, np.float32)
                if val.ndim == 0 or (val.ndim == 1 and len(val) != count):
                    val = np.broadcast_to(val, (count,) + val.shape).copy()
                store.setdefault(key, []).append((start, val))
        self.nv += n
        self.nf += m

    def extend(self, other, matrix=None, mat_offset=0):
        """Appends another accumulator, optionally transformed."""
        base_v = self.nv
        base_f = self.nf
        for v in other.vs:
            self.vs.append(xform(v, matrix) if matrix is not None else v)
        self.uvs.extend(other.uvs)
        for f, mat, smooth in other.chunks:
            mm = mat + mat_offset if np.isscalar(mat) else np.asarray(mat) + mat_offset
            self.chunks.append((f + base_v, mm, smooth))
        for mine, theirs, base in ((self.vattr, other.vattr, base_v), (self.fattr, other.fattr, base_f)):
            for key, parts in theirs.items():
                mine.setdefault(key, []).extend((start + base, val) for start, val in parts)
        self.nv += other.nv
        self.nf += other.nf

    def build(self, name, mats=(), smooth_angle=None):
        me = bpy.data.meshes.new(name)
        if not self.vs:
            return me
        V = np.concatenate(self.vs)
        loops = np.concatenate([f.ravel() for f, _, _ in self.chunks]).astype(np.int32)
        totals = np.concatenate([np.full(len(f), f.shape[1], np.int32) for f, _, _ in self.chunks])
        starts = np.concatenate(([0], np.cumsum(totals)[:-1])).astype(np.int32)
        mat = np.concatenate([np.broadcast_to(np.asarray(m, np.int32), (len(f),)) for f, m, _ in self.chunks]).astype(np.int32)
        smooth = np.concatenate([np.full(len(f), bool(s)) for f, _, s in self.chunks])
        me.vertices.add(len(V))
        me.vertices.foreach_set("co", V.ravel())
        me.loops.add(len(loops))
        me.loops.foreach_set("vertex_index", loops)
        me.polygons.add(len(totals))
        me.polygons.foreach_set("loop_start", starts)
        me.polygons.foreach_set("material_index", mat)
        me.polygons.foreach_set("use_smooth", smooth)
        uv = np.concatenate(self.uvs)
        layer = me.uv_layers.new(name="UVMap")
        layer.uv.foreach_set("vector", uv[loops].ravel())
        for store, domain, count in ((self.vattr, "POINT", len(V)), (self.fattr, "FACE", len(totals))):
            for key, parts in store.items():
                width = 1 if parts[0][1].ndim == 1 else parts[0][1].shape[1]
                full = np.zeros((count, width) if width > 1 else (count,), np.float32)
                for start, val in parts:
                    full[start:start + len(val)] = val
                if width == 1:
                    a = me.attributes.new(key, "FLOAT", domain)
                    a.data.foreach_set("value", full)
                else:
                    if width == 3:
                        full = np.concatenate([full, np.ones((count, 1), np.float32)], axis=1)
                    a = me.attributes.new(key, "FLOAT_COLOR", domain)
                    a.data.foreach_set("color", full.ravel())
        for m in mats:
            me.materials.append(m)
        me.update(calc_edges=True)
        if smooth_angle is not None:
            me.set_sharp_from_angle(angle=smooth_angle)
        return me


def xform(v, m):
    """Applies a 4x4 (or 3x3) matrix to an (n, 3) array."""
    m = np.asarray(m, np.float32)
    if m.shape == (3, 3):
        return v @ m.T
    return v @ m[:3, :3].T + m[:3, 3]


def rot_z(a):
    c, s = math.cos(a), math.sin(a)
    return np.array([[c, -s, 0, 0], [s, c, 0, 0], [0, 0, 1, 0], [0, 0, 0, 1]], np.float32)


def translate(x, y, z):
    m = np.eye(4, dtype=np.float32)
    m[:3, 3] = (x, y, z)
    return m


def scale_m(x, y, z):
    return np.diag([x, y, z, 1]).astype(np.float32)


def box(lo, hi):
    """An axis-aligned box: 8 vertices, 6 outward quads."""
    x0, y0, z0 = lo
    x1, y1, z1 = hi
    v = np.array([[x0, y0, z0], [x1, y0, z0], [x1, y1, z0], [x0, y1, z0], [x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]], np.float32)
    f = np.array([[0, 3, 2, 1], [4, 5, 6, 7], [0, 1, 5, 4], [1, 2, 6, 5], [2, 3, 7, 6], [3, 0, 4, 7]])
    return v, f


def boxes(lo, hi):
    """Many boxes at once: lo and hi are (n, 3)."""
    lo = np.asarray(lo, np.float32).reshape(-1, 3)
    hi = np.asarray(hi, np.float32).reshape(-1, 3)
    n = len(lo)
    sel = np.array([[0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0], [0, 0, 1], [1, 0, 1], [1, 1, 1], [0, 1, 1]], np.float32)
    v = lo[:, None, :] * (1 - sel)[None] + hi[:, None, :] * sel[None]
    f = np.array([[0, 3, 2, 1], [4, 5, 6, 7], [0, 1, 5, 4], [1, 2, 6, 5], [2, 3, 7, 6], [3, 0, 4, 7]])
    f = f[None] + (np.arange(n) * 8)[:, None, None]
    return v.reshape(-1, 3), f.reshape(-1, 4)


def revolve(profile, segs, a0=0.0, a1=TAU, close=None, flip=False):
    """Lathes an (r, z) profile round the z axis. Returns vertices, quads and
    UVs (u round, v along the profile in metres)."""
    p = np.asarray(profile, np.float32)
    k = len(p)
    full = abs((a1 - a0) - TAU) < 1e-6 if close is None else close
    na = segs if full else segs + 1
    ang = a0 + (a1 - a0) * np.arange(na) / segs
    c, s = np.cos(ang), np.sin(ang)
    v = np.stack([p[None, :, 0] * c[:, None], p[None, :, 0] * s[:, None], np.broadcast_to(p[None, :, 1], (na, k))], axis=-1)
    d = np.concatenate(([0], np.cumsum(np.hypot(*np.diff(p, axis=0).T))))
    uv = np.stack([np.broadcast_to((ang / TAU)[:, None], (na, k)), np.broadcast_to(d[None, :], (na, k))], axis=-1)
    i = np.arange(segs)[:, None]
    j = np.arange(k - 1)[None, :]
    i2 = (i + 1) % na if full else i + 1
    a = i * k + j
    b = i2 * k + j
    quads = np.stack([a, b, b + 1, a + 1], axis=-1).reshape(-1, 4)
    if flip:
        quads = quads[:, ::-1]
    return v.reshape(-1, 3), quads, uv.reshape(-1, 2)


def frames_along(path):
    """Parallel-transport frames along a polyline: tangents, normals, binormals."""
    p = np.asarray(path, np.float64)
    t = np.gradient(p, axis=0)
    t /= np.linalg.norm(t, axis=1, keepdims=True) + 1e-12
    n = np.zeros_like(p)
    ref = np.array([0.0, 0.0, 1.0]) if abs(t[0, 2]) < 0.9 else np.array([1.0, 0.0, 0.0])
    n0 = np.cross(t[0], ref)
    n0 /= np.linalg.norm(n0)
    n[0] = n0
    for i in range(1, len(p)):
        v = n[i - 1] - t[i] * np.dot(n[i - 1], t[i])
        ln = np.linalg.norm(v)
        n[i] = v / ln if ln > 1e-9 else n[i - 1]
    b = np.cross(t, n)
    return t, n, b


def tube(path, radii, sides, cap=False, uscale=1.0, squash=None):
    """A generalised cylinder along a polyline. `squash` (n, 2) scales the
    section along its normal and binormal."""
    p = np.asarray(path, np.float64)
    r = np.broadcast_to(np.asarray(radii, np.float64), (len(p),))
    t, n, b = frames_along(p)
    ang = np.arange(sides) * TAU / sides
    c, s = np.cos(ang), np.sin(ang)
    sq = np.ones((len(p), 2)) if squash is None else np.asarray(squash, np.float64)
    v = p[:, None, :] + (r[:, None] * sq[:, 0:1])[:, :, None] * c[None, :, None] * n[:, None, :] + (r[:, None] * sq[:, 1:2])[:, :, None] * s[None, :, None] * b[:, None, :]
    d = np.concatenate(([0], np.cumsum(np.linalg.norm(np.diff(p, axis=0), axis=1))))
    uv = np.stack([np.broadcast_to((ang / TAU * uscale)[None, :], (len(p), sides)), np.broadcast_to(d[:, None], (len(p), sides))], axis=-1)
    i = np.arange(len(p) - 1)[:, None]
    j = np.arange(sides)[None, :]
    j2 = (j + 1) % sides
    quads = np.stack([i * sides + j, i * sides + j2, (i + 1) * sides + j2, (i + 1) * sides + j], axis=-1).reshape(-1, 4)
    v = v.reshape(-1, 3)
    uv = uv.reshape(-1, 2)
    if cap:
        return v, quads, uv, [np.arange(sides)[::-1], (len(p) - 1) * sides + np.arange(sides)]
    return v, quads, uv


def capsule(p0, p1, r0, r1, sides=8, rings=3, squash=(1.0, 1.0)):
    """A tapered capsule from p0 to p1: a tube with rounded ends."""
    p0 = np.asarray(p0, np.float64)
    p1 = np.asarray(p1, np.float64)
    axis = p1 - p0
    length = np.linalg.norm(axis)
    axis = axis / (length + 1e-12)
    pts = []
    rad = []
    for k in range(rings, 0, -1):
        a = (k / rings) * (math.pi / 2)
        pts.append(p0 - axis * r0 * math.sin(a))
        rad.append(r0 * math.cos(a) + 1e-4)
    pts.append(p0)
    rad.append(r0)
    pts.append(p1)
    rad.append(r1)
    for k in range(1, rings + 1):
        a = (k / rings) * (math.pi / 2)
        pts.append(p1 + axis * r1 * math.sin(a))
        rad.append(r1 * math.cos(a) + 1e-4)
    sq = np.tile(np.asarray(squash, np.float64), (len(pts), 1))
    return tube(np.array(pts), np.array(rad), sides, squash=sq)


def ellipsoid(center, radii, sides=10, rings=6):
    cx, cy, cz = center
    rx, ry, rz = radii
    prof = [(math.sin(math.pi * k / rings) + 1e-4, -math.cos(math.pi * k / rings)) for k in range(rings + 1)]
    v, q, uv = revolve(prof, sides)
    v = v * np.array([rx, ry, rz], np.float32) + np.array([cx, cy, cz], np.float32)
    return v, q, uv


def disc(r0, r1, segs, rings=1, z=0.0, a0=0.0, a1=TAU):
    """A flat ring (or a disc when r0 is 0, kept as a tiny hole), facing up."""
    prof = [(max(r0, 1e-4) + (r1 - max(r0, 1e-4)) * k / rings, z) for k in range(rings + 1)]
    v, q, uv = revolve(prof, segs, a0, a1, flip=True)
    return v, q, uv


def smooth_path(pts, n, closed=False):
    """A Catmull-Rom curve through the points, resampled to n points."""
    p = np.asarray(pts, np.float64)
    if closed:
        p = np.concatenate([p[-1:], p, p[:2]])
    else:
        p = np.concatenate([2 * p[:1] - p[1:2], p, 2 * p[-1:] - p[-2:-1]])
    segs = len(p) - 3
    out = []
    per = max(2, int(math.ceil(n / segs)) * 2)
    for i in range(segs):
        p0, p1, p2, p3 = p[i:i + 4]
        t = np.linspace(0, 1, per, endpoint=False)[:, None]
        out.append(0.5 * ((2 * p1) + (-p0 + p2) * t + (2 * p0 - 5 * p1 + 4 * p2 - p3) * t * t + (-p0 + 3 * p1 - 3 * p2 + p3) * t ** 3))
    if not closed:
        out.append(p[-2][None])
    dense = np.concatenate(out)
    if closed:
        dense = np.concatenate([dense, dense[:1]])
    d = np.concatenate(([0], np.cumsum(np.linalg.norm(np.diff(dense, axis=0), axis=1))))
    s = np.linspace(0, d[-1], n + 1 if closed else n)
    res = np.stack([np.interp(s, d, dense[:, k]) for k in range(dense.shape[1])], axis=-1)
    return res[:-1] if closed else res


# ------------------------------------------------------- shader node expression


class NT:
    """Builds a node tree with expressions instead of boilerplate."""

    def __init__(self, tree, clear=True):
        self.t = tree
        if clear:
            tree.nodes.clear()
        self._x = 0

    # -- plumbing
    def node(self, typ, **props):
        n = self.t.nodes.new(typ)
        self._x += 40
        n.location = (self._x, 0)
        for key, val in props.items():
            setattr(n, key, val)
        return n

    def put(self, sock, val):
        if val is None:
            return
        if isinstance(val, V):
            self.t.links.new(val.s, sock)
        elif isinstance(val, bpy.types.NodeSocket):
            self.t.links.new(val, sock)
        else:
            try:
                sock.default_value = val
            except (TypeError, ValueError):
                if isinstance(val, (int, float)):
                    sock.default_value = (val, val, val, 1.0) if len(sock.default_value) == 4 else (val, val, val)
                elif len(val) == 3 and len(sock.default_value) == 4:
                    sock.default_value = (*val, 1.0)
                else:
                    raise

    def v(self, sock):
        return V(self, sock)

    def val(self, x):
        n = self.node("ShaderNodeValue")
        n.outputs[0].default_value = x
        return V(self, n.outputs[0])

    def rgb(self, c):
        n = self.node("ShaderNodeRGB")
        n.outputs[0].default_value = (*c[:3], 1.0)
        return V(self, n.outputs[0])

    # -- scalar maths
    def math(self, op, a, b=None, c=None, clamp=False):
        n = self.node("ShaderNodeMath", operation=op, use_clamp=clamp)
        self.put(n.inputs[0], a)
        if b is not None:
            self.put(n.inputs[1], b)
        if c is not None:
            self.put(n.inputs[2], c)
        return V(self, n.outputs[0])

    def floor(self, a):
        return self.math("FLOOR", a)

    def fract(self, a):
        return self.math("FRACT", a)

    def sin(self, a):
        return self.math("SINE", a)

    def cos(self, a):
        return self.math("COSINE", a)

    def sqrt(self, a):
        return self.math("SQRT", a)

    def abs(self, a):
        return self.math("ABSOLUTE", a)

    def pow(self, a, b):
        return self.math("POWER", a, b)

    def min(self, a, b):
        return self.math("MINIMUM", a, b)

    def max(self, a, b):
        return self.math("MAXIMUM", a, b)

    def lt(self, a, b):
        return self.math("LESS_THAN", a, b)

    def gt(self, a, b):
        return self.math("GREATER_THAN", a, b)

    def atan2(self, y, x):
        return self.math("ARCTAN2", y, x)

    def mod(self, a, b):
        return self.math("FLOORED_MODULO", a, b)

    def exp(self, a):
        return self.math("EXPONENT", a)

    def sat(self, a):
        return self.math("ADD", a, 0.0, clamp=True)

    def remap(self, x, a, b, c=0.0, d=1.0, smooth=False, clamp=True):
        n = self.node("ShaderNodeMapRange", interpolation_type="SMOOTHSTEP" if smooth else "LINEAR", clamp=clamp)
        self.put(n.inputs[0], x)
        self.put(n.inputs[1], a)
        self.put(n.inputs[2], b)
        self.put(n.inputs[3], c)
        self.put(n.inputs[4], d)
        return V(self, n.outputs[0])

    def smoothstep(self, a, b, x):
        return self.remap(x, a, b, 0.0, 1.0, smooth=True)

    def mix(self, a, b, t):
        n = self.node("ShaderNodeMix", data_type="FLOAT")
        self.put(n.inputs[0], t)
        self.put(n.inputs[2], a)
        self.put(n.inputs[3], b)
        return V(self, n.outputs[0])

    # -- colour and vectors
    def mixc(self, a, b, t, blend="MIX", clamp=False):
        n = self.node("ShaderNodeMix", data_type="RGBA", blend_type=blend, clamp_result=clamp)
        self.put(n.inputs[0], t)
        self.put(n.inputs[6], a)
        self.put(n.inputs[7], b)
        return V(self, n.outputs[2])

    def ramp(self, fac, stops, interp="LINEAR"):
        n = self.node("ShaderNodeValToRGB")
        cr = n.color_ramp
        cr.interpolation = interp
        while len(cr.elements) < len(stops):
            cr.elements.new(0.5)
        for el, (pos, col) in zip(cr.elements, stops):
            el.position = pos
            el.color = (*col[:3], 1.0) if len(col) == 3 else col
        self.put(n.inputs[0], fac)
        return V(self, n.outputs[0])

    def hsv(self, col, h=0.5, s=1.0, v=1.0, fac=1.0):
        n = self.node("ShaderNodeHueSaturation")
        self.put(n.inputs["Hue"], h)
        self.put(n.inputs["Saturation"], s)
        self.put(n.inputs["Value"], v)
        self.put(n.inputs["Factor"], fac)
        self.put(n.inputs["Color"], col)
        return V(self, n.outputs[0])

    def xyz(self, vec):
        n = self.node("ShaderNodeSeparateXYZ")
        self.put(n.inputs[0], vec)
        return V(self, n.outputs[0]), V(self, n.outputs[1]), V(self, n.outputs[2])

    def vec(self, x=0.0, y=0.0, z=0.0):
        n = self.node("ShaderNodeCombineXYZ")
        self.put(n.inputs[0], x)
        self.put(n.inputs[1], y)
        self.put(n.inputs[2], z)
        return V(self, n.outputs[0])

    def vmath(self, op, a, b=None, scale=None, out=0):
        n = self.node("ShaderNodeVectorMath", operation=op)
        self.put(n.inputs[0], a)
        if b is not None:
            self.put(n.inputs[1], b)
        if scale is not None:
            self.put(n.inputs[3], scale)
        return V(self, n.outputs[out])

    def mapping(self, vec, loc=(0, 0, 0), rot=(0, 0, 0), scale=(1, 1, 1)):
        n = self.node("ShaderNodeMapping")
        self.put(n.inputs[0], vec)
        n.inputs[1].default_value = loc
        n.inputs[2].default_value = rot
        n.inputs[3].default_value = scale
        return V(self, n.outputs[0])

    # -- inputs
    def coord(self, which="Object"):
        return V(self, self.node("ShaderNodeTexCoord").outputs[which])

    def geom(self, which="Position"):
        return V(self, self.node("ShaderNodeNewGeometry").outputs[which])

    def attr(self, name, out="Factor", kind="GEOMETRY"):
        n = self.node("ShaderNodeAttribute", attribute_type=kind, attribute_name=name)
        return V(self, n.outputs[out])

    def objinfo(self, which="Random"):
        return V(self, self.node("ShaderNodeObjectInfo").outputs[which])

    def lightpath(self, which="Is Camera Ray"):
        return V(self, self.node("ShaderNodeLightPath").outputs[which])

    def layer(self, blend=0.5, which="Fresnel", normal=None):
        n = self.node("ShaderNodeLayerWeight")
        self.put(n.inputs[0], blend)
        self.put(n.inputs[1], normal)
        return V(self, n.outputs[which])

    # -- textures
    def noise(self, vec=None, scale=5.0, detail=2.0, rough=0.5, lac=2.0, dist=0.0, out="Factor", dims="3D", w=None):
        n = self.node("ShaderNodeTexNoise", noise_dimensions=dims)
        self.put(n.inputs["Vector"], vec)
        if w is not None:
            self.put(n.inputs["W"], w)
        self.put(n.inputs["Scale"], scale)
        self.put(n.inputs["Detail"], detail)
        self.put(n.inputs["Roughness"], rough)
        self.put(n.inputs["Lacunarity"], lac)
        self.put(n.inputs["Distortion"], dist)
        return V(self, n.outputs[out])

    def voronoi(self, vec=None, scale=5.0, out="Distance", feature="F1", rand=1.0, dims="3D", metric="EUCLIDEAN"):
        n = self.node("ShaderNodeTexVoronoi", feature=feature, voronoi_dimensions=dims, distance=metric)
        self.put(n.inputs["Vector"], vec)
        self.put(n.inputs["Scale"], scale)
        self.put(n.inputs["Randomness"], rand)
        return V(self, n.outputs[out])

    def white(self, vec=None, w=None, out="Value", dims="3D"):
        n = self.node("ShaderNodeTexWhiteNoise", noise_dimensions=dims)
        if vec is not None:
            self.put(n.inputs["Vector"], vec)
        if w is not None:
            self.put(n.inputs["W"], w)
        return V(self, n.outputs[out])

    def bump(self, height, strength=0.5, distance=0.01, normal=None):
        n = self.node("ShaderNodeBump")
        self.put(n.inputs["Strength"], strength)
        self.put(n.inputs["Distance"], distance)
        self.put(n.inputs["Height"], height)
        self.put(n.inputs["Normal"], normal)
        return V(self, n.outputs[0])

    # -- shaders
    def principled(self, **inputs):
        n = self.node("ShaderNodeBsdfPrincipled")
        for key, val in inputs.items():
            self.put(n.inputs[key.replace("_", " ")], val)
        return V(self, n.outputs[0])

    def shader(self, typ, **inputs):
        n = self.node(typ)
        for key, val in inputs.items():
            self.put(n.inputs[key.replace("_", " ")], val)
        return V(self, n.outputs[0])

    def mix_shader(self, a, b, fac):
        n = self.node("ShaderNodeMixShader")
        self.put(n.inputs[0], fac)
        self.put(n.inputs[1], a)
        self.put(n.inputs[2], b)
        return V(self, n.outputs[0])

    def add_shader(self, a, b):
        n = self.node("ShaderNodeAddShader")
        self.put(n.inputs[0], a)
        self.put(n.inputs[1], b)
        return V(self, n.outputs[0])

    def group(self, tree, **inputs):
        n = self.node("ShaderNodeGroup")
        n.node_tree = tree
        for key, val in inputs.items():
            self.put(n.inputs[key], val)
        return n

    def out_surface(self, surface=None, volume=None, displacement=None):
        n = self.node("ShaderNodeOutputMaterial")
        self.put(n.inputs["Surface"], surface)
        self.put(n.inputs["Volume"], volume)
        self.put(n.inputs["Displacement"], displacement)
        return n


class V:
    """A socket with arithmetic."""

    def __init__(self, nt, sock):
        self.nt = nt
        self.s = sock

    def __add__(self, o):
        return self.nt.math("ADD", self, o)

    __radd__ = __add__

    def __sub__(self, o):
        return self.nt.math("SUBTRACT", self, o)

    def __rsub__(self, o):
        return self.nt.math("SUBTRACT", o, self)

    def __mul__(self, o):
        return self.nt.math("MULTIPLY", self, o)

    __rmul__ = __mul__

    def __truediv__(self, o):
        return self.nt.math("DIVIDE", self, o)

    def __rtruediv__(self, o):
        return self.nt.math("DIVIDE", o, self)

    def __neg__(self):
        return self.nt.math("MULTIPLY", self, -1.0)


def material(name):
    """A fresh material and its expression builder."""
    old = bpy.data.materials.get(name)
    if old is not None:
        bpy.data.materials.remove(old)
    mat = bpy.data.materials.new(name)
    return mat, NT(mat.node_tree)


# ------------------------------------------------------------------ the grade
#
# Every material that changes with the time of day reads one node group,
# FilmGrade. render.py sets its values for the grade being rendered.

GRADE_KEYS = ("night", "lamps", "windows", "wet", "sun")


def grade_group():
    g = bpy.data.node_groups.get("FilmGrade")
    if g is not None:
        return g
    g = bpy.data.node_groups.new("FilmGrade", "ShaderNodeTree")
    out = g.nodes.new("NodeGroupOutput")
    for key in GRADE_KEYS:
        g.interface.new_socket(name=key, in_out="OUTPUT", socket_type="NodeSocketFloat")
        v = g.nodes.new("ShaderNodeValue")
        v.name = key
        v.label = key
        v.outputs[0].default_value = 0.0
        g.links.new(v.outputs[0], out.inputs[key])
    return g


def grade(nt):
    """The grade's values as expressions in this tree."""
    n = nt.group(grade_group())
    return {key: V(nt, n.outputs[key]) for key in GRADE_KEYS}


def set_grade(**vals):
    g = grade_group()
    for key, val in vals.items():
        g.nodes[key].outputs[0].default_value = float(val)

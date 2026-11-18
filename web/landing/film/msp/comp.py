"""The finish: a lens's bloom on the points of light, a little vignette,
fine grain, a touch of colour balance. It runs from the saved EXR, so the
finish can be changed without rendering again."""

import bpy


def _mix(ng, a, b, fac, blend="MIX"):
    n = ng.nodes.new("ShaderNodeMix")
    n.data_type = "RGBA"
    n.blend_type = blend
    for sock, val in ((n.inputs[0], fac), (n.inputs[6], a), (n.inputs[7], b)):
        if isinstance(val, bpy.types.NodeSocket):
            ng.links.new(val, sock)
        elif isinstance(val, (int, float)):
            sock.default_value = val
        else:
            sock.default_value = (*val[:3], 1.0)
    return n.outputs[2]


def _math(ng, op, a, b=None, clamp=False):
    n = ng.nodes.new("ShaderNodeMath")
    n.operation = op
    n.use_clamp = clamp
    for sock, val in ((n.inputs[0], a), (n.inputs[1], b)):
        if val is None:
            continue
        if isinstance(val, bpy.types.NodeSocket):
            ng.links.new(val, sock)
        else:
            sock.default_value = val
    return n.outputs[0]


def build(scene, g, exr_path):
    old = bpy.data.node_groups.get("FilmComp")
    if old is not None:
        bpy.data.node_groups.remove(old)
    ng = bpy.data.node_groups.new("FilmComp", "CompositorNodeTree")
    ng.interface.new_socket(name="Image", in_out="OUTPUT", socket_type="NodeSocketColor")
    img = bpy.data.images.load(exr_path, check_existing=False)
    src = ng.nodes.new("CompositorNodeImage")
    src.image = img
    outs = {o.name: o for o in src.outputs}
    cur = outs.get("Combined") or outs.get("Image") or src.outputs[0]

    if g.get("bloom", 0) > 0:
        gl = ng.nodes.new("CompositorNodeGlare")
        gl.inputs["Type"].default_value = "Bloom"
        gl.inputs["Quality"].default_value = "High"
        gl.inputs["Threshold"].default_value = g.get("bloom_threshold", 2.0)
        gl.inputs["Smoothness"].default_value = 0.5
        gl.inputs["Strength"].default_value = g["bloom"]
        gl.inputs["Size"].default_value = g.get("bloom_size", 0.5)
        gl.inputs["Clamp"].default_value = True
        gl.inputs["Maximum"].default_value = 40.0
        ng.links.new(cur, gl.inputs["Image"])
        cur = gl.outputs["Image"]

    # the lens: a hair of colour fringing toward the corners
    if g.get("fringe", 0.004) > 0:
        ld = ng.nodes.new("CompositorNodeLensdist")
        ld.inputs["Dispersion"].default_value = g.get("fringe", 0.004)
        ld.inputs["Distortion"].default_value = 0.0
        ng.links.new(cur, ld.inputs["Image"])
        cur = ld.outputs["Image"]

    co = ng.nodes.new("CompositorNodeImageCoordinates")
    ng.links.new(cur, co.inputs[0])
    off = ng.nodes.new("ShaderNodeVectorMath")
    off.operation = "SUBTRACT"
    ng.links.new(co.outputs["Normalized"], off.inputs[0])
    off.inputs[1].default_value = (0.5, 0.5, 0.0)
    ln = ng.nodes.new("ShaderNodeVectorMath")
    ln.operation = "LENGTH"
    ng.links.new(off.outputs[0], ln.inputs[0])
    rng_ = ng.nodes.new("ShaderNodeMapRange")
    rng_.interpolation_type = "SMOOTHSTEP"
    ng.links.new(ln.outputs["Value"], rng_.inputs[0])
    rng_.inputs[1].default_value = 0.3
    rng_.inputs[2].default_value = 0.8
    rng_.inputs[3].default_value = 0.0
    rng_.inputs[4].default_value = g.get("vignette", 0.3)
    cur = _mix(ng, cur, (0.0, 0.0, 0.0), rng_.outputs[0])

    # grain: finer than a pixel is not possible, so it is kept faint, and
    # stronger in the mid-tones, as film's is
    if g.get("grain", 0) > 0:
        wn = ng.nodes.new("ShaderNodeTexWhiteNoise")
        wn.noise_dimensions = "2D"
        ng.links.new(co.outputs["Pixel"], wn.inputs["Vector"])
        centred = _math(ng, "SUBTRACT", wn.outputs["Value"], 0.5)
        amount = _math(ng, "MULTIPLY", centred, g["grain"] * 2.0)
        gain = _math(ng, "ADD", amount, 1.0)
        cur = _mix(ng, cur, gain, 1.0, blend="MULTIPLY")

    cb = ng.nodes.new("CompositorNodeColorBalance")
    cb.inputs["Type"].default_value = "Lift/Gamma/Gain"
    cb.inputs[4].default_value = (*g.get("lift", (1, 1, 1)), 1.0)
    cb.inputs[6].default_value = (*g.get("gamma", (1, 1, 1)), 1.0)
    cb.inputs[8].default_value = (*g.get("gain", (1, 1, 1)), 1.0)
    ng.links.new(cur, cb.inputs["Image"])
    cur = cb.outputs[0]

    out = ng.nodes.new("NodeGroupOutput")
    ng.links.new(cur, out.inputs["Image"])
    scene.compositing_node_group = ng
    scene.render.use_compositing = True
    return img

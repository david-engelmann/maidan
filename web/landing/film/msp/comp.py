"""The grade: depth haze, bloom, a vignette and a colour balance, over the
raw render. It runs from the saved EXR, so a grade can be changed without
rendering again."""

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
    image = outs.get("Combined") or outs.get("Image") or src.outputs[0]
    mist = outs.get("Mist")
    depth = outs.get("Depth")
    cur = image

    # haze: the air between here and there takes the colour of the horizon
    if mist is not None and g.get("haze_amt", 0) > 0:
        fac = _math(ng, "POWER", mist, 0.85)
        fac = _math(ng, "MULTIPLY", fac, g["haze_amt"], clamp=True)
        if depth is not None:
            # not the sky itself
            fac = _math(ng, "MULTIPLY", fac, _math(ng, "LESS_THAN", depth, 20000.0))
        cur = _mix(ng, cur, g["haze"], fac)

    # bloom on the lamps, the windows and the water's highlights
    if g.get("bloom", 0) > 0:
        gl = ng.nodes.new("CompositorNodeGlare")
        gl.inputs["Type"].default_value = "Bloom"
        gl.inputs["Quality"].default_value = "High"
        gl.inputs["Threshold"].default_value = g.get("bloom_threshold", 1.6)
        gl.inputs["Smoothness"].default_value = 0.4
        gl.inputs["Strength"].default_value = g["bloom"]
        gl.inputs["Size"].default_value = g.get("bloom_size", 0.55)
        gl.inputs["Clamp"].default_value = True
        gl.inputs["Maximum"].default_value = 30.0
        ng.links.new(cur, gl.inputs["Image"])
        cur = gl.outputs["Image"]

    # a vignette: the corners a little down
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
    rng_.inputs[1].default_value = 0.32
    rng_.inputs[2].default_value = 0.78
    rng_.inputs[3].default_value = 0.0
    rng_.inputs[4].default_value = g.get("vignette", 0.3)
    cur = _mix(ng, cur, (0.0, 0.0, 0.0), rng_.outputs[0])

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

"""Renders a frame of the film from the built scene, then grades it.

    Blender -b scene.blend -P render.py -- --grade blue --shot hero --res 1920x1080 --out /path/frame.png

The raw frame is kept beside the output as an EXR, and a frame whose output
already exists is skipped, so a long render can be stopped and resumed.
`--regrade` grades the saved EXR again without rendering.
"""

import argparse
import os
import sys
import time

import bpy

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--grade", default="blue")
    ap.add_argument("--shot", default="hero")
    ap.add_argument("--t", type=float, default=0.0, help="where in an enter or leave move (0..1)")
    ap.add_argument("--res", default="1920x1080")
    ap.add_argument("--samples", type=int, default=256)
    ap.add_argument("--out", required=True)
    ap.add_argument("--border", default="", help="x0,y0,x1,y1 as fractions of the frame, from the bottom left")
    ap.add_argument("--force", action="store_true")
    ap.add_argument("--regrade", action="store_true")
    ap.add_argument("--noise", type=float, default=0.02, help="adaptive sampling threshold")
    ap.add_argument("--hide", default="", help="collections to leave out, comma separated")
    ap.add_argument("--set", default="", help="grade overrides, key=value pairs separated by commas (tuples with / between numbers)")
    args = ap.parse_args(sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else [])
    out = os.path.abspath(args.out)
    exr = os.path.splitext(out)[0] + ".exr"
    if os.path.exists(out) and not (args.force or args.regrade):
        print(f"[render] {out} exists, skipped")
        return
    from msp import camera, comp, light

    sc = bpy.context.scene
    w, h = (int(x) for x in args.res.split("x"))
    sc.render.resolution_x = w
    sc.render.resolution_y = h
    sc.render.resolution_percentage = 100
    g = dict(light.GRADES[args.grade])
    for pair in [p for p in args.set.split(",") if p]:
        key, val = pair.split("=")
        g[key] = tuple(float(x) for x in val.split("/")) if "/" in val else (float(val) if val.replace(".", "").replace("-", "").isdigit() else val)
    light.GRADES[args.grade] = g
    light.apply(args.grade)
    camera.set_shot(args.shot, w / h, args.t)
    for name in [n for n in args.hide.split(",") if n]:
        c = bpy.data.collections.get(name)
        if c is not None:
            c.hide_render = True

    if not (args.regrade and os.path.exists(exr)):
        prefs = bpy.context.preferences.addons["cycles"].preferences
        prefs.compute_device_type = "METAL"
        prefs.get_devices()
        for d in prefs.devices:
            d.use = d.type == "METAL"
        sc.render.engine = "CYCLES"
        cy = sc.cycles
        cy.device = "GPU"
        cy.samples = args.samples
        cy.use_adaptive_sampling = True
        cy.adaptive_threshold = args.noise
        cy.adaptive_min_samples = 32
        cy.use_denoising = True
        cy.denoiser = "OPENIMAGEDENOISE"
        cy.denoising_input_passes = "RGB_ALBEDO_NORMAL"
        cy.denoising_prefilter = "ACCURATE"
        cy.denoising_use_gpu = True
        cy.max_bounces = 8
        cy.diffuse_bounces = 3
        cy.glossy_bounces = 4
        cy.transmission_bounces = 8
        cy.volume_bounces = 1
        cy.transparent_max_bounces = 48
        cy.caustics_reflective = False
        cy.caustics_refractive = False
        cy.sample_clamp_indirect = 8.0
        cy.sample_clamp_direct = 0.0
        cy.use_light_tree = True
        cy.volume_step_rate = 1.0
        vl = bpy.context.view_layer
        vl.use_pass_mist = True
        vl.use_pass_z = True
        sc.render.use_border = bool(args.border)
        sc.render.use_crop_to_border = bool(args.border)
        if args.border:
            x0, y0, x1, y1 = (float(v) for v in args.border.split(","))
            sc.render.border_min_x, sc.render.border_min_y, sc.render.border_max_x, sc.render.border_max_y = x0, y0, x1, y1
        sc.render.use_compositing = False
        sc.render.image_settings.media_type = "MULTI_LAYER_IMAGE"
        sc.render.image_settings.file_format = "OPEN_EXR_MULTILAYER"
        sc.render.image_settings.color_depth = "16"
        sc.render.image_settings.exr_codec = "DWAA"
        sc.render.filepath = exr
        t0 = time.time()
        bpy.ops.render.render(write_still=True)
        print(f"[render] {args.grade}/{args.shot} {w}x{h} {args.samples}spp: {time.time() - t0:.1f}s")

    # the grade
    comp.build(sc, g, exr)
    sc.render.use_border = False
    if args.border:
        # the EXR holds only the border's pixels
        x0, y0, x1, y1 = (float(v) for v in args.border.split(","))
        sc.render.resolution_x = round(x1 * w) - round(x0 * w)
        sc.render.resolution_y = round(y1 * h) - round(y0 * h)
    sc.render.image_settings.media_type = "IMAGE"
    sc.render.image_settings.file_format = "PNG"
    sc.render.image_settings.color_mode = "RGB"
    sc.render.image_settings.color_depth = "8"
    sc.render.image_settings.compression = 60
    sc.render.filepath = out
    t0 = time.time()
    bpy.ops.render.render(write_still=True)
    print(f"[render] graded {out}: {time.time() - t0:.1f}s")


main()

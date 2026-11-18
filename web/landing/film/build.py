"""Builds the scene and saves it as a .blend.

    Blender -b --factory-startup -P build.py -- --blend /path/scene.blend [--parts fountain,trees]

With --parts and an existing .blend, only those parts are rebuilt.
"""

import argparse
import os
import sys
import time

import bpy

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

PARTS = ["ground", "fountain", "furniture", "trees", "buildings", "people", "traffic"]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--blend", required=True)
    ap.add_argument("--parts", default="all")
    ap.add_argument("--quality", type=float, default=1.0, help="scales tree and crowd counts, for quick looks")
    args = ap.parse_args(sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else [])
    parts = PARTS if args.parts == "all" else [p for p in args.parts.split(",") if p]
    fresh = args.parts == "all" or not os.path.exists(args.blend)
    if fresh:
        for ob in list(bpy.data.objects):
            bpy.data.objects.remove(ob, do_unlink=True)
        for c in list(bpy.data.collections):
            bpy.data.collections.remove(c)
    else:
        bpy.ops.wm.open_mainfile(filepath=args.blend)
    import importlib

    from msp import common

    common.grade_group()
    os.environ["FILM_QUALITY"] = str(args.quality)
    for part in parts:
        t0 = time.time()
        try:
            mod = importlib.import_module(f"msp.{part}")
        except ModuleNotFoundError as err:
            if err.name != f"msp.{part}":
                raise
            print(f"[build] {part}: not written yet, skipped")
            continue
        mod.build()
        print(f"[build] {part}: {time.time() - t0:.1f}s")
    common.purge()
    from msp import camera

    camera.camera_object()
    sc = bpy.context.scene
    sc.render.engine = "CYCLES"
    sc.unit_settings.system = "METRIC"
    os.makedirs(os.path.dirname(os.path.abspath(args.blend)), exist_ok=True)
    bpy.ops.wm.save_as_mainfile(filepath=os.path.abspath(args.blend), compress=True)
    tris = sum(len(ob.data.polygons) for ob in bpy.data.objects if ob.type == "MESH")
    print(f"[build] saved {args.blend}: {len(bpy.data.objects)} objects, {tris} polygons placed")


main()

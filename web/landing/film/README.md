# The landing film

A scripted Blender scene: a handcrafted architectural model of the square
and the blocks round it, photographed in a studio with a macro lens. Pale
wood blocks with laser-etched windows (a share of them cut through and lit
from inside), brass crowns, acrylic towers, a sage felt lawn, frosted beads
on brass pins for trees, a brass and resin fountain, and small turned
figures, white and orange.

Nothing here is wired into the page. The scripts build the scene and render
frames; the frames and the `.blend` are not in the repository.

## Requirements

- Blender 5.3.0 (`/Applications/Blender.app/Contents/MacOS/Blender` on
  macOS). Cycles on the Metal GPU, OpenImageDenoise.
- Python 3 for `fetch_assets.py` (standard library only).
- `ffmpeg`, only for contact sheets and the later encodes.

## Rebuild

```sh
B=/Applications/Blender.app/Contents/MacOS/Blender
WORK=/path/outside/the/repo            # renders, the .blend, the assets

python3 fetch_assets.py $WORK/assets   # about 30 MB, see ASSETS.md
$B -b --factory-startup -P build.py -- --blend $WORK/scene.blend --assets $WORK/assets
sh stills.sh $WORK/scene.blend $WORK/assets $WORK/stills
```

`build.py` takes a few seconds. `--parts model,square,studio` rebuilds only
those parts of an existing `.blend`.

One frame:

```sh
$B -b $WORK/scene.blend -P render.py -- --setup dark --shot hero \
   --res 1920x1080 --samples 512 --assets $WORK/assets --out $WORK/frame.png
```

- `--setup`: `dark` (the studio dimmed, the model lit from within), `dark_full`
  (the same with every lamp on, for the close) or `light` (a daylight studio).
- `--shot`: `hero`, `close`, `plaza`, `fountain`, `trees`, `facade`, `crown`,
  or a move, `enter` or `leave`, with `--t` from 0 to 1. A portrait `--res`
  picks the tall framing of `hero` and `close`.
- The raw frame is kept beside the output as an EXR. `--regrade` runs the
  finish (bloom, vignette, grain, colour balance) again from that EXR without
  rendering. A frame whose output exists is skipped unless `--force` is given,
  so a long run can be stopped and resumed.

Everything is seeded: a rebuild gives the same model.

## What is where

| File | What it makes |
|---|---|
| `msp/site.py` | The plan, in city metres, and the scale (1:400) |
| `msp/model.py` | Baseplate and plinth, a plate per block, the buildings and their crowns |
| `msp/square.py` | The park panel and its engraving, lawn, fountain, trees, lamps, figures |
| `msp/craft.py` | The materials |
| `msp/studio.py` | The table and sweep, the lights, the two set-ups |
| `msp/camera.py` | The shots and the scroll moves |
| `msp/comp.py` | The finish |
| `msp/common.py` | Mesh and shader-node helpers |

The scene is built at the model's true size, in metres: the baseplate is
about 1.2 m square and a figure is 5 mm tall. Depth of field comes from the
lens and stop, not from a filter.

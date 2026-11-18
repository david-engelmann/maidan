#!/bin/sh
# Renders the Phase A stills: the hero in both set-ups and both shapes, the
# storyboard of the scroll journey, and four close looks at the craft.
#
#   sh stills.sh /path/scene.blend /path/assets /path/out
#
# A still that already exists is skipped, so this can be stopped and resumed.
set -eu
BLEND=$1
ASSETS=$2
OUT=$3
BLENDER=${BLENDER:-/Applications/Blender.app/Contents/MacOS/Blender}
HERE=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$OUT"
shot() { # name setup shot res samples [t]
  "$BLENDER" -b "$BLEND" -P "$HERE/render.py" -- --setup "$2" --shot "$3" --res "$4" --samples "$5" --t "${6:-0}" \
    --assets "$ASSETS" --out "$OUT/$1.png" 2>&1 | grep '^\[render\]' || true
}
shot hero-wide-dark dark hero 2560x1440 640
shot hero-wide-light light hero 2560x1440 512
shot hero-tall-dark dark hero 1440x2560 640
shot hero-tall-light light hero 1440x2560 512
shot story-1-arrive dark hero 1920x1080 512
shot story-2-enter-33 dark enter 1920x1080 512 0.3333
shot story-3-enter-66 dark enter 1920x1080 512 0.6667
shot story-4-enter-end dark enter 1920x1080 640 1.0
shot story-5-close-start dark_full leave 1920x1080 640 0.0
shot story-6-close-end dark_full leave 1920x1080 512 1.0
shot macro-fountain dark fountain 1920x1080 640
shot macro-trees dark trees 1920x1080 640
shot macro-facade dark facade 1920x1080 512
shot macro-crown dark crown 1920x1080 512

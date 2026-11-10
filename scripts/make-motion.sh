#!/usr/bin/env bash
# Record the docs' motion clips from the public demo and encode them into
# docs/assets/motion/: a looping GIF of each (for GitHub, which shows Markdown
# images but no video) and an MP4 of each (for docs.maidan.world, where
# book/sync-docs.sh swaps the GIF for a <video>, a tenth of the bytes).
#
#   scripts/make-motion.sh                record, then encode
#   scripts/make-motion.sh --encode-only  encode the frames of the last take
#
# Recording runs ui-tests/capture/motion.capture.ts with its own config. It
# needs `npm ci` in ui-tests/, Playwright's Chromium, and ffmpeg. It starts a
# sandbox on try.maidan.world (MOTION_BASE_URL to point it elsewhere), and the
# demo allows three sandboxes an hour from one address. A take runs about ten
# minutes: before each clip the console sits idle for a minute, so the demo's
# per-minute rate limits never show on screen (the spec fails a clip that met
# one).
#
# Frames are kept in MOTION_FRAMES (a temporary directory by default) so a
# re-encode does not need a new take.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
out="$root/docs/assets/motion"
frames="${MOTION_FRAMES:-${TMPDIR:-/tmp}/maidan-motion-frames}"
ffmpeg="${FFMPEG:-ffmpeg}"
# 960 px keeps a GIF of a full console screen legible inside a docs column.
gif_width="${MOTION_GIF_WIDTH:-960}"
gif_fps="${MOTION_GIF_FPS:-15}"
# The video is sharp on a high-density screen and still the lighter file.
mp4_width="${MOTION_MP4_WIDTH:-1280}"
gif_max=$((4 * 1024 * 1024))
mp4_max=$((1536 * 1024))
# MOTION_CLIPS="board-live thread" encodes only those.
read -r -a clips <<<"${MOTION_CLIPS:-board-live needs-you thread activity catalog}"

if [[ "${1:-}" != "--encode-only" ]]; then
  (cd "$root/ui-tests" && MOTION_FRAMES="$frames" npx playwright test --config capture/motion.config.ts)
fi

mkdir -p "$out"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

size() { wc -c <"$1" | tr -d ' '; }

failed=0
for clip in "${clips[@]}"; do
  list="$frames/$clip/frames.ffconcat"
  if [[ ! -f "$list" ]]; then
    echo "no frames for $clip in $frames; record a take first" >&2
    exit 1
  fi
  # The spec keeps each frame with the time it was taken (frames.ffconcat);
  # fps= turns them into an even rate that keeps that timing.
  input=(-f concat -safe 0 -i "$list")

  # Two passes: a palette built from what changes between frames (the board
  # is mostly still), then the frames mapped onto it. Bayer dithering is
  # ordered, so still areas stay identical from frame to frame and the GIF
  # stores only the parts that moved.
  scale="fps=$gif_fps,scale=$gif_width:-2:flags=lanczos"
  "$ffmpeg" -v error -y "${input[@]}" -vf "$scale,palettegen=max_colors=192:stats_mode=diff" "$work/palette.png"
  "$ffmpeg" -v error -y "${input[@]}" -i "$work/palette.png" \
    -lavfi "$scale[x];[x][1:v]paletteuse=dither=bayer:bayer_scale=4:diff_mode=rectangle" \
    -loop 0 "$out/$clip.gif"

  # H.264 in yuv420p plays everywhere a <video> does; faststart puts the index
  # first so playback begins before the download ends.
  "$ffmpeg" -v error -y "${input[@]}" \
    -vf "fps=30,scale=$mp4_width:-2:flags=lanczos,format=yuv420p" \
    -c:v libx264 -preset veryslow -tune animation -crf 26 -profile:v high -pix_fmt yuv420p \
    -movflags +faststart -an "$out/$clip.mp4"

  gif_size="$(size "$out/$clip.gif")"
  mp4_size="$(size "$out/$clip.mp4")"
  printf '%-12s gif %5d KB   mp4 %5d KB\n' "$clip" $((gif_size / 1024)) $((mp4_size / 1024))
  if ((gif_size > gif_max)); then
    echo "$clip.gif is over 4 MB: shorten the clip or lower MOTION_GIF_FPS" >&2
    failed=1
  fi
  if ((mp4_size > mp4_max)); then
    echo "$clip.mp4 is over 1.5 MB: shorten the clip or lower MOTION_MP4_WIDTH" >&2
    failed=1
  fi
done
exit "$failed"

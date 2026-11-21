#!/usr/bin/env bash
# Record the split-screen clip the README and the landing open on: the real
# console above, three agents' terminals below, one take against a real
# server. Every terminal line is a call the take made and what the server
# answered; the agents are scripted, and the advice is the decision model's
# own answer, so a take needs a provider key.
#
#   MAIDAN_BIN_DIR=target/debug TYPESAFE_API_KEY=... scripts/make-swarm-clip.sh
#   scripts/make-swarm-clip.sh --encode-only     encode the frames of the last take
#
# It starts its own server from MAIDAN_BIN_DIR (the directory holding
# `maidan-server`) on a loopback port with an empty SQLite database, once per
# theme, and stops it afterwards. Needs `npm ci` in ui-tests/, Playwright's
# Chromium, and ffmpeg with libsvtav1 and libaom-av1.
#
# Writes:
#   docs/assets/motion/swarm.gif and .mp4          the README and the docs
#   docs/assets/screens/advice-readme.png          the advice panel, as the console shows it
#   web/landing/public/swarm{,-light}.{avif,gif,png} and -still.avif
#
# Frames are kept in SWARM_FRAMES (a temporary directory by default) so a
# re-encode does not need a new take.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
frames="${SWARM_FRAMES:-${TMPDIR:-/tmp}/maidan-swarm-frames}"
ffmpeg="${FFMPEG:-ffmpeg}"
port="${SWARM_PORT:-18751}"
origin="http://127.0.0.1:${port}"
admin="swarm-clip-admin-token-0123456789abcdef"
encode_only=0
[[ "${1:-}" == "--encode-only" ]] && encode_only=1

server_pid=""
stop_server() {
  [[ -n "$server_pid" ]] && kill "$server_pid" 2>/dev/null || true
  server_pid=""
}
trap stop_server EXIT

take() {
  local theme=$1 dir="$frames/$1"
  : "${MAIDAN_BIN_DIR:?set MAIDAN_BIN_DIR to the directory holding maidan-server}"
  : "${TYPESAFE_API_KEY:?the advice in the take is a real answer from the provider, so set TYPESAFE_API_KEY}"
  rm -rf "$dir" && mkdir -p "$dir/data"
  # An empty database per take: the board opens on exactly what the take seeds.
  DATABASE_URL="sqlite://$dir/data/swarm.db?mode=rwc" MAIDAN_ALLOW_INSECURE_DEV_KEK=1 \
    MAIDAN_SESSION_SECRET=swarm-clip-session-secret-0123456789abcdef MAIDAN_DEV_ADMIN_TOKEN="$admin" \
    MAIDAN_BIND="127.0.0.1:${port}" MAIDAN_PUBLIC_ORIGIN="$origin" MAIDAN_COOKIE_SECURE=0 \
    MAIDAN_RATE_LIMIT_MAX=0 MAIDAN_JEV_LAND_GATE_ENABLED=1 \
    "$MAIDAN_BIN_DIR/maidan-server" >"$dir/server.log" 2>&1 &
  server_pid=$!
  for _ in $(seq 1 60); do
    curl -fsS -m 2 "$origin/health" >/dev/null 2>&1 && break
    kill -0 "$server_pid" 2>/dev/null || { echo "the server exited: $(tail -3 "$dir/server.log")" >&2; exit 1; }
    sleep 1
  done
  (cd "$root/ui-tests" && STAGE_ORIGIN="$origin" STAGE_ADMIN_TOKEN="$admin" STAGE_THEME="$theme" \
    node capture/swarm/stage.mjs "$dir")
  stop_server
}

# The two halves of a take, stacked, at 30 frames a second.
stacked() {
  "$ffmpeg" -v error -y -f concat -safe 0 -i "$1/console/frames.ffconcat" -f concat -safe 0 -i "$1/agents/frames.ffconcat" \
    -filter_complex "[0:v]fps=30,scale=1280:-2:flags=lanczos[a];[1:v]fps=30,scale=1280:-2:flags=lanczos[b];[a][b]vstack=shortest=1,format=yuv420p" \
    -c:v libx264 -preset slow -crf 20 -movflags +faststart "$2"
}
gif() {
  "$ffmpeg" -v error -y -i "$1" -filter_complex \
    "fps=12,scale=960:-2:flags=lanczos,split[a][b];[a]palettegen=max_colors=128:stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=4:diff_mode=rectangle" "$2"
}

# SWARM_THEMES="dark" retakes one theme and encodes both from what is kept.
if [[ $encode_only -eq 0 ]]; then
  for theme in ${SWARM_THEMES:-dark light}; do take "$theme"; done
fi

landing="$root/web/landing/public"
out="$root/docs/assets/motion"
for theme in dark light; do
  suffix=""
  [[ "$theme" == light ]] && suffix="-light"
  dir="$frames/$theme"
  stacked "$dir" "$dir/swarm.mp4"
  gif "$dir/swarm.mp4" "$landing/swarm$suffix.gif"
  # The landing shows an animated AVIF where the browser reads one, a fifth
  # of the GIF's bytes, and a still for a reader who stops it.
  "$ffmpeg" -v error -y -i "$dir/swarm.mp4" -vf "fps=15,scale=1440:-2:flags=lanczos" \
    -c:v libsvtav1 -crf 40 -preset 4 -an -f avif "$landing/swarm$suffix.avif"
  # The still is the last frame: the task done, every call that got it there.
  "$ffmpeg" -v error -y -sseof -0.2 -i "$dir/swarm.mp4" -vframes 1 -vf "scale=1440:-2:flags=lanczos" "$landing/swarm$suffix.png"
  "$ffmpeg" -v error -y -i "$landing/swarm$suffix.png" -c:v libaom-av1 -still-picture 1 -crf 30 "$landing/swarm$suffix-still.avif"
  if [[ "$theme" == dark ]]; then
    cp "$dir/swarm.mp4" "$out/swarm.mp4"
    cp "$landing/swarm.gif" "$out/swarm.gif"
    # The advice panel, cut from the frame the take marked.
    read -r frame x y w h < <(jq -r '[.frame, .x, .y, .w, .h] | @tsv' "$dir/advice-still.json")
    "$ffmpeg" -v error -y -i "$dir/$frame" -vf "crop=$w:$h:$x:$y" "$root/docs/assets/screens/advice-readme.png"
  fi
done
ls -la "$out"/swarm.* "$landing"/swarm* | awk '{print $5, $9}'

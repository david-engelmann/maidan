#!/usr/bin/env bash
# Measures how long a hosted instance goes unanswered across a deploy or a
# restart: start it, deploy, and read the summary when it stops.
#
#   scripts/measure-deploy-gap.sh [--origin URL] [--interval SECS] [--for SECS]
#                                 [--until-version V] [--settle N]
#
# It asks `<origin>/health` once per interval, one request at a time, and
# counts a request answered when the body names a `version` (the server's
# own answer, healthy or not). The Worker's 503 while no server answers, a
# timeout and a network error are unanswered. It stops after `--for` seconds
# (default 900), or once `--until-version` has answered `--settle` times in a
# row (default 5), and prints:
#
#   unanswered=N            requests nothing answered
#   longest_outage_s=S      the longest run of them, from the first one's start
#                           to the end of the answer that ended it (0: none)
#   slowest_answer_s=S      the longest single answer: a request held at the
#                           edge while the container was replaced
#   longest_gap_s=S         the longest time between two answers
#   versions=A>B            the versions seen, in order
#
# Zero-downtime is `unanswered=0`; `slowest_answer_s` is how long the edge
# held a request. The exit status is 0 either way: this measures, it does not
# gate (docs/Hosted Operations.md, "Deploys without downtime").
set -uo pipefail

origin=https://dev.maidan.world
interval=1
budget=900
until_version=""
settle=5
while [ $# -gt 0 ]; do
  case "$1" in
    --origin) origin=$2; shift 2 ;;
    --interval) interval=$2; shift 2 ;;
    --for) budget=$2; shift 2 ;;
    --until-version) until_version=$2; shift 2 ;;
    --settle) settle=$2; shift 2 ;;
    -h|--help) sed -n '2,25p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
origin=${origin%/}
for tool in curl jq awk; do
  command -v "$tool" > /dev/null || { echo "measure-deploy-gap.sh needs $tool" >&2; exit 2; }
done

# Seconds with a fraction where the shell has them (bash 5), whole seconds
# where it does not (macOS's bash 3.2).
now() {
  if [ -n "${EPOCHREALTIME:-}" ]; then echo "${EPOCHREALTIME/,/.}"; else date +%s; fi
}
# a - b, and max(a, b), to a tenth of a second.
sub() { awk -v a="$1" -v b="$2" 'BEGIN { printf "%.1f", a - b }'; }
max() { awk -v a="$1" -v b="$2" 'BEGIN { print (a + 0 > b + 0) ? a : b }'; }

body=$(mktemp)
trap 'rm -f "$body"' EXIT

t0=$(now)
unanswered=0
longest_outage=0
slowest=0
longest_gap=0
outage_start=""
last_answer=""
last_version=""
versions=""
in_a_row=0

echo "measuring $origin/health every ${interval}s (for at most ${budget}s)"
while :; do
  start=$(now)
  [ "$(sub "$start" "$t0" | cut -d. -f1)" -ge "$budget" ] && break
  # Longer than the Worker's hold (90 s), so a held request is timed, not cut.
  curl -sS -o "$body" --max-time 120 "$origin/health" 2>/dev/null
  end=$(now)
  version=$(jq -r '.version // empty' "$body" 2>/dev/null || true)
  at=$(sub "$start" "$t0")
  if [ -n "$version" ]; then
    took=$(sub "$end" "$start")
    slowest=$(max "$slowest" "$took")
    if [ -n "$outage_start" ]; then
      outage=$(sub "$end" "$outage_start")
      longest_outage=$(max "$longest_outage" "$outage")
      echo "t=${at}s answered again after ${outage}s unanswered"
      outage_start=""
    fi
    if [ -n "$last_answer" ]; then
      longest_gap=$(max "$longest_gap" "$(sub "$end" "$last_answer")")
    fi
    last_answer=$end
    if [ "$version" != "$last_version" ]; then
      echo "t=${at}s version ${last_version:-(none)} -> $version (answer took ${took}s)"
      versions=${versions:+$versions>}$version
      last_version=$version
      in_a_row=0
    fi
    in_a_row=$((in_a_row + 1))
    if [ -n "$until_version" ] && [ "$version" = "$until_version" ] && [ "$in_a_row" -ge "$settle" ]; then
      break
    fi
  else
    unanswered=$((unanswered + 1))
    if [ -z "$outage_start" ]; then
      outage_start=$start
      echo "t=${at}s unanswered"
    fi
  fi
  sleep "$interval"
done
if [ -n "$outage_start" ]; then
  longest_outage=$(max "$longest_outage" "$(sub "$(now)" "$outage_start")")
  echo "still unanswered when the measurement stopped"
fi

echo "unanswered=$unanswered"
echo "longest_outage_s=$longest_outage"
echo "slowest_answer_s=$slowest"
echo "longest_gap_s=$longest_gap"
echo "versions=${versions:-none}"

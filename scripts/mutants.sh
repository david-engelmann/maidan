#!/usr/bin/env bash
# The nightly mutation job's two halves: plan the store's shards, and report a
# shard's outcome. `.github/workflows/nightly.yml` runs both; either runs
# locally too.
#
#   scripts/mutants.sh plan [BASE [FIRST_SHARD]]
#       The store is mutated only where it changed: the diff of
#       crates/maidan-store/src from BASE (default: the last commit more than
#       25 hours old, so consecutive nightlies overlap rather than leave a gap)
#       to HEAD. Counts those mutants and splits them into shards of at most
#       STORE_MUTANTS_PER_SHARD, of which at most STORE_MAX_SHARDS run,
#       starting at FIRST_SHARD (default 0). Writes base, count, total (the
#       shard count every job passes to --shard) and matrix (the shard indexes
#       to run, as JSON) to $GITHUB_OUTPUT when set.
#
#   scripts/mutants.sh report LABEL EXIT_CODE [MUTANTS_OUT]
#       Summarizes a cargo-mutants run (default output: mutants.out) into
#       $GITHUB_STEP_SUMMARY, or stdout. Missed and timed-out mutants are
#       findings: they are listed and warned about, and the report passes. An
#       empty EXIT_CODE means cargo-mutants never finished (the step's time
#       limit ended it); that, a failed baseline or any other error fails.
#
# The per-shard size comes from a measured sample; docs/Conventions.md
# ("Nightly mutation testing") has the numbers and how to remeasure.
set -euo pipefail

cd "$(dirname "$0")/.."

STORE_MUTANTS_PER_SHARD="${STORE_MUTANTS_PER_SHARD:-20}"
STORE_MAX_SHARDS="${STORE_MAX_SHARDS:-10}"

summary() {
  if [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then
    cat >>"$GITHUB_STEP_SUMMARY"
  else
    cat
  fi
}

output() {
  if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
    echo "$1" >>"$GITHUB_OUTPUT"
  fi
  echo "$1"
}

plan() {
  local base="${1:-}" first="${2:-0}"
  first="${first:-0}"
  if [[ -z "$base" ]]; then
    base="$(git rev-list -1 --before="25 hours ago" HEAD)"
  fi
  if [[ -z "$base" ]]; then
    echo "no commit older than 25 hours: fetch the full history (fetch-depth: 0)" >&2
    exit 1
  fi
  base="$(git rev-parse --verify "$base^{commit}")"

  local diff
  diff="$(mktemp)"
  git diff "$base" HEAD -- crates/maidan-store/src >"$diff"
  local count=0
  if [[ -s "$diff" ]]; then
    # An empty or Rust-free diff prints no list at all, not an empty one.
    count="$(cargo mutants --list --json --package maidan-store --in-diff "$diff" | jq length)"
    count="${count:-0}"
  fi
  rm -f "$diff"

  local total=$(((count + STORE_MUTANTS_PER_SHARD - 1) / STORE_MUTANTS_PER_SHARD))
  local end=$((first + STORE_MAX_SHARDS < total ? first + STORE_MAX_SHARDS : total))
  local run=$((end > first ? end - first : 0))
  local matrix
  matrix="$(jq -cn --argjson a "$first" --argjson b "$end" '[range($a; $b)]')"

  output "base=$base"
  output "count=$count"
  output "total=$total"
  output "matrix=$matrix"

  {
    echo "### Store mutation plan"
    echo
    echo "$count mutants in \`crates/maidan-store/src\` changed since \`${base:0:12}\`."
    if ((run > 0)); then
      echo "$total shard(s) of at most $STORE_MUTANTS_PER_SHARD; shards $first to $((end - 1)) run."
    fi
  } | summary
  if ((end < total)); then
    echo "::warning title=store mutants not tested::shards $end to $((total - 1)) of $total (up to $(((total - end) * STORE_MUTANTS_PER_SHARD)) mutants) do not run; run nightly.yml by hand with jobs=store, store_base=$base and store_first_shard=$end"
  fi
}

report() {
  local label="$1" code="$2" out="${3:-mutants.out}"
  local planned=0 tested=0
  if [[ -f "$out/mutants.json" ]]; then
    planned="$(jq length "$out/mutants.json")"
  fi
  if [[ -f "$out/outcomes.json" ]]; then
    tested="$(jq '[.outcomes[] | select(.scenario != "Baseline")] | length' "$out/outcomes.json")"
  fi
  local caught=0 missed=0 timeout=0 unviable=0
  [[ -f "$out/caught.txt" ]] && caught="$(wc -l <"$out/caught.txt" | tr -d ' ')"
  [[ -f "$out/missed.txt" ]] && missed="$(wc -l <"$out/missed.txt" | tr -d ' ')"
  [[ -f "$out/timeout.txt" ]] && timeout="$(wc -l <"$out/timeout.txt" | tr -d ' ')"
  [[ -f "$out/unviable.txt" ]] && unviable="$(wc -l <"$out/unviable.txt" | tr -d ' ')"

  {
    echo "### Mutation tests: $label"
    echo
    echo "| tested | caught | missed | timeout | unviable |"
    echo "|---|---|---|---|---|"
    echo "| $tested of $planned | $caught | $missed | $timeout | $unviable |"
    for kind in missed timeout; do
      if [[ -s "$out/$kind.txt" ]]; then
        echo
        echo "<details><summary>$kind</summary>"
        echo
        echo '```'
        cat "$out/$kind.txt"
        echo '```'
        echo "</details>"
      fi
    done
  } | summary

  # cargo-mutants: 0 every mutant caught, 2 some missed, 3 some timed out,
  # 4 the unmutated tree failed its tests, anything else an error.
  case "$code" in
    0) ;;
    2 | 3)
      echo "::warning title=mutants in $label::$missed missed, $timeout timed out (see the job summary)"
      ;;
    "")
      echo "::error title=mutants in $label::the step's time limit ended the run after $tested of $planned mutants: the shard needs to be smaller (docs/Conventions.md, \"Nightly mutation testing\")"
      exit 1
      ;;
    4)
      echo "::error title=mutants in $label::the unmutated tree failed its tests, so no mutant result means anything"
      exit 1
      ;;
    *)
      echo "::error title=mutants in $label::cargo-mutants exited $code"
      exit 1
      ;;
  esac
}

case "${1:-}" in
  plan) shift; plan "$@" ;;
  report) shift; report "$@" ;;
  *) echo "usage: $0 plan [BASE [FIRST_SHARD]] | report LABEL EXIT_CODE [MUTANTS_OUT]" >&2; exit 2 ;;
esac

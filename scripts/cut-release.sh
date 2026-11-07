#!/usr/bin/env bash
# The steps of .github/workflows/cut-release.yml, one subcommand each, so the
# workflow stays short and every step can be read and shellchecked here.
#
#   next-version [X.Y.Z]       the release to cut: X.Y.Z, or the next major
#                              after the newest v*.*.* tag. An X.Y.Z already
#                              tagged is accepted, to resume after the tag
#   tagged X.Y.Z               the commit vX.Y.Z tags, if the tag exists
#   green-base                 the newest first-parent dev commit whose push
#                              `ci` run concluded success, looked up by SHA
#   record-on-dev X.Y.Z        the dev commit that added the vX.Y.Z record, if
#                              dev has one and the tag does not exist (a rerun)
#   wait-checks SHA            wait for the checks on a PR head; fail unless
#                              dev's required checks passed and none failed
#   wait-dev-ci SHA            wait for dev's push `ci` on SHA, or on a newer
#                              first-parent commit that contains it; print the
#                              commit to promote
#   promote SHA                fast-forward main to SHA (RELEASE_TOKEN)
#   tag SHA TAG TITLE          an annotated tag on SHA (GH_TOKEN)
#   wait-run WORKFLOW SHA|TITLE VALUE SINCE
#                              wait for a workflow_dispatch run of WORKFLOW
#                              created after SINCE, matched by head SHA or by
#                              its run name; fail unless it succeeded
#   health TAG                 wait for maidan.world/health to report TAG
#
# Reads REPO (owner/name) and GH_TOKEN; `promote` also reads RELEASE_TOKEN.
# Every GitHub run is looked up by its commit, never by "the newest run":
# `gh run list --limit 1` once answered a months-old commit.
set -euo pipefail

: "${REPO:?set REPO=owner/name}"
POLL="${POLL:-60}"

die() {
  echo "::error::$*" >&2
  exit 1
}

ci_run() { # $1 = sha: "<status> <conclusion>" of its newest dev push ci run, or "none"
  local row
  row="$(gh api -X GET "repos/${REPO}/actions/workflows/ci.yml/runs" \
    -f head_sha="$1" -f event=push -f branch=dev -f per_page=20 \
    --jq '.workflow_runs | max_by(.id) // empty | "\(.status) \(.conclusion // "none")"')"
  echo "${row:-none}"
}

next_version() {
  local want="${1:-}" newest major
  newest="$(git tag --list 'v*.*.*' | sed 's/^v//' | grep -E '^[0-9]+\.[0-9]+\.[0-9]+$' | sort -t. -k1,1n -k2,2n -k3,3n | tail -n 1)"
  [[ -n "$newest" ]] || die "no v*.*.* tag here (fetch the tags)"
  if [[ -n "$want" ]]; then
    want="${want#v}"
    [[ "$want" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "version must be X.Y.Z, got ${want}"
    # A rerun after the tag names it: the run resumes at the release.
    if git rev-parse -q --verify "refs/tags/v${want}" >/dev/null; then
      echo "$want"
      return
    fi
    [[ "$(printf '%s\n%s\n' "$newest" "$want" | sort -t. -k1,1n -k2,2n -k3,3n | tail -n 1)" == "$want" && "$want" != "$newest" ]] ||
      die "v${want} is not newer than the newest tag v${newest}"
    echo "$want"
    return
  fi
  major="${newest%%.*}"
  echo "$((major + 1)).0.0"
}

green_base() {
  local sha row
  while read -r sha; do
    row="$(ci_run "$sha")"
    if [[ "$row" == "completed success" ]]; then
      echo "$sha"
      return
    fi
    echo "dev ${sha:0:8}: ci ${row}" >&2
  done < <(git rev-list --first-parent --max-count=40 origin/dev)
  die "none of dev's 40 newest commits has a green push ci run"
}

tagged() {
  git rev-parse -q --verify "refs/tags/v${1}^{commit}" || true
}

record_on_dev() {
  local version="$1"
  if git show origin/dev:docs/Capabilities.md | grep -m 1 -E '^## \[v[0-9]' | grep -qF "## [v${version}]"; then
    git log --first-parent --format=%H -S "## [v${version}](" origin/dev -- docs/Capabilities.md | tail -n 1
  fi
}

wait_checks() {
  local sha="$1" required runs pending failed missing name
  required="$(gh api "repos/${REPO}/branches/dev" --jq '.protection.required_status_checks.contexts[]')"
  if [[ -z "$required" ]]; then
    echo "::warning::could not read dev's required checks; waiting for the eight CLAUDE.md lists" >&2
    required="$(printf '%s\n' "lint (fmt + clippy + deny)" "secrets scan" "unit tests" \
      "integration (testcontainers)" "docker compose smoke" "scale-out smoke" \
      "promtool (alert rules)" "otlp smoke")"
  fi
  echo "waiting for the checks on ${sha:0:8}; required:"
  while IFS= read -r name; do echo "  ${name}"; done <<<"$required"
  while :; do
    # GitHub Actions' check runs only (CodeRabbit's is advisory), newest per name.
    runs="$(gh api --paginate -X GET "repos/${REPO}/commits/${sha}/check-runs" -f per_page=100 \
      --jq '.check_runs[] | select(.app.slug == "github-actions") | [.id, .name, .status, (.conclusion // "none")] | @tsv' |
      sort -t$'\t' -k2,2 -k1,1n | awk -F'\t' '{last[$2]=$0} END {for (n in last) print last[n]}')"
    failed="$(awk -F'\t' '$3 == "completed" && ($4 == "failure" || $4 == "timed_out" || $4 == "action_required") {print $2 " (" $4 ")"}' <<<"$runs")"
    [[ -z "$failed" ]] || die "checks failed on ${sha:0:8}: $(tr '\n' ',' <<<"$failed")"
    pending="$(awk -F'\t' '$3 != "completed" {print $2}' <<<"$runs")"
    missing=""
    while IFS= read -r name; do
      awk -F'\t' -v n="$name" '$2 == n && $3 == "completed" && ($4 == "success" || $4 == "skipped") {found=1} END {exit !found}' <<<"$runs" ||
        missing+="${name}; "
    done <<<"$required"
    if [[ -z "$pending" && -z "$missing" ]]; then
      echo "every check on ${sha:0:8} is done and the required ones passed"
      return
    fi
    # A required check that finished other than success or skipped (cancelled,
    # neutral, stale) will not turn green by waiting.
    while IFS= read -r name; do
      if awk -F'\t' -v n="$name" '$2 == n && $3 == "completed" && $4 != "success" && $4 != "skipped" {found=1} END {exit !found}' <<<"$runs"; then
        die "required check '${name}' on ${sha:0:8} finished without passing"
      fi
    done <<<"$required"
    echo "$(date -u +%H:%M) waiting: ${missing}$(grep -c . <<<"$pending" || true) running"
    sleep "$POLL"
  done
}

wait_dev_ci() {
  local base="$1" sha row own
  while :; do
    git fetch -q origin dev
    git merge-base --is-ancestor "$base" origin/dev || die "${base:0:8} is not on dev"
    # Newest first: the merge commit's run can be cancelled by a newer merge
    # (dev's ci queue keeps one pending run), and a newer green commit that
    # contains the record is as good a release commit.
    while read -r sha; do
      row="$(ci_run "$sha")"
      if [[ "$row" == "completed success" ]]; then
        echo "dev ci is green on ${sha:0:8}" >&2
        echo "$sha"
        return
      fi
    done < <(git rev-list --first-parent "${base}^..origin/dev")
    own="$(ci_run "$base")"
    case "$own" in
      "completed failure" | "completed timed_out" | "completed startup_failure")
        die "dev's push ci on ${base:0:8} is ${own#completed }; fix dev and run cut release again" ;;
    esac
    echo "$(date -u +%H:%M) dev ci on ${base:0:8}: ${own}" >&2
    sleep "$POLL"
  done
}

promote() {
  local sha="$1" main status now
  [[ "$sha" =~ ^[0-9a-f]{40}$ ]] || die "promote needs a full sha"
  [[ -n "${RELEASE_TOKEN:-}" ]] || die "RELEASE_TOKEN is not set (see docs/Operations.md, 'The release token')"
  [[ "$(ci_run "$sha")" == "completed success" ]] || die "dev's push ci on ${sha:0:8} is not green"
  main="$(gh api "repos/${REPO}/git/ref/heads/main" --jq .object.sha)"
  if [[ "$main" == "$sha" ]]; then
    echo "main is already at ${sha:0:8}"
    return
  fi
  status="$(gh api "repos/${REPO}/compare/${main}...${sha}" --jq .status)"
  [[ "$status" == "ahead" ]] || die "${sha:0:8} is not a fast-forward of main ${main:0:8} (${status}); refusing"
  # force=false: GitHub itself refuses anything but a fast-forward. The token
  # is the maintainer's: an admin passes main's lock and required review
  # (enforce_admins is off), and GITHUB_TOKEN cannot.
  if ! GH_TOKEN="$RELEASE_TOKEN" gh api -X PATCH "repos/${REPO}/git/refs/heads/main" \
    -f sha="$sha" -F force=false --silent; then
    die "RELEASE_TOKEN could not fast-forward main. It must be an admin's token with Contents and Workflows read/write on ${REPO} (docs/Operations.md, 'The release token')."
  fi
  now="$(gh api "repos/${REPO}/git/ref/heads/main" --jq .object.sha)"
  [[ "$now" == "$sha" ]] || die "main is at ${now:0:8} after the update, not ${sha:0:8}"
  echo "main fast-forwarded ${main:0:8} -> ${sha:0:8}"
}

tag() {
  local sha="$1" tag="$2" title="$3" existing object
  existing="$(gh api "repos/${REPO}/git/matching-refs/tags/${tag}" --jq ".[] | select(.ref == \"refs/tags/${tag}\") | .object.sha")"
  if [[ -n "$existing" ]]; then
    object="$(gh api "repos/${REPO}/git/tags/${existing}" --jq .object.sha 2>/dev/null || echo "$existing")"
    [[ "$object" == "$sha" ]] || die "${tag} exists on ${object:0:8}, not ${sha:0:8}"
    echo "${tag} is already on ${sha:0:8}"
    return
  fi
  object="$(gh api -X POST "repos/${REPO}/git/tags" -f tag="$tag" -f message="${tag}: ${title}" \
    -f object="$sha" -f type=commit --jq .sha)"
  gh api -X POST "repos/${REPO}/git/refs" -f ref="refs/tags/${tag}" -f sha="$object" --silent
  echo "tagged ${tag} at ${sha:0:8}"
}

wait_run() {
  local workflow="$1" by="$2" value="$3" since="$4" filter row id waited=0
  case "$by" in
    SHA) filter=".head_sha == \"${value}\"" ;;
    TITLE) filter=".display_title == \"${value}\"" ;;
    *) die "wait-run matches by SHA or TITLE" ;;
  esac
  while :; do
    row="$(gh api -X GET "repos/${REPO}/actions/workflows/${workflow}/runs" \
      -f event=workflow_dispatch -f created=">=${since}" -f per_page=50 \
      --jq "[.workflow_runs[] | select(${filter})] | max_by(.id) // empty | \"\(.id) \(.status) \(.conclusion // \"none\")\"")"
    if [[ -n "$row" ]]; then
      id="${row%% *}"
      case "${row#* }" in
        "completed success")
          echo "${workflow} run ${id} succeeded"
          return ;;
        completed*)
          die "${workflow} run ${id} is ${row#* }: https://github.com/${REPO}/actions/runs/${id}" ;;
      esac
    elif ((waited >= 1800)); then
      # The run before it dispatches this one; half an hour with no run means
      # the dispatch was lost, not that it is slow.
      die "no ${workflow} run for ${value} started in 30 minutes"
    fi
    waited=$((waited + POLL))
    echo "$(date -u +%H:%M) ${workflow} (${value}): ${row:-not started}"
    sleep "$POLL"
  done
}

health() {
  local tag="$1" got="" _
  for _ in $(seq 1 30); do
    got="$(curl -fsS -m 10 https://maidan.world/health | jq -r '.version // empty' || true)"
    if [[ "$got" == "$tag" || "v${got}" == "$tag" ]]; then
      echo "maidan.world serves ${tag}"
      return
    fi
    sleep 20
  done
  die "maidan.world reports ${got:-nothing}, not ${tag}"
}

cmd="${1:-}"
shift || true
case "$cmd" in
  next-version) next_version "$@" ;;
  green-base) green_base ;;
  record-on-dev) record_on_dev "$@" ;;
  tagged) tagged "$@" ;;
  wait-checks) wait_checks "$@" ;;
  wait-dev-ci) wait_dev_ci "$@" ;;
  promote) promote "$@" ;;
  tag) tag "$@" ;;
  wait-run) wait_run "$@" ;;
  health) health "$@" ;;
  *) die "usage: $0 next-version|tagged|green-base|record-on-dev|wait-checks|wait-dev-ci|promote|tag|wait-run|health ..." ;;
esac

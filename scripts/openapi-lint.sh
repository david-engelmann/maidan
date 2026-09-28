#!/usr/bin/env bash
# OpenAPI external verifier: Redocly's recommended ruleset against the
# document a real Maidan serves at `GET /openapi.json`.
#
# The unit tests in crates/maidan-server/src/openapi/lint.rs assert the same
# properties (a summary on every operation, the client errors it can return
# as RFC 9457 problems, 401/429 wherever the middleware can answer them, 3.1
# with no unused components) without Node, and run in the `unit tests` job.
# This reproduces the third-party lint they stand in for.
#
# Boots a source-built Maidan on a throwaway SQLite file, saves the document
# to target/openapi/openapi.json (the path .redocly.lint-ignore.yaml is keyed
# by) and lints it with redocly.yaml. Exits non-zero on any error or warning.
#
# Usage:  scripts/openapi-lint.sh
# Env:    MAIDAN_OPENAPI_PORT (default 18091)
#         MAIDAN_REDOCLY_VERSION (default 2.54.3, the version verified)
#         MAIDAN_BIN_DIR (default ./target/debug; skip the build if set)
set -euo pipefail

cd "$(dirname "$0")/.."
# A throwaway database, so the public development content KEK is acceptable.
export MAIDAN_ALLOW_INSECURE_DEV_KEK=1

port="${MAIDAN_OPENAPI_PORT:-18091}"
redocly="@redocly/cli@${MAIDAN_REDOCLY_VERSION:-2.54.3}"
base="http://127.0.0.1:${port}"
bin_dir="${MAIDAN_BIN_DIR:-}"
out="target/openapi/openapi.json"

if [[ -z "$bin_dir" ]]; then
  echo "=== building maidan-server ==="
  cargo build --quiet --bin maidan-server
  bin_dir="$(cargo metadata --format-version 1 --no-deps | jq -r .target_directory)/debug"
fi

work="$(mktemp -d)"
server_pid=""
cleanup() {
  [[ -n "$server_pid" ]] && kill "$server_pid" 2>/dev/null || true
  rm -rf "$work"
}
trap cleanup EXIT

# A document served by whatever else holds the port would be linted in place
# of this build's, so refuse a port that is already taken.
if curl -s -o /dev/null "${base}/health" 2>/dev/null; then
  echo "port ${port} is already serving; set MAIDAN_OPENAPI_PORT" >&2
  exit 1
fi

echo "=== booting ==="
DATABASE_URL="sqlite://${work}/maidan.db?mode=rwc" \
  MAIDAN_SESSION_SECRET="openapi-lint-session-secret-0123456789abcdef" \
  MAIDAN_BIND="127.0.0.1:${port}" \
  "${bin_dir}/maidan-server" >"${work}/server.log" 2>&1 &
server_pid=$!
for _ in $(seq 1 60); do
  kill -0 "$server_pid" 2>/dev/null || { cat "${work}/server.log" >&2; exit 1; }
  curl -sf "${base}/health" >/dev/null 2>&1 && break
  sleep 1
done
curl -sf "${base}/health" >/dev/null || { cat "${work}/server.log" >&2; exit 1; }

mkdir -p "$(dirname "$out")"
curl -sf "${base}/openapi.json" | jq . >"$out"
echo "=== ${redocly} lint ${out} ==="
npx -y "$redocly" lint --max-problems 1000 "$out"

# Redocly exits zero on warnings; the recommended set is only clean without them.
report="$(npx -y "$redocly" lint --format json "$out" 2>/dev/null)"
problems="$(jq '.problems | length' <<<"$report")"
(( problems == 0 )) || { echo "FAIL: ${problems} problems" >&2; exit 1; }
echo "=== the OpenAPI document lints clean ==="

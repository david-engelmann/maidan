#!/usr/bin/env bash
# SDK black-box test harness. Boots a source-built Maidan on
# SQLite with real member-bound tokens, waits for health, runs the chosen language's
# SDK test suite against it, then tears the server down.
#
# Usage:  scripts/sdk-test.sh [typescript|python|go|rust]   (default: typescript)
# Env:    MAIDAN_SDK_PORT (default 8080).
# Gives the suites MAIDAN_URL, MAIDAN_TOKEN and MAIDAN_WORKSPACE, and a second
# workspace's MAIDAN_OTHER_TOKEN and MAIDAN_OTHER_WORKSPACE.
#         CARGO_TARGET_DIR (the built binaries are read from there).
#         MAIDAN_BIN_DIR (holds maidan-server; skip the build if set)
#
# Build first, then run the binary, so the health-wait covers only boot (a cold
# `cargo run` compile would otherwise outlast it).
set -euo pipefail
cd "$(dirname "$0")/.."
# A throwaway database, so the public development content KEK is acceptable.
export MAIDAN_ALLOW_INSECURE_DEV_KEK=1

lang="${1:-typescript}"
port="${MAIDAN_SDK_PORT:-8080}"
base="http://127.0.0.1:${port}"

bin_dir="${MAIDAN_BIN_DIR:-}"
if [[ -z "$bin_dir" ]]; then
  echo "=== building maidan-server ==="
  cargo build --quiet --bin maidan-server
  # cargo writes under CARGO_TARGET_DIR when it is set, otherwise ./target.
  bin_dir="${CARGO_TARGET_DIR:-target}/debug"
fi

sdk_tmp="$(mktemp -d)"
sdk_db="sqlite://${sdk_tmp}/maidan.db?mode=rwc"
# A per-run operator token: boot opens the empty database's first workspace
# with it, and it opens the workspaces the suites run in.
dev_admin_token="$(od -An -tx1 -N24 /dev/urandom | tr -d ' \n')"

# A per-run signing key, so the suites can export a workspace and import it
# back (`workspaces.import`).
export_key="$(od -An -tx1 -N32 /dev/urandom | tr -d ' \n')"

echo "=== booting (SQLite, authenticated) ==="
DATABASE_URL="${sdk_db}" MAIDAN_SESSION_SECRET="sdk-test-session-secret-change-me-0123456789" \
  MAIDAN_EXPORT_SIGNING_KEY="${export_key}" MAIDAN_DEV_ADMIN_TOKEN="${dev_admin_token}" \
  MAIDAN_RATE_LIMIT_MAX=0 \
  MAIDAN_BIND="127.0.0.1:${port}" "${bin_dir}/maidan-server" &
server_pid=$!
trap 'kill "$server_pid" 2>/dev/null || true; rm -rf "$sdk_tmp"' EXIT

up=0
for _ in $(seq 1 60); do
  if curl -sf -H "authorization: Bearer ${dev_admin_token}" "${base}/me" >/dev/null 2>&1; then
    echo "server is up and the dev admin token resolves"; up=1; break
  fi
  sleep 1
done
[ "$up" -eq 1 ] || { echo "server/token did not become ready within 60s" >&2; exit 1; }

# open_workspace <name> <admin handle>: open a workspace and print its admin
# token and id, one per line.
open_workspace() {
  curl -sf -X POST -H "authorization: Bearer ${dev_admin_token}" \
    -H "content-type: application/json" \
    -d "{\"name\":\"$1\",\"admin_handle\":\"$2\"}" "${base}/operator/workspaces" \
    | python3 -c 'import json,sys; b=json.load(sys.stdin); print(b["token"]["secret"]); print(b["workspace"]["id"])'
}

# The suites' workspace, and a second one with its own admin token, so a suite
# can show that what it reads in the first is invisible from the second.
{ read -r admin_token; read -r workspace_id; } < <(open_workspace sdk-tests admin)
{ read -r other_token; read -r other_workspace; } < <(open_workspace sdk-tests-other other-admin)
test -n "${admin_token}"
test -n "${workspace_id}"
test -n "${other_token}"
test -n "${other_workspace}"

export MAIDAN_URL="${base}"
export MAIDAN_TOKEN="${admin_token}"
export MAIDAN_WORKSPACE="${workspace_id}"
export MAIDAN_OTHER_TOKEN="${other_token}"
export MAIDAN_OTHER_WORKSPACE="${other_workspace}"
echo "=== running ${lang} SDK tests ==="
case "$lang" in
  typescript) (cd sdk/typescript && node --test --test-concurrency=1) ;;
  python)     (cd sdk/python && PYTHONPATH="$PWD/src:${PYTHONPATH:-}" python3 -m pytest -q) ;;
  go)         (cd sdk/go && go test ./...) ;;
  rust)       (cd sdk/rust && cargo test -- --test-threads=1) ;;
  *) echo "unknown language: ${lang}" >&2; exit 2 ;;
esac

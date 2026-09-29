#!/usr/bin/env bash
# A2A conformance: the official A2A TCK (Technology Compatibility Kit) against
# a real Maidan, over all three bindings: JSON-RPC, HTTP+JSON and gRPC.
#
# Boots a source-built Maidan on a throwaway SQLite file with auth ENABLED,
# seeds it with `maidan init`, runs the walkthrough in examples/a2a_interop.py,
# then the TCK at a pinned commit. Fails on any TCK failure, on an exclusion
# that no longer matches a test, or when fewer tests pass than the recorded
# floor (the TCK skips a test whose set-up request fails, so a regression can
# surface as a skip). scripts/a2a-tck/exclusions.txt lists the tests Maidan
# does not run and why.
#
# Usage:  scripts/a2a-tck.sh
# Env:    MAIDAN_A2A_PORT (default 18095)
#         MAIDAN_A2A_GRPC_PORT (default 18096)
#         MAIDAN_BIN_DIR (default ./target/debug; skip the build if set)
#         A2A_TCK_DIR (reuse a checkout of the pinned TCK commit)
set -euo pipefail

cd "$(dirname "$0")/.."
root="$PWD"
# A throwaway database, so the public development content KEK is acceptable.
export MAIDAN_ALLOW_INSECURE_DEV_KEK=1

# a2aproject/a2a-tck: tag 1.0.0.alpha2 plus harness fixes, speaking A2A v1.0.0.
tck_repo="https://github.com/a2aproject/a2a-tck.git"
tck_commit="263b9cfaf16a554bdfb166a7ba5b67716e946349"
# Tests that pass at this commit. Raise it when a fix makes more pass.
min_passed=182

port="${MAIDAN_A2A_PORT:-18095}"
grpc_port="${MAIDAN_A2A_GRPC_PORT:-18096}"
base="http://127.0.0.1:${port}"
bin_dir="${MAIDAN_BIN_DIR:-}"

if [[ -z "$bin_dir" ]]; then
  echo "=== building maidan-server + maidan ==="
  cargo build --quiet --bin maidan-server --bin maidan
  bin_dir="$(cargo metadata --format-version 1 --no-deps | jq -r .target_directory)/debug"
fi

work="$(mktemp -d)"
server_pid=""
cleanup() {
  [[ -n "$server_pid" ]] && kill "$server_pid" 2>/dev/null || true
  rm -rf "$work"
}
trap cleanup EXIT

tck="${A2A_TCK_DIR:-${work}/a2a-tck}"
if [[ ! -d "$tck/.git" ]]; then
  echo "=== fetching the A2A TCK at ${tck_commit:0:12} ==="
  git init --quiet "$tck"
  git -C "$tck" fetch --quiet --depth 1 "$tck_repo" "$tck_commit"
  git -C "$tck" checkout --quiet FETCH_HEAD
fi
[[ "$(git -C "$tck" rev-parse HEAD)" == "$tck_commit" ]] ||
  { echo "$tck is not at the pinned TCK commit $tck_commit" >&2; exit 1; }
if [[ ! -x "$tck/.venv/bin/python" ]]; then
  python3 -m venv "$tck/.venv"
  "$tck/.venv/bin/pip" install --quiet "$tck"
fi

export DATABASE_URL="sqlite://${work}/maidan.db?mode=rwc"
export MAIDAN_SESSION_SECRET="a2a-tck-session-secret-0123456789abcdef"

echo "=== seeding (maidan init) ==="
token="$("${bin_dir}/maidan" init --workspace a2a-tck 2>/dev/null | awk '/^    [A-Za-z0-9_-]{20,}$/ {print $1}')"
[[ -n "$token" ]] || { echo "could not read the admin token from maidan init" >&2; exit 1; }

if curl -s -o /dev/null "${base}/health" 2>/dev/null; then
  echo "port ${port} is already serving; set MAIDAN_A2A_PORT" >&2
  exit 1
fi

echo "=== booting (auth enabled) ==="
# Push configs seal their credentials with the at-rest key; the TCK's webhook
# receiver listens on loopback, which egress refuses by default.
FEDERATION_ENCRYPTION_KEY="$(python3 -c 'import secrets; print(secrets.token_hex(32))')" \
  MAIDAN_ALLOW_PRIVATE_EGRESS=1 MAIDAN_RATE_LIMIT_MAX=0 \
  MAIDAN_A2A_PUBLIC_ORIGIN="${base}" MAIDAN_BIND="127.0.0.1:${port}" \
  MAIDAN_A2A_GRPC_ADDR="127.0.0.1:${grpc_port}" MAIDAN_A2A_GRPC_PUBLIC_ADDR="127.0.0.1:${grpc_port}" \
  "${bin_dir}/maidan-server" >"${work}/server.log" 2>&1 &
server_pid=$!
for _ in $(seq 1 60); do
  kill -0 "$server_pid" 2>/dev/null || { cat "${work}/server.log" >&2; exit 1; }
  curl -sf "${base}/health" >/dev/null 2>&1 && break
  sleep 1
done
curl -sf "${base}/health" >/dev/null || { cat "${work}/server.log" >&2; exit 1; }

echo "=== walkthrough (examples/a2a_interop.py) ==="
MAIDAN_URL="$base" MAIDAN_TOKEN="$token" "$tck/.venv/bin/python" examples/a2a_interop.py

echo "=== A2A TCK (jsonrpc, http_json, grpc) ==="
cd "$tck"
MAIDAN_TOKEN="$token" \
  A2A_TCK_EXCLUSIONS="${root}/scripts/a2a-tck/exclusions.txt" \
  A2A_TCK_MIN_PASSED="$min_passed" \
  PYTHONPATH="${root}/scripts/a2a-tck" \
  "$tck/.venv/bin/python" -m pytest tests/compatibility/ \
  --sut-host="$base" --transport=jsonrpc,http_json,grpc \
  -p maidan_tck -p no:cacheprovider -q --tb=short -rfE

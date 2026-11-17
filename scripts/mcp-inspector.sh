#!/usr/bin/env bash
# MCP external verifier: the official MCP Inspector against a real Maidan.
#
# Maidan's own tests prove its MCP server agrees with itself. This proves it
# agrees with the client most people will point at it: the official Inspector,
# built on the official TypeScript SDK, run as an unmodified external process.
# It is how we found that the SDK could not complete a handshake at all.
#
# Boots a source-built Maidan on a throwaway SQLite file with auth ENABLED,
# seeds it with `maidan init` (a real bearer token), then:
#   1. checks `initialize` negotiation for every supported revision directly;
#   2. runs the Inspector CLI against `/mcp` and `/mcp/streamable`, and against
#      each profile endpoint (`/mcp/worker`, `/mcp/reviewer`, `/mcp/coordinator`),
#      which must list exactly its profile's tools in
#      crates/maidan-mcp/src/profiles.rs: listings connect there (ChatGPT's to
#      the coordinator, which must sell nothing; Cursor's to the worker);
#   3. makes one valid and one invalid call per tool group, and fails if the
#      invalid call's error does not name the argument it got wrong;
#   4. runs scripts/mcp-tool-sweep.py: a valid call to every tool each profile
#      endpoint lists and to every read-only tool, and a bad call to every
#      tool, whose error must name the argument it got wrong.
# Exits non-zero on the first failed check.
#
# Six tools need more than the store, and the server gets each, so the sweep
# can hold them to a real answer:
#   - the multipart artifact tools need the S3 backend: a throwaway MinIO, the
#     image compose.yaml pins, when a Docker daemon answers. Without one they
#     are held only to the error naming S3, and the run says S3 was NOT
#     exercised, at the start and in its last line;
#   - top_up_credits needs a receiving address: a test one on Base Sepolia,
#     with test overage prices;
#   - advise_land_gate needs a land-gate advisor: a local stub of the Jev API.
# The stub also answers the credits watcher's chain RPC, so nothing here
# reaches a real chain or provider, and no key or address in it is real.
#
# A reproducible harness for local use and a report-only CI job, not a
# required gate.
#
# Usage:  scripts/mcp-inspector.sh
# Env:    MAIDAN_MCP_PORT (default 18090)
#         MAIDAN_INSPECTOR_VERSION (default 2.7.0, the version verified)
#         MAIDAN_BIN_DIR (default ./target/debug; skip the build if set)
set -euo pipefail

cd "$(dirname "$0")/.."
# A throwaway database, so the public development content KEK is acceptable.
export MAIDAN_ALLOW_INSECURE_DEV_KEK=1

port="${MAIDAN_MCP_PORT:-18090}"
inspector="@modelcontextprotocol/inspector@${MAIDAN_INSPECTOR_VERSION:-2.7.0}"
base="http://127.0.0.1:${port}"
bin_dir="${MAIDAN_BIN_DIR:-}"

if [[ -z "$bin_dir" ]]; then
  echo "=== building maidan-server + maidan ==="
  cargo build --quiet --bin maidan-server --bin maidan
  bin_dir="$(cargo metadata --format-version 1 --no-deps | jq -r .target_directory)/debug"
fi

fail() {
  echo "FAIL: $*" >&2
  # The log is removed with the work directory on exit, and only it says why
  # the server died.
  if [[ -n "$server_pid" ]] && ! kill -0 "$server_pid" 2>/dev/null; then
    echo "--- the server exited; the end of its log:" >&2
    tail -20 "${work}/server.log" >&2
  fi
  exit 1
}
pass() { echo "  ok  $*"; }

work="$(mktemp -d)"
server_pid=""
stub_pid=""
minio=""
cleanup() {
  [[ -n "$server_pid" ]] && kill "$server_pid" 2>/dev/null || true
  [[ -n "$stub_pid" ]] && kill "$stub_pid" 2>/dev/null || true
  [[ -n "$minio" ]] && docker rm -f "$minio" >/dev/null 2>&1 || true
  rm -rf "$work"
}
trap cleanup EXIT

export DATABASE_URL="sqlite://${work}/maidan.db?mode=rwc"
export MAIDAN_SESSION_SECRET="mcp-inspector-session-secret-0123456789abcdef"
# A throwaway export signing seed, so the export group's probe can export.
export MAIDAN_EXPORT_SIGNING_KEY="0101010101010101010101010101010101010101010101010101010101010101"
# A throwaway secrets key, so the secret tools can store and resolve.
export FEDERATION_ENCRYPTION_KEY="0202020202020202020202020202020202020202020202020202020202020202"

# The stub stands in for TypeSafe's Jev API (the contract
# crates/maidan-server/src/land_gate_advisor.rs speaks) and for the chain's
# JSON-RPC. It refuses a request in any other shape, so a server that stopped
# sending what the real service needs fails here rather than passing on a
# stub that answers anything.
jev_key="mcp-inspector-not-a-typesafe-key"
cat >"${work}/stub.py" <<'PY'
import http.server, json, os, signal, sys

class Stub(http.server.BaseHTTPRequestHandler):
    def answer(self, status, value):
        raw = json.dumps(value).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers.get("Content-Length") or 0)) or b"{}")
        if self.path == "/v1/systemone":
            if self.headers.get("Authorization") != "Bearer " + sys.argv[2]:
                return self.answer(401, {"error": "unknown key"})
            if body.get("questions", {}).get("land", {}).get("type") != "choice" or "state" not in body:
                return self.answer(400, {"error": "not a land question"})
            probabilities = {"green": 0.95, "amber": 0.04, "red": 0.01}
            land = {"type": "choice", "choice": "green", "confidence": 0.95, "probabilities": probabilities}
            usage = {"input_tokens": 1, "output_tokens": 1}
            return self.answer(200, {"model": "jev-stub", "answers": {"land": land}, "usage": usage})
        if self.path == "/rpc":
            # A chain with a head and no transfers: the watcher ticks, credits nothing.
            results = {"eth_blockNumber": "0x64", "eth_getLogs": []}
            if body.get("method") in results:
                return self.answer(200, {"jsonrpc": "2.0", "id": body.get("id"), "result": results[body["method"]]})
            return self.answer(200, {"jsonrpc": "2.0", "id": body.get("id"), "error": {"code": -32601, "message": "no such method"}})
        self.answer(404, {"error": "no such path"})

# A clean exit on the cleanup's kill, or bash reports the job as killed after
# the run's last line.
signal.signal(signal.SIGTERM, lambda *_: sys.exit(0))
server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Stub)
with open(sys.argv[1] + ".tmp", "w") as out:
    out.write(str(server.server_port))
# Renamed into place, so the script never reads a half-written port.
os.replace(sys.argv[1] + ".tmp", sys.argv[1])
server.serve_forever()
PY
python3 "${work}/stub.py" "${work}/stub.port" "$jev_key" >"${work}/stub.log" 2>&1 &
stub_pid=$!
for _ in $(seq 1 50); do
  [[ -s "${work}/stub.port" ]] && break
  kill -0 "$stub_pid" 2>/dev/null || fail "the backend stub exited: $(cat "${work}/stub.log")"
  sleep 0.1
done
[[ -s "${work}/stub.port" ]] || fail "the backend stub did not start: $(cat "${work}/stub.log")"
stub="http://127.0.0.1:$(cat "${work}/stub.port")"

# advise_land_gate: the advisor on, pointed at the stub. The egress guard
# refuses loopback unless it is a listed peer.
export MAIDAN_JEV_LAND_GATE_ENABLED=1
export MAIDAN_JEV_BASE_URL="${stub}/"
export TYPESAFE_API_KEY="$jev_key"
export MAIDAN_PRIVATE_EGRESS_HOSTS=127.0.0.1
# top_up_credits: a receiving address on the test chain. It is the burn
# address, whose key nobody holds, and the sweep only opens an intent.
export MAIDAN_CREDITS_ADDRESS=0x000000000000000000000000000000000000dEaD
export MAIDAN_CREDITS_CHAIN=base-sepolia
export MAIDAN_CREDITS_RPC_URL="${stub}/rpc"
export MAIDAN_OVERAGE_PRICES="mcp_calls=1000000/100000"

# The multipart tools: a throwaway MinIO, so the S3 store the stack runs is
# the one tested. Docker running but MinIO failing to start fails the run, as
# a testcontainers start failure does; only no daemon at all skips S3.
sweep_without=()
if docker info >/dev/null 2>&1; then
  minio_image="$(grep -m1 -oE 'cgr\.dev/chainguard/minio@sha256:[0-9a-f]{64}' compose.yaml)" ||
    fail "compose.yaml pins no cgr.dev/chainguard/minio digest"
  echo "=== starting MinIO for the S3 artifact backend (${minio_image}) ==="
  s3_user=inspector
  s3_secret="inspector-throwaway-secret"
  minio="$(docker run -d --rm -p 127.0.0.1::9000 \
    -e MINIO_ROOT_USER="$s3_user" -e MINIO_ROOT_PASSWORD="$s3_secret" \
    "$minio_image" server /data)" || fail "could not start MinIO"
  s3_endpoint="http://$(docker port "$minio" 9000/tcp | head -1)"
  for _ in $(seq 1 60); do
    curl -sf "${s3_endpoint}/minio/health/ready" >/dev/null 2>&1 && break
    sleep 1
  done
  curl -sf "${s3_endpoint}/minio/health/ready" >/dev/null 2>&1 ||
    fail "MinIO did not become ready: $(docker logs "$minio" 2>&1 | tail -20)"
  docker exec -e "MC_HOST_local=http://${s3_user}:${s3_secret}@127.0.0.1:9000" "$minio" \
    mc mb local/maidan >/dev/null || fail "could not create the MinIO bucket"
  export ARTIFACT_BACKEND=s3
  export S3_ENDPOINT="$s3_endpoint"
  export S3_BUCKET=maidan
  export S3_REGION=us-east-1
  export S3_ACCESS_KEY_ID="$s3_user"
  export S3_SECRET_ACCESS_KEY="$s3_secret"
  pass "MinIO at ${s3_endpoint}, bucket maidan"
else
  echo "=== S3 NOT EXERCISED: no Docker daemon answers, so the four multipart tools are held only to the error naming the S3 backend ==="
  sweep_without+=(--without s3)
fi

# Every log of this run names the source it tested.
echo "=== server commit $(git rev-parse HEAD 2>/dev/null || echo unknown) ==="
echo "=== seeding (maidan init) ==="
token="$("${bin_dir}/maidan" init --workspace inspector 2>/dev/null | awk '/^    [A-Za-z0-9_-]{20,}$/ {print $1}')"
[[ -n "$token" ]] || { echo "could not read the admin token from maidan init" >&2; exit 1; }

# A health check answered by whatever else holds the port would pass every
# later check against the wrong server, so refuse a port that is already taken.
if curl -s -o /dev/null "${base}/health" 2>/dev/null; then
  echo "port ${port} is already serving; set MAIDAN_MCP_PORT" >&2
  exit 1
fi

echo "=== booting (auth enabled) ==="
# A debug build's MCP upload_artifact on S3 overflows tokio's default 2 MiB
# worker stack and aborts the server (the HTTP route and a release build do
# not), and CI runs the debug build. 8 MiB is the main thread's own default.
RUST_MIN_STACK=8388608 MAIDAN_BIND="127.0.0.1:${port}" "${bin_dir}/maidan-server" >"${work}/server.log" 2>&1 &
server_pid=$!
for _ in $(seq 1 60); do
  kill -0 "$server_pid" 2>/dev/null || { cat "${work}/server.log" >&2; exit 1; }
  curl -sf "${base}/health" >/dev/null 2>&1 && break
  sleep 1
done
curl -sf "${base}/health" >/dev/null || { cat "${work}/server.log" >&2; exit 1; }

echo "=== initialize negotiation, every revision ==="
for version in 2026-07-28 2025-11-25 2025-06-18 2025-03-26; do
  headers="${work}/h"
  body="$(curl -s -D "$headers" -X POST "${base}/mcp/streamable" \
    -H "Authorization: Bearer ${token}" \
    -H 'Content-Type: application/json' \
    -H 'Accept: application/json, text/event-stream' \
    -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"${version}\",\"capabilities\":{},\"clientInfo\":{\"name\":\"verifier\",\"version\":\"0\"}}}")"
  got="$(jq -r '.result.protocolVersion // empty' <<<"$body")"
  [[ "$got" == "$version" ]] || fail "initialize ${version} negotiated '${got}': ${body}"
  grep -qi '^mcp-session-id:' "$headers" && fail "initialize ${version} minted a session; it is stateless"
  pass "${version}: echoed, stateless"
done
unknown="$(curl -s -X POST "${base}/mcp/streamable" -H "Authorization: Bearer ${token}" \
  -H 'Content-Type: application/json' -H 'Accept: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"1999-01-01"}}' |
  jq -r '.result.protocolVersion')"
[[ "$unknown" == "2026-07-28" ]] || fail "an unknown revision should get the latest, got '${unknown}'"
pass "unknown revision: offered 2026-07-28"

run() {
  local endpoint="$1"
  shift
  npx -y "$inspector" --cli "${base}${endpoint}" --transport http \
    --header "Authorization: Bearer ${token}" "$@"
}

for endpoint in /mcp/streamable /mcp; do
  echo "=== Inspector ${inspector} against ${endpoint} ==="

  tools="$(run "$endpoint" --method tools/list)" || fail "tools/list: ${tools}"
  count="$(jq '.tools | length' <<<"$tools")"
  (( count > 100 )) || fail "tools/list returned ${count} tools"
  pass "tools/list: ${count} tools, every schema accepted by the SDK"

  whoami="$(run "$endpoint" --method tools/call --tool-name whoami)" || fail "whoami: ${whoami}"
  jq -e '.content[0].text | fromjson | .member_id and (.bypass | not)' <<<"$whoami" >/dev/null ||
    fail "whoami did not return a bearer identity: ${whoami}"
  pass "tools/call whoami: authenticated as the bearer"

  resources="$(run "$endpoint" --method resources/list)" || fail "resources/list: ${resources}"
  jq -e '.resources | length > 0 and all(.uri | test("[{}]") | not)' <<<"$resources" >/dev/null ||
    fail "resources/list must list concrete URIs: ${resources}"
  workspace_uri="$(jq -r '.resources[0].uri' <<<"$resources")"
  pass "resources/list: concrete (${workspace_uri})"

  read="$(run "$endpoint" --method resources/read --uri "$workspace_uri")" || fail "resources/read: ${read}"
  jq -e '.contents | length > 0' <<<"$read" >/dev/null || fail "resources/read returned nothing: ${read}"
  pass "resources/read: the listed resource is readable"

  templates="$(run "$endpoint" --method resources/templates/list)" || fail "resources/templates/list: ${templates}"
  jq -e '.resourceTemplates | length >= 4 and all(.uriTemplate | test("[{]"))' <<<"$templates" >/dev/null ||
    fail "resources/templates/list: ${templates}"
  pass "resources/templates/list: $(jq '.resourceTemplates | length' <<<"$templates") templates"

  prompts="$(run "$endpoint" --method prompts/list)" || fail "prompts/list: ${prompts}"
  jq -e '.prompts | length > 0' <<<"$prompts" >/dev/null || fail "prompts/list: ${prompts}"
  pass "prompts/list: $(jq '.prompts | length' <<<"$prompts") prompts"
done

profile_tools() { # profile_tools NAME: NAME_TOOLS in profiles.rs, one per line, sorted
  python3 - "$1" <<'PY'
import re, sys
src = open("crates/maidan-mcp/src/profiles.rs").read()
body = re.search(sys.argv[1].upper() + r"_TOOLS: &\[&str\] = &\[(.*?)\];", src, re.S)
print("\n".join(sorted(re.findall(r'"([a-z_]+)"', body.group(1)))))
PY
}
# The member the bearer belongs to, as the full catalog names it: every
# profile's whoami must name the same one, not merely some member.
bearer_member="$(run /mcp/streamable --method tools/call --tool-name whoami |
  jq -r '.content[0].text | fromjson | .member_id')"
[[ -n "$bearer_member" && "$bearer_member" != null ]] || fail "whoami on /mcp/streamable named no member"
for profile in worker reviewer coordinator; do
  endpoint="/mcp/${profile}"
  echo "=== Inspector ${inspector} against ${endpoint} ==="
  tools="$(run "$endpoint" --method tools/list)" || fail "tools/list on ${endpoint}: ${tools}"
  listed="$(jq -r '.tools[].name' <<<"$tools" | sort)"
  want="$(profile_tools "$profile")"
  [[ -n "$want" && "$listed" == "$want" ]] ||
    fail "${endpoint} does not list the ${profile} profile: $(diff <(echo "$want") <(echo "$listed") | grep '^[<>]' | tr '\n' ' ')"
  pass "tools/list on ${endpoint}: the ${profile} profile's $(wc -l <<<"$listed" | tr -d ' ') tools, every schema accepted by the SDK"
  whoami="$(run "$endpoint" --method tools/call --tool-name whoami)" || fail "whoami on ${endpoint}: ${whoami}"
  jq -e --arg m "$bearer_member" '.content[0].text | fromjson | .member_id == $m' <<<"$whoami" >/dev/null ||
    fail "whoami on ${endpoint} did not name the bearer ${bearer_member}: ${whoami}"
  pass "tools/call whoami on ${endpoint}: authenticated as the bearer"
  [[ "$profile" == coordinator ]] && coordinator_tools="$listed"
done
if grep -qx -e top_up_credits -e get_credits <<<"$coordinator_tools"; then
  fail "/mcp/coordinator lists a tool that sells credits"
fi
pass "/mcp/coordinator lists neither top_up_credits nor get_credits"

echo "=== one valid and one invalid call per tool group (/mcp/streamable) ==="
endpoint=/mcp/streamable
# `text` is the tool's JSON result, or the error the call printed.
call() {
  local tool="$1"
  shift
  local args=()
  for pair in "$@"; do args+=(--tool-arg "$pair"); done
  run "$endpoint" --method tools/call --tool-name "$tool" ${args[@]+"${args[@]}"} 2>&1
}
field() { jq -r ".content[0].text | fromjson | $1" <<<"$2"; }

me="$(call whoami)" || fail "whoami: ${me}"
ws="$(field .workspace_id "$me")"
member="$(field .member_id "$me")"
made="$(call create_channel "workspace_id=${ws}" name=probes)" || fail "create_channel: ${made}"
channel="$(field .id "$made")"
made="$(call create_thread "channel_id=${channel}" title=probe)" || fail "create_thread: ${made}"
thread="$(field .id "$made")"
made="$(call post_message "thread_id=${thread}" body=probe)" || fail "post_message: ${made}"
message="$(field .id "$made")"
made="$(call request_approval prompt=probe "thread_id=${thread}")" || fail "request_approval: ${made}"
gate="$(field .gate_id "$made")"
made="$(call upload_artifact kind=attachment content_base64=cHJvYmU=)" || fail "upload_artifact: ${made}"
sha="$(field .sha256 "$made")"

# group | a valid call's tool | its arguments | an invalid call's tool | its
# arguments | the argument its error must name. Every module that dispatches a
# tool in crates/maidan-mcp/src/tools/mod.rs has a row, and the check below
# fails when a new module has none. The invalid call must fail and name the
# argument it got wrong, so the error is one an agent can act on. Where a
# group's read takes no argument, the invalid call is one of its writes with a
# required argument missing, which fails before it writes anything. whoami
# takes no argument and its group has no other tool, so it has no invalid call.
probes="
approval|get_approval_gate|gate_id=${gate}|get_approval_gate|gate_id=not-a-uuid|gate_id
artifact|get_artifact_metadata|sha256=${sha}|get_artifact_metadata|sha256=not-a-digest|sha256
automation|list_slash_commands|workspace_id=${ws}|list_slash_commands|workspace_id=not-a-uuid|workspace_id
budget|usage_rollup|workspace_id=${ws}|usage_rollup|workspace_id=not-a-uuid|workspace_id
change_pull|get_swarm_metrics|workspace_id=${ws}|get_swarm_metrics|workspace_id=not-a-uuid|workspace_id
channel|list_channels|workspace_id=${ws}|list_channels|workspace_id=not-a-uuid|workspace_id
credits|get_credits|workspace_id=${ws}|get_credits|workspace_id=not-a-uuid|workspace_id
delivery|list_result_deliveries|thread_id=${thread}|list_result_deliveries|thread_id=not-a-uuid|thread_id
event_log|get_log_snapshot|workspace_id=${ws}|get_log_snapshot|workspace_id=not-a-uuid|workspace_id
event_wait|wait_for_event|kinds=[\"thread_landed\"] timeout_ms=1|wait_for_event|kinds=[\"no_such_kind\"]|no_such_kind
explorer|list_tombstones|workspace_id=${ws}|list_tombstones|workspace_id=not-a-uuid|workspace_id
export|export_workspace|workspace_id=${ws}|export_workspace|workspace_id=not-a-uuid|workspace_id
freeze|list_frozen_members||freeze_member|reason=probe|member_id
glossary|list_glossary_terms||get_glossary_term||term
land_gate|get_land_gate|thread_id=${thread}|get_land_gate|thread_id=not-a-uuid|thread_id
member|list_members|workspace_id=${ws}|list_members|workspace_id=not-a-uuid|workspace_id
memory_block|list_memory_blocks||get_memory_block||label
message|list_messages|thread_id=${thread}|list_messages|thread_id=not-a-uuid|thread_id
projector|list_github_issue_links||link_github_issue|repo=o/r issue_number=1|thread_id
recipe|list_recipes||create_recipe|name=probe spec={}|channel_id
reference|list_references|src_kind=thread src_id=${thread}|list_references|src_id=not-a-uuid|src_id
review|get_review_status|thread_id=${thread}|get_review_status|thread_id=not-a-uuid|thread_id
room|get_room|workspace_id=${ws}|get_room|workspace_id=not-a-uuid|workspace_id
schedule|list_task_schedules||create_task_schedule|title=probe|channel_id
secret|list_secrets||create_secret|value=probe|name
seed|seed_from_message|message_id=${message} title=seeded|seed_from_message|message_id=not-a-uuid title=seeded|message_id
share|list_share_tickets||create_share_ticket|expires_at=2030-01-01T00:00:00Z|channel_id
skill|list_member_skills|member_id=${member}|list_member_skills|member_id=not-a-uuid|member_id
snapshot|snapshot_thread_context|thread_id=${thread}|snapshot_thread_context|thread_id=not-a-uuid|thread_id
social|list_pins|thread_id=${thread}|list_pins|thread_id=not-a-uuid|thread_id
spawn|get_spawn_budget||set_spawn_budget|max_depth=-1|max_depth
thread|get_thread_context|thread_id=${thread}|get_thread_context|thread_id=not-a-uuid|thread_id
thread_lookup|find_thread|repo=o/r pr_number=1|find_thread|repo=o/r|pr_number
whoami|whoami||||
"

modules="$(sed -nE 's/.*"[a-z_]+" => ([a-z_]+)::.*/\1/p' crates/maidan-mcp/src/tools/mod.rs | sort -u)"
probed="$(awk -F'|' 'NF > 1 {print $1}' <<<"$probes" | sort -u)"
missing="$(comm -23 <(echo "$modules") <(echo "$probed"))"
[[ -z "$missing" ]] || fail "tool groups with no probe row: ${missing//$'\n'/ }"

ok() { jq -e '.isError != true' <<<"$1" >/dev/null 2>&1; }
while IFS='|' read -r group tool valid bad_tool bad named; do
  [[ -n "$group" ]] || continue
  # shellcheck disable=SC2086 # a row's arguments are space-separated pairs
  if ! out="$(call "$tool" $valid)" || ! ok "$out"; then
    fail "${group}: ${tool} ${valid}: ${out}"
  fi
  if [[ -n "$bad_tool" ]]; then
    # shellcheck disable=SC2086
    if out="$(call "$bad_tool" $bad)" && ok "$out"; then
      fail "${group}: ${bad_tool} ${bad} succeeded: ${out}"
    fi
    grep -q "$named" <<<"$out" || fail "${group}: ${bad_tool} ${bad} failed without naming ${named}: ${out}"
  fi
  pass "${group}: ${tool}${bad_tool:+, and ${bad_tool} refused naming ${named}}"
done <<<"$probes"

python3 scripts/mcp-tool-sweep.py "$base" "$token" ${sweep_without[@]+"${sweep_without[@]}"} ||
  fail "the tool sweep (above) failed"

if (( ${#sweep_without[@]} )); then
  echo "=== all MCP external checks passed, but S3 was NOT exercised (no Docker daemon) ==="
else
  echo "=== all MCP external checks passed ==="
fi

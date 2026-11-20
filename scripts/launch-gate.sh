#!/usr/bin/env bash
# shellcheck disable=SC2016,SC2329 # jq programs are single-quoted; check() runs functions by name
# The maidan-launch-1.0 gate (docs/Launch Gate.md) in one command, against
# maidan.world or any ORIGIN. It asks nothing. It reads the operator token,
# opens two throwaway tenants with the operator API, runs every leg a script
# can run without weakening the product (HTTP, MCP, the OAuth endpoints, and
# the console in headless Chromium), erases both tenants (on failure too),
# and ends in PASS or FAIL with an evidence directory.
#
#   EXPECT_VERSION=vN.0.0 scripts/launch-gate.sh --env-file FILE [--evidence DIR]
#
# The operator token (operator:global) is OPERATOR_TOKEN when that is set,
# else the PROD_MAIDAN_ADMIN_TOKEN line of the dotenv FILE (--env-file, or
# GATE_ENV_FILE; --token-key KEY reads another line). The file is read, never
# sourced. No token, cookie or invite link is printed or written anywhere:
# each lives in a shell variable and reaches curl on a pipe, not its argv.
#
# ORIGIN overrides https://maidan.world; a local server works (the runbook
# says how). EXPECT_VERSION, when set, must be what /health reports, or the
# run stops before it creates anything. EVIDENCE_DIR (or --evidence) defaults
# to ~/launch-gate-<UTC time> and must be outside the repository.
#
# What needs a person by design (the Access one-time code, the Turnstile
# check, an OAuth Allow on a session someone signed in to) it leaves out
# without asking, and says so at the end and in record.md.
#
# Exit status: 0 on PASS, 1 on FAIL, 2 when it could not start.
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ORIGIN="${ORIGIN:-https://maidan.world}"
ORIGIN="${ORIGIN%/}"
ENV_FILE="${GATE_ENV_FILE:-}"
TOKEN_KEY="${GATE_TOKEN_KEY:-PROD_MAIDAN_ADMIN_TOKEN}"
EV="${EVIDENCE_DIR:-$HOME/launch-gate-$(date -u +%Y%m%dT%H%MZ)}"

usage() { sed -n '3,/^set -uo/p' "$0" | sed '$d; s/^# \{0,1\}//'; }
while [ $# -gt 0 ]; do
  case "$1" in
    --env-file) ENV_FILE="${2:?--env-file needs a path}"; shift 2 ;;
    --token-key) TOKEN_KEY="${2:?--token-key needs a name}"; shift 2 ;;
    --evidence) EV="${2:?--evidence needs a directory}"; shift 2 ;;
    -h | --help) usage; exit 0 ;;
    *) echo "unknown argument: $1 (see --help)" >&2; exit 2 ;;
  esac
done

die() { echo "launch-gate: $*" >&2; exit 2; }
for tool in curl jq openssl uuidgen; do
  command -v "$tool" >/dev/null || die "needs $tool"
done

# The token: the environment first, then the named line of the dotenv file.
# Only a KEY=value line is read (an `export ` prefix and one layer of quotes
# are dropped); nothing in the file runs.
OPERATOR_TOKEN="${OPERATOR_TOKEN:-}"
if [ -z "$OPERATOR_TOKEN" ] && [ -n "$ENV_FILE" ]; then
  [ -r "$ENV_FILE" ] || die "cannot read the env file $ENV_FILE"
  OPERATOR_TOKEN="$(sed -nE "s/^[[:space:]]*(export[[:space:]]+)?${TOKEN_KEY}[[:space:]]*=[[:space:]]*//p" "$ENV_FILE" |
    tail -1 | sed -E "s/[[:space:]]+\$//; s/^\"(.*)\"\$/\\1/; s/^'(.*)'\$/\\1/")"
  [ -n "$OPERATOR_TOKEN" ] || die "no ${TOKEN_KEY}= line in $ENV_FILE"
fi
[ -n "$OPERATOR_TOKEN" ] ||
  die "no operator token: set OPERATOR_TOKEN, or pass --env-file FILE (a dotenv file with a ${TOKEN_KEY}= line)"

case "$EV" in /*) ;; *) EV="$PWD/$EV" ;; esac
case "$EV/" in "$ROOT"/*) die "keep the evidence outside the repository: $EV" ;; esac
[ ! -e "$EV" ] || [ -z "$(ls -A "$EV")" ] || die "the evidence directory is not empty: $EV"
mkdir -p "$EV" || die "cannot create $EV"
EV="$(cd "$EV" && pwd -P)"
case "$EV/" in "$(cd "$ROOT" && pwd -P)"/*) rmdir "$EV"; die "keep the evidence outside the repository: $EV" ;; esac
chmod 700 "$EV"
mkdir -p "$EV/http" "$EV/mcp" "$EV/console"
: >"$EV/checks.txt"
exec > >(tee -a "$EV/run.log") 2>&1

STAMP="$(date -u +%Y%m%d%H%M)"
TITLE="Launch gate run $STAMP"
PERSON_A_HANDLE=gate-person-a
AGENT_A_HANDLE=gate-agent-a
AGENT_B_HANDLE=gate-agent-b
WS_A="" WS_B="" ADMIN_A="" ADMIN_B="" COOKIE_A="" COOKIE_B="" AGENT_A_TOKEN="" AGENT_B_TOKEN=""
PERSON_A="" AGENT="" CH="" T="" L="" VERSION="" CLIENT_ID="" OIDC=""
MCP_A=0 MCP_B=0

# What the run leaves to a person, and why. Printed at the end and kept in
# record.md however far the run got.
HUMAN_ONLY=(
  "Waitlist join and the invited sign-up (runbook 1a to 1e). The waitlist form takes a Turnstile check, and sign-up takes the Cloudflare Access one-time code mailed to a fresh address; a script that got past either would be defeating it. The gate opens both tenants with POST /operator/workspaces instead and checks that an invite link starts the Access sign-in. launch_gate_e2e covers the sign-up callback in CI, against a mock identity provider."
  "The OAuth Allow (runbook 3a and 3c), and with it the PKCE token exchange and that token on /mcp/streamable. Consent takes a session the person signed in to through Access, and refuses one made from a token by design (the gate checks that refusal), so no script approves a grant. Discovery, registration, authorize and the refusals run here; launch_gate_e2e runs the Allow and the exchange in CI, and Next 23 P6 (2026-11-09) recorded claude.ai's connector on maidan.world."
  "The console's Add an agent and Invite a person buttons. Both take a session the person signed in to and refuse one made from a token, by design, so the gate makes the same calls on the bearer routes they mirror (POST /workspaces/{id}/agents and /workspaces/{id}/invites). ui-tests covers the buttons in CI."
)

# --- Output -------------------------------------------------------------------

bold() { printf '\n\033[1m%s\033[0m\n' "$*"; }
record() { printf '%s  %s\n' "$1" "$2" | tee -a "$EV/checks.txt"; }
pass() { record PASS "$*"; }
fail() { record FAIL "$*"; }
skip() { record SKIP "$*"; }
check() { # check WHAT COMMAND...: PASS when the command succeeds
  local what="$1"
  shift
  if "$@" >/dev/null 2>&1; then pass "$what"; else fail "$what"; fi
}
is() { [ "$1" = "$2" ]; }

# --- HTTP ---------------------------------------------------------------------

# Every credential reaches curl as a header read from a pipe, so it is never
# in a process listing, a file or this script's output.
cred_header() { # cred_header KIND VALUE
  case "$1" in
    bearer) printf 'Authorization: Bearer %s\n' "$2" ;;
    cookie) printf 'Cookie: maidan_session=%s\n' "$2" ;;
    *) printf 'X-Launch-Gate: anonymous\n' ;;
  esac
}

# http METHOD PATH [KIND VALUE [BODY [CONTENT_TYPE]]]: sets BODY, STATUS and
# LOCATION (a redirect's target). It writes nothing, so it is the only call
# made for an answer that carries a secret.
http() {
  local method="$1" path="$2" kind="${3:-none}" value="${4:-}" data="${5:-}" ctype="${6:-application/json}" out meta
  local args=(-sS -X "$method" --max-time "${HTTP_TIMEOUT:-60}" -w '\n%{http_code} %{redirect_url}'
    -H 'Accept: application/json')
  [ -n "$data" ] && args+=(-H "Content-Type: $ctype" --data-binary "$data")
  # What a browser sends from the console's own page, so the console routes'
  # origin checks see a same-origin request.
  [ "$kind" = cookie ] && args+=(-H 'Sec-Fetch-Site: same-origin' -H "Origin: $ORIGIN")
  out="$(curl "${args[@]}" -H @<(cred_header "$kind" "$value") "$ORIGIN$path" 2>&1)"
  meta="${out##*$'\n'}"
  BODY="${out%$'\n'*}"
  [ "$BODY" = "$out" ] && BODY=""
  STATUS="${meta%% *}"
  LOCATION="${meta#* }"
}
save() { # save NAME [JQ_FILTER]: BODY, filtered, as evidence (never a secret)
  if ! printf '%s' "$BODY" | jq "${2:-.}" >"$EV/http/$1.json" 2>/dev/null; then
    rm -f "$EV/http/$1.json"
    printf '%s\n' "${BODY:0:4000}" >"$EV/http/$1.txt"
  fi
}
got() { printf '%s' "$BODY" | jq -r "$1 // empty" 2>/dev/null; }
status_in() { local s; for s in "$@"; do [ "$STATUS" = "$s" ] && return 0; done; return 1; }
holds_none() { local t; for t in "$@"; do case "$BODY" in *"$t"*) return 1 ;; esac; done; return 0; }
lacks() { ! grep -qF -- "$2" "$1"; }

# The console session the server issues for a bearer, as the console's
# "paste a token" sign-in gets it. Sets SESSION_COOKIE; writes nothing.
session_for() { # session_for TOKEN
  SESSION_COOKIE="$(curl -sS -o /dev/null -D - -X POST --max-time 60 \
    -H @<(cred_header bearer "$1") "$ORIGIN/auth/session/from-token" 2>/dev/null | tr -d '\r' |
    sed -nE 's/^[Ss]et-[Cc]ookie: maidan_session=([^;]+).*/\1/p' | tail -1)"
  [ -n "$SESSION_COOKIE" ]
}

# --- MCP ----------------------------------------------------------------------

# mcp NAME TOOL ARGS_JSON WHO: one tools/call on /mcp/streamable as tenant
# WHO's agent (a or b), the way launch_gate_e2e's helper makes it. The decoded
# answer is evidence mcp/NAME.json. Every call counts toward that tenant's
# mcp_calls meter, refused ones too.
mcp() {
  local name="$1" tool="$2" arguments="$3" who="$4" token
  if [ "$who" = a ]; then
    token="$AGENT_A_TOKEN" MCP_A=$((MCP_A + 1))
  else
    token="$AGENT_B_TOKEN" MCP_B=$((MCP_B + 1))
  fi
  http POST /mcp/streamable bearer "$token" "$(jq -cn --arg name "$tool" --argjson args "$arguments" \
    '{jsonrpc: "2.0", id: 1, method: "tools/call", params: {name: $name, arguments: $args}}')"
  token=""
  printf '%s' "$BODY" | jq --arg status "$STATUS" '
    if .error then {refused: (.error.message // .error)}
    elif .result then (.result | if .isError then {refused: .content[0].text}
                                 else (.content[0].text | fromjson? // .) end)
    else {http_status: $status, body: .} end' >"$EV/mcp/$name.json" 2>/dev/null ||
    jq -n --arg s "$STATUS" --arg b "${BODY:0:2000}" '{http_status: $s, body: $b}' >"$EV/mcp/$name.json"
}
mfield() { jq -r "$2 // empty" "$EV/mcp/$1.json" 2>/dev/null; }
refused() { jq -e 'type == "object" and (has("refused") or has("http_status"))' "$EV/mcp/$1.json" >/dev/null 2>&1; }
answered() { [ -s "$EV/mcp/$1.json" ] && ! refused "$1"; }

# --- The console, in headless Chromium ------------------------------------------

CONSOLE_WHY=""
console_ready() { # sets CONSOLE_WHY when the console leg cannot run here
  if ! command -v node >/dev/null; then
    CONSOLE_WHY="node is not installed"
  elif [ ! -d "$ROOT/ui-tests/node_modules/@playwright/test" ] &&
    ! (cd "$ROOT/ui-tests" && npm ci --no-audit --no-fund --loglevel=error >/dev/null 2>&1); then
    CONSOLE_WHY="npm ci in ui-tests failed"
  elif ! (cd "$ROOT/ui-tests" && npx --no-install playwright install chromium >/dev/null 2>&1); then
    CONSOLE_WHY="Playwright could not install Chromium"
  fi
}
# console STEP PHASE WHO [KEY=VALUE...]: one phase of ui-tests/gate/launch-gate-console.mjs
# as tenant WHO's person. Its PASS and FAIL lines join the checks, and its SET
# lines set shell variables. The cookie goes in the environment, never argv.
console() {
  local step="$1" phase="$2" who="$3" line rc="" said=0 why=""
  shift 3
  while IFS= read -r line; do
    case "$line" in
      "PASS  "*) pass "${line#PASS  }"; said=1 ;;
      "FAIL  "*) fail "${line#FAIL  }"; said=1 ;;
      "SET CH="*) CH="${line#SET CH=}" ;;
      "RC="*) rc="${line#RC=}" ;;
      *) [ -z "$why" ] && why="${line:0:200}" ;;
    esac
  done < <(
    cd "$ROOT/ui-tests" || exit
    export GATE_ORIGIN="$ORIGIN" GATE_PHASE="$phase" GATE_STEP="$step" GATE_SHOTS="$EV/console"
    if [ "$who" = a ]; then export GATE_WS="$WS_A" GATE_COOKIE="$COOKIE_A"; else export GATE_WS="$WS_B" GATE_COOKIE="$COOKIE_B"; fi
    for pair in "$@"; do export "${pair?}"; done
    node gate/launch-gate-console.mjs 2>&1
    echo "RC=$?"
  )
  if [ "$rc" != 0 ] && [ "$said" = 0 ]; then
    fail "$step: the console's $phase phase stopped before any check (exit ${rc:-none}): ${why:-no output}"
  fi
}

# --- Cleanup: both tenants are erased however the run ends ---------------------

erase() { # erase LETTER WS ADMIN_TOKEN
  local who="$1" ws="$2" token="$3"
  [ -n "$ws" ] && [ -n "$token" ] || return 0
  http DELETE "/workspaces/$ws" bearer "$token" "$(jq -cn --arg w "$ws" '{confirm_workspace_id: $w}')"
  save "erase-$who"
  if is "$STATUS" 200 && is "$(got .workspace_erased)" true; then
    pass "cleanup: workspace $who ($ws) is erased"
  else
    fail "cleanup: workspace $who ($ws) was not erased (HTTP $STATUS): erase it with DELETE /operator/workspaces/$ws (docs/Operations.md, Erasing a workspace as the operator)"
  fi
  http GET "/workspaces/$ws" bearer "$token"
  check "cleanup: $who's admin token is refused once its workspace is gone (HTTP $STATUS)" is "$STATUS" 401
}
CLEANED=0
cleanup() {
  [ "$CLEANED" = 1 ] && return
  CLEANED=1
  if [ -n "$WS_A$WS_B" ]; then
    bold "Cleanup"
    erase a "$WS_A" "$ADMIN_A"
    erase b "$WS_B" "$ADMIN_B"
  fi
  OPERATOR_TOKEN="" ADMIN_A="" ADMIN_B="" COOKIE_A="" COOKIE_B="" AGENT_A_TOKEN="" AGENT_B_TOKEN=""
}

report() {
  local passed failed skipped verdict item
  passed="$(grep -c '^PASS' "$EV/checks.txt")"
  failed="$(grep -c '^FAIL' "$EV/checks.txt")"
  skipped="$(grep -c '^SKIP' "$EV/checks.txt")"
  verdict=PASS
  if [ "$failed" -gt 0 ] || [ "$passed" -eq 0 ]; then verdict=FAIL; fi
  {
    echo "# Launch gate run: $ORIGIN, ${VERSION:-unknown version}, $(date -u +%Y-%m-%dT%H:%MZ)"
    echo
    echo "**$verdict**: $passed passed, $failed failed, $skipped skipped, by \`scripts/launch-gate.sh\`."
    echo
    echo "| Id | Value |"
    echo "|---|---|"
    echo "| Workspace A | ${WS_A:-none} (\`launch-gate-a-$STAMP\`) |"
    echo "| Workspace B | ${WS_B:-none} (\`launch-gate-b-$STAMP\`) |"
    echo "| Person A | ${PERSON_A:-none} |"
    echo "| The agent | ${AGENT:-none} |"
    echo "| Channel \`launch\` | ${CH:-none} |"
    echo "| The thread | ${T:-none} |"
    echo "| The OAuth client it registered | ${CLIENT_ID:-none} |"
    echo
    echo '```'
    cat "$EV/checks.txt"
    echo '```'
    echo
    echo "## Left to a person, by design"
    echo
    for item in "${HUMAN_ONLY[@]}"; do echo "- $item"; done
    echo
    echo "## Evidence"
    echo
    echo "- \`http/\`: the API's answers, with every secret taken out."
    echo "- \`mcp/\`: the agents' tool answers."
    echo "- \`console/\`: the console's screens, from headless Chromium at 1440 x 900."
    echo "- \`run.log\`: this run's output."
  } >"$EV/record.md"
  bold "$verdict: $passed passed, $failed failed, $skipped skipped"
  echo "Left to a person, by design (record.md says why):"
  for item in "${HUMAN_ONLY[@]}"; do echo "  - ${item%%.*}."; done
  echo "Evidence: $EV"
}
COMPLETED=0
finish() {
  local rc=$?
  trap '' INT TERM
  # A run that stopped early proves only part of the path, so it is no PASS.
  if [ "$COMPLETED" != 1 ] && ! grep -q '^FAIL' "$EV/checks.txt"; then
    fail "the run stopped before its last step (exit $rc)"
  fi
  cleanup
  report
  if grep -q '^FAIL' "$EV/checks.txt" || ! grep -q '^PASS' "$EV/checks.txt"; then exit 1; fi
  exit "$rc"
}
trap 'exit 130' INT
trap 'exit 143' TERM
trap finish EXIT

# =============================================================================

bold "Launch gate against $ORIGIN, evidence in $EV"

bold "Step 0: the instance"
# A sleeping container is held at the edge for up to 90 seconds while it starts.
HTTP_TIMEOUT=150 http GET /health
save health
VERSION="$(got .version)"
if ! is "$STATUS" 200 || ! is "$(got .status)" ok; then
  fail "0: /health is ok (HTTP $STATUS); nothing was created"
  exit 1
fi
pass "0: /health is ok, on ${VERSION:-no version}"
if [ -n "${EXPECT_VERSION:-}" ]; then
  if [ "$VERSION" != "$EXPECT_VERSION" ]; then
    fail "0: /health reports EXPECT_VERSION $EXPECT_VERSION (it reports ${VERSION:-nothing}); nothing was created"
    exit 1
  fi
  pass "0: it is the release under test, $EXPECT_VERSION"
fi
http GET /status
if is "$STATUS" 200; then
  printf '%s\n' "$BODY" >"$EV/http/status.html"
  pass "0: /status answers"
elif [ "$ORIGIN" != https://maidan.world ] && is "$STATUS" 404; then
  skip "0: /status: this origin has none (on maidan.world the Worker serves it)"
else
  fail "0: /status answers (HTTP $STATUS)"
fi
http GET /operator/status bearer "$OPERATOR_TOKEN"
save operator-status
if ! is "$STATUS" 200; then
  fail "0: the operator token holds operator:global (GET /operator/status answered $STATUS); nothing was created"
  exit 1
fi
pass "0: the operator token holds operator:global"
http GET /.well-known/maidan.json
save well-known-maidan '.auth'
OIDC="$(got .auth.oidc)"

bold "Step 1: two tenants, opened by the operator"
open_tenant() { # open_tenant LETTER: sets WS_x and ADMIN_x (and PERSON_A)
  local who="$1" name="launch-gate-$1-$STAMP" handle="gate-person-$1" ws person kind caps
  http POST /operator/workspaces bearer "$OPERATOR_TOKEN" \
    "$(jq -cn --arg n "$name" --arg h "$handle" '{name: $n, admin_handle: $h}')"
  save "provision-$who" '{workspace, member, token: (.token | del(.secret))}'
  if ! is "$STATUS" 201 || [ -z "$(got .token.secret)" ]; then
    fail "1: the operator opens workspace $who (HTTP $STATUS)"
    exit 1
  fi
  ws="$(got .workspace.id)" person="$(got .member.id)" kind="$(got .member.kind)"
  caps="$(got '.token.capabilities | join(" ")')"
  if [ "$who" = a ]; then
    WS_A="$ws" ADMIN_A="$(got .token.secret)" PERSON_A="$person"
  else
    WS_B="$ws" ADMIN_B="$(got .token.secret)"
  fi
  BODY=""
  pass "1: the operator opens workspace $who, $name ($ws)"
  check "1: its first member is a person (kind $kind)" is "$kind" human
  check "1: its admin token administers it and nothing across tenants" \
    jq -en --arg c " $caps " '($c | contains(" workspace:admin ")) and ($c | contains(" token:admin "))
      and ($c | contains(" operator:global ") | not) and ($c | contains(" audit:read-global ") | not)'
}
open_tenant a
open_tenant b
echo "  WS_A=$WS_A WS_B=$WS_B PERSON_A=$PERSON_A"
check "1: the two workspaces are distinct" test "$WS_A" != "$WS_B"
http GET "/workspaces/$WS_A/members" bearer "$ADMIN_A"
save members-a-new
check "1: workspace a holds one member, its person" \
  jq -en --argjson m "${BODY:-null}" '($m | length) == 1 and $m[0].handle == "gate-person-a" and $m[0].kind == "human"'
http GET "/workspaces/$WS_B/members" bearer "$ADMIN_B"
save members-b-new
check "1: workspace b holds one member, its person" \
  jq -en --argjson m "${BODY:-null}" '($m | length) == 1 and $m[0].handle == "gate-person-b" and $m[0].kind == "human"'
http GET "/workspaces/$WS_A/channels" bearer "$ADMIN_A"
save channels-a-new
echo "  A workspace the operator opens has $(got length) channels; a sign-up opens on #general."

bold "Step 1f: A invites a person and revokes; B is kept out"
http POST "/workspaces/$WS_A/invites" bearer "$ADMIN_A" \
  "$(jq -cn --arg e "launch-gate-invitee-$STAMP@example.com" '{email: $e, role: "member"}')"
save invite-a '{invite}'
INVITE_ID="$(got .invite.id)" ACCEPT_PATH="$(got .accept_path)"
BODY=""
check "1f: A's admin invites a person as a member (HTTP $STATUS)" test "$STATUS" = 201 -a -n "$INVITE_ID"
if [ "$OIDC" = true ]; then
  http GET "$ACCEPT_PATH"
  check "1f: the live invite link starts the sign-in at the identity provider (HTTP $STATUS)" status_in 302 303 307
else
  skip "1f: the invite link's sign-in: this origin has no identity provider"
fi
http GET "/workspaces/$WS_A/invites" bearer "$ADMIN_B"
check "1f: B's admin is refused A's invites (HTTP $STATUS)" status_in 403 404
http POST "/workspaces/$WS_A/invites" bearer "$ADMIN_B" '{"email":"launch-gate-intruder@example.com","role":"member"}'
check "1f: B's admin cannot invite into A (HTTP $STATUS)" status_in 403 404
http DELETE "/workspaces/$WS_A/invites/$INVITE_ID" bearer "$ADMIN_A"
check "1f: A revokes the invite (HTTP $STATUS)" status_in 200 204
http GET "/workspaces/$WS_A/invites" bearer "$ADMIN_A"
save invites-a
check "1f: A's invite list shows it revoked" \
  jq -en --argjson l "${BODY:-null}" --arg id "$INVITE_ID" '[$l[] | select(.id == $id and .revoked_at != null)] | length == 1'
if [ "$OIDC" = true ]; then
  http GET "$ACCEPT_PATH"
  check "1f: the revoked invite link is refused (HTTP $STATUS)" status_in 400 403 404 410
fi
ACCEPT_PATH=""

bold "Each person's console session"
session_for "$ADMIN_A" && COOKIE_A="$SESSION_COOKIE"
session_for "$ADMIN_B" && COOKIE_B="$SESSION_COOKIE"
SESSION_COOKIE=""
check "1: the server issues person A a console session for the admin token" test -n "$COOKIE_A"
check "1: and person B" test -n "$COOKIE_B"
http GET /auth/session cookie "$COOKIE_A"
save session-a
check "1: A's session is person A in workspace A" \
  test "$(got .workspace_id)" = "$WS_A" -a "$(got .member_id)" = "$PERSON_A"
console_ready
[ -n "$CONSOLE_WHY" ] &&
  skip "console: no headless Chromium here ($CONSOLE_WHY): its checks run on the API the console calls, without screens"

bold "Step 2: a channel for the work"
[ -z "$CONSOLE_WHY" ] && console 2 channel a
if [ -z "$CH" ]; then
  http POST "/ui/api/workspaces/$WS_A/channels" cookie "$COOKIE_A" '{"name":"launch"}'
  CH="$(got .id)"
  check "2: A creates launch on the console's API (HTTP $STATUS)" test -n "$CH"
fi
http GET "/ui/api/workspaces/$WS_A/channels" cookie "$COOKIE_A"
save channels-a
check "2: workspace A's channels are launch only" jq -en --argjson c "${BODY:-null}" '[$c[].name] == ["launch"]'
echo "  CH=$CH"

bold "Step 3: OAuth, on $ORIGIN's own authorization server"
http GET /.well-known/oauth-protected-resource/mcp/streamable
save oauth-protected-resource
check "3: the MCP resource names this origin as its authorization server" \
  jq -en --argjson m "${BODY:-null}" --arg o "$ORIGIN" '$m.resource == ($o + "/mcp/streamable") and ($m.authorization_servers | index($o))'
http GET /.well-known/oauth-authorization-server
save oauth-authorization-server
check "3: the server's metadata: this issuer, S256 only, a registration endpoint" \
  jq -en --argjson m "${BODY:-null}" --arg o "$ORIGIN" '$m.issuer == $o and $m.code_challenge_methods_supported == ["S256"]
    and $m.registration_endpoint == ($o + "/oauth/register")'
WWW="$(curl -sS -o /dev/null -X POST --max-time 60 -H 'Content-Type: application/json' -d '{}' \
  -w '%{http_code} %header{www-authenticate}' "$ORIGIN/mcp/streamable" 2>/dev/null)"
check "3: /mcp/streamable without a token is a 401 that points at the resource metadata" \
  test "${WWW%% *}" = 401 -a "${WWW#*resource_metadata=}" != "$WWW"
REDIRECT_URI="http://127.0.0.1:53682/callback"
http POST /oauth/register none "" "$(jq -cn --arg r "$REDIRECT_URI" '{client_name: "Launch gate",
  redirect_uris: [$r], token_endpoint_auth_method: "none", grant_types: ["authorization_code"], response_types: ["code"]}')"
save oauth-register
CLIENT_ID="$(got .client_id)"
check "3: dynamic registration takes a public client with a loopback redirect (HTTP $STATUS)" \
  test "$STATUS" = 201 -a -n "$CLIENT_ID" -a "$(got .token_endpoint_auth_method)" = none
VERIFIER="$(openssl rand -base64 48 | tr '+/' '-_' | tr -d '=\n')"
CHALLENGE="$(printf '%s' "$VERIFIER" | openssl dgst -sha256 -binary | openssl base64 -A | tr '+/' '-_' | tr -d '=')"
uri() { jq -rn --arg v "$1" '$v | @uri'; }
AUTHORIZE="/oauth/authorize?response_type=code&client_id=$(uri "$CLIENT_ID")&redirect_uri=$(uri "$REDIRECT_URI")"
AUTHORIZE+="&code_challenge=$CHALLENGE&code_challenge_method=S256&scope=$(uri 'workspace:read thread:transition')"
AUTHORIZE+="&state=launch-gate&resource=$(uri "$ORIGIN/mcp/streamable")"
if [ "$OIDC" = true ]; then
  http GET "$AUTHORIZE"
  check "3: a signed-out authorize is sent to sign in (HTTP $STATUS)" \
    test "${LOCATION#"$ORIGIN"/auth/oidc/login?return_to=}" != "$LOCATION"
else
  skip "3: the signed-out authorize's sign-in: this origin has no identity provider"
fi
http GET "$AUTHORIZE" cookie "$COOKIE_A"
CONSENT="${LOCATION#"$ORIGIN"}"
REQUEST_ID="${CONSENT#/ui/oauth/consent?request=}"
check "3: authorize on A's session takes the client, its loopback redirect, S256, the scope and the resource, and sends A to consent (HTTP $STATUS)" \
  test "$STATUS" = 307 -a "$REQUEST_ID" != "$CONSENT"
http GET "$CONSENT" cookie "$COOKIE_A"
save oauth-consent-page
check "3: the consent page refuses a session made from a token (HTTP $STATUS)" \
  test "$STATUS" = 403 -a "$(got .detail)" = "consent needs a browser session the person signed in to"
http POST /ui/api/oauth/consent cookie "$COOKIE_A" "request_id=$(uri "$REQUEST_ID")&approved=true" \
  application/x-www-form-urlencoded
save oauth-consent-decision
check "3: so does the consent decision (HTTP $STATUS)" \
  test "$STATUS" = 403 -a "$(got .detail)" = "consent needs a browser session the person signed in to"
http POST /oauth/token none "" "grant_type=authorization_code&client_id=$(uri "$CLIENT_ID")&code=launch-gate-unapproved&redirect_uri=$(uri "$REDIRECT_URI")&code_verifier=$VERIFIER&resource=$(uri "$ORIGIN/mcp/streamable")" \
  application/x-www-form-urlencoded
save oauth-token-unapproved
check "3: the token endpoint refuses a code nobody approved (HTTP $STATUS, $(got .error))" \
  test "$STATUS" = 400 -a "$(got .error)" = invalid_grant
VERIFIER=""

bold "Step 3b: the agents"
add_agent() { # add_agent LETTER HANDLE: sets AGENT_x_TOKEN
  local who="$1" handle="$2" ws token
  if [ "$who" = a ]; then ws="$WS_A" token="$ADMIN_A"; else ws="$WS_B" token="$ADMIN_B"; fi
  # No display name, so the console shows each tenant's agent by its own handle.
  http POST "/workspaces/$ws/agents" bearer "$token" "$(jq -cn --arg h "$handle" '{handle: $h}')"
  token=""
  save "agent-$who" '{member, token: (.token | del(.secret))}'
  check "3b: $who's admin adds $handle, an agent with the worker set and no admin (HTTP $STATUS)" \
    jq -en --argjson r "${BODY:-null}" '$r.member.kind == "agent" and ($r.token.capabilities | index("thread:transition"))
      and ($r.token.capabilities | index("token:admin") | not)'
  if [ "$who" = a ]; then AGENT_A_TOKEN="$(got .token.secret)"; else AGENT_B_TOKEN="$(got .token.secret)"; fi
  BODY=""
}
add_agent a "$AGENT_A_HANDLE"
add_agent b "$AGENT_B_HANDLE"
mcp whoami whoami '{}' a
AGENT="$(mfield whoami .member_id)"
check "3b: the agent's token works in workspace A" test "$(mfield whoami .workspace_id)" = "$WS_A"
check "3b: the agent holds thread:transition" jq -e '.capabilities | index("thread:transition")' "$EV/mcp/whoami.json"
mcp members list_members "$(jq -cn --arg w "$WS_A" '{workspace_id: $w}')" a
check "3b: workspace A holds person A and the agent" jq -e --arg p "$PERSON_A" --arg a "$AGENT" \
  '[.. | objects | .id? // empty] | (index($p) != null and index($a) != null)' "$EV/mcp/members.json"
mcp channels list_channels "$(jq -cn --arg w "$WS_A" '{workspace_id: $w}')" a
check "3b: the agent sees launch" jq -e --arg c "$CH" '[.. | objects | .id? // empty] | index($c) != null' "$EV/mcp/channels.json"
mcp whoami-b whoami '{}' b
check "3b: B's agent token works in workspace B" test "$(mfield whoami-b .workspace_id)" = "$WS_B"
[ -z "$CONSOLE_WHY" ] && console 3b people a GATE_PERSON="$PERSON_A_HANDLE" GATE_AGENT="$AGENT_A_HANDLE"
echo "  PERSON_A=$PERSON_A AGENT=$AGENT CH=$CH"

bold "Step 4: the agent runs a change"
mcp create create_thread "$(jq -cn --arg c "$CH" --arg t "$TITLE" '{channel_id: $c, title: $t,
  attributes: {kind: "change", engine: "session", acceptance: "the launch gate passes"}}')" a
T="$(jq -r '.id // .thread.id // empty' "$EV/mcp/create.json")"
check "4: create_thread returned a thread" test -n "$T"
mcp owner set_thread_owner "$(jq -cn --arg t "$T" --arg o "$PERSON_A" '{thread_id: $t, owner_id: $o}')" a
check "4: person A owns it" answered owner
mcp claim claim_next_workspace_thread "$(jq -cn --arg w "$WS_A" '{workspace_id: $w, engine: "session"}')" a
L="$(mfield claim .claim_lease_id)"
check "4: the claim took the thread" test "$(mfield claim .id)" = "$T" -a -n "$L"
check "4: the claim's assignee is the agent" test "$(mfield claim .assignee_id)" = "$AGENT"
mcp ack acknowledge_claim "$(jq -cn --arg t "$T" --arg l "$L" '{thread_id: $t, claim_lease_id: $l}')" a
check "4: the claim is acknowledged" answered ack
mcp result set_thread_result "$(jq -cn --arg t "$T" --arg l "$L" \
  '{thread_id: $t, claim_lease_id: $l, result: {summary: "the launch path works end to end"}}')" a
check "4: the result is set" answered result
mcp usage report_usage "$(jq -cn --arg t "$T" --arg l "$L" --arg u "$(uuidgen | tr '[:upper:]' '[:lower:]')" '{
  thread_id: $t, usage_report_id: $u, claim_lease_id: $l, model: "claude-opus-4-5",
  tokens: {input: 1200, output: 340, cache_read: 0, cache_write_5m: 0, cache_write_1h: 0},
  usd_micros: 8700,
  price_snapshot: {input_usd_micros_per_million: 3000000, output_usd_micros_per_million: 15000000,
    cache_read_usd_micros_per_million: 300000, cache_write_5m_usd_micros_per_million: 3750000,
    cache_write_1h_usd_micros_per_million: 6000000},
  turns: 1}')" a
check "4: report_usage answered stopped: false" test "$(jq -r '.stopped' "$EV/mcp/usage.json")" = false
mcp review transition_thread "$(jq -cn --arg t "$T" '{thread_id: $t, action: "start_review"}')" a
check "4: start_review left the thread in_review" \
  test "$(jq -r '[.. | objects | .state? // empty] | first // empty' "$EV/mcp/review.json")" = in_review
http GET "/threads/$T/usage-rollup" bearer "$AGENT_A_TOKEN"
save usage-rollup
check "4: the thread's usage rollup answers (HTTP $STATUS)" is "$STATUS" 200

bold "Step 5: the agent cannot close its own work"
mcp close transition_thread "$(jq -cn --arg t "$T" '{thread_id: $t, action: "close"}')" a
check "5: the agent's close is refused for separation of duties" grep -q 'separation of duties' "$EV/mcp/close.json"
mcp release release_claim "$(jq -cn --arg t "$T" --arg l "$L" '{thread_id: $t, claim_lease_id: $l}')" a
check "5: the release succeeded" answered release

bold "Step 8, during: B's agent is refused A's thread and queue"
mcp cross-read get_thread_context "$(jq -cn --arg t "$T" '{thread_id: $t}')" b
check "8: B's agent is refused A's thread: token is not valid for this workspace" \
  grep -q 'token is not valid for this workspace' "$EV/mcp/cross-read.json"
check "8: the refusal holds nothing of A's thread" lacks "$EV/mcp/cross-read.json" "$TITLE"
mcp own-read get_thread_context "$(jq -cn --arg t "$T" '{thread_id: $t}')" a
check "8: A's agent reads the same thread, so the refusal is the tenant boundary" answered own-read
mcp cross-claim claim_next_workspace_thread "$(jq -cn --arg w "$WS_A" '{workspace_id: $w}')" b
check "8: B's agent is refused A's queue" refused cross-claim

bold "Step 6: person A reviews and closes, in the console"
[ -z "$CONSOLE_WHY" ] && console 6 review a GATE_CH="$CH" GATE_T="$T" GATE_TITLE="$TITLE"
http GET "/ui/api/threads/$T" cookie "$COOKIE_A"
if ! is "$(got .state)" closed; then
  # Without the console's close, its own API closes the task, so the steps
  # after this one still test a closed thread.
  http GET "/ui/api/threads/$T/reviews" cookie "$COOKIE_A"
  if holds_none '"approve"'; then
    http GET "/ui/api/threads/$T/review-packet" cookie "$COOKIE_A"
    http POST "/ui/api/threads/$T/reviews" cookie "$COOKIE_A" \
      "$(jq -cn --arg r "$(got .evidence_root)" '{decision: "approve", evidence_root: $r}')"
    check "6: A approves the review packet on the console's API (HTTP $STATUS)" status_in 200 201
  fi
  http POST "/ui/api/threads/$T" cookie "$COOKIE_A" '{"action":"close"}'
  check "6: A closes the task on the console's API (HTTP $STATUS)" status_in 200 201
fi
http GET "/ui/api/threads/$T/review-packet" cookie "$COOKIE_A"
save review-packet
http GET "/threads/$T" bearer "$ADMIN_A"
save closed
check "6: the thread is closed" is "$(got .state)" closed

bold "Step 8, after: person B sees nothing of A"
for path in "/ui/api/threads/$T" "/ui/api/threads/$T/review-packet" "/ui/api/channels/$CH/threads"; do
  http GET "$path" cookie "$COOKIE_B"
  check "8: B's session is refused ${path/$T/T} (HTTP $STATUS), with nothing of A in the answer" \
    eval 'status_in 403 404 && holds_none "$TITLE" "$PERSON_A" "$AGENT"'
done
http GET "/threads/$T" bearer "$ADMIN_B"
check "8: B's admin token is refused A's thread (HTTP $STATUS)" status_in 403 404
http GET "/workspaces/$WS_A/members" bearer "$ADMIN_B"
check "8: B's admin token is refused A's members (HTTP $STATUS)" status_in 403 404
[ -z "$CONSOLE_WHY" ] && console 8 tenant b GATE_FORBIDDEN="$TITLE,$PERSON_A_HANDLE,$AGENT_A_HANDLE" \
  GATE_OWN="gate-person-b,$AGENT_B_HANDLE"

bold "Step 7: metering"
echo "  The agents made $MCP_A MCP calls in A and $MCP_B in B. The meters flush every 60 seconds..."
meter() { # meter WS METER: this month's quantity, from the last read
  jq -rn --argjson m "${METERS:-[]}" --arg w "$1" --arg k "$2" \
    '[$m[] | select(.workspace_id == $w) | .meters[]? | select(.meter == $k) | .quantity] | add // 0' 2>/dev/null || echo 0
}
METERS="[]" GOT_A=0 GOT_B=0
for _ in $(seq 1 18); do
  http GET /operator/usage-meters bearer "$OPERATOR_TOKEN"
  METERS="$(got "[.[] | select(.workspace_id == \"$WS_A\" or .workspace_id == \"$WS_B\")]")"
  GOT_A="$(meter "$WS_A" mcp_calls)" GOT_B="$(meter "$WS_B" mcp_calls)"
  [ "$GOT_A" -ge "$MCP_A" ] && [ "$GOT_B" -ge "$MCP_B" ] && break
  sleep 10
done
printf '%s\n' "${METERS:-[]}" | jq . >"$EV/http/usage-meters.json"
check "7: A's mcp_calls is the $MCP_A calls its agent made (it reads $GOT_A)" test "$GOT_A" -eq "$MCP_A"
check "7: A's requests is at least its mcp_calls (it reads $(meter "$WS_A" requests))" \
  test "$(meter "$WS_A" requests)" -ge "$GOT_A"
check "7: B's mcp_calls is the $MCP_B calls its own agent made, none of A's (it reads $GOT_B)" test "$GOT_B" -eq "$MCP_B"

COMPLETED=1
exit 0

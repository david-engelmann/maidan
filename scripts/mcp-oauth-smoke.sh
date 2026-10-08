#!/usr/bin/env bash
# Walks the MCP authorization flow (spec revision 2026-07-28) with curl, the way
# an MCP client does, and says at each step whether Maidan answers yet. See
# docs/OAuth-Authorization-Server.md, "Local setup".
#
#   PASS  the step works
#   GAP   Maidan does not answer this step yet; the label names the build PR
#   NOTE  a fact about the authorization server in use, not a failure
#   FAIL  the step is broken
#
# Until Maidan publishes protected resource metadata, the authorization-server
# steps run against the reference server from scripts/oauth-dev-provider.sh.
# Once Maidan publishes it, the script follows Maidan's own metadata instead.
#
# Environment:
#   MAIDAN_URL           the local Maidan (http://127.0.0.1:8080)
#   MCP_PATH             the MCP endpoint whose resource is tested (/mcp)
#   OAUTH_ISSUER         the reference issuer, used while Maidan publishes no
#                        metadata (http://127.0.0.1:8081/realms/maidan-mcp-reference)
#   OAUTH_USER           who signs in at the authorization server (ada)
#   OAUTH_PASSWORD_FILE  a file holding that person's password
#   CIMD_PORT            loopback port that serves the test client metadata (33419)
#   STRICT=1             count GAP lines as failures (for when the build lands)
#
# Exit status: 0 when nothing FAILed (and, with STRICT=1, no GAP), 1 otherwise.
set -euo pipefail

maidan_url="${MAIDAN_URL:-http://127.0.0.1:8080}"
maidan_url="${maidan_url%/}"
mcp_path="${MCP_PATH:-/mcp}"
resource="${maidan_url}${mcp_path}"
reference_issuer="${OAUTH_ISSUER:-http://127.0.0.1:8081/realms/maidan-mcp-reference}"
user="${OAUTH_USER:-ada}"
password_file="${OAUTH_PASSWORD_FILE:?set OAUTH_PASSWORD_FILE to a file holding the password of ${OAUTH_USER:-ada}}"
cimd_port="${CIMD_PORT:-33419}"
redirect_uri="http://127.0.0.1:33418/callback"

work="$(mktemp -d)"
cimd_pid=""
cleanup() {
  [ -n "$cimd_pid" ] && kill "$cimd_pid" 2>/dev/null
  rm -rf "$work"
}
trap cleanup EXIT

fails=0
gaps=0
pass() { echo "PASS  $*"; }
gap() { echo "GAP   $*"; gaps=$((gaps + 1)); }
note() { echo "NOTE  $*"; }
fail() { echo "FAIL  $*"; fails=$((fails + 1)); }

# json FILE EXPR: evaluate a Python expression over the parsed document `d`.
json() {
  python3 -c 'import json, sys
try:
    d = json.load(open(sys.argv[1]))
except Exception:
    d = None
try:
    v = eval(sys.argv[2])
except Exception:
    v = None
print("" if v is None else (json.dumps(v) if isinstance(v, (list, dict)) else v))' "$1" "$2"
}

# The value of one query parameter in a URL.
query_param() {
  python3 -c 'import sys, urllib.parse as u
q = u.parse_qs(u.urlsplit(sys.argv[1]).query)
print(q.get(sys.argv[2], [""])[0])' "$1" "$2"
}

# The first form in an HTML page: its action resolved against BASE_URL on the
# first line, then its hidden inputs, url-encoded, on the second.
form_fields() {
  python3 -c 'import html, re, sys, urllib.parse as u
page = open(sys.argv[1]).read()
m = re.search(r"<form[^>]*action=\"([^\"]+)\"", page)
print(u.urljoin(sys.argv[2], html.unescape(m.group(1))) if m else "")
hidden = re.findall(r"<input[^>]*type=\"hidden\"[^>]*name=\"([^\"]+)\"[^>]*value=\"([^\"]*)\"", page)
print(u.urlencode([(k, html.unescape(v)) for k, v in hidden]))' "$1" "$2"
}

# The audience of a JWT access token, or nothing for an opaque token.
jwt_aud() {
  python3 -c 'import base64, json, sys
parts = sys.argv[1].split(".")
if len(parts) != 3:
    sys.exit(0)
body = json.loads(base64.urlsafe_b64decode(parts[1] + "=" * (-len(parts[1]) % 4)))
aud = body.get("aud")
print(" ".join(aud) if isinstance(aud, list) else (aud or ""))' "$1"
}

pkce_pair() {
  python3 -c 'import base64, hashlib, secrets
v = secrets.token_urlsafe(48)
c = base64.urlsafe_b64encode(hashlib.sha256(v.encode()).digest()).rstrip(b"=").decode()
print(v, c)'
}

initialize='{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2026-07-28","capabilities":{},"clientInfo":{"name":"mcp-oauth-smoke","version":"0"}}}'

echo "== Maidan as the protected resource: $resource"

# 1. An unauthenticated MCP request gets 401 with a resource_metadata pointer.
code="$(curl -sS -o "$work/m1.body" -D "$work/m1.head" -w '%{http_code}' -X POST "$resource" \
  -H 'content-type: application/json' -H 'accept: application/json, text/event-stream' \
  --data "$initialize")"
challenge="$(tr -d '\r' <"$work/m1.head" | sed -n 's/^[Ww][Ww][Ww]-[Aa]uthenticate: //p' | head -n 1)"
if [ "$code" != "401" ]; then
  fail "an unauthenticated MCP request answered $code, not 401"
elif [[ "$challenge" == *resource_metadata=* ]]; then
  pass "401 with WWW-Authenticate: $challenge"
else
  gap "401 carries no WWW-Authenticate resource_metadata pointer (build PR 1)"
fi

# 2. Protected resource metadata (RFC 9728), path-inserted first, then the root.
prm_url=""
if [[ "$challenge" =~ resource_metadata=\"([^\"]+)\" ]]; then
  prm_url="${BASH_REMATCH[1]}"
fi
issuer=""
for candidate in "$prm_url" "$maidan_url/.well-known/oauth-protected-resource$mcp_path" \
  "$maidan_url/.well-known/oauth-protected-resource"; do
  [ -n "$candidate" ] || continue
  code="$(curl -sS -o "$work/prm.json" -w '%{http_code}' "$candidate")"
  if [ "$code" = "200" ]; then
    if [ "$(json "$work/prm.json" 'd["resource"]')" = "$resource" ]; then
      issuer="$(json "$work/prm.json" 'd["authorization_servers"][0]')"
      pass "protected resource metadata at $candidate names $issuer"
    else
      fail "metadata at $candidate names resource $(json "$work/prm.json" 'd.get("resource")'), not $resource"
    fi
    break
  fi
done
if [ -z "$issuer" ]; then
  gap "no protected resource metadata for $resource (build PR 1)"
  issuer="$reference_issuer"
  note "the authorization-server steps below run against the reference server $issuer"
fi
case "$issuer" in
  "$maidan_url"*) as_is_maidan=1 ;;
  *) as_is_maidan=0 ;;
esac

echo "== Authorization server: $issuer"

# 3. Metadata discovery in the spec's order, and the issuer check.
split="$(python3 -c 'import sys, urllib.parse as u
p = u.urlsplit(sys.argv[1]); print(p.scheme + "://" + p.netloc, p.path.rstrip("/"))' "$issuer")"
origin="${split%% *}"
issuer_path="${split#* }"
[ "$issuer_path" = "$origin" ] && issuer_path=""
metadata_url=""
for candidate in "$origin/.well-known/oauth-authorization-server$issuer_path" \
  "$origin/.well-known/openid-configuration$issuer_path" "$issuer/.well-known/openid-configuration"; do
  if curl -sSf -o "$work/as.json" "$candidate" 2>/dev/null; then
    metadata_url="$candidate"
    break
  fi
done
if [ -z "$metadata_url" ]; then
  fail "no authorization server metadata for $issuer"
  echo "== $fails failed, $gaps gaps"
  exit 1
fi
if [ "$(json "$work/as.json" 'd["issuer"]')" = "$issuer" ]; then
  pass "metadata at $metadata_url, issuer matches"
else
  fail "metadata at $metadata_url names issuer $(json "$work/as.json" 'd.get("issuer")')"
fi
if [ "$(json "$work/as.json" '"S256" in d["code_challenge_methods_supported"]')" = "True" ]; then
  pass "S256 PKCE advertised"
else
  fail "code_challenge_methods_supported lacks S256; MCP clients must refuse to proceed"
fi
if [ "$(json "$work/as.json" '"plain" in d["code_challenge_methods_supported"]')" = "True" ]; then
  note "the server also advertises plain PKCE; Maidan's server will not"
fi
iss_supported="$(json "$work/as.json" 'd.get("authorization_response_iss_parameter_supported")')"
if [ "$iss_supported" = "True" ]; then
  pass "RFC 9207 iss in authorization responses advertised"
else
  note "authorization_response_iss_parameter_supported is not true"
fi
cimd="$(json "$work/as.json" 'd.get("client_id_metadata_document_supported") is True and "none" in d.get("token_endpoint_auth_methods_supported", [])')"
if [ "$cimd" = "True" ]; then
  pass "CIMD advertised, with the public-client method none"
else
  note "CIMD is not advertised with none (Claude then falls back to DCR)"
fi
authorize="$(json "$work/as.json" 'd["authorization_endpoint"]')"
token="$(json "$work/as.json" 'd["token_endpoint"]')"
register="$(json "$work/as.json" 'd.get("registration_endpoint")')"
revoke="$(json "$work/as.json" 'd.get("revocation_endpoint")')"
scope="$(json "$work/prm.json" '" ".join(d.get("scopes_supported", []))' 2>/dev/null || true)"

# Sign in through the authorization server's own pages, the way a browser does:
# the login form, then the consent form. Prints the redirect back to the client.
sign_in() {
  local url="$1" jar="$2" page="$work/page.html" location fields next hidden
  location="$(curl -sS -c "$jar" -b "$jar" -o "$page" -w '%{redirect_url}' "$url")"
  for _ in 1 2 3 4 5 6; do
    if [[ "$location" == "$redirect_uri"* ]]; then
      echo "$location"
      return 0
    fi
    if [ -n "$location" ]; then
      # A redirect within the server, such as from the login form to consent.
      location="$(curl -sS -c "$jar" -b "$jar" -o "$page" -w '%{redirect_url}' "$location")"
      continue
    fi
    fields="$(form_fields "$page" "$url")"
    next="${fields%%$'\n'*}"
    hidden="${fields#*$'\n'}"
    [ -n "$next" ] || return 1
    if grep -q 'name="password"' "$page"; then
      location="$(curl -sS -c "$jar" -b "$jar" -o "$page" -w '%{redirect_url}' "$next" \
        --data-urlencode "username=$user" --data-urlencode "password@$password_file")"
    elif grep -q 'name="accept"' "$page"; then
      location="$(curl -sS -c "$jar" -b "$jar" -o "$page" -w '%{redirect_url}' "$next" \
        --data "${hidden:+$hidden&}accept=Yes")"
    else
      return 1
    fi
  done
  return 1
}

# authorize_url CLIENT_ID CHALLENGE STATE RESOURCE
authorize_url() {
  python3 -c 'import sys, urllib.parse as u
base, client, challenge, state, resource, scope, redirect = sys.argv[1:8]
q = {"response_type": "code", "client_id": client, "redirect_uri": redirect,
     "code_challenge": challenge, "code_challenge_method": "S256", "state": state,
     "resource": resource}
if scope:
    q["scope"] = scope
print(base + ("&" if "?" in base else "?") + u.urlencode(q))' "$authorize" "$1" "$2" "$3" "$4" "$scope" "$redirect_uri"
}

# full_flow LABEL CLIENT_ID: authorize, check state and iss, exchange the code.
full_flow() {
  local label="$1" client="$2" pair verifier challenge state url back got_iss
  pair="$(pkce_pair)"
  verifier="${pair% *}"
  challenge="${pair#* }"
  state="$(python3 -c 'import secrets; print(secrets.token_urlsafe(16))')"
  url="$(authorize_url "$client" "$challenge" "$state" "$resource")"
  if ! back="$(sign_in "$url" "$work/jar")"; then
    fail "$label: sign-in did not return to the client"
    return 1
  fi
  [ "$(query_param "$back" state)" = "$state" ] || { fail "$label: state not echoed"; return 1; }
  got_iss="$(query_param "$back" iss)"
  if [ "$got_iss" = "$issuer" ]; then
    pass "$label: code returned with state and iss=$got_iss"
  elif [ -z "$got_iss" ] && [ "$iss_supported" != "True" ]; then
    note "$label: no iss in the response, allowed since the server does not advertise it"
  else
    fail "$label: iss '$got_iss' does not match $issuer"
    return 1
  fi
  curl -sS -o "$work/token.json" "$token" --data-urlencode grant_type=authorization_code \
    --data-urlencode "code=$(query_param "$back" code)" --data-urlencode "code_verifier=$verifier" \
    --data-urlencode "redirect_uri=$redirect_uri" --data-urlencode "client_id=$client" \
    --data-urlencode "resource=$resource"
  access="$(json "$work/token.json" 'd["access_token"]')"
  refresh="$(json "$work/token.json" 'd.get("refresh_token")')"
  if [ -z "$access" ]; then
    fail "$label: token exchange answered $(cat "$work/token.json")"
    return 1
  fi
  local aud
  aud="$(jwt_aud "$access")"
  if [ -z "$aud" ]; then
    pass "$label: opaque access token issued; Maidan checks its audience on use"
  elif [[ " $aud " == *" $resource "* ]]; then
    pass "$label: access token audience is $aud"
  else
    fail "$label: access token audience '$aud' does not name $resource"
  fi
}

# 4. Dynamic client registration (RFC 7591), the fallback path.
client_id=""
if [ -n "$register" ]; then
  curl -sS -o "$work/reg.json" -H 'content-type: application/json' "$register" --data \
    '{"client_name":"mcp-oauth-smoke","redirect_uris":["'"$redirect_uri"'"],"grant_types":["authorization_code","refresh_token"],"response_types":["code"],"token_endpoint_auth_method":"none","application_type":"native"}'
  client_id="$(json "$work/reg.json" 'd["client_id"]')"
  if [ -n "$client_id" ]; then
    pass "DCR registered public client $client_id"
  else
    fail "DCR refused: $(cat "$work/reg.json")"
  fi
else
  note "no registration_endpoint, so Cursor, VS Code and Gemini CLI need a pre-registered client"
fi

# 5. The code flow with PKCE and the resource indicator, for the DCR client.
if [ -n "$client_id" ] && full_flow "DCR client" "$client_id"; then
  dcr_access="$access"
  # 6. A wrong PKCE verifier is refused.
  pair="$(pkce_pair)"
  url="$(authorize_url "$client_id" "${pair#* }" s2 "$resource")"
  if back="$(sign_in "$url" "$work/jar")"; then
    curl -sS -o "$work/bad.json" "$token" --data-urlencode grant_type=authorization_code \
      --data-urlencode "code=$(query_param "$back" code)" --data-urlencode code_verifier=not-the-verifier-not-the-verifier-not-the-verifier \
      --data-urlencode "redirect_uri=$redirect_uri" --data-urlencode "client_id=$client_id" \
      --data-urlencode "resource=$resource"
    if [ "$(json "$work/bad.json" 'd.get("error")')" = "invalid_grant" ]; then
      pass "a wrong PKCE verifier is refused (invalid_grant)"
    else
      fail "a wrong PKCE verifier answered $(cat "$work/bad.json")"
    fi
  fi
  # 7. A resource the server does not serve is refused (RFC 8707 invalid_target),
  #    at the authorization request or, at the latest, at the token request.
  foreign="http://127.0.0.1:9/not-this-server"
  pair="$(pkce_pair)"
  url="$(authorize_url "$client_id" "${pair#* }" s3 "$foreign")"
  location="$(curl -sS -c "$work/jar" -b "$work/jar" -o "$work/page.html" -w '%{redirect_url}' "$url")"
  if [ "$(query_param "$location" error)" = "invalid_target" ] || grep -q invalid_target "$work/page.html"; then
    pass "a foreign resource indicator is refused at authorization (invalid_target)"
  elif [ -n "$(query_param "$location" code)" ]; then
    curl -sS -o "$work/foreign.json" "$token" --data-urlencode grant_type=authorization_code \
      --data-urlencode "code=$(query_param "$location" code)" --data-urlencode "code_verifier=${pair% *}" \
      --data-urlencode "redirect_uri=$redirect_uri" --data-urlencode "client_id=$client_id" \
      --data-urlencode "resource=$foreign"
    if [ "$(json "$work/foreign.json" 'd.get("error")')" = "invalid_target" ]; then
      pass "a foreign resource indicator is refused at the token request (invalid_target)"
    else
      fail "a foreign resource indicator got $(json "$work/foreign.json" 'd.get("error") or "a token"')"
    fi
  else
    fail "a foreign resource indicator was not refused: ${location:-see the returned page}"
  fi
  # 8. Refresh tokens rotate, and a used one is refused.
  if [ -n "$refresh" ]; then
    curl -sS -o "$work/r1.json" "$token" --data-urlencode grant_type=refresh_token \
      --data-urlencode "refresh_token=$refresh" --data-urlencode "client_id=$client_id" \
      --data-urlencode "resource=$resource"
    rotated="$(json "$work/r1.json" 'd.get("refresh_token")')"
    curl -sS -o "$work/r2.json" "$token" --data-urlencode grant_type=refresh_token \
      --data-urlencode "refresh_token=$refresh" --data-urlencode "client_id=$client_id"
    if [ -n "$rotated" ] && [ "$rotated" != "$refresh" ] && [ "$(json "$work/r2.json" 'd.get("error")')" = "invalid_grant" ]; then
      pass "the refresh token rotated and the used one is refused (invalid_grant)"
    else
      fail "refresh rotation: new=$(json "$work/r1.json" 'd.get("error") or "issued"'), reuse=$(cat "$work/r2.json")"
    fi
    # 9. Revocation (RFC 7009) ends the grant.
    if [ -n "$revoke" ] && [ -n "$rotated" ]; then
      curl -sS -o /dev/null "$revoke" --data-urlencode "token=$rotated" \
        --data-urlencode token_type_hint=refresh_token --data-urlencode "client_id=$client_id"
      curl -sS -o "$work/r3.json" "$token" --data-urlencode grant_type=refresh_token \
        --data-urlencode "refresh_token=$rotated" --data-urlencode "client_id=$client_id"
      if [ "$(json "$work/r3.json" 'd.get("error")')" = "invalid_grant" ]; then
        pass "a revoked refresh token is refused (invalid_grant)"
      else
        fail "a revoked refresh token answered $(cat "$work/r3.json")"
      fi
    fi
  else
    note "no refresh token issued"
  fi
fi

# 10. Client ID Metadata Documents, served here on loopback the way a client hosts one.
mkdir -p "$work/cimd"
cimd_base="http://127.0.0.1:$cimd_port"
cat >"$work/cimd/client.json" <<JSON
{"client_id":"$cimd_base/client.json","client_name":"mcp-oauth-smoke (CIMD)",
 "redirect_uris":["$redirect_uri"],"grant_types":["authorization_code","refresh_token"],
 "response_types":["code"],"token_endpoint_auth_method":"none"}
JSON
# The shape a client that follows SEP-3149 publishes: methods as a list, no singular field.
cat >"$work/cimd/plural.json" <<JSON
{"client_id":"$cimd_base/plural.json","client_name":"mcp-oauth-smoke (CIMD, plural methods)",
 "redirect_uris":["$redirect_uri"],"grant_types":["authorization_code","refresh_token"],
 "response_types":["code"],"token_endpoint_auth_methods_supported":["none","private_key_jwt"]}
JSON
(cd "$work/cimd" && exec python3 -m http.server "$cimd_port" --bind 127.0.0.1 >/dev/null 2>&1) &
cimd_pid=$!
for _ in $(seq 1 20); do curl -sf -o /dev/null "$cimd_base/client.json" && break; sleep 0.2; done
if [ "$cimd" = "True" ]; then
  full_flow "CIMD client" "$cimd_base/client.json" || true
  pair="$(pkce_pair)"
  url="$(authorize_url "$cimd_base/plural.json" "${pair#* }" s4 "$resource")"
  if sign_in "$url" "$work/jar" >/dev/null; then
    pass "a CIMD document that lists token_endpoint_auth_methods_supported is accepted"
  elif [ "$as_is_maidan" = 1 ]; then
    fail "Maidan refused a CIMD document that lists token_endpoint_auth_methods_supported"
  else
    note "the reference server refuses a CIMD document that lists token_endpoint_auth_methods_supported (ChatGPT's shape); Maidan's must accept it"
  fi
fi

echo "== Maidan with a token in hand"

# 11. Maidan accepts only tokens its own authorization server issued for it.
if [ -n "${dcr_access:-}" ]; then
  code="$(curl -sS -o "$work/m3.body" -w '%{http_code}' -X POST "$resource" \
    -H "authorization: Bearer $dcr_access" -H 'content-type: application/json' \
    -H 'accept: application/json, text/event-stream' --data "$initialize")"
  if [ "$as_is_maidan" = 1 ]; then
    if [ "$code" = "200" ]; then
      pass "Maidan accepts the token its own server issued"
    else
      fail "Maidan answered $code to a token its own server issued"
    fi
  else
    if [ "$code" = "401" ]; then
      pass "Maidan refuses a token another server issued (401, no passthrough)"
    else
      fail "Maidan answered $code to a token another server issued"
    fi
    gap "Maidan issues no tokens of its own (build PRs 2 to 5)"
  fi
fi

# 12. Too little scope is a 403 naming the scopes the call needs.
if [ "$as_is_maidan" = 1 ]; then
  note "check insufficient_scope with a read-only token on a write tool (build PR 4 adds the challenge)"
else
  gap "no insufficient_scope challenge to test without Maidan's tokens (build PR 4)"
fi

echo "== $fails failed, $gaps gaps"
if [ "$fails" -gt 0 ] || { [ "${STRICT:-0}" = 1 ] && [ "$gaps" -gt 0 ]; }; then
  exit 1
fi

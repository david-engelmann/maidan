#!/usr/bin/env bash
# A development OAuth 2.1 authorization server for building Maidan's MCP
# authorization: Keycloak 26.8 with its experimental Client ID Metadata Document
# (cimd) and Resource Indicators (resource-indicators) features, importing the
# realms in examples/keycloak through its admin API. See docs/OAuth-Authorization-Server.md, "Local
# setup".
#
#   scripts/oauth-dev-provider.sh up       start it, import the realms, add a user
#   scripts/oauth-dev-provider.sh status   is it answering?
#   scripts/oauth-dev-provider.sh down     stop it
#
# It runs the container in examples/oauth-dev/compose.yaml when Docker Compose
# answers (DOCKER_HOST is honored), and otherwise the Keycloak distribution in
# KC_HOME (Java 21). Development only: plain HTTP and an embedded database.
#
# Environment:
#   MAIDAN_URL      the local Maidan the realms point at (http://127.0.0.1:8080)
#   KC_PORT         Keycloak's port on 127.0.0.1 (8081)
#   KC_HOME         a Keycloak 26.8 distribution, used when there is no Docker
#   OAUTH_DEV_DIR   state: generated passwords, the substituted realms, logs
#                   (${TMPDIR:-/tmp}/maidan-oauth-dev)
#   OAUTH_DEV_MODE  compose or dist, to choose instead of detecting
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
maidan_url="${MAIDAN_URL:-http://127.0.0.1:8080}"
maidan_url="${maidan_url%/}"
kc_port="${KC_PORT:-8081}"
kc_url="http://127.0.0.1:${kc_port}"
state="${OAUTH_DEV_DIR:-${TMPDIR:-/tmp}/maidan-oauth-dev}"
compose_file="$repo_root/examples/oauth-dev/compose.yaml"
realm="maidan-mcp-reference"
dev_user="ada"

compose() {
  if docker compose version >/dev/null 2>&1; then
    docker compose -f "$compose_file" "$@"
  else
    docker-compose -f "$compose_file" "$@"
  fi
}

mode() {
  if [ -n "${OAUTH_DEV_MODE:-}" ]; then
    echo "$OAUTH_DEV_MODE"
  elif { docker compose version || docker-compose version; } >/dev/null 2>&1; then
    echo compose
  else
    echo dist
  fi
}

# A random secret in a file only this user can read, made once and reused.
secret_file() {
  local file="$state/$1"
  if [ ! -s "$file" ]; then
    (umask 077 && head -c 24 /dev/urandom | od -An -tx1 | tr -d ' \n' >"$file")
  fi
  echo "$file"
}

answering() {
  curl -sf -m 2 "$kc_url/realms/master/.well-known/openid-configuration" >/dev/null
}

wait_until_answering() {
  for _ in $(seq 1 90); do
    answering && return 0
    sleep 2
  done
  echo "FAIL: Keycloak did not answer on $kc_url within 180 s" >&2
  return 1
}

# The realm files name the default Maidan address; point them at this one.
prepare_import() {
  mkdir -p "$state/import"
  rm -f "$state/import"/*.json
  for file in "$repo_root"/examples/keycloak/*.json; do
    sed "s#http://127.0.0.1:8080#${maidan_url}#g" "$file" >"$state/import/${file##*/}"
  done
}

admin_token() {
  curl -sS -m 10 "$kc_url/realms/master/protocol/openid-connect/token" \
    --data-urlencode grant_type=password --data-urlencode client_id=admin-cli \
    --data-urlencode username=admin --data-urlencode "password@$(secret_file admin.pw)" |
    python3 -c 'import json, sys; print(json.load(sys.stdin)["access_token"])'
}

# Keycloak lets a token name the MCP resource (RFC 8707) only when the client's
# tokens already carry the resource server's audience. A realm file that lists
# client scopes replaces Keycloak's built-in ones, so the audience scope that
# dynamically registered clients inherit is added here, after the import, on
# every up, and left alone when it is already there.
# (Client ID Metadata Document clients get it from their executor instead.)
audience_scope_id() {
  curl -sS -f -H @"$1" "$kc_url/admin/realms/$realm/client-scopes" |
    python3 -c 'import json, sys; print(next((s["id"] for s in json.load(sys.stdin) if s["name"] == "maidan-mcp-audience"), ""))'
}

ensure_audience_scope() {
  local auth="$1" base="$kc_url/admin/realms/$realm" id
  id="$(audience_scope_id "$auth")"
  if [ -z "$id" ]; then
    curl -sS -f -o /dev/null -H @"$auth" -H 'content-type: application/json' \
      -X POST "$base/client-scopes" --data '{"name":"maidan-mcp-audience","protocol":"openid-connect",
        "description":"Puts the Maidan MCP resource server in the audience of every new client",
        "attributes":{"include.in.token.scope":"false","display.on.consent.screen":"false"},
        "protocolMappers":[{"name":"maidan-mcp audience","protocol":"openid-connect",
          "protocolMapper":"oidc-audience-mapper","config":{"included.client.audience":"maidan-mcp",
          "access.token.claim":"true","id.token.claim":"false","introspection.token.claim":"true"}}]}'
    id="$(audience_scope_id "$auth")"
  fi
  # Keycloak answers 409 when the scope is already a realm default.
  if ! curl -sS -f -H @"$auth" "$base/default-default-client-scopes" | grep -q "\"$id\""; then
    curl -sS -f -o /dev/null -H @"$auth" -X PUT "$base/default-default-client-scopes/$id"
  fi
}

# Import each realm that is not there yet, then one person in the reference
# realm, with names and a verified email so that Keycloak 26 asks for no
# profile update on first sign-in. The bearer goes in a header file, not argv.
import_realms_and_user() {
  local token auth name exists body
  token="$(admin_token)"
  auth="$(mktemp)"
  printf 'authorization: Bearer %s\n' "$token" >"$auth"
  for file in "$state"/import/*.json; do
    name="$(python3 -c 'import json, sys; print(json.load(open(sys.argv[1]))["realm"])' "$file")"
    if [ "$(curl -sS -o /dev/null -w '%{http_code}' -H @"$auth" "$kc_url/admin/realms/$name")" = "404" ]; then
      curl -sS -f -o /dev/null -H @"$auth" -H 'content-type: application/json' \
        -X POST "$kc_url/admin/realms" --data-binary @"$file"
      echo "Imported realm $name"
    fi
  done
  ensure_audience_scope "$auth"
  exists="$(curl -sS -f -H @"$auth" "$kc_url/admin/realms/$realm/users?username=$dev_user&exact=true")"
  if [ "$exists" = "[]" ]; then
    body="$(python3 -c 'import json, sys; print(json.dumps({
      "username": sys.argv[1], "email": sys.argv[1] + "@example.com", "emailVerified": True,
      "enabled": True, "firstName": "Ada", "lastName": "Lovelace",
      "credentials": [{"type": "password", "temporary": False, "value": open(sys.argv[2]).read()}]}))' \
      "$dev_user" "$(secret_file user.pw)")"
    curl -sS -f -o /dev/null -H @"$auth" -H 'content-type: application/json' \
      -X POST "$kc_url/admin/realms/$realm/users" --data-binary @- <<<"$body"
  fi
  rm -f "$auth"
}

up() {
  mkdir -p "$state"
  chmod 700 "$state"
  if answering; then
    echo "Something already answers on $kc_url; run '$0 down' first, or set KC_PORT" >&2
    exit 1
  fi
  prepare_import
  local admin_pw
  admin_pw="$(cat "$(secret_file admin.pw)")"
  case "$(mode)" in
    compose)
      KC_ADMIN_PASSWORD="$admin_pw" KC_PORT="$kc_port" compose up -d
      ;;
    dist)
      : "${KC_HOME:?no Docker Compose answered; set KC_HOME to a Keycloak 26.8 distribution}"
      KC_BOOTSTRAP_ADMIN_USERNAME=admin KC_BOOTSTRAP_ADMIN_PASSWORD="$admin_pw" \
        nohup "$KC_HOME/bin/kc.sh" start-dev --http-host=127.0.0.1 --http-port="$kc_port" \
        --features=cimd,resource-indicators >"$state/keycloak.log" 2>&1 &
      echo $! >"$state/keycloak.pid"
      ;;
    *) echo "OAUTH_DEV_MODE must be compose or dist" >&2; exit 1 ;;
  esac
  wait_until_answering
  import_realms_and_user
  cat <<MSG
Keycloak answers on $kc_url ($(mode) mode).
  Reference issuer: $kc_url/realms/$realm
  Resource it accepts: $maidan_url/mcp
  User: $dev_user, password in $state/user.pw
  Admin password: $state/admin.pw
Next: OAUTH_ISSUER=$kc_url/realms/$realm OAUTH_USER=$dev_user \\
      OAUTH_PASSWORD_FILE=$state/user.pw MAIDAN_URL=$maidan_url scripts/mcp-oauth-smoke.sh
MSG
}

down() {
  case "$(mode)" in
    # Compose interpolates the whole file even to stop it.
    compose) KC_ADMIN_PASSWORD=unused compose down ;;
    dist)
      if [ -s "$state/keycloak.pid" ]; then
        pkill -P "$(cat "$state/keycloak.pid")" 2>/dev/null || true
        kill "$(cat "$state/keycloak.pid")" 2>/dev/null || true
        rm -f "$state/keycloak.pid"
      fi
      ;;
  esac
}

case "${1:-}" in
  up) up ;;
  down) down ;;
  status)
    if answering; then echo "Keycloak answers on $kc_url"; else echo "Nothing answers on $kc_url"; exit 1; fi
    ;;
  *) echo "usage: $0 up|status|down" >&2; exit 2 ;;
esac

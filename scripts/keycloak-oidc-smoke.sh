#!/usr/bin/env bash
# Sign in to a Maidan console through Keycloak the way a browser does, with
# curl and two cookie jars, and print who the session is. It checks the
# Keycloak recipe in docs/OIDC.md end to end: Maidan's redirect carries an
# S256 PKCE challenge, Keycloak's login form accepts the person, the callback
# issues a maidan_session cookie, and /ui/api/me answers as a human member.
#
#   MAIDAN_URL=http://127.0.0.1:8080 MAIDAN_WORKSPACE=<workspace id> \
#   KC_USERNAME=ada KC_PASSWORD=... ./scripts/keycloak-oidc-smoke.sh
#
# Needs curl and jq, a Maidan started with the recipe's MAIDAN_OIDC_* settings,
# and a Keycloak realm imported from examples/keycloak/maidan-realm.json with
# the user in it. The password is read from the environment and sent only to
# Keycloak's own login form.
set -euo pipefail

BASE="${MAIDAN_URL:-http://127.0.0.1:8080}"
: "${MAIDAN_WORKSPACE:?set MAIDAN_WORKSPACE to the workspace id}"
: "${KC_USERNAME:?set KC_USERNAME to the Keycloak user}"
: "${KC_PASSWORD:?set KC_PASSWORD to the password of that user}"
for cmd in curl jq; do
  command -v "$cmd" >/dev/null 2>&1 || { echo "missing required command: $cmd" >&2; exit 1; }
done

jar_dir=$(mktemp -d)
trap 'rm -rf "$jar_dir"' EXIT
maidan_jar="$jar_dir/maidan"
idp_jar="$jar_dir/idp"

fail() { echo "FAIL: $*" >&2; exit 1; }
location() { # location <curl args...>: the Location header of one response
  curl -sS -o /dev/null -w '%{redirect_url}' "$@"
}

# 1. Maidan starts the flow and sends the browser to Keycloak.
auth_url=$(location -c "$maidan_jar" "$BASE/auth/oidc/login?workspace_id=$MAIDAN_WORKSPACE")
[[ -n "$auth_url" ]] || fail "GET /auth/oidc/login did not redirect"
case "$auth_url" in *"code_challenge_method=S256"*) ;; *) fail "the redirect carries no S256 PKCE challenge: $auth_url" ;; esac
echo "1. Maidan redirects to ${auth_url%%\?*} with an S256 PKCE challenge"

# 2. Keycloak shows its login form.
form=$(curl -sS -c "$idp_jar" -b "$idp_jar" "$auth_url")
action=$(grep -o 'id="kc-form-login"[^>]*action="[^"]*"' <<<"$form" | sed -E 's/.*action="([^"]*)".*/\1/; s/&amp;/\&/g')
[[ -n "$action" ]] || fail "no Keycloak login form at the authorization URL"
echo "2. Keycloak shows its login form"

# 3. The person signs in; Keycloak sends the browser back with a code.
# The password goes to curl through a file in the private temp dir, not argv.
(umask 077; printf '%s' "$KC_PASSWORD" >"$jar_dir/password")
callback=$(location -c "$idp_jar" -b "$idp_jar" \
  --data-urlencode "username=$KC_USERNAME" --data-urlencode "password@$jar_dir/password" \
  --data-urlencode "credentialId=" "$action")
case "$callback" in
  "$BASE/auth/oidc/callback?"*"code="*) ;;
  *) fail "Keycloak did not redirect to Maidan's callback with a code (wrong password, or a required action such as a profile update is pending): ${callback:-no redirect}" ;;
esac
echo "3. Keycloak accepts $KC_USERNAME and redirects to /auth/oidc/callback"

# 4. Maidan exchanges the code and sets its session cookie.
status=$(curl -sS -o "$jar_dir/callback.body" -w '%{http_code}' -c "$maidan_jar" -b "$maidan_jar" "$callback")
grep -q 'maidan_session' "$maidan_jar" || fail "the callback answered $status without a session cookie: $(cat "$jar_dir/callback.body")"
echo "4. The callback answers $status and sets the maidan_session cookie"

# 5. The session is a human member of the workspace.
session=$(curl -fsS -b "$maidan_jar" "$BASE/auth/session")
me=$(curl -fsS -b "$maidan_jar" "$BASE/ui/api/me")
member=$(jq -r .member_id <<<"$me")
[[ "$(jq -r .workspace_id <<<"$session")" == "$MAIDAN_WORKSPACE" ]] || fail "the session is for another workspace: $session"
[[ "$(jq -r .is_bearer <<<"$me")" == false ]] || fail "/ui/api/me answered as a bearer token"
members=$(curl -fsS -b "$maidan_jar" "$BASE/ui/api/workspaces/$MAIDAN_WORKSPACE/members")
row=$(jq -c --arg m "$member" '(if type == "array" then . else (.members // .items // []) end) | map(select(.id == $m)) | .[0]' <<<"$members")
[[ "$(jq -r .kind <<<"$row")" == human ]] || fail "the signed-in member is not a human: $row"
echo "5. The session is member $member ($(jq -r .handle <<<"$row"), human), session expires $(jq -r .expires_at <<<"$session")"
echo "   capabilities: $(jq -r '.capabilities | join(", ")' <<<"$me")"
echo "OK: signed in through Keycloak"

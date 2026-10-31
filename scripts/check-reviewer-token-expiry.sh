#!/usr/bin/env bash
# Reviewer-token expiry check (Next 14, Next 12's demo instance): a directory
# reviewer's token must outlast the longest review wait, so a review that sits in
# a queue never comes back to a dead credential.
#
#   MAIDAN_URL=https://demo.example \
#   MAIDAN_REVIEWER_TOKEN=<the token handed to the directory> \
#   MAIDAN_ADMIN_TOKEN=<a token:admin token in the same workspace> \
#   scripts/check-reviewer-token-expiry.sh
#
# MAIDAN_REVIEW_WAIT_DAYS (default 28) is the longest wait to cover. 28 is Next 16's
# estimate made whole: one assumed rejection and resubmit, at up to two weeks of
# review wait each. Raise it for a slower directory.
#
# The reviewer token names itself through GET /me (member, workspace, token id);
# the admin token reads that token's record, since a bearer cannot read its own
# expiry. Passes when the token is live and expires no sooner than now plus the
# wait, or never expires (reported, since a bounded expiry is the safer shape).
# Neither token is printed. Exit 0 pass, 1 fail, 2 usage or transport error.
set -euo pipefail

BASE_URL="${MAIDAN_URL:-http://127.0.0.1:8080}"
WAIT_DAYS="${MAIDAN_REVIEW_WAIT_DAYS:-28}"

for cmd in curl jq; do
  command -v "$cmd" >/dev/null 2>&1 || { echo "missing required command: $cmd" >&2; exit 2; }
done
if [ -z "${MAIDAN_REVIEWER_TOKEN:-}" ] || [ -z "${MAIDAN_ADMIN_TOKEN:-}" ]; then
  echo "MAIDAN_REVIEWER_TOKEN and MAIDAN_ADMIN_TOKEN are required" >&2
  exit 2
fi
case "$WAIT_DAYS" in ''|*[!0-9]*) echo "MAIDAN_REVIEW_WAIT_DAYS must be a whole number of days" >&2; exit 2 ;; esac

me=$(curl -fsS -H "authorization: Bearer ${MAIDAN_REVIEWER_TOKEN}" "${BASE_URL}/me") || {
  echo "FAIL: the reviewer token is refused by ${BASE_URL}/me (expired, revoked or wrong)" >&2
  exit 1
}
token_id=$(printf '%s' "$me" | jq -r '.token_id // empty')
member_id=$(printf '%s' "$me" | jq -er '.member_id')
workspace_id=$(printf '%s' "$me" | jq -er '.workspace_id')
[ -n "$token_id" ] || { echo "FAIL: the reviewer credential is a session, not a bearer token" >&2; exit 1; }

tokens=$(curl -fsS -H "authorization: Bearer ${MAIDAN_ADMIN_TOKEN}" \
  "${BASE_URL}/workspaces/${workspace_id}/members/${member_id}/tokens") || {
  echo "could not list the reviewer's tokens with the admin token (needs token:admin in ${workspace_id})" >&2
  exit 2
}

verdict=$(printf '%s' "$tokens" | jq -r --arg id "$token_id" --argjson days "$WAIT_DAYS" '
  def secs: sub("\\.[0-9]+"; "") | sub("\\+00:00$"; "Z") | fromdateiso8601;
  (map(select(.id == $id)) | first) as $t
  | if $t == null then {ok: false, why: "token \($id) is not listed for its member"}
    elif $t.revoked_at != null then {ok: false, why: "token \($id) was revoked at \($t.revoked_at)"}
    elif $t.expires_at == null then {ok: true, why: "token \($id) (\($t.label // "no label")) never expires; a bounded expiry past the wait is the safer shape"}
    else ($t.expires_at | secs) as $exp
      | (now + $days * 86400) as $need
      | if $exp >= $need
        then {ok: true, why: "token \($id) (\($t.label // "no label")) expires \($t.expires_at), \((($exp - now) / 86400) | floor) days out, covering the \($days)-day wait"}
        else {ok: false, why: "token \($id) (\($t.label // "no label")) expires \($t.expires_at), \((($exp - now) / 86400) | floor) days out, short of the \($days)-day wait"}
        end
    end
  | if .ok then "PASS: \(.why)" else "FAIL: \(.why)" end')
echo "$verdict"
case "$verdict" in PASS:*) exit 0 ;; *) exit 1 ;; esac

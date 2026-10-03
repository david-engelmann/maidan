#!/usr/bin/env bash
# Open Work's live tables against git history (docs/Open Work.md, "The rule").
#
# Fails when:
#   - the "Last reconciled against `main` at `<sha>`" stamp is not a commit on
#     this branch's history (the file was reconciled against something else);
#   - a PR listed under "Now: in flight" has merged, which means its row
#     should have been deleted;
#   - the "Now: in flight" heading is missing, or a row there names its PR in a
#     form other than `| #NNNN |`, so the check above would read nothing.
# A merged PR is one whose squash commit ("... (#NNNN)") is in HEAD's history.
# Needs full history (actions/checkout with fetch-depth: 0).
set -euo pipefail
cd "$(dirname "$0")/.."
doc="docs/Open Work.md"
fail=0

stamp=$(grep -oE 'Last reconciled against `main` at `[0-9a-f]{7,40}`' "$doc" | grep -oE '[0-9a-f]{7,40}' | head -1 || true)
if [[ -z "$stamp" ]]; then
  echo "open work: no reconciliation stamp"; fail=1
elif ! git merge-base --is-ancestor "$stamp" HEAD 2>/dev/null; then
  echo "open work: the stamp $stamp is not in this branch's history"; fail=1
fi

# Without the heading, or with a row whose PR cell this does not read, the
# loop below checks nothing and passes.
if ! grep -q '^## Now: in flight' "$doc"; then
  echo "open work: no \"## Now: in flight\" section"; fail=1
fi
now=$(awk '/^## Now: in flight/{on=1; next} /^## /{on=0} on' "$doc")
odd=$(printf '%s\n' "$now" | grep -E '^\|[^|]*#[0-9]+' | grep -vE '^\| #[0-9]+ ' || true)
if [[ -n "$odd" ]]; then
  echo "open work: rows under Now name a PR in a form this check does not read (write \`| #NNNN |\`):"
  printf '%s\n' "$odd" | sed 's/^/  /'
  fail=1
fi
# Read the history once: `git log | grep -q` under pipefail fails exactly when
# grep matches, since git log then dies of SIGPIPE.
subjects=$(git log --format=%s HEAD)
for pr in $(printf '%s\n' "$now" | grep -oE '^\| #[0-9]+ ' | grep -oE '[0-9]+' || true); do
  if printf '%s\n' "$subjects" | grep -E "\(#${pr}\)\$" >/dev/null; then
    echo "open work: #$pr has merged but is still listed under Now"; fail=1
  fi
done

[[ $fail -eq 0 ]] && echo "open work: ok"
exit $fail

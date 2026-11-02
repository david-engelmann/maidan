#!/usr/bin/env bash
# No tracked file links a domain Maidan does not own. Problem `type` URIs once
# named maidan.dev, which belongs to someone else, so every API error linked
# a stranger's site. History (the CHANGELOG and its fragments, retros,
# archives) records what was true then and is not scanned, and neither is this
# file, which has to name the domain.
set -euo pipefail
cd "$(dirname "$0")/.."

foreign='maidan\.dev'

# git grep exits 0 on a hit, 1 on none, and above 1 on an error, which must
# not pass.
set +e
git grep -nIE "$foreign" -- . ':!CHANGELOG.md' ':!changelog.d' ':!scripts/check-our-domains.sh' ':!docs/Retros' ':!docs/archive' ':!docs/Clusters' >&2
rc=$?
set -e
case $rc in
  0) echo "our domains: the lines above link a domain Maidan does not own; use maidan.world" >&2; exit 1 ;;
  1) echo "our domains: ok" ;;
  *) echo "our domains: git grep failed ($rc); refusing to pass" >&2; exit 1 ;;
esac

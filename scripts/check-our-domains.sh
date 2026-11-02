#!/usr/bin/env bash
# No tracked file links a domain Maidan does not own. Problem `type` URIs once
# named maidan.dev, which belongs to someone else, so every API error linked
# a stranger's site. History (released CHANGELOG entries, retros, archives)
# records what was true then and is not scanned.
set -euo pipefail
cd "$(dirname "$0")/.."

# Written split so this file does not match itself.
foreign='maidan''\.dev'

# git grep exits 0 on a hit, 1 on none, and above 1 on an error, which must
# not pass.
set +e
git grep -nIE "$foreign" -- . ':!CHANGELOG.md' ':!docs/Retros' ':!docs/archive' ':!docs/Clusters' >&2
rc=$?
set -e
case $rc in
  0) echo "our domains: the lines above link a domain Maidan does not own; use maidan.world" >&2; exit 1 ;;
  1) echo "our domains: ok" ;;
  *) echo "our domains: git grep failed ($rc); refusing to pass" >&2; exit 1 ;;
esac

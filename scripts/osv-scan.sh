#!/usr/bin/env bash
# Scan the lockfiles cargo-deny does not read for known vulnerabilities, with
# osv-scanner against the OSV database.
#
# cargo-deny checks the root Cargo.lock against RustSec. Everything else that
# pins third-party code is scanned here: every tracked lockfile except the root
# Cargo.lock (fuzz/Cargo.lock, ui-tests/package-lock.json, sdk/go/go.mod), and
# the Rust SDK resolved fresh, because sdk/rust ignores its Cargo.lock and a new
# user of the crate gets whatever resolves today. The TypeScript and Python
# SDKs have no runtime dependencies, so there is nothing to lock; the script
# fails if one gains a dependency without a lockfile beside it.
#
# Accepted advisories, each with its reason, are in .config/osv-scanner.toml.
#
# Usage:  scripts/osv-scan.sh
# Env:    OSV_SCANNER (reuse an osv-scanner binary; it must match the pinned hash)
set -euo pipefail

cd "$(dirname "$0")/.."
root="$PWD"

version="v2.6.0"
case "$(uname -s)/$(uname -m)" in
  Linux/x86_64) asset=linux_amd64 sha256=ca69b3d3cd08f889a49dc0a383122f71cc528b83803671df5fd874d97485b108 ;;
  Linux/aarch64) asset=linux_arm64 sha256=2c71403eb443d05891c4f268c3ad771cf4f16e5443463fd7851ef8f454d3c7e4 ;;
  Darwin/arm64) asset=darwin_arm64 sha256=98c460dcd37de25819babd757d04542045b6243113e209edcd4d89fedb0256b4 ;;
  Darwin/x86_64) asset=darwin_amd64 sha256=60c5296637e977b28eeda5c7f13573e447659a632922737f94d11fa7e30ad6ca ;;
  *) echo "no pinned osv-scanner build for $(uname -s)/$(uname -m)" >&2; exit 1 ;;
esac
scanner="${OSV_SCANNER:-${TMPDIR:-/tmp}/osv-scanner-${version}-${asset}}"

if [[ ! -f "$scanner" ]]; then
  curl -fsSL --retry 3 -o "$scanner.part" \
    "https://github.com/google/osv-scanner/releases/download/${version}/osv-scanner_${asset}"
  mv "$scanner.part" "$scanner"
fi
if command -v sha256sum >/dev/null; then
  echo "${sha256}  ${scanner}" | sha256sum -c --quiet
else
  echo "${sha256}  ${scanner}" | shasum -a 256 -c --quiet
fi
chmod +x "$scanner"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# A dependency-free SDK has nothing to scan until it gains a dependency; then
# it needs a lockfile, which the discovery below picks up.
python3 - <<'EOF'
import json, sys, tomllib
from pathlib import Path

errors = []
ts = json.loads(Path("sdk/typescript/package.json").read_text())
for key in ("dependencies", "optionalDependencies", "peerDependencies"):
    if ts.get(key) and not Path("sdk/typescript/package-lock.json").exists():
        errors.append(f"sdk/typescript/package.json declares {key} but has no package-lock.json")
py = tomllib.loads(Path("sdk/python/pyproject.toml").read_text())
if py["project"].get("dependencies") and not any(
    Path("sdk/python", lock).exists() for lock in ("uv.lock", "poetry.lock", "requirements.txt")
):
    errors.append("sdk/python/pyproject.toml declares dependencies but has no lockfile")
for error in errors:
    print(f"error: {error}; commit a lockfile so this scan can read it", file=sys.stderr)
sys.exit(1 if errors else 0)
EOF

lockfiles=()
while IFS= read -r path; do
  [[ "$path" == "Cargo.lock" ]] && continue
  lockfiles+=("$path")
done < <(git ls-files | grep -E '(^|/)(Cargo\.lock|package-lock\.json|yarn\.lock|pnpm-lock\.yaml|go\.mod|poetry\.lock|uv\.lock|Pipfile\.lock|requirements[^/]*\.txt)$')

# The published crate's consumers resolve its dependencies themselves, so scan
# a fresh resolution rather than whatever a local checkout last locked.
mkdir -p "$work/sdk-rust"
cp -R sdk/rust/Cargo.toml sdk/rust/src "$work/sdk-rust/"
(cd "$work/sdk-rust" && cargo generate-lockfile --quiet)
lockfiles+=("$work/sdk-rust/Cargo.lock")

args=()
for lockfile in "${lockfiles[@]}"; do
  echo "scanning ${lockfile#"$work/"}"
  args+=(-L "$lockfile")
done

"$scanner" scan source --config "$root/.config/osv-scanner.toml" "${args[@]}"

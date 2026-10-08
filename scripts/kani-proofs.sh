#!/usr/bin/env bash
# Run the Kani proofs locally. Not a CI job: Kani's toolchain is large and
# these proofs are not on the required checks.
#
#   cargo install --locked kani-verifier
#   export KANI_HOME="${KANI_HOME:-$HOME/.kani}"
#   cargo kani setup
#   scripts/kani-proofs.sh
#
# The harnesses are #[cfg(kani)] and are absent from a normal build. See
# docs/Kani.md.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

if ! command -v cargo-kani >/dev/null 2>&1; then
  echo "cargo-kani is not on PATH. Install with: cargo install --locked kani-verifier" >&2
  exit 1
fi

export KANI_HOME="${KANI_HOME:-$HOME/.kani}"

echo "== maidan-types (cursor arithmetic) =="
cargo kani -p maidan-types --output-format terse

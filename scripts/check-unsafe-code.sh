#!/usr/bin/env bash
# No unsafe code unless a crate is listed below (HEALTH-003). Every crate sets
# `unsafe_code` in its Cargo.toml `[lints.rust]` (which covers its tests,
# benches and examples) and in each crate root. The compiler then rejects a new
# unsafe block: in a `forbid` crate outright, in a `deny` crate unless the site
# carries `#[allow(unsafe_code)] // <why>`. This script fails if a crate drops
# the setting, a crate not listed here relaxes forbid to deny, an allow comes
# without its reason, a crate-wide `#![allow(unsafe_code)]` appears, or the
# lint is named in any other form (combined, multiline, expect, cfg_attr).
set -euo pipefail
cd "$(dirname "$0")/.."
# Crates that write the process environment (edition 2024 makes
# `std::env::set_var`/`remove_var` unsafe) in tests or before the runtime in
# `main`, and maidan-search for the sqlite-vec FFI.
deny_crates=" maidan-auth maidan-cli maidan-env maidan-search maidan-server "
bad=0
for dir in crates/*/; do
  crate=$(basename "$dir")
  level=forbid
  case "$deny_crates" in *" $crate "*) level=deny ;; esac
  # The line must sit in the `[lints.rust]` table: the same line under
  # `[package.metadata]` would pass a plain grep and set nothing.
  if ! awk -v want="unsafe_code = \"$level\"" '
    /^\[/ { in_rust = ($0 == "[lints.rust]") }
    in_rust && $0 == want { found = 1 }
    END { exit !found }
  ' "$dir/Cargo.toml"; then
    echo "$dir/Cargo.toml: [lints.rust] needs unsafe_code = \"$level\"" >&2
    bad=1
  fi
  for root in "$dir"src/lib.rs "$dir"src/main.rs "$dir"src/bin/*.rs; do
    [ -f "$root" ] || continue
    if ! grep -qx "#!\[$level(unsafe_code)\]" "$root"; then
      echo "$root: needs #![$level(unsafe_code)]" >&2
      bad=1
    fi
  done
done
# Only two spellings may name the lint: the crate-root `#![forbid|deny(...)]`
# line and the one-line site allow with its reason. Anything else that
# mentions `unsafe_code` (a combined `allow(dead_code, unsafe_code)`, an
# attribute split over lines, `expect`/`warn`, `cfg_attr`, a crate-wide
# `#![allow(...)]`) is rejected rather than parsed, so it cannot slip past the
# scope and reason checks.
if hits=$(rg -n '#!\[[^]]*allow[^]]*unsafe_code' crates); then
  echo "crate-wide allow(unsafe_code); allow each site instead:" >&2
  echo "$hits" >&2
  bad=1
fi
if hits=$(rg -n --no-heading 'unsafe_code' crates -g '*.rs' |
  rg -v '^[^:]+:[0-9]+:(#!\[(forbid|deny)\(unsafe_code\)\]|[[:space:]]*#\[allow\(unsafe_code\)\] // \S.*)$'); then
  echo "unsafe_code must appear only as #![forbid|deny(unsafe_code)] or a one-line #[allow(unsafe_code)] // <reason>:" >&2
  echo "$hits" >&2
  bad=1
fi
[ "$bad" = 0 ] || exit 1
echo "unsafe code: ok ($(rg -c '#\[allow\(unsafe_code\)\]' crates | awk -F: '{n += $2} END {print n}') allowed sites)"

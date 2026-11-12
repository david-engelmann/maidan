#!/usr/bin/env bash
# No unsafe code unless a crate is listed below (HEALTH-003). Every crate sets
# `unsafe_code` in its Cargo.toml `[lints.rust]` (which covers its tests,
# benches and examples) and in each crate root. The compiler then rejects a new
# unsafe block: in a `forbid` crate outright, in a `deny` crate unless the site
# carries `#[allow(unsafe_code)] // <why>`. This script fails if a crate drops
# the setting, a crate not listed here relaxes forbid to deny, an allow comes
# without its reason, or a crate-wide `#![allow(unsafe_code)]` appears.
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
  if ! grep -qx "unsafe_code = \"$level\"" "$dir/Cargo.toml"; then
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
if hits=$(rg -n '#!\[allow\(unsafe_code\)\]' crates); then
  echo "crate-wide allow(unsafe_code); allow each site instead:" >&2
  echo "$hits" >&2
  bad=1
fi
if hits=$(rg -n '#\[allow\(unsafe_code\)\]' crates | rg -v '#\[allow\(unsafe_code\)\] // \S'); then
  echo "#[allow(unsafe_code)] without a trailing // reason:" >&2
  echo "$hits" >&2
  bad=1
fi
[ "$bad" = 0 ] || exit 1
echo "unsafe code: ok ($(rg -c '#\[allow\(unsafe_code\)\]' crates | awk -F: '{n += $2} END {print n}') allowed sites)"

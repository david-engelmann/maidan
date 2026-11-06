### The remaining crates link their tests once

- **Changed:** maidan-types, maidan-a2a, maidan-artifacts, maidan-auth, maidan-bus and maidan-cli each build one integration-test binary from `tests/it/main.rs`, written by `scripts/gen-integration-main.py`, instead of one binary per `tests/*.rs` file. Run one file with `cargo test -p maidan-types --test it event_kinds_contract::`.

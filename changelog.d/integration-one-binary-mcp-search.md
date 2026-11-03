### maidan-mcp and maidan-search build one integration-test binary each

- **Changed:** `maidan-mcp` (14 test files) and `maidan-search` (17, besides the two sqlite-vec ones that keep their own feature-gated targets) set `autotests = false` and build a single `it` binary from `tests/it/main.rs`, written by the same `scripts/gen-integration-main.py` as maidan-server and maidan-store. Run one file with `cargo test -p maidan-mcp --test it wire_snapshots::`.

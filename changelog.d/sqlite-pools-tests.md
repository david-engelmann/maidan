### Test SQLite pools open like the server's

- **Changed:** Every SQLite pool in `crates/*/tests` now opens through `maidan_search::sqlite_pool_options`, so tests run with sqlite-vec and the per-connection PRAGMAs the server uses (#1725).
- **Changed:** `scripts/check-sqlite-pools.sh` covers `crates/*/tests` too. The two tests that check what a bare pool does are marked `// sqlite-pools: bare on purpose`.

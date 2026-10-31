### Every SQLite pool loads sqlite-vec

- **Fixed:** The CLI (`maidan` store, MCP stdio and shred-residue paths) and the in-crate test pools of maidan-mcp and maidan-server now open SQLite through `maidan_search::sqlite_pool_options`, so every connection has sqlite-vec and the per-connection PRAGMAs. A CI check (`scripts/check-sqlite-pools.sh`) refuses a bare `SqlitePoolOptions::new()` in `crates/*/src` outside maidan-search.

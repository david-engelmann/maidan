### Unaudited authority changes are unreachable from non-test code

- **Changed:** The 36 store methods that change authority without writing an audit row (D-A) now compile only under maidan-store's `test-support` feature, which only dev-dependencies enable. Request handlers, MCP tools and the CLI can't call them, so a regression fails `cargo build --workspace --lib --bins` instead of depending on a source scan. `maidan init` now mints its bootstrap admin token through the audited form and records a `token.mint` audit row.

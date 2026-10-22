### The MCP tool catalog is one checked-in JSON file

- **Changed:** the 258-tool `tools/list` catalog moves from 5,000 lines of `json!` literals in `crates/maidan-mcp/src/tools/catalog.rs` to `crates/maidan-mcp/src/tools/catalog.json`, embedded with `include_str!` and parsed once (`OnceLock`). `scripts/regen_derived.py` (run by `make regen`) now reads that file instead of the `tools_list_full.json` wire snapshot. The wire output and every snapshot are byte-identical; the member-scope gate test reads parsed schemas instead of grepping Rust source.

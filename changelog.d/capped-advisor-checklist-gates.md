### Fixed

- **Fixed:** `scripts/capped` refuses non-positive or non-finite time/memory caps; the land-gate advisor eval bounds JSONL size and requires a safe http(s) URL plus thread id; MCP resource notification checks also require `resources/list` and `resources/templates/list`; and sqlite-pool checks cover `crates/*/benches` (search excluded; store/search hot benches mark bare pools on purpose).

### Authority audit covers every crate and example

- **Changed:** `authority_audit_contract` now scans every workspace crate and example, not only the server and MCP sources. The two carve-outs are named and pinned: `maidan init` mints the first admin token through the audited form, and the Playwright harnesses (`ui_test_server`, `capture_server`) are the only files still allowed to call the unaudited store methods, each with its exact set of calls listed.

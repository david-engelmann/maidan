### An over-permission scan of the MCP tool surface

- **Added:** `mcp_over_permission_contract` checks that every MCP tool
  declares a known capability, that a read-only tool declares a read
  capability (or is named in the privileged-read pin), and that a
  mutating tool does not declare a read capability unless it is named
  in the read-cap-write pin. An unpinned mutating tool may declare any
  non-read capability. A fixed list of privileged store calls is
  checked one handler-call level deep against the declaration;
  reviewed exceptions (holder-side token mint and rotate, and seating
  the creator of a private channel) are named and pinned.

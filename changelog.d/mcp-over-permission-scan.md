### An over-permission scan of the MCP tool surface

- **Added:** `mcp_over_permission_contract` checks that every MCP tool declares a known capability, that the declaration is the narrowest one covering what the handler does, and that no handler does more than the declaration allows. Privileged reads, personal and holder-side writes that declare a read capability, and the reviewed handler-call exceptions (holder-side token mint and rotate, and seating the creator of a private channel) are named and pinned.

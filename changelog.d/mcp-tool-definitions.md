### Tool definitions say when to use them and describe every parameter

- **Changed:** 52 MCP tool descriptions were rewritten from their handlers. Each says what the tool does and how it differs from its nearest sibling, when to use a named alternative, what it requires, returns and changes, and gives every parameter a description. Several now state behavior the old text left out: `unassign_thread` works on any holder's claim and charges the time worked, `list_messages` returns only the oldest page, the member-scoped reads refuse another member's id, and `register_fsm_hook` needs `workspace:admin`.
- **Fixed:** `get_workspace_context` declares its `thread_cursor` parameter, so an agent can page past the first threads.
- **Added:** `listings/glama/Dockerfile`, the image Glama builds to list the tools: the newest signed release, served over stdio on a local database.

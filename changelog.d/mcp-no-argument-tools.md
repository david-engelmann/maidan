### Every tool refuses a bad call by name, and the Inspector calls them all

- **Fixed:** a tool that declares no arguments now refuses any argument, and names it (`unknown field `x`: whoami takes no arguments`). Every tool with arguments already refused an undeclared one. The 18 tools without arguments dropped it and ran: an agent that passed a token id to `rotate_token`, or a workspace id to `list_recipes`, got an answer for something other than what it asked.
- **Fixed:** a missing argument to `get_thread_context` or `get_workspace_context` is reported once (`invalid params: missing field `thread_id``), not as `invalid params: invalid params: …`.
- **Added:** `scripts/mcp-tool-sweep.py`, run by `scripts/mcp-inspector.sh`, calls every tool on a running server:
  - a valid call to every tool on `/mcp/worker`, `/mcp/reviewer` and `/mcp/coordinator`, made on that endpoint, writes included (one task taken from creation through claim, usage, result and review, a plan and its amendment, a cancel, an approval and a notification);
  - a valid call to every read-only tool in the full catalog;
  - a bad call to every tool, whose error must name the argument at fault.

  A validated version (the release record) needs every tool on a listed surface to answer both. Until now the Inspector called one tool per group.

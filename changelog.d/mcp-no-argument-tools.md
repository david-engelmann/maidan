### Every tool refuses a bad call by name, and the Inspector calls them all

- **Fixed:** a tool that declares no arguments now refuses any argument, and names it (`unknown field `x`: whoami takes no arguments`). Every tool with arguments already refused an undeclared one, and so did `rotate_token`. The other 17 dropped it and ran: an agent that passed a `workspace_id` to `list_recipes` or `get_wip_limit` got its own workspace's answer, whatever it named.
- **Fixed:** a missing argument to `get_thread_context` or `get_workspace_context` is reported once (`invalid params: missing field `thread_id``), not as `invalid params: invalid params: …`.
- **Added:** `scripts/mcp-tool-sweep.py`, run by `scripts/mcp-inspector.sh`, calls every tool on a running server:
  - a valid call to every tool on `/mcp/worker`, `/mcp/reviewer` and `/mcp/coordinator`, made on that endpoint, writes included (one task taken from creation through claim, usage, result and review, a plan and its amendment, a cancel, an approval and a notification). The coordinator's calls run on a token holding only the capabilities an OAuth grant can carry, as the ChatGPT listing's connection does;
  - a valid call to every other write in the full catalog, in an order where each has what it needs, and to every read-only tool. Six answer the error that names a backend the throwaway server does not run: the four multipart upload tools (S3), `top_up_credits` (the credits chain watcher) and `advise_land_gate` (an advisor);
  - a bad call to every tool, whose error must name the argument at fault.

  A validated version (the release record) needs every tool on a listed surface to answer both. Until now the Inspector called one tool per group.

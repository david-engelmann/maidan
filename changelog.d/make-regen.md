### One command regenerates every golden and catalog copy

- **Added:** `make regen` (`scripts/regen.sh`) rewrites the profile golden, the MCP and server wire snapshots, the MCP reference, and, through `scripts/regen_derived.py`, every file that copies the MCP catalog: `contracts/mcp-tool-names.json`, `listings/docker/tools.json`, the ToolHive tool list and the tool count in `docs/Protocols.md`, `README.md` and `book/src/introduction.md`. Two PRs that each add a tool conflicted in all of these; a conflict is now resolved by taking either side and running it.

### A read-only token can no longer open group DMs or approval gates

- **Changed:** opening a group DM (`POST /workspaces/{wid}/group-dms` and MCP `open_group_dm`) and opening an approval gate (MCP `request_approval`) now need `message:post` instead of `workspace:read`. Both reach people, so a compromised read-only token can no longer put members in a conversation or ask them to approve something. The worker preset and the recipe agent scope already carry `message:post`; a token minted with only `workspace:read` (or `workspace:read` plus `workspace:write`) is now refused with `403`.

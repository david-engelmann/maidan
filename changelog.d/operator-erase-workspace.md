### An operator can erase a workspace nobody can sign in to

- **Added:** `DELETE /operator/workspaces/{id}` (`operator:global`), with `{confirm_workspace_id, reason}`: the erase of `DELETE /workspaces/{id}` for a workspace whose own admin cannot run it. A legal hold refuses it with a 409, the operator's own workspace is refused, and the audit row names the operator and the reason.
- **Changed:** when the launch gate cannot erase a test workspace, it names the operator route to erase it with.

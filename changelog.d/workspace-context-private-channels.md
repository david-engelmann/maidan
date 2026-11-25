### The workspace context names only channels the reader may see

- **Security:** `get_workspace_context` and `GET /workspaces/{id}/context` listed every channel in the workspace, private ones included, to any member with `workspace:read`. They now apply the rule `list_channels` uses: a private channel is listed only to its members. Threads were already filtered, so no message content was exposed, only private channel names and topics.

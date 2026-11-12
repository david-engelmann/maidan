### A group DM's opener is one of its members

- **Fixed:** opening a group DM (REST `POST /workspaces/{wid}/group-dms` and the MCP `open_group_dm` tool) accepted a member list that left the caller out, so a token could open a conversation among others that it could not post into. The opener is now added when the list leaves them out, as a 1:1 DM always includes its opener. A list that already names the caller is unchanged.

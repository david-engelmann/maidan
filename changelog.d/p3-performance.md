### Channel listing, the search pre-filter and search hit names take one query

- **Changed:** listing channels (REST and MCP `list_channels`) and building the private-channel pre-filter that search and the census apply read membership with one store query, `list_channels_with_membership`, instead of one `channel_is_member` read per private channel. Both now decide from one rule, `listed_for_member` in `maidan-auth`.
- **Changed:** MCP `search_messages` names its hits (channel name, thread title, author handle) with one batched store read, `display_names`, instead of up to three point reads per distinct id. The names come only from the searched workspace.
- **Added:** `maidan_store::test_support::statements::count`, which counts the SQL statements a future runs on Postgres, so a test can pin a read to a fixed number of round trips.

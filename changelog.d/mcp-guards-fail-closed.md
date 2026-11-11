### The MCP id guard fails closed

- **Security:** every id argument an MCP tool's schema declares (`*_id`,
  `*_ids`, `sha256`, or a `uuid` format) now has one recorded decision:
  checked before dispatch for channel, thread or message access, held to the
  caller by the workspace and member gates, or exempt with the reason its
  handler binds it. A tool with an id argument nobody classified is refused
  before it runs, where the old name list's default let it through
  unchecked (MCP-005). Which callers may call which tools is unchanged.
- **Added:** classification tests for `thread_id` and `message_id` beside the
  `channel_id` and `member_id` ones, a contract that walks every tool's
  schema, a planted unclassified tool that dispatch refuses, and a
  two-tenant check that a thread-scoped tool refuses another workspace's
  thread.

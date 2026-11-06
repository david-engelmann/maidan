### Fixed

- **Fixed:** `mcp-stdio` bounds each request line and answers a bad line without exiting. A streamable session that has expired cannot be touched, replayed, or closed back into life, and its replay log is capped in bytes. Frame capture withholds credential-bearing tools and redacts signing keys, webhook secrets, client secrets, PKCE verifiers, and session ids. A slash command has to end at whitespace, and `@Handle` mentions fold case, skip a removed member, and surface a database error.

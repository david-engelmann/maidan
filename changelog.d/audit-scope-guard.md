### Security

- **Changed:** every `*_audited` store write, on SQLite and Postgres, now refuses an audit row whose scope is not the workspace the write changed (or `instance` for a workspace's change), and rolls the change back with it. One guard, `audit::append_scoped`, resolves what was changed (a workspace, member, channel, thread or message) and is the only way to the audit table; a contract test fails when an audited method writes its record any other way. Follow-up to #1134.

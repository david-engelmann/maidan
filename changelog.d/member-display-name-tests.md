### Display-name changes are tested on both backends and across workspaces

- **Added:** Tests check that a display-name change commits with its event and audit row in the member's own workspace on SQLite and Postgres, and rolls back when the audit row aims at another workspace.
- **Added:** Tests check that renaming another member is refused in the same workspace and across workspaces, and that another workspace's export carries no rename.

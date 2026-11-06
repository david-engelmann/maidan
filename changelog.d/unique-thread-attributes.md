### One open thread per ticket and repository: unique thread attributes

- **Added:** A `repo` task attribute (`owner/name`; a GitHub URL is read as its `owner/name`) on REST, MCP `create_thread` and `set_thread_attributes`, and the SDK models. It is stored in `maidan_thread_attributes.repo`, and find-by-attribute filters on it (`attr.repo`).
- **Added:** `GET`/`PUT /workspaces/{id}/unique-thread-attributes` lists the attribute key sets a workspace keeps unique as tuples among its open threads. Reading takes `workspace:read`. Setting takes `token:admin` and is audited as `workspace.unique_thread_attributes.set`.
  - `{"unique": [["linear_issue", "repo"]]}` keeps one open thread per Linear ticket in each repository, so a ticket's halves in two repositories are allowed.
  - `[["linear_issue"]]` keeps one open thread per ticket.
  - A thread missing a key of a set is not checked against that set.
  - Only `kind: change` threads are held to the sets. A review or plan on the same ticket and repository, such as a red-team review, never conflicts.
  - `{"unique": []}`, the default, keeps nothing unique.
  - Stored in `maidan_workspaces.unique_thread_attributes` (migration 0223 on Postgres, 0222 on SQLite).
- **Added:** A thread create, or an attribute set on an open thread, that would give a second open thread the same tuple is refused and writes nothing.
  - REST answers `409` `conflict` with `key_set` and `holder_thread_id`.
  - MCP answers error `-32011` with `data.key_set` and `data.holder_thread_id`.
  - The holder is named only to a caller who may read it.
  - Closed, archived and erased threads hold nothing, and another workspace's threads never count.
  - Concurrent writers of one tuple leave one winner: Postgres takes a transaction-scoped advisory lock per (workspace, key set, values), and SQLite relies on its write lock.
  - The router must stamp both `linear_issue` and `repo`. See the Swarms doc, "One owner per ticket".

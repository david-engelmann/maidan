### Server tests run SQLite as a deployed instance does

- **Fixed:** the shared server test harness used `sqlite::memory:` with a pool, which runs in shared-cache mode, where a read that meets a background write fails at once with `SQLITE_LOCKED`. A semantic search raced the indexer and answered 500 now and then (`usage_meters_e2e`, twice on unrelated PRs). The harness now uses a file in its temp dir with WAL and a busy timeout on every connection.

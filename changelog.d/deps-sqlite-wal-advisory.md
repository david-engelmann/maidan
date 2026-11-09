### The SQLite WAL advisory is accepted until sqlx allows the fix

- **Security:** RUSTSEC-2026-0459 (CVE-2026-119375; bundled SQLite before 3.54 can return uninitialized memory when a database is opened beside a `-wal` file a less trusted party wrote) is accepted in `deny.toml` and the OSV scan. Maidan opens only the operator's `DATABASE_URL` file and files it creates itself. The fixed `libsqlite3-sys` 0.39 is outside every sqlx release's range, so the ignore drops when sqlx allows it.

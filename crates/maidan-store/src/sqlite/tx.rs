//! How the SQLite store opens a transaction that writes.
//!
//! sqlx's `begin()` issues a deferred `BEGIN`: the transaction reads from a
//! snapshot and asks for the write lock only at its first write. In WAL mode,
//! when another connection has committed since that snapshot was taken, the
//! upgrade can never succeed (the write would build on rows that are no longer
//! current), so SQLite returns `SQLITE_BUSY_SNAPSHOT` ("database is locked")
//! at once. The busy handler is not consulted, so `busy_timeout` does not
//! help. `BEGIN IMMEDIATE` waits for the write lock, under `busy_timeout`,
//! before the transaction reads anything, so no commit can land between its
//! read and its write. Readers are not blocked by it in WAL mode.
//!
//! The rule in this module's callers: a transaction that can write begins with
//! [`begin_write`] or [`begin_write_on`]. `tests/sqlite_write_lock_contract.rs`
//! fails on any other `begin` in `src/sqlite/` unless it names a read-only
//! transaction or a savepoint, with a reason.

use sqlx::{Connection, Sqlite, SqliteConnection, SqlitePool, Transaction};

/// A transaction on `pool` that holds the write lock from its first statement.
pub(crate) async fn begin_write(
    pool: &SqlitePool,
) -> Result<Transaction<'static, Sqlite>, sqlx::Error> {
    pool.begin_with("BEGIN IMMEDIATE").await
}

/// As [`begin_write`], on a connection the caller may already have in a
/// transaction. There it opens a savepoint: the write lock belongs to the
/// outer transaction, which took it when it began with [`begin_write`].
/// (`begin_with` refuses a custom statement on a connection already in a
/// transaction, so the two cases cannot share one call.)
pub(crate) async fn begin_write_on(
    conn: &mut SqliteConnection,
) -> Result<Transaction<'_, Sqlite>, sqlx::Error> {
    if conn.is_in_transaction() {
        Connection::begin(conn).await
    } else {
        Connection::begin_with(conn, "BEGIN IMMEDIATE").await
    }
}

#[cfg(test)]
mod tests {
    //! These drive two connections to one file-backed WAL database by hand,
    //! so the interleaving that failed in CI (#2139) happens every run rather
    //! than under load: connection A reads, connection B commits a write, A
    //! writes.

    use std::time::{Duration, Instant};

    use sqlx::{Row, SqlitePool};

    use super::{begin_write, begin_write_on};

    /// Long enough that an error arriving well inside it did not wait for it.
    const BUSY_TIMEOUT: Duration = Duration::from_secs(30);

    /// The server's pool shape: file-backed, WAL, `busy_timeout` on every
    /// connection; two connections, one for each side of the race.
    async fn wal_pool(dir: &tempfile::TempDir) -> SqlitePool {
        let url = format!("sqlite://{}?mode=rwc", dir.path().join("tx.db").display());
        let busy_ms = u64::try_from(BUSY_TIMEOUT.as_millis()).unwrap();
        let pool = maidan_search::sqlite_pool_options_with(busy_ms)
            .max_connections(2)
            .connect(&url)
            .await
            .unwrap();
        sqlx::query("CREATE TABLE t (id INTEGER PRIMARY KEY, n INTEGER NOT NULL)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO t (id, n) VALUES (1, 0)")
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    async fn read_n<'e, E>(executor: E) -> i64
    where
        E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
    {
        sqlx::query("SELECT n FROM t WHERE id = 1")
            .fetch_one(executor)
            .await
            .unwrap()
            .get("n")
    }

    /// The primary result code of a SQLite error (`SQLITE_BUSY` is 5, and
    /// `SQLITE_BUSY_SNAPSHOT` is 5 extended to 517).
    fn primary_code(err: &sqlx::Error) -> Option<i32> {
        let code = err.as_database_error()?.code()?;
        code.parse::<i32>().ok().map(|c| c & 0xff)
    }

    /// Fails the next write on `conn` at once, rather than after
    /// `BUSY_TIMEOUT`, when another connection holds the write lock.
    async fn no_wait(conn: &mut sqlx::SqliteConnection) {
        sqlx::query("PRAGMA busy_timeout = 0")
            .execute(&mut *conn)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn a_deferred_transaction_that_reads_first_cannot_write_after_another_commit() {
        let dir = tempfile::tempdir().unwrap();
        let pool = wal_pool(&dir).await;

        let mut a = pool.begin().await.unwrap();
        let seen = read_n(&mut *a).await;

        let mut b = pool.acquire().await.unwrap();
        sqlx::query("UPDATE t SET n = n + 1 WHERE id = 1")
            .execute(&mut *b)
            .await
            .unwrap();

        let started = Instant::now();
        let err = sqlx::query("UPDATE t SET n = ? WHERE id = 1")
            .bind(seen + 1)
            .execute(&mut *a)
            .await
            .unwrap_err();
        assert_eq!(primary_code(&err), Some(5), "expected SQLITE_BUSY: {err}");
        assert!(
            started.elapsed() < BUSY_TIMEOUT / 2,
            "the stale upgrade should fail at once, not wait out busy_timeout"
        );
    }

    #[tokio::test]
    async fn begin_write_holds_the_write_lock_before_its_first_write() {
        let dir = tempfile::tempdir().unwrap();
        let pool = wal_pool(&dir).await;

        let mut a = begin_write(&pool).await.unwrap();
        let seen = read_n(&mut *a).await;

        // B cannot commit between A's read and A's write: A holds the lock.
        let mut b = pool.acquire().await.unwrap();
        no_wait(&mut b).await;
        let err = sqlx::query("UPDATE t SET n = n + 1 WHERE id = 1")
            .execute(&mut *b)
            .await
            .unwrap_err();
        assert_eq!(primary_code(&err), Some(5), "expected SQLITE_BUSY: {err}");
        // A reader is not blocked by it.
        assert_eq!(read_n(&mut *b).await, seen);

        sqlx::query("UPDATE t SET n = ? WHERE id = 1")
            .bind(seen + 1)
            .execute(&mut *a)
            .await
            .unwrap();
        a.commit().await.unwrap();

        sqlx::query("UPDATE t SET n = n + 1 WHERE id = 1")
            .execute(&mut *b)
            .await
            .unwrap();
        assert_eq!(read_n(&mut *b).await, seen + 2);
    }

    #[tokio::test]
    async fn begin_write_on_a_bare_connection_holds_the_write_lock() {
        let dir = tempfile::tempdir().unwrap();
        let pool = wal_pool(&dir).await;

        let mut conn = pool.acquire().await.unwrap();
        let mut a = begin_write_on(&mut conn).await.unwrap();
        let seen = read_n(&mut *a).await;

        let mut b = pool.acquire().await.unwrap();
        no_wait(&mut b).await;
        let err = sqlx::query("UPDATE t SET n = n + 1 WHERE id = 1")
            .execute(&mut *b)
            .await
            .unwrap_err();
        assert_eq!(primary_code(&err), Some(5), "expected SQLITE_BUSY: {err}");

        sqlx::query("UPDATE t SET n = ? WHERE id = 1")
            .bind(seen + 1)
            .execute(&mut *a)
            .await
            .unwrap();
        a.commit().await.unwrap();
        assert_eq!(read_n(&mut *b).await, seen + 1);
    }

    #[tokio::test]
    async fn begin_write_on_inside_a_transaction_opens_a_savepoint() {
        let dir = tempfile::tempdir().unwrap();
        let pool = wal_pool(&dir).await;

        let mut outer = begin_write(&pool).await.unwrap();
        let mut inner = begin_write_on(&mut outer).await.unwrap();
        sqlx::query("UPDATE t SET n = 7 WHERE id = 1")
            .execute(&mut *inner)
            .await
            .unwrap();
        inner.rollback().await.unwrap();
        // Rolling the savepoint back keeps the outer transaction open.
        assert_eq!(read_n(&mut *outer).await, 0);

        let mut inner = begin_write_on(&mut outer).await.unwrap();
        sqlx::query("UPDATE t SET n = 9 WHERE id = 1")
            .execute(&mut *inner)
            .await
            .unwrap();
        inner.commit().await.unwrap();
        outer.commit().await.unwrap();
        assert_eq!(read_n(&pool).await, 9);
    }
}

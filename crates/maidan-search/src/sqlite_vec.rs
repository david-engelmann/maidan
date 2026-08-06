//! Load the statically linked `sqlite-vec` extension into sqlx SQLite connections.

use std::sync::Once;

use sqlx::sqlite::{SqliteConnection, SqlitePoolOptions};

#[cfg(feature = "sqlite-vec")]
use std::{ffi::c_void, os::raw::c_char};

#[cfg(feature = "sqlite-vec")]
use libsqlite3_sys::{sqlite3, SQLITE_OK};

static AUTO_EXT: Once = Once::new();

#[cfg(feature = "sqlite-vec")]
#[link(name = "sqlite_vec0")]
extern "C" {
    fn sqlite3_vec_init(
        db: *mut sqlite3,
        pz_err_msg: *mut *mut c_char,
        p_api: *const c_void,
    ) -> i32;
}

/// Register `sqlite-vec` for every new SQLite connection in this process.
///
/// Call before opening pools when not using [`pool_options`].
pub fn ensure_auto_extension() {
    #[cfg(feature = "sqlite-vec")]
    AUTO_EXT.call_once(|| unsafe {
        libsqlite3_sys::sqlite3_auto_extension(Some(std::mem::transmute::<
            *const (),
            unsafe extern "C" fn(
                *mut sqlite3,
                *mut *mut c_char,
                *const libsqlite3_sys::sqlite3_api_routines,
            ) -> i32,
        >(
            sqlite_vec::sqlite3_vec_init as *const ()
        )));
    });
    #[cfg(not(feature = "sqlite-vec"))]
    let _ = &AUTO_EXT;
}

/// `SqlitePoolOptions` that loads `sqlite-vec` and applies the per-connection
/// PRAGMAs on **each** new connection (default 5000 ms `busy_timeout`).
pub fn pool_options() -> SqlitePoolOptions {
    pool_options_with(5000)
}

/// As [`pool_options`], with a configurable `busy_timeout` (Cluster 166).
///
/// `foreign_keys` and `busy_timeout` are **per-connection** settings in SQLite,
/// so they must run on every connection the pool opens — not once on a single
/// pooled connection (the prior `configure_pool` bug left the other connections
/// with FKs off and fail-fast-on-busy). `journal_mode = WAL` is file-level and
/// idempotent per connection.
pub fn pool_options_with(busy_timeout_ms: u64) -> SqlitePoolOptions {
    ensure_auto_extension();
    SqlitePoolOptions::new().after_connect(move |conn, _| {
        Box::pin(async move {
            load_on_connection(conn).await?;
            sqlx::query("PRAGMA foreign_keys = ON")
                .execute(&mut *conn)
                .await?;
            sqlx::query("PRAGMA journal_mode = WAL")
                .execute(&mut *conn)
                .await?;
            sqlx::query(&format!("PRAGMA busy_timeout = {busy_timeout_ms}"))
                .execute(&mut *conn)
                .await?;
            Ok(())
        })
    })
}

/// Load `sqlite-vec` on an existing connection (idempotent).
pub async fn load_on_connection(conn: &mut SqliteConnection) -> Result<(), sqlx::Error> {
    #[cfg(feature = "sqlite-vec")]
    {
        let mut handle = conn.lock_handle().await?;
        let db = handle.as_raw_handle();
        let rc = unsafe { sqlite3_vec_init(db.as_ptr(), std::ptr::null_mut(), std::ptr::null()) };
        if rc != SQLITE_OK {
            return Err(sqlx::Error::Configuration(
                "sqlite-vec init failed on connection".into(),
            ));
        }
        Ok(())
    }
    #[cfg(not(feature = "sqlite-vec"))]
    {
        let _ = conn;
        Ok(())
    }
}

/// Whether `vec_distance_cosine` is callable on this pool.
pub async fn vec_available(pool: &sqlx::SqlitePool) -> bool {
    #[cfg(feature = "sqlite-vec")]
    {
        sqlx::query_scalar::<_, String>("SELECT vec_version()")
            .fetch_one(pool)
            .await
            .is_ok()
    }
    #[cfg(not(feature = "sqlite-vec"))]
    {
        let _ = pool;
        false
    }
}

#[cfg(test)]
mod pragma_tests {
    use super::pool_options_with;

    // A file-backed DB (not `:memory:`, which is per-connection) so the pool
    // hands out several connections against the same database.
    #[tokio::test]
    async fn pragmas_apply_to_every_pooled_connection() {
        let path = std::env::temp_dir().join(format!("maidan_pragma_{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let url = format!("sqlite://{}?mode=rwc", path.display());
        let pool = pool_options_with(1234)
            .max_connections(3)
            .connect(&url)
            .await
            .unwrap();

        // Hold three connections at once to force three distinct ones.
        let mut held = Vec::new();
        for _ in 0..3 {
            let mut c = pool.acquire().await.unwrap();
            let fk: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
                .fetch_one(&mut *c)
                .await
                .unwrap();
            assert_eq!(fk, 1, "foreign_keys must be ON on every connection");
            let bt: i64 = sqlx::query_scalar("PRAGMA busy_timeout")
                .fetch_one(&mut *c)
                .await
                .unwrap();
            assert_eq!(bt, 1234, "busy_timeout must be set on every connection");
            held.push(c);
        }
        drop(held);
        pool.close().await;
        let _ = std::fs::remove_file(&path);
    }
}

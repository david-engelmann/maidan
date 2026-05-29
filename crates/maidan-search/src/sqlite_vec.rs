//! Load the statically linked `sqlite-vec` extension into sqlx SQLite connections.

use std::ffi::c_void;
use std::os::raw::c_char;
use std::sync::Once;

use libsqlite3_sys::{sqlite3, SQLITE_OK};
use sqlx::sqlite::{SqliteConnection, SqlitePoolOptions};

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
        >(sqlite_vec::sqlite3_vec_init as *const ())));
    });
    #[cfg(not(feature = "sqlite-vec"))]
    let _ = &AUTO_EXT;
}

/// `SqlitePoolOptions` that loads `sqlite-vec` on each new connection.
pub fn pool_options() -> SqlitePoolOptions {
    ensure_auto_extension();
    SqlitePoolOptions::new().after_connect(|conn, _| {
        Box::pin(async move {
            load_on_connection(conn).await?;
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

//! sqlite-vec extension loads and accelerates semantic distance in SQL.

use maidan_search::{sqlite_pool_options, vec_available, vec_ready};

#[tokio::test]
async fn sqlite_vec_extension_registers_on_pool_connect() {
    let pool = sqlite_pool_options()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    assert!(
        vec_available(&pool).await,
        "vec_version() should succeed after pool_options connect"
    );
}

/// A connection opened before the extension was registered for the process
/// still runs vector queries once `vec_ready` has checked it: the check and
/// the query share that connection.
#[tokio::test]
async fn a_connection_opened_before_registration_is_ready_after_the_check() {
    use sqlx::Connection;
    let mut conn = sqlx::SqliteConnection::connect("sqlite::memory:")
        .await
        .expect("open a plain connection");
    assert!(vec_ready(&mut conn).await, "vec_ready loads the extension");
    let distance: f64 = sqlx::query_scalar("SELECT vec_distance_cosine('[1,0]', '[1,0]')")
        .fetch_one(&mut conn)
        .await
        .expect("vec_distance_cosine runs on the checked connection");
    assert!(distance.abs() < 1e-6);
}

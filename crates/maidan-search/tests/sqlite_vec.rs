//! sqlite-vec extension loads and accelerates semantic distance in SQL.

use maidan_search::{sqlite_pool_options, vec_available};

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

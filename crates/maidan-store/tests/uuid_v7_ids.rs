//! Store-minted ids are UUIDv7 and sort by creation time (Wave 4 #44).

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewMember, NewWorkspace};
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn store_minted_ids_are_v7_and_sort_by_creation() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store = SqliteStore::new(pool);
    let mut ids = Vec::new();
    for name in ["a", "b", "c"] {
        let ws = store
            .create_workspace(NewWorkspace { name: name.into() })
            .await
            .unwrap();
        let member = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: name.into(),
                display_name: None,
                kind: MemberKind::Agent,
            })
            .await
            .unwrap();
        assert_eq!(ws.id.0.get_version_num(), 7);
        assert_eq!(member.id.0.get_version_num(), 7);
        ids.push(ws.id.0);
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted, "v7 ids sort in creation order");
}

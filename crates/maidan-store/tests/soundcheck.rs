//! Soundcheck gate pointer (Cluster 385, Wave 2 #25 remainder): require
//! arms the gate; a pass from a soundcheck-skilled member ≠ the
//! implementer is green/landable; amber (flags-then-still-engages) and
//! fail are not a land. Both backends.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    LandColor, MemberKind, NewChannel, NewMember, NewThread, NewWorkspace, SoundcheckStatus,
    SOUNDCHECK_KIND, SOUNDCHECK_SKILL,
};
use sqlx::sqlite::SqlitePoolOptions;

async fn sqlite() -> SqliteStore {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    SqliteStore::new(pool)
}

async fn run_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .expect("ws");
    let mk = |handle: &'static str| {
        let ws = ws.id;
        async move {
            store
                .create_member(NewMember {
                    workspace_id: ws,
                    handle: handle.into(),
                    display_name: None,
                    kind: MemberKind::Agent,
                })
                .await
                .expect(handle)
        }
    };
    let owner = mk("owner").await;
    let assignee = mk("assignee").await;
    let checker = mk("soundcheck").await;
    let unskilled = mk("unskilled").await;
    store
        .add_member_skill(checker.id, SOUNDCHECK_SKILL)
        .await
        .expect("skill");
    store
        .add_member_skill(owner.id, SOUNDCHECK_SKILL)
        .await
        .expect("owner also skilled");
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "c".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("t".into()),
        })
        .await
        .expect("thread");
    store
        .set_thread_owner(thread.id, Some(owner.id))
        .await
        .expect("owner");
    store
        .assign_thread(thread.id, assignee.id)
        .await
        .expect("assign");

    // Vacuous: no row → not required, green, landable.
    let s = store.get_soundcheck_standing(thread.id).await.unwrap();
    assert!(!s.required);
    assert!(s.pointer.is_none());
    assert_eq!(s.land, LandColor::Green);
    assert!(s.landable);

    // Require arms a pending row.
    let s = store.require_soundcheck(thread.id).await.unwrap();
    assert!(s.required);
    assert!(s.pointer.is_none());
    assert_eq!(s.land, LandColor::Red);
    assert!(!s.landable);
    // Idempotent — does not wipe a later pointer (checked after set).
    store.require_soundcheck(thread.id).await.unwrap();

    // Unskilled recorder is rejected; pending row stays.
    let denied = store
        .set_soundcheck_pointer(thread.id, unskilled.id, SoundcheckStatus::Pass, None, None)
        .await;
    assert!(
        matches!(denied, Err(StoreError::InvalidInput(ref m)) if m.contains("soundcheck skill")),
        "unskilled write must be InvalidInput, got {denied:?}"
    );
    let s = store.get_soundcheck_standing(thread.id).await.unwrap();
    assert!(s.pointer.is_none());
    assert!(!s.landable);

    // Owner (implementer) pass is stored but not a land.
    let s = store
        .set_soundcheck_pointer(thread.id, owner.id, SoundcheckStatus::Pass, None, None)
        .await
        .unwrap();
    assert_eq!(s.pointer.as_ref().unwrap().status, SoundcheckStatus::Pass);
    assert_eq!(s.pointer.as_ref().unwrap().land, LandColor::Green);
    assert_eq!(s.pointer.as_ref().unwrap().kind, SOUNDCHECK_KIND);
    assert_eq!(s.land, LandColor::Red);
    assert!(!s.landable, "implementer pass is not a land");

    // Amber from a skilled third party is flags-then-still-engages — not a land.
    let s = store
        .set_soundcheck_pointer(
            thread.id,
            checker.id,
            SoundcheckStatus::Pass,
            Some("deadbeef"),
            Some(LandColor::Amber),
        )
        .await
        .unwrap();
    assert_eq!(s.pointer.as_ref().unwrap().land, LandColor::Amber);
    assert_eq!(
        s.pointer.as_ref().unwrap().artifact_sha.as_deref(),
        Some("deadbeef")
    );
    assert_eq!(s.land, LandColor::Amber);
    assert!(!s.landable);
    assert_eq!(s.recorded_by, Some(checker.id));

    // Fail is red. Requested green on a fail is ignored.
    let s = store
        .set_soundcheck_pointer(
            thread.id,
            checker.id,
            SoundcheckStatus::Fail,
            None,
            Some(LandColor::Green),
        )
        .await
        .unwrap();
    assert_eq!(s.pointer.as_ref().unwrap().status, SoundcheckStatus::Fail);
    assert_eq!(s.pointer.as_ref().unwrap().land, LandColor::Red);
    assert_eq!(s.land, LandColor::Red);
    assert!(!s.landable);

    // Require after a pointer leaves the pointer in place.
    let s = store.require_soundcheck(thread.id).await.unwrap();
    assert!(s.pointer.is_some());
    assert_eq!(s.land, LandColor::Red);

    // Qualifying green pass from the soundcheck agent.
    let s = store
        .set_soundcheck_pointer(thread.id, checker.id, SoundcheckStatus::Pass, None, None)
        .await
        .unwrap();
    assert_eq!(s.land, LandColor::Green);
    assert!(s.landable);
    assert_eq!(s.recorded_by, Some(checker.id));

    // Assignee (implementer) pass is not a land even with the skill.
    store
        .add_member_skill(assignee.id, SOUNDCHECK_SKILL)
        .await
        .unwrap();
    let s = store
        .set_soundcheck_pointer(thread.id, assignee.id, SoundcheckStatus::Pass, None, None)
        .await
        .unwrap();
    assert_eq!(s.land, LandColor::Red);
    assert!(!s.landable);

    // Clear disarms.
    assert!(store.clear_soundcheck(thread.id).await.unwrap());
    let s = store.get_soundcheck_standing(thread.id).await.unwrap();
    assert!(!s.required);
    assert!(s.landable);
    assert!(!store.clear_soundcheck(thread.id).await.unwrap());
}

#[tokio::test]
async fn soundcheck_pointer_require_pass_amber_and_sod_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn soundcheck_pointer_require_pass_amber_and_sod_postgres() {
    use maidan_store::{run_postgres_migrations, PostgresStore};
    use sqlx::postgres::PgPoolOptions;
    use std::time::Duration as StdDuration;
    use testcontainers::{runners::AsyncRunner, ImageExt};
    use testcontainers_modules::postgres::Postgres;

    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            eprintln!("skipping: docker unavailable ({err})");
            return;
        }
    };
    let host = container.get_host().await.expect("host");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(StdDuration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    let store = PostgresStore::new(pool);
    run_suite(&store).await;
}

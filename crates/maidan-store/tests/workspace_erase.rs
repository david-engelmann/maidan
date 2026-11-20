//! Workspace full erasure deletes the workspace row, on both backends, and
//! also when the workspace has done work: a thread's history names members
//! through keys that do not cascade, and on Postgres those failed the erase.

use maidan_auth::hash_secret;
use maidan_fsm::ThreadAction;
use maidan_store::test_support::SetResultClaiming;
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::*;

async fn sqlite_store() -> SqliteStore {
    let pool = maidan_search::sqlite_pool_options()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    SqliteStore::for_tests(pool)
}

/// A workspace with a person, an agent and a task the agent claimed, handed
/// a result in for and sent to review: rows in the thread's history that name
/// both members. Then another workspace beside it, which must survive.
async fn a_worked_workspace_is_erased_and_its_neighbour_is_not(store: &dyn Store) {
    let mut ids = Vec::new();
    for name in ["worked", "neighbour"] {
        let ws = store
            .create_workspace(NewWorkspace { name: name.into() })
            .await
            .expect("workspace");
        let person = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: "person".into(),
                display_name: None,
                kind: MemberKind::Human,
            })
            .await
            .expect("person");
        let agent = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: "agent".into(),
                display_name: None,
                kind: MemberKind::Agent,
            })
            .await
            .expect("agent");
        store
            .create_api_token(NewApiToken {
                workspace_id: ws.id,
                member_id: agent.id,
                app_installation_id: None,
                label: None,
                token_hash: hash_secret(&format!("token-{name}")),
                capabilities: vec!["workspace:read".into()],
                expires_at: None,
            })
            .await
            .expect("token");
        let channel = store
            .create_channel(NewChannel {
                workspace_id: ws.id,
                name: "work".into(),
                topic: None,
                private: false,
            })
            .await
            .expect("channel");
        let thread = store
            .create_thread(NewThread {
                channel_id: channel.id,
                parent_thread_id: None,
                title: Some("a task".into()),
                description: None,
                created_by: Some(person.id),
            })
            .await
            .expect("thread");
        let claimed = store
            .claim_next_thread_with_event(channel.id, agent.id, Some(60), None, None, None, None)
            .await
            .expect("claim")
            .0
            .expect("the open thread");
        assert_eq!(claimed.id, thread.id);
        store
            .set_thread_result_claiming(thread.id, agent.id, &serde_json::json!({"ok": true}))
            .await
            .expect("result");
        store
            .transition_thread(thread.id, agent.id, ThreadAction::StartReview)
            .await
            .expect("start review");
        ids.push((ws.id, thread.id));
    }
    let (worked, worked_thread) = ids[0];
    let (neighbour, neighbour_thread) = ids[1];

    let result = store.erase_workspace(worked).await.expect("erase");
    assert!(result.workspace_erased);
    assert!(matches!(
        store.get_workspace(worked).await,
        Err(StoreError::NotFound)
    ));
    assert!(matches!(
        store.get_thread(worked_thread).await,
        Err(StoreError::NotFound)
    ));

    // The neighbour keeps its workspace, its thread and the thread's state.
    store.get_workspace(neighbour).await.expect("the neighbour");
    let kept = store
        .get_thread(neighbour_thread)
        .await
        .expect("the neighbour's thread");
    assert_eq!(kept.state, ThreadState::InReview);
    assert_eq!(
        store.list_members(neighbour).await.expect("members").len(),
        2
    );
}

#[tokio::test]
async fn a_worked_workspace_is_erased_and_its_neighbour_is_not_sqlite() {
    let store = sqlite_store().await;
    a_worked_workspace_is_erased_and_its_neighbour_is_not(&store).await;
}

#[tokio::test]
async fn a_worked_workspace_is_erased_and_its_neighbour_is_not_postgres() {
    use maidan_store::{run_postgres_migrations, PostgresStore};
    use sqlx::postgres::PgPoolOptions;
    use std::time::Duration;
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
            maidan_store::test_support::docker::skip_start_failure(err).await;
            return;
        }
    };
    let host = container.get_host().await.expect("host");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");

    // The erase removes what names a member through a key that does not
    // cascade, and the purge removes what names a content or workspace key.
    // A key like that on any other table would fail an erase the same way,
    // in whichever order Postgres ran the cascade: give it ON DELETE CASCADE
    // or SET NULL, or teach the erase about it and add its parent here.
    let unhandled: Vec<String> = sqlx::query_scalar(
        "SELECT c.conrelid::regclass::text || '.' || c.conname
         FROM pg_constraint c
         WHERE c.contype = 'f'
           AND c.confdeltype IN ('a', 'r')
           AND c.conparentid = 0
           AND (c.confrelid::regclass::text NOT IN (
                  'maidan_members', 'maidan_delegation_grants',
                  'maidan_content_keys', 'maidan_workspace_keys')
                OR array_length(c.conkey, 1) <> 1)
         ORDER BY 1",
    )
    .fetch_all(&pool)
    .await
    .expect("catalog");
    assert!(
        unhandled.is_empty(),
        "foreign keys a workspace erase does not handle: {unhandled:?}"
    );

    let store = PostgresStore::for_tests(pool);
    a_worked_workspace_is_erased_and_its_neighbour_is_not(&store).await;
}

#[tokio::test]
async fn erase_workspace_removes_row_and_peers() {
    let store = sqlite_store().await;
    let ws = store
        .create_workspace(NewWorkspace {
            name: "erase-me".into(),
        })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "bot".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: member.id,
            app_installation_id: None,
            label: None,
            token_hash: hash_secret("test-token"),
            capabilities: vec!["workspace:read".into()],
            expires_at: None,
        })
        .await
        .unwrap();
    let peer = store
        .create_peer(NewPeer {
            workspace_id: ws.id,
            remote_workspace_id: ws.id,
            name: "peer".into(),
            base_url: "https://peer.example".into(),
            token_hash: "a".repeat(64),
            outbound_secret_ciphertext: None,
        })
        .await
        .unwrap();

    let result = store.erase_workspace(ws.id).await.unwrap();
    assert!(result.workspace_erased);
    assert!(result.purge.api_tokens_revoked >= 1);

    assert!(matches!(
        store.get_workspace(ws.id).await,
        Err(StoreError::NotFound)
    ));
    assert!(matches!(
        store.get_peer(peer.id).await,
        Err(StoreError::NotFound)
    ));
}

#[tokio::test]
async fn erase_unknown_workspace_is_not_found() {
    let store = sqlite_store().await;
    assert!(matches!(
        store
            .erase_workspace(WorkspaceId(uuid::Uuid::new_v4()))
            .await,
        Err(StoreError::NotFound)
    ));
}

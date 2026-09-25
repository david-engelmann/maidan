//! What a withdrawal leaves behind, with and without a legal hold. Both
//! backends.
//!
//! - Without a hold, withdrawing (tombstoning) a message forgets its earlier
//!   versions too, so the withdrawal withdraws.
//! - Under a hold, the member's view is the same, but the store keeps the
//!   message's last words and earlier versions, readable only through the
//!   audited preserved read, which records itself first or returns nothing.
//! - Lifting the hold disposes of what it kept, and its audit row says how much.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    EditMessage, MemberKind, MessageId, NewAuditEvent, NewChannel, NewMember, NewMessage,
    NewThread, NewWorkspace, ThreadId,
};
use sqlx::sqlite::SqlitePoolOptions;

fn event(action: &str) -> NewAuditEvent {
    NewAuditEvent {
        actor_id: None,
        action: action.into(),
        target_kind: None,
        target_id: None,
        metadata: serde_json::json!({}),
    }
}

/// Post a message, edit it once, withdraw it. Returns its id.
async fn say_then_withdraw(
    store: &dyn Store,
    thread: ThreadId,
    author: maidan_types::MemberId,
    first: &str,
    last: &str,
) -> MessageId {
    let message = store
        .post_message(NewMessage {
            thread_id: thread,
            author_id: author,
            body: first.into(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .unwrap();
    store
        .edit_message(
            message.id,
            author,
            EditMessage {
                body: last.into(),
                metadata: serde_json::json!({}),
                content: None,
            },
        )
        .await
        .unwrap();
    store.tombstone_message(message.id).await.unwrap();
    message.id
}

async fn run_suite<F, Fut>(store: &dyn Store, break_audit: F)
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let ws = store
        .create_workspace(NewWorkspace {
            name: "held".into(),
        })
        .await
        .unwrap()
        .id;
    let author = store
        .create_member(NewMember {
            workspace_id: ws,
            handle: "author".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap()
        .id;
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: "c".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: None,
        })
        .await
        .unwrap()
        .id;

    // No hold: the withdrawal takes the history with it.
    let forgotten = say_then_withdraw(store, thread, author, "draft", "final").await;
    assert!(store.get_message(forgotten).await.unwrap().body.is_empty());
    assert!(
        store
            .list_message_edits(forgotten, 50)
            .await
            .unwrap()
            .is_empty(),
        "an unheld withdrawal left its earlier versions behind"
    );

    // Under a hold, the member sees the same, and the store keeps the words.
    let first = store
        .place_legal_hold_audited(ws, "matter 1", None, Box::new(|_| event("place")))
        .await
        .unwrap();
    let second = store
        .place_legal_hold_audited(ws, "matter 2", None, Box::new(|_| event("place")))
        .await
        .unwrap();
    assert_ne!(first.id, second.id, "a second matter is a second hold");
    assert_eq!(store.list_workspace_legal_holds(ws).await.unwrap().len(), 2);
    let kept = say_then_withdraw(store, thread, author, "what I said", "what I meant").await;
    assert!(
        store.get_message(kept).await.unwrap().body.is_empty(),
        "the live row is blank"
    );
    let preserved = store
        .read_preserved_messages_audited(ws, event("legal_hold.preserved_read"))
        .await
        .unwrap();
    assert_eq!(preserved.len(), 1, "only what was withdrawn under the hold");
    let p = &preserved[0];
    assert_eq!(p.message_id, kept);
    assert_eq!(p.author_id, author);
    assert_eq!(p.body, "what I meant");
    assert_eq!(p.edits.len(), 1);
    assert_eq!(p.edits[0].body_before, "what I said");
    assert!(store
        .list_audit_for_workspace(ws, 50)
        .await
        .unwrap()
        .iter()
        .chain(store.list_audit(200).await.unwrap().iter())
        .any(|a| a.action == "legal_hold.preserved_read"));

    // Lifting one matter's hold releases nothing while another stands.
    assert!(store
        .lift_legal_hold_audited(ws, first.id, event("legal_hold.lift.first"))
        .await
        .unwrap());
    assert_eq!(
        store
            .read_preserved_messages_audited(ws, event("legal_hold.preserved_read"))
            .await
            .unwrap()
            .len(),
        1,
        "the other matter still holds the words"
    );
    assert!(!store
        .lift_legal_hold_audited(ws, first.id, event("legal_hold.lift.again"))
        .await
        .unwrap());

    // Lifting the last disposes of it, and says so.
    assert!(store
        .lift_legal_hold_audited(ws, second.id, event("legal_hold.lift"))
        .await
        .unwrap());
    assert!(store
        .read_preserved_messages_audited(ws, event("legal_hold.preserved_read"))
        .await
        .unwrap()
        .is_empty());
    assert!(store.list_message_edits(kept, 50).await.unwrap().is_empty());
    let lift = store
        .list_audit(200)
        .await
        .unwrap()
        .into_iter()
        .find(|a| a.action == "legal_hold.lift")
        .expect("the lift is recorded");
    assert_eq!(lift.metadata["disposed"]["withdrawn_messages"], 1);
    assert_eq!(lift.metadata["disposed"]["edit_versions"], 1);

    // A preserved read that cannot be recorded returns nothing.
    store
        .place_legal_hold_audited(ws, "matter 3", None, Box::new(|_| event("place")))
        .await
        .unwrap();
    say_then_withdraw(store, thread, author, "again", "again, edited").await;
    break_audit().await;
    assert!(store
        .read_preserved_messages_audited(ws, event("legal_hold.preserved_read"))
        .await
        .is_err());
}

#[tokio::test]
async fn a_hold_keeps_what_is_withdrawn_sqlite() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store = SqliteStore::new(pool.clone());
    run_suite(&store, || async {
        sqlx::query(
            "CREATE TRIGGER audit_down BEFORE INSERT ON maidan_audit
             BEGIN SELECT RAISE(ABORT, 'audit down'); END",
        )
        .execute(&pool)
        .await
        .unwrap();
    })
    .await;
}

#[tokio::test]
async fn a_hold_keeps_what_is_withdrawn_postgres() {
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
        Ok(container) => container,
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
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    let store = PostgresStore::new(pool.clone());
    run_suite(&store, || async {
        sqlx::query(
            "CREATE FUNCTION audit_down() RETURNS trigger AS $$
             BEGIN RAISE EXCEPTION 'audit down'; END $$ LANGUAGE plpgsql",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TRIGGER audit_down BEFORE INSERT ON maidan_audit
             FOR EACH ROW EXECUTE FUNCTION audit_down()",
        )
        .execute(&pool)
        .await
        .unwrap();
    })
    .await;
}

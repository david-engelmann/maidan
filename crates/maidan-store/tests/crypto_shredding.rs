//! Crypto-shredding on both backends: message words are sealed before they
//! are hashed, a withdrawal destroys the message's key, and the chain still
//! verifies over the ciphertext. Also the per-workspace artifact erase, whose
//! bytes go only with the last reference, and never under an upload of the
//! same bytes. Postgres runs under testcontainers (skipped without Docker).

use std::sync::Arc;
use std::time::Duration;

use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::*;
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::{PgPool, SqlitePool};
use testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt};
use testcontainers_modules::postgres::Postgres;
use uuid::Uuid;

const WORDS: &str = "the launch code is 0000";

#[derive(Clone)]
enum Db {
    Sqlite(SqlitePool),
    Pg(PgPool),
}

impl Db {
    fn store(&self, keys: Arc<ContentKeyring>) -> Arc<dyn Store> {
        match self {
            Db::Sqlite(pool) => Arc::new(SqliteStore::new(pool.clone(), keys)),
            Db::Pg(pool) => Arc::new(PostgresStore::new(pool.clone(), keys)),
        }
    }

    /// `SELECT COUNT(*) ...` with one uuid parameter, written with `?`.
    async fn count(&self, sql: &str, param: Uuid) -> i64 {
        match self {
            Db::Sqlite(pool) => sqlx::query_scalar(sql)
                .bind(param)
                .fetch_one(pool)
                .await
                .unwrap(),
            Db::Pg(pool) => sqlx::query_scalar(&sql.replace('?', "$1"))
                .bind(param)
                .fetch_one(pool)
                .await
                .unwrap(),
        }
    }

    async fn count_by_log_id(&self, sql: &str, log_id: i64) -> i64 {
        match self {
            Db::Sqlite(pool) => sqlx::query_scalar(sql)
                .bind(log_id)
                .fetch_one(pool)
                .await
                .unwrap(),
            Db::Pg(pool) => sqlx::query_scalar(&sql.replace('?', "$1"))
                .bind(log_id)
                .fetch_one(pool)
                .await
                .unwrap(),
        }
    }
}

async fn sqlite() -> Db {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    maidan_store::configure_sqlite_pool(&pool).await.unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    Db::Sqlite(pool)
}

async fn postgres() -> Option<(ContainerAsync<Postgres>, Db)> {
    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            eprintln!("skipping: docker unavailable ({err})");
            return None;
        }
    };
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&format!(
            "postgres://postgres:postgres@{host}:{port}/postgres"
        ))
        .await
        .unwrap();
    run_postgres_migrations(&pool).await.unwrap();
    Some((container, Db::Pg(pool)))
}

fn kek(byte: u8) -> Arc<ContentKeyring> {
    Arc::new(ContentKeyring::new([byte; 32], Vec::new()))
}

struct Room {
    ws: Workspace,
    alice: Member,
    thread: Thread,
}

async fn room(store: &dyn Store, name: &str) -> Room {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let alice = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "alice".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "general".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("t".into()),
        })
        .await
        .unwrap();
    Room { ws, alice, thread }
}

async fn post(store: &dyn Store, room: &Room, body: &str) -> (Message, StoredEvent) {
    store
        .post_message_with_event(
            NewMessage {
                thread_id: room.thread.id,
                author_id: room.alice.id,
                body: body.into(),
                metadata: serde_json::json!({"note": format!("meta {body}")}),
                content: None,
            },
            None,
        )
        .await
        .unwrap()
}

fn message_of(event: &Event) -> &Message {
    match event {
        Event::MessagePosted { message, .. } | Event::MessageEdited { message, .. } => message,
        other => panic!("not a message event: {other:?}"),
    }
}

/// Every form of the event a reader could be handed.
fn every_wire_form(event: &StoredEvent) -> String {
    format!(
        "{}{}{}",
        event.payload,
        serde_json::to_string(event).unwrap(),
        serde_json::to_string(&KeyedEvent(event.clone())).unwrap()
    )
}

async fn words_are_sealed_before_hashing_and_open_with_the_live_key(db: Db) {
    let store = db.store(kek(1));
    let room = room(store.as_ref(), "seal").await;
    let (message, appended) = post(store.as_ref(), &room, WORDS).await;

    // What the log holds, and what the hash covers, is ciphertext.
    let stored = store.get_stored_event(appended.id).await.unwrap();
    assert!(!stored.payload.to_string().contains(WORDS));
    assert!(!stored.payload.to_string().contains("meta "));
    assert_eq!(stored.payload["message"]["body"], "");
    assert!(stored.payload.get("sealed").is_some());
    assert!(!stored.is_shredded());
    // The default wire form carries no key; a whole-log reader's does.
    assert!(!serde_json::to_string(&stored)
        .unwrap()
        .contains("content_key"));
    assert!(serde_json::to_string(&KeyedEvent(stored.clone()))
        .unwrap()
        .contains("content_key"));

    let opened = stored.opened_event().unwrap();
    assert_eq!(message_of(&opened).body, WORDS);
    assert_eq!(
        message_of(&opened).metadata["note"],
        format!("meta {WORDS}")
    );
    assert_eq!(message_of(&opened).id, message.id);

    // An edit reuses the message's key.
    let (_, edited) = store
        .edit_message_with_event(
            message.id,
            room.alice.id,
            EditMessage {
                body: "edited words".into(),
                metadata: serde_json::json!({}),
                content: None,
            },
            None,
        )
        .await
        .unwrap();
    let edited = store.get_stored_event(edited.id).await.unwrap();
    assert!(!edited.payload.to_string().contains("edited words"));
    assert_eq!(edited.content_key, stored.content_key);
    assert_eq!(
        message_of(&edited.opened_event().unwrap()).body,
        "edited words"
    );

    assert!(store.verify_event_chain(room.ws.id).await.unwrap().ok);
}

async fn withdrawal_shreds_the_key_and_the_chain_still_verifies(db: Db) {
    let store = db.store(kek(1));
    let room = room(store.as_ref(), "shred").await;
    let (message, posted) = post(store.as_ref(), &room, WORDS).await;
    let (keep, kept) = post(store.as_ref(), &room, "kept words").await;

    // Queued copies of the words that must not outlive the key.
    let sub = store
        .create_webhook_subscription(NewWebhookSubscription {
            workspace_id: room.ws.id,
            url: "https://hooks.example/x".into(),
            label: None,
            event_kinds: vec![],
            secret_ciphertext: "c".into(),
        })
        .await
        .unwrap();
    store
        .enqueue_webhook_delivery(sub.id, posted.id, WORDS)
        .await
        .unwrap();
    store
        .enqueue_webhook_delivery(sub.id, kept.id, "kept words")
        .await
        .unwrap();

    store
        .tombstone_message_with_event(message.id, None)
        .await
        .unwrap();

    assert_eq!(
        db.count(
            "SELECT COUNT(*) FROM maidan_content_keys
             WHERE id = ? AND wrapped_key IS NULL AND kek_id IS NULL AND shredded_at IS NOT NULL",
            message.id.0,
        )
        .await,
        1
    );
    let shredded = store.get_stored_event(posted.id).await.unwrap();
    assert!(shredded.is_shredded());
    assert!(shredded.content_key.is_none());
    assert!(!every_wire_form(&shredded).contains(WORDS));
    // Opening a shredded event yields the redacted state, not an error.
    let redacted = shredded.opened_event().unwrap();
    assert_eq!(message_of(&redacted).body, "");
    assert!(matches!(
        redacted,
        Event::MessagePosted {
            sealed: Some(_),
            ..
        }
    ));
    // The log is unchanged, so its hashes still verify.
    let report = store.verify_event_chain(room.ws.id).await.unwrap();
    assert!(report.ok, "{report:?}");

    let delivery_sql = "SELECT COUNT(*) FROM maidan_webhook_deliveries WHERE log_id = ?";
    assert_eq!(db.count_by_log_id(delivery_sql, posted.id).await, 0);
    assert_eq!(db.count_by_log_id(delivery_sql, kept.id).await, 1);

    // Other messages keep their words.
    let kept = store.get_stored_event(kept.id).await.unwrap();
    assert_eq!(message_of(&kept.opened_event().unwrap()).body, "kept words");
    assert_eq!(store.get_message(keep.id).await.unwrap().body, "kept words");

    // A shredded subject stays shredded: a later event about it is sealed
    // under a throwaway key.
    let late = store
        .append_event(&Event::MessageEdited {
            occurred_at: chrono::Utc::now(),
            workspace_id: room.ws.id,
            channel_id: room.thread.channel_id,
            thread_id: room.thread.id,
            dm_conversation_id: None,
            editor_id: room.alice.id,
            message: Message {
                body: "resurrected words".into(),
                ..store.get_message(message.id).await.unwrap()
            },
            sealed: None,
        })
        .await
        .unwrap();
    let late = store.get_stored_event(late.id).await.unwrap();
    assert!(late.is_shredded());
    assert!(!every_wire_form(&late).contains("resurrected words"));
    assert!(store.verify_event_chain(room.ws.id).await.unwrap().ok);
}

async fn a_plain_tombstone_shreds_too(db: Db) {
    let store = db.store(kek(1));
    let room = room(store.as_ref(), "plain").await;
    let (message, posted) = post(store.as_ref(), &room, WORDS).await;
    store.tombstone_message(message.id).await.unwrap();
    assert!(store
        .get_stored_event(posted.id)
        .await
        .unwrap()
        .is_shredded());
}

async fn federated_subjects_belong_to_their_origin(db: Db) {
    let store = db.store(kek(1));
    let room = room(store.as_ref(), "federated").await;
    let message = store
        .post_message(NewMessage {
            thread_id: room.thread.id,
            author_id: room.alice.id,
            body: WORDS.into(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .unwrap();
    let posted = |message: Message| Event::MessagePosted {
        occurred_at: chrono::Utc::now(),
        workspace_id: room.ws.id,
        channel_id: room.thread.channel_id,
        thread_id: room.thread.id,
        dm_conversation_id: None,
        message,
        sealed: None,
    };
    let tombstoned = |message_id| Event::MessageTombstoned {
        occurred_at: chrono::Utc::now(),
        workspace_id: room.ws.id,
        channel_id: room.thread.channel_id,
        thread_id: room.thread.id,
        dm_conversation_id: None,
        message_id,
    };
    let origin = PeerId(Uuid::now_v7());
    let other = PeerId(Uuid::now_v7());
    let from_origin = store
        .append_federated_event(&posted(message.clone()), origin)
        .await
        .unwrap();
    let local = store.append_event(&posted(message.clone())).await.unwrap();

    // Another peer naming the same message id cannot shred the origin's words,
    // and neither can a local withdrawal.
    store
        .append_federated_event(&tombstoned(message.id), other)
        .await
        .unwrap();
    store.append_event(&tombstoned(message.id)).await.unwrap();
    assert!(store
        .get_stored_event(local.id)
        .await
        .unwrap()
        .is_shredded());
    let still = store.get_stored_event(from_origin.id).await.unwrap();
    assert!(!still.is_shredded());
    assert_eq!(message_of(&still.opened_event().unwrap()).body, WORDS);

    // The origin's own tombstone does.
    store
        .append_federated_event(&tombstoned(message.id), origin)
        .await
        .unwrap();
    assert!(store
        .get_stored_event(from_origin.id)
        .await
        .unwrap()
        .is_shredded());
    assert!(store.verify_event_chain(room.ws.id).await.unwrap().ok);
}

async fn an_event_the_origin_shredded_arrives_as_ciphertext(db: Db) {
    let store = db.store(kek(1));
    let room = room(store.as_ref(), "arrived").await;
    let (message, posted) = post(store.as_ref(), &room, WORDS).await;
    store
        .tombstone_message_with_event(message.id, None)
        .await
        .unwrap();
    // What a peer receives for it: sealed, no key.
    let shipped = store.get_stored_event(posted.id).await.unwrap();
    let event = shipped.opened_event().unwrap();

    let origin = PeerId(Uuid::now_v7());
    let ingested = store.append_federated_event(&event, origin).await.unwrap();
    let ingested = store.get_stored_event(ingested.id).await.unwrap();
    assert!(ingested.is_shredded());
    assert_eq!(ingested.payload["sealed"], shipped.payload["sealed"]);
    assert!(!every_wire_form(&ingested).contains(WORDS));
    assert_eq!(
        db.count(
            "SELECT COUNT(*) FROM maidan_content_keys WHERE id = ? AND shredded_at IS NOT NULL",
            content_subject(message.id, Some(origin)),
        )
        .await,
        1
    );
}

async fn rotation_rewraps_under_the_new_kek(db: Db) {
    let old = db.store(kek(1));
    let room = room(old.as_ref(), "rotate").await;
    let (_, posted) = post(old.as_ref(), &room, WORDS).await;

    // An unrelated KEK is an error, never mistaken for a shredded key.
    let stranger = db.store(kek(9));
    assert!(matches!(
        stranger.get_stored_event(posted.id).await,
        Err(StoreError::ContentKey(_))
    ));

    let rotated = db.store(Arc::new(ContentKeyring::new([2; 32], vec![[1; 32]])));
    assert_eq!(rotated.content_keys_needing_rewrap().await.unwrap(), 1);
    let read = rotated.get_stored_event(posted.id).await.unwrap();
    assert_eq!(message_of(&read.opened_event().unwrap()).body, WORDS);
    assert_eq!(rotated.rewrap_content_keys(100).await.unwrap(), 1);
    assert_eq!(rotated.rewrap_content_keys(100).await.unwrap(), 0);
    assert_eq!(rotated.content_keys_needing_rewrap().await.unwrap(), 0);

    // The old KEK can be dropped from configuration now.
    let new_only = db.store(kek(2));
    let read = new_only.get_stored_event(posted.id).await.unwrap();
    assert_eq!(message_of(&read.opened_event().unwrap()).body, WORDS);
    assert!(matches!(
        old.get_stored_event(posted.id).await,
        Err(StoreError::ContentKey(_))
    ));
}

/// A store seals under the keyring it was built with. Nothing falls back to
/// the public development KEK, so that keyring opens none of it.
async fn a_store_seals_under_the_keyring_it_is_given(db: Db) {
    let store = db.store(kek(3));
    let room = room(store.as_ref(), "given").await;
    let (_, posted) = post(store.as_ref(), &room, WORDS).await;

    let read = store.get_stored_event(posted.id).await.unwrap();
    assert_eq!(message_of(&read.opened_event().unwrap()).body, WORDS);
    let dev = db.store(Arc::new(ContentKeyring::insecure_dev()));
    assert!(matches!(
        dev.get_stored_event(posted.id).await,
        Err(StoreError::ContentKey(_))
    ));
}

async fn workspace_purge_destroys_its_content_keys(db: Db) {
    let store = db.store(kek(1));
    let room = room(store.as_ref(), "purge").await;
    let other = self::room(store.as_ref(), "bystander").await;
    post(store.as_ref(), &room, WORDS).await;
    post(store.as_ref(), &room, "more").await;
    let (_, bystander) = post(store.as_ref(), &other, "bystander words").await;

    let result = store.purge_workspace_messages(room.ws.id).await.unwrap();
    assert_eq!(result.content_keys_destroyed, 2);
    let keys_sql = "SELECT COUNT(*) FROM maidan_content_keys WHERE workspace_id = ?";
    assert_eq!(db.count(keys_sql, room.ws.id.0).await, 0);
    assert_eq!(db.count(keys_sql, other.ws.id.0).await, 1);
    let bystander = store.get_stored_event(bystander.id).await.unwrap();
    assert_eq!(
        message_of(&bystander.opened_event().unwrap()).body,
        "bystander words"
    );
}

fn erase_audit(erasure: &ArtifactErasure) -> NewAuditEvent {
    NewAuditEvent {
        actor_id: None,
        action: "artifact.erase".into(),
        target_kind: Some("workspace".into()),
        target_id: Some(erasure.workspace_id.0),
        metadata: serde_json::json!({"last_reference": erasure.last_reference}),
    }
}

async fn a_shared_artifact_goes_with_its_last_reference(db: Db) {
    let store = db.store(kek(1));
    let a = room(store.as_ref(), "a").await;
    let b = room(store.as_ref(), "b").await;
    let sha = "ab".repeat(32);
    for (room, who) in [(&a, a.alice.id), (&b, b.alice.id)] {
        store
            .upsert_artifact_with_event(
                NewArtifact {
                    sha256: sha.clone(),
                    size_bytes: 4,
                    mime_type: Some("text/plain".into()),
                    kind: ArtifactKind::Attachment,
                    uploaded_by: Some(who),
                },
                Some(room.ws.id),
            )
            .await
            .unwrap();
    }

    let first = store
        .erase_artifact_audited(a.ws.id, &sha, Box::new(erase_audit))
        .await
        .unwrap();
    assert!(!first.last_reference);
    assert!(!store.artifact_ref_exists(a.ws.id, &sha).await.unwrap());
    assert!(matches!(
        store.get_artifact_for_workspace(a.ws.id, &sha).await,
        Err(StoreError::NotFound)
    ));
    assert!(store
        .get_artifact_for_workspace(b.ws.id, &sha)
        .await
        .is_ok());
    assert!(matches!(
        store
            .erase_artifact_audited(a.ws.id, &sha, Box::new(erase_audit))
            .await,
        Err(StoreError::NotFound)
    ));

    // A held workspace keeps its reference.
    let hold = store
        .place_legal_hold(b.ws.id, "matter", None)
        .await
        .unwrap();
    assert!(matches!(
        store
            .erase_artifact_audited(b.ws.id, &sha, Box::new(erase_audit))
            .await,
        Err(StoreError::Conflict(_))
    ));
    store.lift_legal_hold(b.ws.id, hold.id).await.unwrap();

    let last = store
        .erase_artifact_audited(b.ws.id, &sha, Box::new(erase_audit))
        .await
        .unwrap();
    assert!(last.last_reference);
    assert!(matches!(
        store.get_artifact_by_sha(&sha).await,
        Err(StoreError::NotFound)
    ));
    let audits = store.list_audit_for_workspace(b.ws.id, 10).await.unwrap();
    assert!(audits.iter().any(|e| e.action == "artifact.erase"));
}

async fn enqueue_mail_about(
    store: &dyn Store,
    workspace_id: WorkspaceId,
    source_log_id: Option<i64>,
) -> Option<MailOutboxId> {
    store
        .enqueue_mail(NewMailOutbox {
            workspace_id: Some(workspace_id),
            source_log_id,
            to_address: "alice@example.test".into(),
            subject: "New Maidan notification".into(),
            body: format!("about {source_log_id:?}"),
        })
        .await
        .unwrap()
}

async fn withdrawal_takes_its_notification_mail(db: Db) {
    let store = db.store(kek(1));
    let room = room(store.as_ref(), "mail").await;
    let (message, posted) = post(store.as_ref(), &room, WORDS).await;
    let (_, kept) = post(store.as_ref(), &room, "kept words").await;
    let about = |log_id: Option<i64>| enqueue_mail_about(store.as_ref(), room.ws.id, log_id);
    // Pending, dead-lettered and sent copies about the message.
    about(Some(posted.id)).await.expect("queued");
    let dead = about(Some(posted.id)).await.expect("queued");
    store
        .mark_mail_failed(dead, "smtp down", None)
        .await
        .unwrap();
    let sent = about(Some(posted.id)).await.expect("queued");
    store.mark_mail_delivered(sent).await.unwrap();
    // Mail about another message, and mail about no message at all.
    about(Some(kept.id)).await.expect("queued");
    about(None).await.expect("queued");
    assert_eq!(store.count_dead_mail().await.unwrap(), 1);

    let linked = "SELECT COUNT(*) FROM maidan_mail_outbox WHERE content_key_id = ?";
    let in_workspace = "SELECT COUNT(*) FROM maidan_mail_outbox WHERE workspace_id = ?";
    assert_eq!(db.count(linked, message.id.0).await, 3);

    store
        .tombstone_message_with_event(message.id, None)
        .await
        .unwrap();
    assert_eq!(db.count(linked, message.id.0).await, 0);
    assert_eq!(db.count(in_workspace, room.ws.id.0).await, 2);
    assert_eq!(store.count_dead_mail().await.unwrap(), 0);
    // A notification about a message withdrawn first is never queued.
    assert!(about(Some(posted.id)).await.is_none());
    assert_eq!(db.count(in_workspace, room.ws.id.0).await, 2);

    // Workspace purge destroys the keys, and the mail linked to them with it.
    store.purge_workspace_messages(room.ws.id).await.unwrap();
    assert_eq!(db.count(in_workspace, room.ws.id.0).await, 1);
}

/// Record, rather than perform, a blob delete; optionally park inside it until
/// released.
fn recording_delete(
    ran: Arc<std::sync::atomic::AtomicBool>,
    entered: Option<tokio::sync::oneshot::Sender<()>>,
    release: Option<tokio::sync::oneshot::Receiver<()>>,
) -> BlobDelete<'static> {
    Box::new(move || {
        Box::pin(async move {
            ran.store(true, std::sync::atomic::Ordering::SeqCst);
            if let Some(entered) = entered {
                let _ = entered.send(());
            }
            if let Some(release) = release {
                let _ = release.await;
            }
            Ok(())
        })
    })
}

fn upload(sha: &str, by: MemberId) -> NewArtifact {
    NewArtifact {
        sha256: sha.into(),
        size_bytes: 4,
        mime_type: None,
        kind: ArtifactKind::Attachment,
        uploaded_by: Some(by),
    }
}

/// The bytes of an orphaned sha are deleted under its lock: an upload of the
/// same bytes that arrives during the delete waits, and commits its row only
/// after it (and then restores the bytes, `restore_if_reaped`). A sha a row
/// holds again is never deleted.
async fn an_upload_waits_for_a_blob_reap_in_progress(db: Db) {
    use std::sync::atomic::{AtomicBool, Ordering};
    let store = db.store(kek(1));
    let room = room(store.as_ref(), "reap").await;
    let sha = "cd".repeat(32);

    let ran = Arc::new(AtomicBool::new(false));
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let reaper = {
        let (store, sha, ran) = (store.clone(), sha.clone(), ran.clone());
        tokio::spawn(async move {
            store
                .reap_artifact_blob(
                    &sha,
                    recording_delete(ran, Some(entered_tx), Some(release_rx)),
                )
                .await
        })
    };
    entered_rx.await.unwrap();
    let uploader = {
        let (store, sha, who, ws) = (store.clone(), sha.clone(), room.alice.id, room.ws.id);
        tokio::spawn(async move {
            store
                .upsert_artifact_with_event(upload(&sha, who), Some(ws))
                .await
        })
    };
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        !uploader.is_finished(),
        "an upload of the same bytes must wait for the delete"
    );
    release_tx.send(()).unwrap();
    assert_eq!(reaper.await.unwrap().unwrap(), BlobReap::Deleted);
    assert!(ran.load(Ordering::SeqCst));
    uploader.await.unwrap().unwrap();

    // The row exists again, so a later reap keeps the bytes.
    let ran = Arc::new(AtomicBool::new(false));
    let kept = store
        .reap_artifact_blob(&sha, recording_delete(ran.clone(), None, None))
        .await
        .unwrap();
    assert_eq!(kept, BlobReap::Referenced);
    assert!(!ran.load(Ordering::SeqCst), "a held sha is never deleted");

    // After the last reference goes, the reap deletes.
    let erased = store
        .erase_artifact_audited(room.ws.id, &sha, Box::new(erase_audit))
        .await
        .unwrap();
    assert!(erased.last_reference);
    let ran = Arc::new(AtomicBool::new(false));
    let reaped = store
        .reap_artifact_blob(&sha, recording_delete(ran.clone(), None, None))
        .await
        .unwrap();
    assert_eq!(reaped, BlobReap::Deleted);
    assert!(ran.load(Ordering::SeqCst));
}

impl Db {
    async fn residue(&self, workspace: Option<WorkspaceId>) -> Vec<(Uuid, String)> {
        let found = match self {
            Db::Sqlite(pool) => maidan_store::find_shred_residue_sqlite(pool, workspace).await,
            Db::Pg(pool) => maidan_store::find_shred_residue_postgres(pool, workspace).await,
        };
        let mut found: Vec<_> = found
            .unwrap()
            .into_iter()
            .map(|r| (r.subject, r.table))
            .collect();
        found.sort();
        found
    }

    /// Run a statement with uuid parameters, written with `?`.
    async fn execute(&self, sql: &str, params: &[Uuid]) {
        match self {
            Db::Sqlite(pool) => {
                let mut query = sqlx::query(sql);
                for param in params {
                    query = query.bind(*param);
                }
                query.execute(pool).await.map(|_| ())
            }
            Db::Pg(pool) => {
                let mut numbered = sql.to_string();
                for n in 1..=params.len() {
                    numbered = numbered.replacen('?', &format!("${n}"), 1);
                }
                let mut query = sqlx::query(&numbered);
                for param in params {
                    query = query.bind(*param);
                }
                query.execute(pool).await.map(|_| ())
            }
        }
        .unwrap();
    }
}

async fn the_residue_check_finds_words_a_withdrawal_left(db: Db) {
    let store = db.store(kek(1));
    let other = room(store.as_ref(), "residue-other").await;
    let room = room(store.as_ref(), "residue").await;
    let (message, _) = post(store.as_ref(), &room, WORDS).await;
    store
        .edit_message_with_event(
            message.id,
            room.alice.id,
            EditMessage {
                body: "second words".into(),
                metadata: serde_json::json!({}),
                content: None,
            },
            None,
        )
        .await
        .unwrap();
    post(store.as_ref(), &room, "live words").await;
    store
        .tombstone_message_with_event(message.id, None)
        .await
        .unwrap();
    // A withdrawal leaves nothing behind, and live messages are not residue.
    assert_eq!(db.residue(Some(room.ws.id)).await, vec![]);

    // Copies that escaped the withdrawal: words put back on the row, an
    // earlier version, a vector computed from the words and a log payload
    // written unsealed.
    let subject = message.id.0;
    db.execute(
        "UPDATE maidan_messages SET body = 'leftover' WHERE id = ?",
        &[subject],
    )
    .await;
    db.execute(
        "INSERT INTO maidan_message_edits (message_id, editor_id, body_before, body_after)
         VALUES (?, ?, 'first words', 'second words')",
        &[subject, room.alice.id.0],
    )
    .await;
    let vector = match &db {
        Db::Sqlite(_) => "x'00'",
        Db::Pg(_) => "array_fill(0, ARRAY[1024])::vector",
    };
    db.execute(
        &format!(
            "INSERT INTO maidan_emb_hash_v1 (message_id, embedding)
             VALUES (?, {vector})"
        ),
        &[subject],
    )
    .await;
    let unsealed = match &db {
        Db::Sqlite(_) => "json_set(payload, '$.message.body', 'leftover')",
        Db::Pg(_) => "jsonb_set(payload, '{message,body}', '\"leftover\"')",
    };
    db.execute(
        &format!(
            "UPDATE maidan_events SET payload = {unsealed}
             WHERE id = (SELECT MIN(id) FROM maidan_events WHERE content_key_id = ?)"
        ),
        &[subject],
    )
    .await;
    let table = |name: &str| (subject, name.to_string());
    assert_eq!(
        db.residue(Some(room.ws.id)).await,
        vec![
            table("maidan_emb_hash_v1"),
            table("maidan_events"),
            table("maidan_message_edits"),
            table("maidan_messages"),
        ]
    );

    // Scoped to one workspace, or across all of them.
    let (elsewhere, _) = post(store.as_ref(), &other, WORDS).await;
    store.tombstone_message(elsewhere.id).await.unwrap();
    db.execute(
        "UPDATE maidan_messages SET body = 'leftover' WHERE id = ?",
        &[elsewhere.id.0],
    )
    .await;
    assert_eq!(
        db.residue(Some(other.ws.id)).await,
        vec![(elsewhere.id.0, "maidan_messages".to_string())]
    );
    assert_eq!(db.residue(None).await.len(), 5);

    // Under a legal hold the earlier versions stay on purpose.
    db.execute(
        "INSERT INTO maidan_preserved_messages (message_id, workspace_id, body, tombstoned_at)
         VALUES (?, ?, 'first words', CURRENT_TIMESTAMP)",
        &[subject, room.ws.id.0],
    )
    .await;
    assert_eq!(
        db.residue(Some(room.ws.id)).await,
        vec![
            table("maidan_emb_hash_v1"),
            table("maidan_events"),
            table("maidan_messages"),
        ]
    );
}

macro_rules! on_both_backends {
    ($($name:ident),* $(,)?) => {
        mod sqlite_backend {
            $(
                #[tokio::test]
                async fn $name() {
                    super::$name(super::sqlite().await).await;
                }
            )*
        }
        mod postgres_backend {
            $(
                #[tokio::test]
                async fn $name() {
                    let Some((_container, db)) = super::postgres().await else {
                        return;
                    };
                    super::$name(db).await;
                }
            )*
        }
    };
}

on_both_backends!(
    words_are_sealed_before_hashing_and_open_with_the_live_key,
    withdrawal_shreds_the_key_and_the_chain_still_verifies,
    a_plain_tombstone_shreds_too,
    federated_subjects_belong_to_their_origin,
    an_event_the_origin_shredded_arrives_as_ciphertext,
    rotation_rewraps_under_the_new_kek,
    a_store_seals_under_the_keyring_it_is_given,
    workspace_purge_destroys_its_content_keys,
    a_shared_artifact_goes_with_its_last_reference,
    withdrawal_takes_its_notification_mail,
    the_residue_check_finds_words_a_withdrawal_left,
);

/// The reap race needs a second connection that can be blocked, so SQLite runs
/// on a file here, not a single-connection memory database.
mod blob_reap {
    #[tokio::test]
    async fn sqlite_backend() {
        let dir = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}?mode=rwc", dir.path().join("reap.db").display());
        let pool = super::SqlitePoolOptions::new()
            .max_connections(4)
            .connect(&url)
            .await
            .unwrap();
        maidan_store::configure_sqlite_pool(&pool).await.unwrap();
        super::run_sqlite_migrations(&pool).await.unwrap();
        super::an_upload_waits_for_a_blob_reap_in_progress(super::Db::Sqlite(pool)).await;
    }

    #[tokio::test]
    async fn postgres_backend() {
        let Some((_container, db)) = super::postgres().await else {
            return;
        };
        super::an_upload_waits_for_a_blob_reap_in_progress(db).await;
    }
}

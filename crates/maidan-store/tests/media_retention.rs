//! Media retention on both backends: a workspace's media ref expires once it
//! is past the TTL and nothing in that workspace references it, through the
//! same audited erase as `DELETE /artifacts/{sha}`. A thread link or a live
//! message naming the sha keeps it, evidence kinds are never expired, a held
//! workspace keeps everything, and another workspace's ref to the same bytes
//! is its own. The media quota's usage counts each media sha once.

use chrono::Utc;
use maidan_store::media_retention::expire_media_artifacts;
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ArtifactKind, AuditScope, Member, MemberKind, NewArtifact, NewAuditEvent, NewChannel,
    NewMember, NewMessage, NewThread, NewWorkspace, RetentionDays, ThreadId, WorkspaceId,
};
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;

/// The instance's media TTL in these suites, in days.
const INSTANCE_DAYS: i64 = 5;

struct Room {
    workspace: WorkspaceId,
    thread: ThreadId,
    member: Member,
}

async fn room(store: &dyn Store, name: &str) -> Room {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .expect("workspace");
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "agent".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("member");
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "general".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("channel");
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("work".into()),
            description: None,
        })
        .await
        .expect("thread");
    Room {
        workspace: ws.id,
        thread: thread.id,
        member,
    }
}

fn sha(n: u32) -> String {
    format!("{n:064x}")
}

async fn upload(store: &dyn Store, room: &Room, sha256: &str, kind: ArtifactKind, size: i64) {
    store
        .upsert_artifact_with_event(
            NewArtifact {
                sha256: sha256.into(),
                size_bytes: size,
                mime_type: None,
                filename: None,
                kind,
                uploaded_by: Some(room.member.id),
            },
            Some(room.workspace),
        )
        .await
        .expect("upload");
}

async fn post(
    store: &dyn Store,
    room: &Room,
    metadata: serde_json::Value,
) -> maidan_types::MessageId {
    store
        .post_message(NewMessage {
            thread_id: room.thread,
            author_id: room.member.id,
            body: "see attached".into(),
            metadata,
            content: None,
        })
        .await
        .expect("message")
        .id
}

fn policy_audit(workspace_id: WorkspaceId) -> maidan_store::AuditFor<RetentionDays> {
    Box::new(move |days| NewAuditEvent {
        scope: AuditScope::Workspace(workspace_id),
        actor_id: None,
        action: "retention_policy.set".into(),
        target_kind: Some("workspace".into()),
        target_id: Some(workspace_id.0),
        metadata: serde_json::to_value(days).expect("days"),
    })
}

enum Db {
    Sqlite(sqlx::SqlitePool),
    Postgres(sqlx::PgPool),
}

impl Db {
    /// Move every ref to `sha256` back `days` days, in the database's clock.
    async fn age(&self, sha256: &str, days: i64) {
        match self {
            Db::Sqlite(pool) => {
                sqlx::query(
                    "UPDATE maidan_artifact_refs
                     SET created_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?)
                     WHERE sha256 = ?",
                )
                .bind(format!("-{days} days"))
                .bind(sha256)
                .execute(pool)
                .await
                .expect("age sqlite");
            }
            Db::Postgres(pool) => {
                sqlx::query(
                    "UPDATE maidan_artifact_refs
                     SET created_at = NOW() - make_interval(days => $1)
                     WHERE sha256 = $2",
                )
                .bind(days as i32)
                .bind(sha256)
                .execute(pool)
                .await
                .expect("age postgres");
            }
        }
    }
}

async fn holds(store: &dyn Store, workspace_id: WorkspaceId, sha256: &str) -> bool {
    store
        .artifact_ref_exists(workspace_id, sha256)
        .await
        .expect("ref exists")
}

async fn run_expiry_suite(store: &dyn Store, db: &Db) {
    let a = room(store, "a").await;
    let b = room(store, "b").await;
    let held = room(store, "held").await;
    let own = room(store, "own-ttl").await;

    let shared = sha(1);
    let linked = sha(2);
    let in_object = sha(3);
    let in_scalar = sha(4);
    let in_bare = sha(5);
    let withdrawn = sha(6);
    let transcript = sha(7);
    let young = sha(8);
    let held_sha = sha(9);
    let own_sha = sha(10);
    let fresh = sha(11);

    // Both workspaces hold the same bytes; only B links them on a thread.
    upload(store, &a, &shared, ArtifactKind::Attachment, 10).await;
    upload(store, &b, &shared, ArtifactKind::Attachment, 10).await;
    store
        .link_thread_artifact(b.thread, &shared, b.member.id)
        .await
        .expect("link in b");

    upload(store, &a, &linked, ArtifactKind::Screenshot, 10).await;
    store
        .link_thread_artifact(a.thread, &linked, a.member.id)
        .await
        .expect("link in a");

    upload(store, &a, &in_object, ArtifactKind::Screenshot, 10).await;
    upload(store, &a, &in_scalar, ArtifactKind::Recording, 10).await;
    upload(store, &a, &in_bare, ArtifactKind::Attachment, 10).await;
    post(
        store,
        &a,
        json!({"artifacts": [{"sha256": in_object, "name": "shot.png"}]}),
    )
    .await;
    post(store, &a, json!({"artifact_sha256": in_scalar})).await;
    post(store, &a, json!({"artifacts": [in_bare]})).await;

    upload(store, &a, &withdrawn, ArtifactKind::Attachment, 10).await;
    let withdrawn_msg = post(store, &a, json!({"sha256": withdrawn})).await;
    store
        .tombstone_message(withdrawn_msg)
        .await
        .expect("withdraw");

    upload(store, &a, &transcript, ArtifactKind::Transcript, 10).await;
    upload(store, &a, &young, ArtifactKind::Attachment, 10).await;
    upload(store, &held, &held_sha, ArtifactKind::Attachment, 10).await;
    upload(store, &own, &own_sha, ArtifactKind::Attachment, 10).await;
    upload(store, &a, &fresh, ArtifactKind::Attachment, 10).await;

    for old in [
        &shared,
        &linked,
        &in_object,
        &in_scalar,
        &in_bare,
        &withdrawn,
        &transcript,
        &held_sha,
    ] {
        db.age(old, 10).await;
    }
    // Inside the instance's TTL, past the workspace's own.
    db.age(&young, 3).await;
    db.age(&own_sha, 3).await;

    store
        .place_legal_hold(held.workspace, "matter", None)
        .await
        .expect("hold");
    let instance = RetentionDays {
        media_days: Some(INSTANCE_DAYS),
        ..RetentionDays::default()
    };
    store
        .set_retention_policy_audited(
            own.workspace,
            RetentionDays {
                media_days: Some(1),
                ..RetentionDays::default()
            },
            instance,
            policy_audit(own.workspace),
        )
        .await
        .expect("own media_days");

    let mut erased: Vec<(WorkspaceId, String, bool)> =
        expire_media_artifacts(store, Utc::now(), Some(INSTANCE_DAYS), 100)
            .await
            .expect("expire")
            .into_iter()
            .map(|e| (e.workspace_id, e.sha256, e.last_reference))
            .collect();
    erased.sort_by(|x, y| x.1.cmp(&y.1));
    assert_eq!(
        erased,
        vec![
            (a.workspace, shared.clone(), false),
            (a.workspace, withdrawn.clone(), true),
            (own.workspace, own_sha.clone(), true),
        ],
        "only unreferenced media past its workspace's TTL expires"
    );

    assert!(!holds(store, a.workspace, &shared).await);
    assert!(
        holds(store, b.workspace, &shared).await,
        "B's ref to the same bytes is B's, and B's thread links it"
    );
    assert!(
        store.get_artifact_by_sha(&shared).await.is_ok(),
        "the bytes stay while B references them"
    );
    for (sha256, why) in [
        (&linked, "a thread link keeps it"),
        (
            &in_object,
            "an artifacts array with a {sha256} object keeps it",
        ),
        (&in_scalar, "a scalar artifact_sha256 keeps it"),
        (&in_bare, "an artifacts array with a bare sha keeps it"),
        (&transcript, "a transcript is evidence, never expired"),
        (&young, "inside the instance's TTL"),
        (&fresh, "a ref newer than the TTL stays"),
    ] {
        assert!(holds(store, a.workspace, sha256).await, "{why}");
    }
    assert!(
        !holds(store, a.workspace, &withdrawn).await,
        "a withdrawn message no longer protects its sha"
    );
    assert!(
        holds(store, held.workspace, &held_sha).await,
        "a held workspace keeps everything"
    );
    assert!(
        !holds(store, own.workspace, &own_sha).await,
        "the workspace's own media_days overrides the instance's"
    );

    let expiries: Vec<_> = store
        .list_audit_for_workspace(a.workspace, 100)
        .await
        .expect("audit")
        .into_iter()
        .filter(|row| row.action == "artifact.expire")
        .collect();
    assert_eq!(expiries.len(), 2, "one audit row per expired ref");
    assert!(
        expiries.iter().all(|row| row.actor_id.is_none()),
        "a system action has no actor"
    );
    assert!(expiries
        .iter()
        .any(|row| row.metadata["sha256"] == json!(shared)
            && row.metadata["last_reference"] == json!(false)));

    // A referenced ref is not erased even when asked for directly, and the
    // next sweep finds nothing left to do.
    let cutoff = Utc::now() - chrono::Duration::days(INSTANCE_DAYS);
    assert!(store
        .expire_media_artifact_audited(
            a.workspace,
            &in_object,
            cutoff,
            Box::new(|_: &maidan_types::ArtifactErasure| unreachable_audit()),
        )
        .await
        .expect("recheck")
        .is_none());
    assert!(
        expire_media_artifacts(store, Utc::now(), Some(INSTANCE_DAYS), 100)
            .await
            .expect("again")
            .is_empty()
    );
    // With expiry off on the instance and no workspace setting, nothing goes.
    assert!(expire_media_artifacts(store, Utc::now(), None, 100)
        .await
        .expect("off")
        .is_empty());
}

fn unreachable_audit() -> NewAuditEvent {
    panic!("a referenced ref is not erased, so it writes no audit row")
}

async fn run_usage_suite(store: &dyn Store) {
    let a = room(store, "usage-a").await;
    let b = room(store, "usage-b").await;
    assert_eq!(
        store.media_artifact_bytes(a.workspace).await.expect("used"),
        0
    );

    upload(store, &a, &sha(1), ArtifactKind::Screenshot, 100).await;
    upload(store, &a, &sha(1), ArtifactKind::Screenshot, 100).await;
    upload(store, &a, &sha(2), ArtifactKind::Recording, 1_000).await;
    upload(store, &a, &sha(3), ArtifactKind::Transcript, 50_000).await;
    upload(store, &b, &sha(1), ArtifactKind::Attachment, 100).await;
    upload(store, &b, &sha(4), ArtifactKind::Attachment, 7).await;

    assert_eq!(
        store.media_artifact_bytes(a.workspace).await.expect("used"),
        1_100,
        "each media sha once; the transcript is not media"
    );
    assert_eq!(
        store.media_artifact_bytes(b.workspace).await.expect("used"),
        107,
        "another workspace's usage is its own"
    );
}

async fn sqlite() -> (SqliteStore, sqlx::SqlitePool) {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    (SqliteStore::for_tests(pool.clone()), pool)
}

async fn postgres() -> Option<(
    testcontainers::ContainerAsync<testcontainers_modules::postgres::Postgres>,
    maidan_store::PostgresStore,
    sqlx::PgPool,
)> {
    use maidan_store::{run_postgres_migrations, PostgresStore};
    use sqlx::postgres::PgPoolOptions;
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
            return None;
        }
    };
    let host = container.get_host().await.expect("host");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(std::time::Duration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    Some((container, PostgresStore::for_tests(pool.clone()), pool))
}

#[tokio::test]
async fn unreferenced_media_past_its_ttl_expires_and_everything_else_stays_sqlite() {
    let (store, pool) = sqlite().await;
    run_expiry_suite(&store, &Db::Sqlite(pool)).await;
}

#[tokio::test]
async fn unreferenced_media_past_its_ttl_expires_and_everything_else_stays_postgres() {
    let Some((_container, store, pool)) = postgres().await else {
        return;
    };
    run_expiry_suite(&store, &Db::Postgres(pool)).await;
}

#[tokio::test]
async fn media_usage_counts_each_media_sha_once_per_workspace_sqlite() {
    let (store, _pool) = sqlite().await;
    run_usage_suite(&store).await;
}

#[tokio::test]
async fn media_usage_counts_each_media_sha_once_per_workspace_postgres() {
    let Some((_container, store, _pool)) = postgres().await else {
        return;
    };
    run_usage_suite(&store).await;
}

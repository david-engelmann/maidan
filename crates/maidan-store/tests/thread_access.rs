//! The listings that filter by thread access in the query (`readable_by` on
//! A2A tasks and pending gates) agree with the per-thread rule,
//! `maidan_auth::can_access_thread`, on both backends: for every member and
//! every kind of thread, a row is listed and counted exactly when the rule
//! lets that member read its thread. Pages hold `limit` readable rows however
//! many hidden ones sit between them.

use std::collections::{BTreeSet, HashMap};

use chrono::{Duration, TimeZone, Utc};
use maidan_auth::AuthContext;
use maidan_store::{
    prelude::*, run_sqlite_migrations, A2aTaskQuery, A2aTaskWrite, PendingGateQuery,
};
use maidan_types::{
    ApiTokenId, ChannelMemberRole, MemberId, MemberKind, NewApprovalGate, NewChannel, NewMember,
    NewThread, NewWorkspace, ThreadId, WorkspaceId,
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
    SqliteStore::for_tests(pool)
}

async fn member(store: &dyn Store, workspace_id: WorkspaceId, handle: &str) -> MemberId {
    store
        .create_member(NewMember {
            workspace_id,
            handle: handle.into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("member")
        .id
}

async fn thread_in(
    store: &dyn Store,
    workspace_id: WorkspaceId,
    name: &str,
    private: bool,
) -> ThreadId {
    let channel = store
        .create_channel(NewChannel {
            workspace_id,
            name: name.into(),
            topic: None,
            private,
        })
        .await
        .expect("channel");
    store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: None,
            description: None,
        })
        .await
        .expect("thread")
        .id
}

fn auth(workspace_id: WorkspaceId, member_id: MemberId) -> AuthContext {
    AuthContext::from_token(
        ApiTokenId(uuid::Uuid::now_v7()),
        member_id,
        workspace_id,
        vec![],
    )
}

struct Seeder<'a> {
    store: &'a dyn Store,
    minute: i64,
}

impl Seeder<'_> {
    /// A task on `thread_id`, one minute after the last; its id says where.
    async fn task(&mut self, workspace_id: WorkspaceId, thread_id: Option<ThreadId>) -> String {
        self.minute += 1;
        let id = format!("task-{:04}", self.minute);
        let status_at =
            Utc.with_ymd_and_hms(2026, 9, 30, 12, 0, 0).unwrap() + Duration::minutes(self.minute);
        self.store
            .upsert_a2a_task(A2aTaskWrite {
                workspace_id,
                task_id: &id,
                context_id: None,
                thread_id,
                state: "TASK_STATE_COMPLETED",
                status_at,
                task_json: serde_json::json!({ "id": id }),
            })
            .await
            .expect("upsert");
        id
    }
}

fn query(readable_by: Option<MemberId>) -> A2aTaskQuery<'static> {
    A2aTaskQuery {
        limit: 1000,
        readable_by,
        ..Default::default()
    }
}

async fn run_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace { name: "ws".into() })
        .await
        .expect("workspace")
        .id;
    let other = store
        .create_workspace(NewWorkspace {
            name: "other".into(),
        })
        .await
        .expect("other workspace")
        .id;
    let alice = member(store, ws, "alice").await;
    let bob = member(store, ws, "bob").await;
    let carol = member(store, ws, "carol").await;
    let erin = member(store, ws, "erin").await;
    let dave = member(store, other, "dave").await;

    let open = thread_in(store, ws, "general", false).await;
    let private = thread_in(store, ws, "secret", true).await;
    let secret = store.get_thread(private).await.expect("thread").channel_id;
    store
        .add_channel_member(secret, alice, ChannelMemberRole::Member)
        .await
        .expect("join");
    let dm = store
        .open_dm_conversation(ws, alice, bob)
        .await
        .expect("dm")
        .thread_id;
    let group = store
        .open_group_dm_conversation(ws, &[alice, carol, erin], None)
        .await
        .expect("group dm")
        .thread_id;
    let foreign = thread_in(store, other, "general", false).await;

    let threads = [
        ("open", Some(open)),
        ("private", Some(private)),
        ("dm", Some(dm)),
        ("group", Some(group)),
        ("foreign", Some(foreign)),
        ("none", None),
    ];
    let mut seed = Seeder { store, minute: 0 };
    let mut task_thread = HashMap::new();
    let mut gate_thread = HashMap::new();
    for (name, thread_id) in threads {
        task_thread.insert(seed.task(ws, thread_id).await, name);
        let gate = store
            .create_approval_gate(&NewApprovalGate {
                workspace_id: ws,
                thread_id,
                requested_by: alice,
                prompt: "ok?".into(),
                schema: None,
            })
            .await
            .expect("gate");
        gate_thread.insert(gate.id, name);
    }
    // Another tenant's task on its own open thread.
    let theirs = seed.task(other, Some(foreign)).await;

    for (who, member_id) in [
        ("alice", alice),
        ("bob", bob),
        ("carol", carol),
        ("erin", erin),
    ] {
        let mut expected = BTreeSet::new();
        for (name, thread_id) in threads {
            let readable = match thread_id {
                None => true,
                Some(thread_id) => {
                    maidan_auth::can_access_thread(store, &auth(ws, member_id), thread_id)
                        .await
                        .expect("rule")
                }
            };
            if readable {
                expected.insert(name);
            }
        }
        let tasks: BTreeSet<_> = store
            .list_a2a_tasks(ws, query(Some(member_id)))
            .await
            .expect("list")
            .iter()
            .map(|row| task_thread[row.id.as_str()])
            .collect();
        assert_eq!(tasks, expected, "{who}'s tasks follow the thread rule");
        let count = store
            .count_a2a_tasks(ws, query(Some(member_id)))
            .await
            .expect("count");
        assert_eq!(count, expected.len() as i64, "{who}'s task count");

        let gate_query = PendingGateQuery {
            limit: 1000,
            readable_by: Some(member_id),
            ..Default::default()
        };
        let gates: BTreeSet<_> = store
            .page_pending_approval_gates(ws, gate_query.clone())
            .await
            .expect("gates")
            .iter()
            .map(|gate| gate_thread[&gate.id])
            .collect();
        assert_eq!(gates, expected, "{who}'s gates follow the thread rule");
        let count = store
            .count_pending_approval_gates(ws, gate_query)
            .await
            .expect("gate count");
        assert_eq!(count, expected.len() as i64, "{who}'s gate count");
    }
    // Sanity on the fixture: each member sees something the others do not.
    let listed = |member_id| async move {
        store
            .list_a2a_tasks(ws, query(Some(member_id)))
            .await
            .expect("list")
            .len()
    };
    assert_eq!(listed(alice).await, 5, "all but the foreign thread");
    assert_eq!(listed(bob).await, 3, "open, dm, none");
    assert_eq!(listed(carol).await, 3, "open, group, none");

    // No reader (auth bypass) keeps every row of the workspace, and only it.
    let all = store.list_a2a_tasks(ws, query(None)).await.expect("all");
    assert_eq!(all.len(), threads.len());
    assert!(all.iter().all(|row| row.workspace_id == ws));
    let tenant = store
        .list_a2a_tasks(other, query(Some(dave)))
        .await
        .expect("other tenant");
    assert_eq!(
        tenant.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        [theirs.as_str()],
        "a tenant lists its own tasks, never another's"
    );
    let row = store
        .get_a2a_task(&theirs)
        .await
        .expect("get")
        .expect("row");
    assert_eq!(row.thread_id, Some(foreign), "the row keeps its thread");

    // Bob can read one task in forty: a page of two is two of his, and the
    // walk ends exactly after his last.
    let mut bobs = Vec::new();
    for i in 0..40 {
        let thread_id = if i % 10 == 3 {
            Some(open)
        } else {
            Some(private)
        };
        let id = seed.task(ws, thread_id).await;
        if thread_id == Some(open) {
            bobs.push(id);
        }
    }
    let mut expected: Vec<String> = store
        .list_a2a_tasks(ws, query(Some(bob)))
        .await
        .expect("bob")
        .into_iter()
        .map(|row| row.id)
        .collect();
    assert_eq!(expected.len(), 3 + bobs.len());
    expected.sort_by(|a, b| b.cmp(a));
    let mut walked = Vec::new();
    let mut cursor: Option<(chrono::DateTime<Utc>, String)> = None;
    loop {
        let page = store
            .list_a2a_tasks(
                ws,
                A2aTaskQuery {
                    before: cursor.as_ref().map(|(at, id)| (*at, id.as_str())),
                    limit: 2,
                    readable_by: Some(bob),
                    ..Default::default()
                },
            )
            .await
            .expect("page");
        let full = page.len() == 2;
        cursor = page.last().map(|row| (row.updated_at, row.id.clone()));
        walked.extend(page.into_iter().map(|row| row.id));
        if !full {
            break;
        }
    }
    assert_eq!(walked, expected, "every readable task once, newest first");
}

#[tokio::test]
async fn listings_filter_by_thread_access_in_the_query_sqlite() {
    run_suite(&sqlite().await).await;
}

#[tokio::test]
async fn listings_filter_by_thread_access_in_the_query_postgres() {
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
            return;
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
    run_suite(&PostgresStore::for_tests(pool)).await;
}

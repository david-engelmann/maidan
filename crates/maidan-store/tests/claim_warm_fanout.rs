//! Siblings that share a prompt prefix warm, then fan out (Context Economics
//! C5). `claim_next` hands out one sibling first and holds the rest back until
//! that claim reports usage, which means the provider has written the shared
//! prefix, so the rest read it instead of each writing it cold. Siblings share
//! a prefix when they are one recipe run's children or carry the same task
//! fingerprint; other children of a parent, and unrelated work, are never held.
//! The hold lasts at most 120s, ends when the first claim is released or its
//! lease lapses, and a sibling usage report within 5 minutes skips it. Every
//! case runs on both backends and both claim paths.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ChannelId, MemberId, MemberKind, NewChannel, NewMember, NewThread, NewUsageLedgerEntry,
    NewWorkspace, PriceSnapshot, Thread, ThreadId, TokenUsage, WorkspaceId,
};
use sqlx::sqlite::SqlitePoolOptions;
use uuid::Uuid;

/// Raw access to the store's database, to backdate rows and add the recipe
/// run and fingerprint rows that mark a group.
#[derive(Clone)]
enum Db {
    Pg(sqlx::PgPool),
    Lite(sqlx::SqlitePool),
}

impl Db {
    /// Run `sql`, written with `$n` placeholders, binding `ids` in order.
    async fn exec(&self, sql: &str, ids: &[Uuid]) {
        match self {
            Db::Pg(pool) => {
                let mut q = sqlx::query(sql);
                for id in ids {
                    q = q.bind(*id);
                }
                q.execute(pool).await.expect(sql);
            }
            Db::Lite(pool) => {
                let sql = sql.replace('$', "?");
                let mut q = sqlx::query(&sql);
                for id in ids {
                    q = q.bind(*id);
                }
                q.execute(pool).await.expect(&sql);
            }
        }
    }

    /// A SQL time `secs` seconds ago.
    fn ago(&self, secs: i64) -> String {
        match self {
            Db::Pg(_) => format!("(NOW() - make_interval(secs => {secs}))"),
            Db::Lite(_) => {
                format!("strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now', '-{secs} seconds')")
            }
        }
    }
}

async fn sqlite() -> (SqliteStore, Db) {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    (SqliteStore::for_tests(pool.clone()), Db::Lite(pool))
}

/// How the siblings in a world are grouped.
#[derive(Clone, Copy, PartialEq)]
enum Group {
    /// Children of one recipe run.
    Recipe,
    /// Children with one task fingerprint.
    Fingerprint,
    /// Plain children of one parent (plan, review, manual sub-threads).
    None,
}

struct World {
    ws: WorkspaceId,
    channel: ChannelId,
    members: Vec<MemberId>,
    parent: Thread,
    children: Vec<Thread>,
    with_event: bool,
}

async fn world(store: &dyn Store, db: &Db, group: Group, with_event: bool) -> World {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "fanout".into(),
        })
        .await
        .expect("ws");
    let mut members = Vec::new();
    for handle in ["m0", "m1", "m2", "m3", "m4", "m5"] {
        members.push(
            store
                .create_member(NewMember {
                    workspace_id: ws.id,
                    handle: handle.into(),
                    display_name: None,
                    kind: MemberKind::Agent,
                })
                .await
                .expect("member")
                .id,
        );
    }
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "fanout".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("channel");
    let new_thread = |title: &'static str, parent| NewThread {
        channel_id: channel.id,
        parent_thread_id: parent,
        title: Some(title.into()),
        description: None,
        created_by: None,
    };
    let parent = store
        .create_thread(new_thread("parent", None))
        .await
        .expect("parent");
    let mut children = Vec::new();
    for title in ["child 1", "child 2", "child 3"] {
        children.push(
            store
                .create_thread(new_thread(title, Some(parent.id)))
                .await
                .expect("child"),
        );
    }
    let now = db.ago(0);
    match group {
        Group::Recipe => {
            let recipe = Uuid::new_v4();
            db.exec(
                &format!(
                    "INSERT INTO maidan_recipes (id, workspace_id, channel_id, name, spec, created_by, created_at, updated_at)
                     VALUES ($1, $2, $3, 'r', '{{}}', $4, {now}, {now})"
                ),
                &[recipe, ws.id.0, channel.id.0, members[0].0],
            )
            .await;
            db.exec(
                &format!(
                    "INSERT INTO maidan_recipe_runs (id, recipe_id, workspace_id, root_thread_id, params, spec_snapshot, created_by, created_at)
                     VALUES ($1, $2, $3, $4, '{{}}', '{{}}', $5, {now})"
                ),
                &[Uuid::new_v4(), recipe, ws.id.0, parent.id.0, members[0].0],
            )
            .await;
        }
        Group::Fingerprint => {
            // Creating a thread stores its fingerprint; give the children one.
            let fingerprint = "a".repeat(64);
            for child in &children {
                db.exec(
                    &format!(
                        "UPDATE maidan_task_fingerprints SET fingerprint = '{fingerprint}' WHERE thread_id = $1"
                    ),
                    &[child.id.0],
                )
                .await;
            }
        }
        Group::None => {}
    }
    World {
        ws: ws.id,
        channel: channel.id,
        members,
        parent,
        children,
        with_event,
    }
}

impl World {
    async fn claim(&self, store: &dyn Store, member: usize) -> Option<Thread> {
        let member = self.members[member];
        if self.with_event {
            store
                .claim_next_thread_with_event(self.channel, member, Some(60), None, None)
                .await
                .expect("claim")
                .0
        } else {
            store
                .claim_next_thread(self.channel, member, Some(60), None)
                .await
                .expect("claim")
        }
    }

    /// Claim the parent, then the first child: the start of every case.
    async fn claim_first_child(&self, store: &dyn Store) -> Thread {
        let p = self.claim(store, 0).await.expect("parent claim");
        assert_eq!(p.id, self.parent.id);
        let first = self.claim(store, 1).await.expect("first child");
        assert_eq!(first.id, self.children[0].id);
        first
    }

    async fn report_usage(&self, store: &dyn Store, thread: &Thread, member: usize) {
        let _ = self.ws;
        store
            .report_accounted_usage(&NewUsageLedgerEntry {
                usage_report_id: Uuid::new_v4(),
                thread_id: thread.id,
                reporter: self.members[member],
                claim_lease_id: thread.claim_lease_id.expect("leased"),
                model: "provider/model".into(),
                tokens: TokenUsage {
                    input: 1_000_000,
                    ..Default::default()
                },
                usd_micros: 10,
                price_snapshot: PriceSnapshot {
                    input_usd_micros_per_million: 10,
                    ..Default::default()
                },
                turns: 1,
                evidence: Default::default(),
            })
            .await
            .expect("usage");
    }
}

fn is_child(world: &World, thread: &Option<Thread>) -> bool {
    thread
        .as_ref()
        .is_some_and(|t| world.children.iter().any(|c| c.id == t.id))
}

async fn set_thread(db: &Db, thread: ThreadId, set: &str) {
    db.exec(
        &format!("UPDATE maidan_threads SET {set} WHERE id = $1"),
        &[thread.0],
    )
    .await;
}

async fn run_suite(store: &dyn Store, db: &Db) {
    for with_event in [false, true] {
        // A recipe's children warm, then fan out; unrelated work does not wait.
        for group in [Group::Recipe, Group::Fingerprint] {
            let w = world(store, db, group, with_event).await;
            let loner = store
                .create_thread(NewThread {
                    channel_id: w.channel,
                    parent_thread_id: None,
                    title: Some("unrelated".into()),
                    description: None,
                    created_by: None,
                })
                .await
                .expect("loner");
            let first = w.claim_first_child(store).await;
            let b = w.claim(store, 2).await.expect("loner claim");
            assert_eq!(b.id, loner.id, "siblings wait; other work does not");
            assert!(
                w.claim(store, 3).await.is_none(),
                "no sibling fans out before the first reports usage"
            );
            w.report_usage(store, &first, 1).await;
            assert_eq!(
                w.claim(store, 3).await.expect("fan out").id,
                w.children[1].id
            );
            assert_eq!(
                w.claim(store, 4).await.expect("fan out").id,
                w.children[2].id
            );
        }

        // Plain children of one parent share no prefix: none is held.
        let w = world(store, db, Group::None, with_event).await;
        w.claim_first_child(store).await;
        assert_eq!(
            w.claim(store, 2).await.expect("not held").id,
            w.children[1].id
        );
        assert_eq!(
            w.claim(store, 3).await.expect("not held").id,
            w.children[2].id
        );

        // The hold is capped at 120s: a silent first claim stops holding.
        let w = world(store, db, Group::Recipe, with_event).await;
        let first = w.claim_first_child(store).await;
        set_thread(db, first.id, &format!("claimed_at = {}", db.ago(100))).await;
        assert!(w.claim(store, 2).await.is_none(), "held within 120s");
        set_thread(db, first.id, &format!("claimed_at = {}", db.ago(121))).await;
        assert_eq!(w.claim(store, 2).await.expect("cap").id, w.children[1].id);

        // Releasing the first child ends the hold.
        let w = world(store, db, Group::Recipe, with_event).await;
        let first = w.claim_first_child(store).await;
        assert!(w.claim(store, 2).await.is_none(), "held");
        set_thread(
            db,
            first.id,
            "assignee_id = NULL, assignment_expires_at = NULL, claim_lease_id = NULL, claimed_at = NULL",
        )
        .await;
        assert!(is_child(&w, &w.claim(store, 2).await), "released: not held");

        // The first child's lease lapsing ends the hold.
        let w = world(store, db, Group::Recipe, with_event).await;
        let first = w.claim_first_child(store).await;
        assert!(w.claim(store, 2).await.is_none(), "held");
        set_thread(
            db,
            first.id,
            &format!("assignment_expires_at = {}", db.ago(5)),
        )
        .await;
        assert!(is_child(&w, &w.claim(store, 2).await), "lapsed: not held");

        // A sibling's usage report under 5 minutes old skips the hold; one
        // over 5 minutes old no longer does, so the hold applies again.
        let w = world(store, db, Group::Recipe, with_event).await;
        let first = w.claim_first_child(store).await;
        w.report_usage(store, &first, 1).await;
        db.exec(
            &format!(
                "UPDATE maidan_usage_ledger SET accepted_at = {} WHERE thread_id = $1",
                db.ago(400)
            ),
            &[first.id.0],
        )
        .await;
        // The first claim has reported, so it holds nobody: the second goes.
        let second = w.claim(store, 2).await.expect("second");
        assert_eq!(second.id, w.children[1].id);
        assert!(
            w.claim(store, 3).await.is_none(),
            "a report over 5 minutes old: the fresh second claim holds"
        );
        db.exec(
            &format!(
                "UPDATE maidan_usage_ledger SET accepted_at = {} WHERE thread_id = $1",
                db.ago(200)
            ),
            &[first.id.0],
        )
        .await;
        assert_eq!(
            w.claim(store, 3).await.expect("warm").id,
            w.children[2].id,
            "a report under 5 minutes old skips the hold"
        );
    }
}

#[tokio::test]
async fn siblings_warm_then_fan_out_sqlite() {
    let (store, db) = sqlite().await;
    run_suite(&store, &db).await;
}

#[tokio::test]
async fn siblings_warm_then_fan_out_postgres() {
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
    let store = PostgresStore::for_tests(pool.clone());
    run_suite(&store, &Db::Pg(pool)).await;
}

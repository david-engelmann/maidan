//! Claim footprints on both backends: the queue never hands out work whose
//! footprint overlaps a live claim, a footprint set on a claimed thread that
//! would overlap one is refused with the claims it overlaps, footprints free
//! themselves when a claim ends, and nothing crosses a repository or a
//! workspace. Postgres runs under testcontainers (skipped without Docker).

use std::sync::Arc;
use std::time::Duration;

use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::*;
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::SqlitePoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

const REPO: &str = "example/app";

struct Room {
    ws: WorkspaceId,
    channel: ChannelId,
    agents: Vec<MemberId>,
}

async fn room(store: &dyn Store, name: &str, agents: usize) -> Room {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let mut ids = Vec::new();
    for n in 0..agents {
        ids.push(
            store
                .create_member(NewMember {
                    workspace_id: ws.id,
                    handle: format!("agent{n}"),
                    display_name: None,
                    kind: MemberKind::Agent,
                })
                .await
                .unwrap()
                .id,
        );
    }
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    Room {
        ws: ws.id,
        channel: channel.id,
        agents: ids,
    }
}

async fn task(store: &dyn Store, channel: ChannelId, title: &str) -> ThreadId {
    store
        .create_thread(NewThread {
            channel_id: channel,
            parent_thread_id: None,
            title: Some(title.into()),
            description: None,
        })
        .await
        .unwrap()
        .id
}

fn footprint(mode: FootprintMode, items: &[(FootprintKind, &str)]) -> Footprint {
    Footprint {
        repo: REPO.into(),
        mode,
        items: items
            .iter()
            .map(|(kind, value)| FootprintItem {
                kind: *kind,
                value: (*value).into(),
            })
            .collect(),
    }
}

async fn declare(store: &dyn Store, thread: ThreadId, fp: Footprint) {
    let conflicts = store.set_thread_footprint(thread, Some(fp)).await.unwrap();
    assert!(conflicts.is_empty(), "an unclaimed thread always stores");
}

async fn claim(store: &dyn Store, room: &Room, agent: usize) -> Option<Thread> {
    store
        .claim_next_thread(room.channel, room.agents[agent], Some(600))
        .await
        .unwrap()
}

/// Two tasks in one lane: the second waits for the first claim to end, the
/// counts call it blocked meanwhile, and a release frees it.
async fn work_in_a_held_lane_waits_until_the_claim_ends(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "lane", 2).await;
    let first = task(store.as_ref(), room.channel, "first").await;
    let second = task(store.as_ref(), room.channel, "second").await;
    let lane = footprint(FootprintMode::Exclusive, &[(FootprintKind::Lane, "bridge")]);
    declare(store.as_ref(), first, lane.clone()).await;
    declare(store.as_ref(), second, lane).await;

    let held = claim(store.as_ref(), &room, 0).await.unwrap();
    assert_eq!(held.id, first);
    assert!(
        claim(store.as_ref(), &room, 1).await.is_none(),
        "the lane is held, so nothing is handed out"
    );
    let depth = store.channel_queue_depth(room.channel, None).await.unwrap();
    assert_eq!((depth.ready, depth.blocked, depth.assigned), (0, 1, 1));

    let active = store
        .list_active_claims(
            room.ws,
            Some("https://github.com/Example/App.git".into()),
            None,
        )
        .await
        .unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].thread_id, first);
    assert_eq!(active[0].assignee_id, room.agents[0]);
    assert!(active[0].lease_expires_at.is_some());

    store
        .release_claim(first, room.agents[0], held.claim_lease_id.unwrap())
        .await
        .unwrap();
    let next = claim(store.as_ref(), &room, 1).await.unwrap();
    assert!(
        [first, second].contains(&next.id),
        "the release freed the lane"
    );
}

/// A lapsed lease frees the footprint without anyone releasing it.
async fn a_lapsed_lease_frees_its_footprint(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "lapse", 2).await;
    let first = task(store.as_ref(), room.channel, "first").await;
    let second = task(store.as_ref(), room.channel, "second").await;
    let branch = |value| footprint(FootprintMode::Shared, &[(FootprintKind::Branch, value)]);
    declare(store.as_ref(), first, branch("feature/agent-x")).await;
    declare(store.as_ref(), second, branch("feature/agent-x")).await;

    // Branches are exclusive even when both sides say shared.
    let lapsing = store
        .claim_next_thread(room.channel, room.agents[0], Some(1))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(lapsing.id, first);
    assert!(claim(store.as_ref(), &room, 1).await.is_none());
    tokio::time::sleep(Duration::from_millis(2_100)).await;
    assert!(store
        .list_active_claims(room.ws, None, None)
        .await
        .unwrap()
        .is_empty());
    assert!(claim(store.as_ref(), &room, 1).await.is_some());
}

/// Shared work runs side by side; exclusive work does not share; a glob
/// overlaps the paths under it and not its siblings; another repository is
/// another world.
async fn overlap_follows_mode_kind_and_repository(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "overlap", 6).await;
    let held = task(store.as_ref(), room.channel, "held").await;
    declare(
        store.as_ref(),
        held,
        footprint(
            FootprintMode::Exclusive,
            &[
                (FootprintKind::Glob, "src/bridge/**"),
                (FootprintKind::Crate, "core"),
            ],
        ),
    )
    .await;
    assert_eq!(claim(store.as_ref(), &room, 0).await.unwrap().id, held);

    let cases: Vec<(&str, Footprint, bool)> = vec![
        (
            "a path under the held glob",
            footprint(
                FootprintMode::Shared,
                &[(FootprintKind::Path, "src/bridge/x.rs")],
            ),
            false,
        ),
        (
            "a parent directory of the held glob",
            footprint(FootprintMode::Shared, &[(FootprintKind::Glob, "src/**")]),
            false,
        ),
        (
            "the same crate",
            footprint(FootprintMode::Shared, &[(FootprintKind::Crate, "core")]),
            false,
        ),
        (
            "a sibling directory",
            footprint(
                FootprintMode::Exclusive,
                &[(FootprintKind::Path, "src/gateway/y.rs")],
            ),
            true,
        ),
        (
            "another crate",
            footprint(FootprintMode::Exclusive, &[(FootprintKind::Crate, "edge")]),
            true,
        ),
        (
            "the same glob in another repository",
            Footprint {
                repo: "example/other".into(),
                ..footprint(
                    FootprintMode::Exclusive,
                    &[(FootprintKind::Glob, "src/bridge/**")],
                )
            },
            true,
        ),
    ];
    for (n, (what, fp, claimable)) in cases.into_iter().enumerate() {
        let thread = task(store.as_ref(), room.channel, what).await;
        declare(store.as_ref(), thread, fp).await;
        // A deferred thread keeps its footprint and stays deferred, and a
        // claimed one stays held, so neither is handed to a later case.
        let got = claim(store.as_ref(), &room, 1 + n % 5).await;
        assert_eq!(got.map(|t| t.id) == Some(thread), claimable, "{what}");
    }

    // Two shared claims on one lane run side by side.
    let lane = footprint(FootprintMode::Shared, &[(FootprintKind::Lane, "docs")]);
    let a = task(store.as_ref(), room.channel, "docs a").await;
    let b = task(store.as_ref(), room.channel, "docs b").await;
    declare(store.as_ref(), a, lane.clone()).await;
    declare(store.as_ref(), b, lane).await;
    let first = claim(store.as_ref(), &room, 1).await.unwrap();
    let second = claim(store.as_ref(), &room, 2).await.unwrap();
    assert_ne!(first.id, second.id);
}

/// A footprint set after claiming that overlaps another live claim is
/// refused, names that claim and its lease, and leaves the old footprint.
async fn a_footprint_refined_after_claiming_is_refused_on_overlap(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "refine", 2).await;
    let blocker = task(store.as_ref(), room.channel, "blocker").await;
    declare(
        store.as_ref(),
        blocker,
        footprint(
            FootprintMode::Exclusive,
            &[(FootprintKind::Lane, "gateway")],
        ),
    )
    .await;
    assert_eq!(claim(store.as_ref(), &room, 0).await.unwrap().id, blocker);
    let mine = task(store.as_ref(), room.channel, "mine").await;
    assert_eq!(claim(store.as_ref(), &room, 1).await.unwrap().id, mine);

    let wider = footprint(
        FootprintMode::Exclusive,
        &[
            (FootprintKind::Lane, "bridge"),
            (FootprintKind::Lane, "gateway"),
        ],
    );
    let conflicts = store.set_thread_footprint(mine, Some(wider)).await.unwrap();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].thread_id, blocker);
    assert_eq!(conflicts[0].assignee_id, room.agents[0]);
    assert_eq!(conflicts[0].value, "gateway");
    assert!(conflicts[0].lease_expires_at.is_some());
    assert_eq!(store.get_thread_footprint(mine).await.unwrap(), None);

    let narrow = footprint(FootprintMode::Exclusive, &[(FootprintKind::Lane, "bridge")]);
    assert!(store
        .set_thread_footprint(mine, Some(narrow.clone()))
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        store.get_thread_footprint(mine).await.unwrap(),
        Some(narrow)
    );
}

/// Two tenants with the same repository and lane never block each other, and
/// neither sees the other's claims.
async fn workspaces_never_block_each_other(store: Arc<dyn Store>) {
    let a = room(store.as_ref(), "tenant-a", 1).await;
    let b = room(store.as_ref(), "tenant-b", 1).await;
    let lane = footprint(FootprintMode::Exclusive, &[(FootprintKind::Lane, "bridge")]);
    let in_a = task(store.as_ref(), a.channel, "a").await;
    let in_b = task(store.as_ref(), b.channel, "b").await;
    declare(store.as_ref(), in_a, lane.clone()).await;
    declare(store.as_ref(), in_b, lane).await;
    assert_eq!(claim(store.as_ref(), &a, 0).await.unwrap().id, in_a);
    assert_eq!(claim(store.as_ref(), &b, 0).await.unwrap().id, in_b);
    let seen_by_b = store.list_active_claims(b.ws, None, None).await.unwrap();
    assert_eq!(seen_by_b.len(), 1);
    assert_eq!(seen_by_b[0].thread_id, in_b);
    // B's member reading A's workspace sees nothing of it.
    assert!(store
        .list_active_claims(a.ws, None, Some(b.agents[0]))
        .await
        .unwrap()
        .is_empty());
}

/// The claims list keeps to the reader's read rule: a private channel's
/// claims are listed only for its members.
async fn the_claims_list_keeps_to_the_read_rule(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "private", 2).await;
    let private = store
        .create_channel(NewChannel {
            workspace_id: room.ws,
            name: "secret".into(),
            topic: None,
            private: true,
        })
        .await
        .unwrap();
    store
        .add_channel_member(private.id, room.agents[0], ChannelMemberRole::Member)
        .await
        .unwrap();
    let hidden = task(store.as_ref(), private.id, "hidden").await;
    declare(
        store.as_ref(),
        hidden,
        footprint(FootprintMode::Exclusive, &[(FootprintKind::Lane, "x")]),
    )
    .await;
    store
        .claim_next_thread(private.id, room.agents[0], Some(600))
        .await
        .unwrap()
        .unwrap();
    let member = store
        .list_active_claims(room.ws, None, Some(room.agents[0]))
        .await
        .unwrap();
    let outsider = store
        .list_active_claims(room.ws, None, Some(room.agents[1]))
        .await
        .unwrap();
    assert_eq!(member.len(), 1);
    assert!(outsider.is_empty());
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
                    let Some((_container, store)) = super::postgres().await else {
                        return;
                    };
                    super::$name(store).await;
                }
            )*
        }
    };
}

/// A paused repository's work waits in the queue, counted as blocked, while
/// other repositories, threads without a footprint and claims already held
/// carry on; resuming frees it. A pause in one workspace binds no other.
async fn a_paused_repository_hands_out_none_of_its_work(store: Arc<dyn Store>) {
    let other_room = room(store.as_ref(), "not-paused", 1).await;
    let room = room(store.as_ref(), "paused", 4).await;
    let held = task(store.as_ref(), room.channel, "held").await;
    let waits = task(store.as_ref(), room.channel, "waits").await;
    let elsewhere = task(store.as_ref(), room.channel, "elsewhere").await;
    let plain = task(store.as_ref(), room.channel, "plain").await;
    declare(
        store.as_ref(),
        held,
        footprint(
            FootprintMode::Exclusive,
            &[(FootprintKind::Lane, "gateway")],
        ),
    )
    .await;
    declare(
        store.as_ref(),
        waits,
        footprint(FootprintMode::Exclusive, &[(FootprintKind::Lane, "bridge")]),
    )
    .await;
    let mut other_repo = footprint(FootprintMode::Exclusive, &[(FootprintKind::Lane, "bridge")]);
    other_repo.repo = "example/other".into();
    declare(store.as_ref(), elsewhere, other_repo).await;
    assert_eq!(claim(store.as_ref(), &room, 0).await.unwrap().id, held);

    let pause = store
        .pause_repo_audited(
            room.ws,
            REPO,
            room.agents[3],
            Some("deploy freeze"),
            audit_for(room.ws, "repo.pause"),
        )
        .await
        .unwrap();
    assert_eq!(pause.repo, REPO);
    assert_eq!(pause.paused_by, room.agents[3]);
    assert_eq!(
        store.list_repo_pauses(room.ws).await.unwrap(),
        vec![pause.clone()]
    );
    assert!(store
        .list_repo_pauses(other_room.ws)
        .await
        .unwrap()
        .is_empty());

    let depth = store.channel_queue_depth(room.channel, None).await.unwrap();
    assert_eq!(
        (depth.assigned, depth.ready, depth.blocked),
        (1, 2, 1),
        "the paused repository's waiting thread is blocked; the held one runs on"
    );
    let mut handed = vec![
        claim(store.as_ref(), &room, 1).await.unwrap().id,
        claim(store.as_ref(), &room, 2).await.unwrap().id,
    ];
    handed.sort_by_key(|id| id.0);
    let mut expected = vec![elsewhere, plain];
    expected.sort_by_key(|id| id.0);
    assert_eq!(handed, expected);
    assert!(
        claim(store.as_ref(), &room, 3).await.is_none(),
        "nothing in the paused repository is handed out"
    );
    let active = store
        .list_active_claims(room.ws, Some(REPO.into()), None)
        .await
        .unwrap();
    assert_eq!(active.len(), 1, "the claim held before the pause stands");
    assert_eq!(active[0].thread_id, held);

    // The same repository in another workspace is not paused.
    let theirs = task(store.as_ref(), other_room.channel, "theirs").await;
    declare(
        store.as_ref(),
        theirs,
        footprint(FootprintMode::Exclusive, &[(FootprintKind::Lane, "bridge")]),
    )
    .await;
    assert_eq!(
        claim(store.as_ref(), &other_room, 0).await.unwrap().id,
        theirs
    );

    assert!(store
        .resume_repo_audited(room.ws, REPO, audit(room.ws, "repo.resume"))
        .await
        .unwrap());
    assert_eq!(claim(store.as_ref(), &room, 3).await.unwrap().id, waits);
    assert!(
        !store
            .resume_repo_audited(room.ws, REPO, audit(room.ws, "repo.resume"))
            .await
            .unwrap(),
        "a repository that is not paused has nothing to resume"
    );
}

fn audit(ws: WorkspaceId, action: &str) -> NewAuditEvent {
    NewAuditEvent {
        scope: AuditScope::Workspace(ws),
        actor_id: None,
        action: action.into(),
        target_kind: Some("repo".into()),
        target_id: None,
        metadata: serde_json::json!({}),
    }
}

fn audit_for(ws: WorkspaceId, action: &str) -> maidan_store::AuditFor<RepoPause> {
    let event = audit(ws, action);
    Box::new(move |_| event)
}

/// A worker serving one repository is handed only that repository's work:
/// a thread whose footprint names another repository, or no repository, is
/// left for others, in either claim scope.
async fn a_claimer_serving_one_repository_takes_only_its_work(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "per-repo", 2).await;
    let plain = task(store.as_ref(), room.channel, "plain").await;
    let other = task(store.as_ref(), room.channel, "other").await;
    let mine = task(store.as_ref(), room.channel, "mine").await;
    let mut other_repo = footprint(FootprintMode::Shared, &[(FootprintKind::Lane, "x")]);
    other_repo.repo = "example/other".into();
    declare(store.as_ref(), other, other_repo).await;
    declare(
        store.as_ref(),
        mine,
        footprint(FootprintMode::Shared, &[(FootprintKind::Lane, "x")]),
    )
    .await;

    let (claimed, _) = store
        .claim_next_thread_with_event(room.channel, room.agents[0], Some(600), Some(REPO))
        .await
        .unwrap();
    assert_eq!(
        claimed.map(|t| t.id),
        Some(mine),
        "the oldest work is skipped: it is not this repository's"
    );
    let (claimed, _) = store
        .claim_next_workspace_thread_with_event(room.ws, room.agents[1], Some(600), Some(REPO))
        .await
        .unwrap();
    assert!(claimed.is_none(), "nothing else names the repository");
    let (claimed, _) = store
        .claim_next_workspace_thread_with_event(room.ws, room.agents[1], Some(600), None)
        .await
        .unwrap();
    assert_eq!(
        claimed.map(|t| t.id),
        Some(plain),
        "without a repository, any work"
    );
    let (claimed, _) = store
        .claim_next_thread_with_event(
            room.channel,
            room.agents[1],
            Some(600),
            Some("example/other"),
        )
        .await
        .unwrap();
    assert_eq!(claimed.map(|t| t.id), Some(other));
}

async fn sqlite() -> Arc<dyn Store> {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    maidan_store::configure_sqlite_pool(&pool).await.unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    Arc::new(SqliteStore::for_tests(pool))
}

async fn postgres() -> Option<(testcontainers::ContainerAsync<Postgres>, Arc<dyn Store>)> {
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
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&format!(
            "postgres://postgres:postgres@{host}:{port}/postgres"
        ))
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    Some((container, Arc::new(PostgresStore::for_tests(pool))))
}

on_both_backends!(
    work_in_a_held_lane_waits_until_the_claim_ends,
    a_lapsed_lease_frees_its_footprint,
    overlap_follows_mode_kind_and_repository,
    a_footprint_refined_after_claiming_is_refused_on_overlap,
    workspaces_never_block_each_other,
    the_claims_list_keeps_to_the_read_rule,
    a_paused_repository_hands_out_none_of_its_work,
    a_claimer_serving_one_repository_takes_only_its_work,
);

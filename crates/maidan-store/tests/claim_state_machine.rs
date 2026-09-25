//! `claim_next` and its fenced follow-ups, against a model (Wave 4 #45).
//!
//! Random sequences of claims (durable and leased), releases, renewals,
//! acknowledgements and lease expiries run against the real SQLite store and
//! a small model of who holds what. After every step each thread's holder and
//! fencing token match the model, and so does every member's live-claim
//! count. A fenced call with a stale token, or from a member that no longer
//! holds the thread, must change nothing.

use std::collections::HashMap;

use chrono::{Duration, Utc};
use maidan_store::{prelude::*, run_sqlite_migrations, StoreError};
use maidan_types::{
    ChannelId, ClaimLeaseId, MemberId, MemberKind, NewChannel, NewMember, NewThread, NewWorkspace,
    ThreadId,
};
use proptest::prelude::*;
use sqlx::sqlite::SqlitePoolOptions;

const THREADS: usize = 3;
const MEMBERS: usize = 3;

#[derive(Debug, Clone)]
enum Op {
    Claim {
        member: usize,
        leased: bool,
    },
    Release {
        member: usize,
        thread: usize,
        stale: bool,
    },
    Renew {
        member: usize,
        thread: usize,
        stale: bool,
    },
    Acknowledge {
        member: usize,
        thread: usize,
        stale: bool,
    },
    Expire {
        thread: usize,
    },
}

fn op() -> impl Strategy<Value = Op> {
    let m = 0..MEMBERS;
    let t = 0..THREADS;
    prop_oneof![
        3 => (m.clone(), any::<bool>()).prop_map(|(member, leased)| Op::Claim { member, leased }),
        2 => (m.clone(), t.clone(), any::<bool>())
            .prop_map(|(member, thread, stale)| Op::Release { member, thread, stale }),
        1 => (m.clone(), t.clone(), any::<bool>())
            .prop_map(|(member, thread, stale)| Op::Renew { member, thread, stale }),
        1 => (m, t.clone(), any::<bool>())
            .prop_map(|(member, thread, stale)| Op::Acknowledge { member, thread, stale }),
        1 => t.prop_map(|thread| Op::Expire { thread }),
    ]
}

#[derive(Debug, Clone, Copy)]
struct Holder {
    member: usize,
    lease: ClaimLeaseId,
    leased: bool,
    expired: bool,
}

#[derive(Default)]
struct Model {
    holders: HashMap<usize, Holder>,
    /// The last token each thread was claimed with before its current one.
    stale: HashMap<usize, ClaimLeaseId>,
}

impl Model {
    fn claimable(&self, thread: usize) -> bool {
        match self.holders.get(&thread) {
            None => true,
            Some(h) => h.leased && h.expired,
        }
    }

    fn live_claims(&self, member: usize) -> i64 {
        self.holders
            .values()
            .filter(|h| h.member == member && !(h.leased && h.expired))
            .count() as i64
    }
}

struct World {
    store: SqliteStore,
    pool: sqlx::SqlitePool,
    channel: ChannelId,
    threads: Vec<ThreadId>,
    members: Vec<MemberId>,
}

async fn world() -> World {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store = SqliteStore::new(pool.clone());
    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "queue".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let mut threads = Vec::new();
    for i in 0..THREADS {
        let t = store
            .create_thread(NewThread {
                channel_id: channel.id,
                parent_thread_id: None,
                title: Some(format!("t{i}")),
            })
            .await
            .unwrap();
        threads.push(t.id);
        // Distinct creation times, so "oldest claimable" is unambiguous.
        tokio::time::sleep(std::time::Duration::from_millis(3)).await;
    }
    let mut members = Vec::new();
    for i in 0..MEMBERS {
        members.push(
            store
                .create_member(NewMember {
                    workspace_id: ws.id,
                    handle: format!("m{i}"),
                    display_name: None,
                    kind: MemberKind::Agent,
                })
                .await
                .unwrap()
                .id,
        );
    }
    World {
        store,
        pool,
        channel: channel.id,
        threads,
        members,
    }
}

async fn run(ops: Vec<Op>) {
    let w = world().await;
    let mut model = Model::default();
    let token_for = |model: &Model, thread: usize, stale: bool| -> ClaimLeaseId {
        if stale {
            model.stale.get(&thread).copied().unwrap_or_default()
        } else {
            model
                .holders
                .get(&thread)
                .map(|h| h.lease)
                .unwrap_or_default()
        }
    };
    let holds = |model: &Model, member: usize, thread: usize, lease: ClaimLeaseId| {
        model
            .holders
            .get(&thread)
            .is_some_and(|h| h.member == member && h.lease == lease)
    };

    for op in ops {
        match op {
            Op::Claim { member, leased } => {
                let lease = leased.then_some(3600);
                let got = w
                    .store
                    .claim_next_thread(w.channel, w.members[member], lease)
                    .await
                    .unwrap();
                let expected = (0..THREADS).find(|t| model.claimable(*t));
                match (got, expected) {
                    (None, None) => {}
                    (Some(t), Some(i)) => {
                        assert_eq!(t.id, w.threads[i], "claimed the oldest claimable thread");
                        let token = t.claim_lease_id.expect("a claim carries a token");
                        if let Some(old) = model.holders.get(&i) {
                            model.stale.insert(i, old.lease);
                        }
                        model.holders.insert(
                            i,
                            Holder {
                                member,
                                lease: token,
                                leased,
                                expired: false,
                            },
                        );
                    }
                    (got, expected) => panic!("claim: store {got:?}, model {expected:?}"),
                }
            }
            Op::Release {
                member,
                thread,
                stale,
            } => {
                let lease = token_for(&model, thread, stale);
                let r = w
                    .store
                    .release_claim(w.threads[thread], w.members[member], lease)
                    .await;
                if holds(&model, member, thread, lease) {
                    r.expect("the holder may release");
                    let old = model.holders.remove(&thread).unwrap();
                    model.stale.insert(thread, old.lease);
                } else {
                    assert!(matches!(r, Err(StoreError::NotFound)), "{r:?}");
                }
            }
            Op::Renew {
                member,
                thread,
                stale,
            } => {
                let lease = token_for(&model, thread, stale);
                let r = w
                    .store
                    .renew_claim(w.threads[thread], w.members[member], lease, 3600)
                    .await;
                if holds(&model, member, thread, lease) {
                    r.expect("the holder may renew");
                    let h = model.holders.get_mut(&thread).unwrap();
                    h.leased = true;
                    h.expired = false;
                } else {
                    assert!(matches!(r, Err(StoreError::NotFound)), "{r:?}");
                }
            }
            Op::Acknowledge {
                member,
                thread,
                stale,
            } => {
                let lease = token_for(&model, thread, stale);
                let r = w
                    .store
                    .acknowledge_claim(w.threads[thread], w.members[member], lease)
                    .await;
                if holds(&model, member, thread, lease) {
                    r.expect("the holder may acknowledge");
                } else {
                    assert!(matches!(r, Err(StoreError::NotFound)), "{r:?}");
                }
            }
            Op::Expire { thread } => {
                // Move a leased claim's deadline into the past, as time would.
                let past = (Utc::now() - Duration::hours(1)).to_rfc3339();
                sqlx::query(
                    "UPDATE maidan_threads SET assignment_expires_at = ?
                     WHERE id = ? AND assignment_expires_at IS NOT NULL",
                )
                .bind(past)
                .bind(w.threads[thread].0)
                .execute(&w.pool)
                .await
                .unwrap();
                if let Some(h) = model.holders.get_mut(&thread) {
                    if h.leased {
                        h.expired = true;
                    }
                }
            }
        }

        for (i, id) in w.threads.iter().enumerate() {
            let t = w.store.get_thread(*id).await.unwrap();
            match model.holders.get(&i) {
                Some(h) => {
                    assert_eq!(
                        t.assignee_id,
                        Some(w.members[h.member]),
                        "thread {i} holder"
                    );
                    assert_eq!(t.claim_lease_id, Some(h.lease), "thread {i} token");
                }
                None => assert_eq!(t.assignee_id, None, "thread {i} should be free"),
            }
        }
        for (m, id) in w.members.iter().enumerate() {
            assert_eq!(
                w.store.count_live_claims(*id).await.unwrap(),
                model.live_claims(m),
                "member {m} live claims"
            );
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn claims_follow_the_model(ops in proptest::collection::vec(op(), 1..40)) {
        tokio::runtime::Runtime::new().unwrap().block_on(run(ops));
    }
}

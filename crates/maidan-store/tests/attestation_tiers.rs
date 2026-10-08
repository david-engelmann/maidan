//! Attestation tiers on both backends (Open Work Next 3). A hand-off judges
//! each piece of evidence once, in its own transaction: a worker's result or
//! link is self-reported, anyone else's link is attached, and a land-gate pass
//! the close gate would accept is verified. The tiers are pinned in the packet
//! and covered by its root, so nothing after the hand-off moves them, and an
//! approval names the tiers it was shown.

use std::time::Duration;

use maidan_fsm::ThreadAction;
use maidan_store::attribution::with_attribution;
use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::*;
use serde_json::json;
use sqlx::{postgres::PgPoolOptions, sqlite::SqlitePoolOptions};
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

const BY_WORKER: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const BY_OWNER: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const BY_DELEGATE: &str = "3333333333333333333333333333333333333333333333333333333333333333";

fn acting_as(actor: MemberId, subject: MemberId) -> Option<Attribution> {
    Some(Attribution {
        actor_id: actor,
        subject_id: subject,
        grant_id: Some(DelegationGrantId(uuid::Uuid::new_v4())),
    })
}

fn tiers(packet: &ReviewPacket) -> Vec<(EvidenceKind, Option<&str>, AttestationTier)> {
    packet
        .manifest
        .attestations
        .iter()
        .map(|a| (a.kind, a.sha256.as_deref(), a.tier))
        .collect()
}

async fn packet(store: &dyn Store, thread: ThreadId) -> ReviewPacket {
    store
        .latest_review_packet(thread)
        .await
        .expect("read")
        .expect("a packet")
}

async fn run_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "tiers".into(),
        })
        .await
        .expect("workspace")
        .id;
    let mut ids = Vec::new();
    for handle in ["owner", "worker", "reviewer", "verifier", "outsider"] {
        ids.push(
            store
                .create_member(NewMember {
                    workspace_id: ws,
                    handle: handle.into(),
                    display_name: None,
                    kind: MemberKind::Agent,
                })
                .await
                .expect("member")
                .id,
        );
    }
    let [owner, worker, reviewer, verifier, outsider] = ids[..] else {
        unreachable!()
    };
    store
        .add_member_skill(verifier, LAND_GATE_SKILL)
        .await
        .expect("skill");
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("channel")
        .id;
    for sha in [BY_WORKER, BY_OWNER, BY_DELEGATE] {
        store.record_artifact_ref(ws, sha).await.expect("held");
    }
    let new_thread = |title: &str| NewThread {
        channel_id: channel,
        parent_thread_id: None,
        title: Some(title.into()),
        description: None,
    };

    // Only the work's own account: the warning is on.
    let own = store
        .create_thread(new_thread("own"))
        .await
        .expect("thread")
        .id;
    store.set_review_requirement(own, 1).await.expect("req");
    store.claim_thread(own, worker).await.expect("claim");
    let result = json!({"status": "done"});
    store
        .set_thread_result(own, worker, &result)
        .await
        .expect("result");
    store
        .link_thread_artifact(own, BY_WORKER, worker)
        .await
        .expect("link");
    store
        .transition_thread(own, worker, ThreadAction::StartReview)
        .await
        .expect("hand off");
    let first = packet(store, own).await;
    let result_sha = result_sha256(&result).expect("hash");
    assert_eq!(
        tiers(&first),
        vec![
            (
                EvidenceKind::Result,
                Some(result_sha.as_str()),
                AttestationTier::SelfReported
            ),
            (
                EvidenceKind::Artifact,
                Some(BY_WORKER),
                AttestationTier::SelfReported
            ),
        ]
    );
    assert!(first.self_reported_only);
    assert_eq!(first.evidence_root, first.manifest.root().expect("root"));

    // Sent back. The owner, who never held the thread, links evidence; a
    // delegate that worked the thread links some with the outsider's token.
    store
        .submit_review(
            own,
            reviewer,
            ReviewDecision::RequestChanges,
            Some("show the logs"),
            None,
        )
        .await
        .expect("send back");
    store
        .link_thread_artifact(own, BY_OWNER, owner)
        .await
        .expect("owner link");
    let laundered = with_attribution(
        acting_as(worker, outsider),
        store.link_thread_artifact(own, BY_DELEGATE, outsider),
    )
    .await
    .expect("delegate link");
    assert_eq!(laundered.0.linked_by, outsider);
    store
        .transition_thread(own, worker, ThreadAction::StartReview)
        .await
        .expect("hand off again");
    let second = packet(store, own).await;
    assert_eq!(
        tiers(&second)[1..],
        [
            (
                EvidenceKind::Artifact,
                Some(BY_WORKER),
                AttestationTier::SelfReported
            ),
            (
                EvidenceKind::Artifact,
                Some(BY_OWNER),
                AttestationTier::Attached
            ),
            (
                EvidenceKind::Artifact,
                Some(BY_DELEGATE),
                AttestationTier::SelfReported
            ),
        ],
        "a delegate that worked the thread does not launder its link"
    );
    assert!(
        !second.self_reported_only,
        "the owner's link is not the work's own account"
    );
    let approved_root = second.evidence_root.clone();
    store
        .submit_review(
            own,
            reviewer,
            ReviewDecision::Approve,
            None,
            Some(&approved_root),
        )
        .await
        .expect("approve what was shown");

    // A land-gate pass recorded after the hand-off does not reach back into
    // the packet, and judging it later does not disturb the approval: the
    // close compares the evidence, not re-judged tiers.
    store.require_land_gate(own).await.expect("arm");
    let standing = store
        .set_land_gate_pointer(own, verifier, LandGateStatus::Pass, Some(BY_WORKER), None)
        .await
        .expect("pass");
    assert!(standing.landable, "{standing:?}");
    assert_eq!(packet(store, own).await, second, "the packet did not move");
    assert_eq!(store.review_status(own).await.expect("status").approvals, 1);

    // Handed over again with the same evidence, the pass is in the packet as
    // verified, so the root moves and the approval of the old tiers is stale.
    store
        .submit_review(
            own,
            reviewer,
            ReviewDecision::RequestChanges,
            Some("once more, with the gate"),
            None,
        )
        .await
        .expect("send back");
    store
        .transition_thread(own, worker, ThreadAction::StartReview)
        .await
        .expect("third hand-off");
    let third = packet(store, own).await;
    assert!(third.manifest.same_evidence(&second.manifest));
    assert_ne!(
        third.evidence_root, approved_root,
        "the same evidence under other tiers is another root"
    );
    assert_eq!(
        tiers(&third).last(),
        Some(&(
            EvidenceKind::LandGate,
            Some(BY_WORKER),
            AttestationTier::Verified
        ))
    );
    let stale = store
        .submit_review(
            own,
            reviewer,
            ReviewDecision::Approve,
            None,
            Some(&approved_root),
        )
        .await;
    assert!(
        matches!(&stale, Err(StoreError::Conflict(m)) if m.contains("stale evidence")),
        "{stale:?}"
    );
    store
        .submit_review(
            own,
            reviewer,
            ReviewDecision::Approve,
            None,
            Some(&third.evidence_root),
        )
        .await
        .expect("approve the third");
    store
        .transition_thread(own, reviewer, ThreadAction::Close)
        .await
        .expect("closes");

    // A pass the close gate would refuse (its recorder worked the thread)
    // verifies nothing, and a worker's evidence alone still warns.
    let gated = store
        .create_thread(new_thread("gated"))
        .await
        .expect("thread")
        .id;
    store
        .add_member_skill(worker, LAND_GATE_SKILL)
        .await
        .expect("skill");
    store.claim_thread(gated, worker).await.expect("claim");
    store
        .set_thread_result(gated, worker, &result)
        .await
        .expect("result");
    store.require_land_gate(gated).await.expect("arm");
    store
        .set_land_gate_pointer(gated, worker, LandGateStatus::Pass, None, None)
        .await
        .expect("own pass");
    store
        .transition_thread(gated, worker, ThreadAction::StartReview)
        .await
        .expect("hand off");
    let own_pass = packet(store, gated).await;
    assert!(own_pass
        .manifest
        .attestations
        .iter()
        .all(|a| a.tier == AttestationTier::SelfReported));
    assert!(own_pass.self_reported_only);

    // The usual order: the gate is armed, the work is handed over before any
    // pass exists, the reviewer approves, then the verifier passes it. The
    // close checks the evidence the approval named and does not judge the
    // tiers again, so the pass recorded after the hand-off does not refuse
    // it. A delegate that worked the thread posts the result with a
    // non-worker's token: still self-reported.
    let usual = store
        .create_thread(new_thread("usual"))
        .await
        .expect("thread")
        .id;
    store.set_review_requirement(usual, 1).await.expect("req");
    store.require_land_gate(usual).await.expect("arm");
    store.claim_thread(usual, outsider).await.expect("claim");
    with_attribution(
        acting_as(outsider, reviewer),
        store.set_thread_result(usual, reviewer, &result),
    )
    .await
    .expect("delegate result");
    store
        .transition_thread(usual, outsider, ThreadAction::StartReview)
        .await
        .expect("hand off");
    let handed = packet(store, usual).await;
    assert_eq!(
        tiers(&handed),
        vec![(
            EvidenceKind::Result,
            Some(result_sha.as_str()),
            AttestationTier::SelfReported
        )],
        "a delegate that worked the thread does not launder the result"
    );
    assert_eq!(handed.manifest.attestations[0].attested_by, reviewer);
    store
        .submit_review(
            usual,
            verifier,
            ReviewDecision::Approve,
            None,
            Some(&handed.evidence_root),
        )
        .await
        .expect("approve");
    store
        .set_land_gate_pointer(usual, verifier, LandGateStatus::Pass, None, None)
        .await
        .expect("pass after the hand-off");
    store
        .transition_thread(usual, owner, ThreadAction::Close)
        .await
        .expect("a pass after the hand-off does not refuse the close");

    // A member who links evidence and only later works the thread: the packet
    // already handed keeps the link as attached.
    let later = store
        .create_thread(new_thread("later"))
        .await
        .expect("thread")
        .id;
    store
        .link_thread_artifact(later, BY_OWNER, outsider)
        .await
        .expect("link");
    store
        .transition_thread(later, owner, ThreadAction::StartReview)
        .await
        .expect("hand off");
    let before = packet(store, later).await;
    assert_eq!(
        tiers(&before),
        vec![(
            EvidenceKind::Artifact,
            Some(BY_OWNER),
            AttestationTier::Attached
        )]
    );
    store
        .submit_review(
            later,
            reviewer,
            ReviewDecision::RequestChanges,
            Some("needs a worker"),
            None,
        )
        .await
        .expect("send back");
    store.claim_thread(later, outsider).await.expect("claim");
    assert_eq!(
        store
            .latest_review_packet(later)
            .await
            .expect("read")
            .expect("packet"),
        before,
        "becoming a worker later does not move the packet already handed"
    );

    // No evidence at all: nothing to warn about.
    let empty = store
        .create_thread(new_thread("empty"))
        .await
        .expect("thread")
        .id;
    store
        .transition_thread(empty, owner, ThreadAction::StartReview)
        .await
        .expect("hand off");
    let nothing = packet(store, empty).await;
    assert!(nothing.manifest.attestations.is_empty());
    assert!(!nothing.self_reported_only);
}

const BY_DELEGATOR: &str = "4444444444444444444444444444444444444444444444444444444444444444";
const BY_PLAIN: &str = "5555555555555555555555555555555555555555555555555555555555555555";

/// Links from two members who never worked the thread: one has delegated to
/// the thread's worker, one never delegated. Returns the thread; the caller
/// then strips the recorded actors, as rows written before migration 0147 are.
async fn legacy_setup(store: &dyn Store) -> (ThreadId, MemberId) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "legacy".into(),
        })
        .await
        .expect("workspace")
        .id;
    let mut ids = Vec::new();
    for handle in ["owner", "worker", "delegator", "plain"] {
        ids.push(
            store
                .create_member(NewMember {
                    workspace_id: ws,
                    handle: handle.into(),
                    display_name: None,
                    kind: MemberKind::Agent,
                })
                .await
                .expect("member")
                .id,
        );
    }
    let [owner, worker, delegator, plain] = ids[..] else {
        unreachable!()
    };
    store
        .create_delegation_grant(NewDelegationGrant {
            workspace_id: ws,
            subject_id: delegator,
            delegate_id: worker,
            capabilities: vec!["workspace:write".into()],
            purpose: "cover the shift".into(),
            authorized_by: owner,
            expires_at: chrono::Utc::now() + chrono::Duration::hours(8),
        })
        .await
        .expect("grant");
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: "old".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("channel")
        .id;
    let thread = store
        .create_thread(NewThread {
            channel_id: channel,
            parent_thread_id: None,
            title: Some("legacy".into()),
            description: None,
        })
        .await
        .expect("thread")
        .id;
    store.claim_thread(thread, worker).await.expect("claim");
    for (sha, by) in [(BY_DELEGATOR, delegator), (BY_PLAIN, plain)] {
        store.record_artifact_ref(ws, sha).await.expect("held");
        store
            .link_thread_artifact(thread, sha, by)
            .await
            .expect("link");
    }
    (thread, worker)
}

/// A row without an actor predates migration 0147. From a member who never
/// delegated, the member acted, so the link is attached. From one who did, a
/// delegate (here the thread's own worker) may have carried it, so it is not
/// shown to be independent and counts as self-reported.
async fn legacy_check(store: &dyn Store, thread: ThreadId, worker: MemberId) {
    store
        .transition_thread(thread, worker, ThreadAction::StartReview)
        .await
        .expect("hand off");
    let packet = packet(store, thread).await;
    assert_eq!(
        tiers(&packet),
        vec![
            (
                EvidenceKind::Artifact,
                Some(BY_DELEGATOR),
                AttestationTier::SelfReported
            ),
            (
                EvidenceKind::Artifact,
                Some(BY_PLAIN),
                AttestationTier::Attached
            ),
        ],
        "an unrecorded actor is independent only when no delegate could have acted"
    );
}

#[tokio::test]
async fn evidence_is_tiered_once_at_the_hand_off_on_sqlite() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store = SqliteStore::for_tests(pool.clone());
    run_suite(&store).await;

    let (thread, worker) = legacy_setup(&store).await;
    let unrecorded: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM maidan_thread_artifacts
         WHERE thread_id = ?1 AND (linked_actor_id IS NULL OR linked_actor_id <> linked_by)",
    )
    .bind(thread.0)
    .fetch_one(&pool)
    .await
    .expect("count");
    assert_eq!(
        unrecorded, 0,
        "a direct link records its member as the actor"
    );
    sqlx::query("UPDATE maidan_thread_artifacts SET linked_actor_id = NULL WHERE thread_id = ?1")
        .bind(thread.0)
        .execute(&pool)
        .await
        .expect("strip actors");
    legacy_check(&store, thread, worker).await;
}

#[tokio::test]
async fn evidence_is_tiered_once_at_the_hand_off_on_postgres() {
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
    let host = container.get_host().await.expect("container host");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("container port");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&format!(
            "postgres://postgres:postgres@{host}:{port}/postgres"
        ))
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    let store = PostgresStore::for_tests(pool.clone());
    run_suite(&store).await;

    let (thread, worker) = legacy_setup(&store).await;
    let unrecorded: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM maidan_thread_artifacts
         WHERE thread_id = $1 AND (linked_actor_id IS NULL OR linked_actor_id <> linked_by)",
    )
    .bind(thread.0)
    .fetch_one(&pool)
    .await
    .expect("count");
    assert_eq!(
        unrecorded, 0,
        "a direct link records its member as the actor"
    );
    sqlx::query("UPDATE maidan_thread_artifacts SET linked_actor_id = NULL WHERE thread_id = $1")
        .bind(thread.0)
        .execute(&pool)
        .await
        .expect("strip actors");
    legacy_check(&store, thread, worker).await;
}

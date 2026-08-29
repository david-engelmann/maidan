//! Cluster 318 (token round): evidence for the "far fewer tokens" claim.
//!
//! The README says agents "pull exactly the context a step needs … instead of
//! re-stuffing the prompt, so the same work costs far fewer tokens." That is a
//! claim without a number. This measures it: the scoped thread **context pack**
//! (`GET /threads/:id/context`) vs the naive baseline of dumping **every message
//! in the channel** into the prompt — plus the lean-edits lever (edit metadata vs
//! full `body_before`/`body_after`).
//!
//! Reported in **bytes** (exact, tokenizer-independent — the serialized JSON is
//! literally what an agent receives) and an estimated token count (`chars/4`, a
//! standard rough approximation for English; the *ratio* is tokenizer-independent
//! to first order). `token_pack_evidence` is `#[ignore]`d — a measurement tool,
//! not a pass/fail gate. The estimator math is pure and unit-tested in CI.
//!
//! ```sh
//! cargo test -p maidan-server --test token_pack -- --ignored --nocapture
//! ```

use std::sync::Arc;

use maidan_server::thread_context::{build_thread_context, ThreadContextLimits};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{
    EditMessage, MemberKind, Message, NewChannel, NewMember, NewMessage, NewThread, NewWorkspace,
};
use sqlx::sqlite::SqlitePoolOptions;

/// Rough prompt-token estimate: ~4 characters per token (the widely-cited
/// approximation for English BPE tokenizers). Exact counts depend on the model's
/// tokenizer; the byte counts reported alongside are exact, and the pack-vs-naive
/// *ratio* is stable across estimators.
fn estimate_tokens(s: &str) -> usize {
    s.chars().count().div_ceil(4)
}

/// naive / pack, guarding against divide-by-zero.
fn reduction_ratio(naive: usize, pack: usize) -> f64 {
    if pack == 0 {
        return 0.0;
    }
    naive as f64 / pack as f64
}

#[tokio::test]
#[ignore = "measurement tool, not a CI gate; run with --ignored --nocapture"]
async fn token_pack_evidence() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool));

    // A realistic-ish channel: several threads of substantive messages, so the
    // numbers reflect real content, not "msg 1".
    let threads_in_channel = 8;
    let messages_per_thread = 40;
    let body = "The auth service returns 500 when the OIDC discovery document is \
                cached past its TTL; we should honor Cache-Control and re-fetch on a \
                stale read before minting the session cookie.";

    let ws = store
        .create_workspace(NewWorkspace { name: "tok".into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "agent".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "eng".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();

    let mut target_thread = None;
    for t in 0..threads_in_channel {
        let thread = store
            .create_thread(NewThread {
                channel_id: channel.id,
                parent_thread_id: None,
                title: Some(format!("investigation-{t}")),
            })
            .await
            .unwrap();
        let mut first_msg = None;
        for m in 0..messages_per_thread {
            let msg = store
                .post_message(NewMessage {
                    thread_id: thread.id,
                    author_id: member.id,
                    body: format!("[{t}.{m}] {body}"),
                    metadata: serde_json::json!({}),
                    content: None,
                })
                .await
                .unwrap();
            if first_msg.is_none() {
                first_msg = Some(msg.id);
            }
        }
        // The target thread also accrues an edit history — the lean-edits lever.
        if t == 0 {
            target_thread = Some(thread.id);
            for e in 0..15 {
                store
                    .edit_message(
                        first_msg.unwrap(),
                        member.id,
                        EditMessage {
                            body: format!("[edit {e}] {body}"),
                            metadata: serde_json::json!({}),
                            content: None,
                        },
                    )
                    .await
                    .unwrap();
            }
        }
    }
    let target = target_thread.unwrap();

    // (1) scoped pack (default: bounded window, edit metadata only).
    let pack = build_thread_context(store.as_ref(), target, ThreadContextLimits::default())
        .await
        .unwrap();
    let pack_bytes = serde_json::to_vec(&pack).unwrap().len();

    // (2) naive baseline: every message in the channel, full bodies, dumped.
    let mut all: Vec<Message> = Vec::new();
    for thread in store.list_threads_for_workspace(ws.id).await.unwrap() {
        if thread.channel_id == channel.id {
            all.extend(store.list_messages(thread.id, 10_000).await.unwrap());
        }
    }
    let naive_bytes = serde_json::to_vec(&all).unwrap().len();

    // (3) the lean-edits lever, in isolation: same pack with full edit bodies.
    let pack_full_edits = build_thread_context(
        store.as_ref(),
        target,
        ThreadContextLimits {
            include_edits: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let pack_full_edits_bytes = serde_json::to_vec(&pack_full_edits).unwrap().len();

    println!("\n=== Maidan context-pack token evidence (Cluster 318) ===");
    println!(
        "channel: {threads_in_channel} threads × {messages_per_thread} messages = {} total",
        all.len()
    );
    println!("token estimate: ~chars/4 (bytes are exact; ratio is tokenizer-independent)\n");
    // JSON is ~ASCII (1 byte/char), so bytes/4 is the same estimate as chars/4.
    println!(
        "  {:<34} {:>8} bytes   ~{:>7} tokens",
        "scoped pack (GET .../context)",
        pack_bytes,
        pack_bytes.div_ceil(4)
    );
    println!(
        "  {:<34} {:>8} bytes   ~{:>7} tokens",
        "naive: dump whole channel",
        naive_bytes,
        naive_bytes.div_ceil(4)
    );
    println!(
        "  {:<34} {:>8} bytes   ~{:>7} tokens",
        "pack with full edit bodies",
        pack_full_edits_bytes,
        pack_full_edits_bytes.div_ceil(4)
    );
    println!(
        "\n  scoped pack vs naive channel dump: {:.1}× fewer tokens",
        reduction_ratio(naive_bytes, pack_bytes)
    );
    println!(
        "  lean edits vs full edit bodies:    {:.1}× fewer tokens on the pack\n",
        reduction_ratio(pack_full_edits_bytes, pack_bytes)
    );

    assert!(
        pack_bytes < naive_bytes,
        "the scoped pack must be smaller than the channel dump"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_tokens_is_chars_over_four_rounded_up() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2);
        assert_eq!(estimate_tokens(&"x".repeat(400)), 100);
    }

    #[test]
    fn reduction_ratio_is_naive_over_pack_and_guards_zero() {
        assert_eq!(reduction_ratio(1000, 100), 10.0);
        assert_eq!(reduction_ratio(500, 0), 0.0);
    }
}

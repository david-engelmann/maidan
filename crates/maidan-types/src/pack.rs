//! Token-budgeted context packing (Cluster 360, G-dev-1).
//!
//! A thread context pack caps message *rows* (`message_limit`), not *tokens*: a
//! page of wide `ContentBlock` messages within the row cap can still overflow a
//! model's context window. Worse, a long middle is precisely the region a model
//! attends to least ("Lost in the Middle"), so paying tokens for it is doubly
//! wasteful. This module supplies the pure primitives to cap a pack by an
//! estimated token budget: keep the thread's framing (its first message) and its
//! most-recent tail, and fold the elided middle into an auditable [`PackElision`]
//! marker so the omission is visible and recoverable.
//!
//! The math is deliberately pure and model-independent (`chars/4`) so a caller can
//! budget a pack without a live tokenizer, and so the whole fold is unit-testable
//! with no store. The REST assembler (`build_thread_context`) and the MCP pack
//! (`get_thread_context`) both fold through [`fold_messages_to_budget`].

use serde::{Deserialize, Serialize};

use crate::ids::{ChannelId, MessageId, ThreadId};
use crate::models::{Message, Thread, ThreadState};

/// Rough prompt-token estimate: ~4 characters per token, the widely-cited
/// approximation for English BPE tokenizers. Exact counts are tokenizer-specific;
/// this is intentionally model-independent so the pack's budget math is cheap and
/// stable. `chars/4`, rounded up.
pub fn estimate_tokens(s: &str) -> usize {
    s.chars().count().div_ceil(4)
}

/// Estimated token cost of one message as it rides the pack: the serialized JSON
/// (what the agent actually receives), estimated at `chars/4`. A serialization
/// failure — not expected for a well-formed [`Message`] — falls back to the body
/// length so the budgeter never panics.
pub fn message_tokens(message: &Message) -> usize {
    match serde_json::to_string(message) {
        Ok(json) => estimate_tokens(&json),
        Err(_) => estimate_tokens(&message.body),
    }
}

/// An auditable record of the messages a context pack dropped to fit its token
/// budget (Cluster 360). The pack keeps the thread's opening message (framing) and
/// its most-recent tail; the elided middle is summarized here, so the omission is
/// visible in the response and recoverable (page from `first_elided_id`, or
/// refetch the thread without a budget).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PackElision {
    /// Number of messages folded away.
    pub elided_message_count: usize,
    /// Estimated tokens the elided messages would have cost.
    pub elided_token_estimate: usize,
    /// The oldest elided message — page from here to recover the middle.
    pub first_elided_id: MessageId,
    /// The newest elided message.
    pub last_elided_id: MessageId,
    /// A human/agent-readable summary naming the omission and how to recover it.
    pub summary: String,
}

/// Grounding for a **child** thread's context pack (Cluster 360, G-dev-1): a
/// compact orientation to the parent it was spawned from, so a fresh claimer of a
/// sub-task knows *why it exists* (the parent's opening ask) and *what the parent
/// concluded* (its latest recorded decision). Deliberately bounded — one framing
/// message plus one decision payload, not the parent's whole history.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ParentGrounding {
    pub thread_id: ThreadId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub state: ThreadState,
    /// The parent's opening (oldest) message — the framing / task statement.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opening_message: Option<Message>,
    /// The parent's latest recorded decision/result payload (Cluster 234), if any.
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_result: Option<serde_json::Value>,
}

impl ParentGrounding {
    /// Decide whether a child thread may carry grounding for its (already-fetched)
    /// parent, and build it. Returns `None` — grounding withheld — when:
    /// - the parent is in a **different channel** than the child (the child's
    ///   access does not imply access to a parent elsewhere), or
    /// - the child's channel is a **DM** channel (grounding is a task concept; DM
    ///   threads are conversational and share the one `__dm__` channel across
    ///   unrelated conversations, so same-channel is not same-audience), or
    /// - the parent is **tombstoned**.
    ///
    /// The same-channel + non-DM rule is what makes grounding safe without a second
    /// access check: a caller that passed `ensure_thread_access` on the child, whose
    /// parent shares that non-DM channel, is by construction allowed to read the
    /// parent. `child_channel_is_dm` and `child_channel_id` describe the *child's*
    /// channel.
    pub fn assemble(
        parent: Thread,
        child_channel_id: ChannelId,
        child_channel_is_dm: bool,
        opening_message: Option<Message>,
        latest_result: Option<serde_json::Value>,
    ) -> Option<Self> {
        if parent.tombstoned_at.is_some()
            || parent.channel_id != child_channel_id
            || child_channel_is_dm
        {
            return None;
        }
        Some(Self {
            thread_id: parent.id,
            title: parent.title,
            state: parent.state,
            opening_message,
            latest_result,
        })
    }
}

/// Fold a page of messages (ordered oldest→newest) to fit `budget_tokens`,
/// preserving the thread's framing and recency ("Lost in the Middle"): the first
/// message and a suffix of the most-recent messages are kept verbatim; the elided
/// middle is captured in a [`PackElision`]. Returns the kept messages (still
/// oldest→newest) and the elision marker.
///
/// Returns `(messages, None)` — no fold — when the page already fits the budget,
/// or is too short to fold (0, 1, or 2 messages, where dropping the middle cannot
/// help without discarding the framing or the tail). The single newest message is
/// always kept even if it alone exceeds the budget: a pack with no recent message
/// is useless, and the honest elision marker still tells the reader the pack is
/// over budget.
pub fn fold_messages_to_budget(
    messages: Vec<Message>,
    budget_tokens: usize,
) -> (Vec<Message>, Option<PackElision>) {
    // With ≤2 messages there is no middle to fold: an over-budget 2-message page is
    // its framing plus its tail, and dropping either loses signal.
    if messages.len() <= 2 {
        return (messages, None);
    }
    let costs: Vec<usize> = messages.iter().map(message_tokens).collect();
    let total: usize = costs.iter().sum();
    if total <= budget_tokens {
        return (messages, None);
    }

    let n = messages.len();
    // Keep the opener (index 0) always; fill the rest of the budget with the newest
    // messages, walking backward and stopping before the opener. The newest message
    // (idx n-1) is kept unconditionally.
    let mut kept_tail_start = n;
    let mut running = costs[0];
    let mut idx = n;
    while idx > 1 {
        let i = idx - 1;
        let is_newest = i == n - 1;
        if is_newest || running + costs[i] <= budget_tokens {
            running += costs[i];
            kept_tail_start = i;
            idx -= 1;
        } else {
            break;
        }
    }

    // The elided middle is the half-open range [1, kept_tail_start). If it is empty,
    // everything fit around the opener after all — no marker.
    if kept_tail_start <= 1 {
        return (messages, None);
    }
    let first_elided_id = messages[1].id;
    let last_elided_id = messages[kept_tail_start - 1].id;
    let elided_message_count = kept_tail_start - 1;
    let elided_token_estimate: usize = costs[1..kept_tail_start].iter().sum();
    let kept_tail = n - kept_tail_start;
    let summary = format!(
        "{elided_message_count} earlier message(s) elided to fit a {budget_tokens}-token context \
         budget (~{elided_token_estimate} tokens). The thread's opening message and its \
         {kept_tail} most-recent message(s) are kept; page from message id {first_elided_id} \
         (or refetch without token_budget) to recover the middle."
    );

    let mut kept = Vec::with_capacity(1 + kept_tail);
    for (i, message) in messages.into_iter().enumerate() {
        if i == 0 || i >= kept_tail_start {
            kept.push(message);
        }
    }

    (
        kept,
        Some(PackElision {
            elided_message_count,
            elided_token_estimate,
            first_elided_id,
            last_elided_id,
            summary,
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{MemberId, ThreadId};
    use chrono::Utc;

    fn parent_thread(channel_id: ChannelId, tombstoned: bool) -> Thread {
        Thread {
            id: ThreadId::new(),
            channel_id,
            parent_thread_id: None,
            title: Some("parent task".into()),
            state: ThreadState::Open,
            assignee_id: None,
            assignment_expires_at: None,
            claim_lease_id: None,
            work_started_at: None,
            owner_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            tombstoned_at: tombstoned.then(Utc::now),
        }
    }

    #[test]
    fn grounding_is_produced_for_a_same_channel_non_dm_parent() {
        let ch = ChannelId::new();
        let g = ParentGrounding::assemble(
            parent_thread(ch, false),
            ch,
            false,
            Some(msg("do the thing")),
            Some(serde_json::json!({"decision": "ship"})),
        );
        let g = g.expect("same-channel non-dm parent grounds");
        assert_eq!(g.title.as_deref(), Some("parent task"));
        assert!(g.opening_message.is_some());
        assert_eq!(
            g.latest_result,
            Some(serde_json::json!({"decision": "ship"}))
        );
    }

    #[test]
    fn grounding_is_withheld_cross_channel_dm_or_tombstoned() {
        let ch = ChannelId::new();
        let other = ChannelId::new();
        // Cross-channel parent: the child's access does not imply the parent's.
        assert!(
            ParentGrounding::assemble(parent_thread(other, false), ch, false, None, None).is_none()
        );
        // DM channel: same-channel is not same-audience.
        assert!(
            ParentGrounding::assemble(parent_thread(ch, false), ch, true, None, None).is_none()
        );
        // Tombstoned parent.
        assert!(
            ParentGrounding::assemble(parent_thread(ch, true), ch, false, None, None).is_none()
        );
    }

    fn msg(body: &str) -> Message {
        Message {
            id: MessageId::new(),
            thread_id: ThreadId::new(),
            author_id: MemberId::new(),
            body: body.to_string(),
            metadata: serde_json::json!({}),
            content: None,
            posted_at: Utc::now(),
            edited_at: None,
            tombstoned_at: None,
        }
    }

    #[test]
    fn estimate_tokens_is_chars_over_four_rounded_up() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2);
        assert_eq!(estimate_tokens(&"x".repeat(400)), 100);
    }

    #[test]
    fn a_pack_within_budget_is_not_folded() {
        let messages = vec![msg("a"), msg("b"), msg("c"), msg("d")];
        let (kept, elision) = fold_messages_to_budget(messages.clone(), 100_000);
        assert_eq!(kept.len(), messages.len());
        assert!(elision.is_none());
    }

    #[test]
    fn short_pages_never_fold() {
        for len in 0..=2 {
            let messages: Vec<Message> = (0..len).map(|_| msg(&"x".repeat(1000))).collect();
            let (kept, elision) = fold_messages_to_budget(messages.clone(), 1);
            assert_eq!(kept.len(), messages.len());
            assert!(elision.is_none());
        }
    }

    #[test]
    fn folding_keeps_the_opener_and_the_recent_tail_and_records_the_middle() {
        // Five fat messages, a budget that fits only the opener + roughly two tail
        // messages. Bodies are distinct so we can assert identity.
        let messages = vec![
            msg(&"o".repeat(400)), // opener  (~ >100 tokens serialized)
            msg(&"1".repeat(400)),
            msg(&"2".repeat(400)),
            msg(&"3".repeat(400)),
            msg(&"4".repeat(400)), // newest tail
        ];
        let per = message_tokens(&messages[0]);
        // Room for the opener + two more messages, not all five.
        let budget = per * 3 + per / 2;
        let (kept, elision) = fold_messages_to_budget(messages.clone(), budget);

        let elision = elision.expect("over-budget page folds");
        // The opener is always first; the newest is always last.
        assert_eq!(kept.first().unwrap().id, messages[0].id);
        assert_eq!(kept.last().unwrap().id, messages[4].id);
        // Some middle was elided and the kept set shrank.
        assert!(kept.len() < messages.len());
        assert!(elision.elided_message_count >= 1);
        // The elided ids point strictly inside the original middle.
        assert_eq!(elision.first_elided_id, messages[1].id);
        assert!(elision.elided_token_estimate > 0);
        assert!(elision.summary.contains("elided"));
        // Kept messages stay in oldest→newest order and none of them is elided.
        let kept_ids: Vec<_> = kept.iter().map(|m| m.id).collect();
        assert!(kept_ids.windows(2).all(|w| {
            let a = messages.iter().position(|m| m.id == w[0]).unwrap();
            let b = messages.iter().position(|m| m.id == w[1]).unwrap();
            a < b
        }));
    }

    #[test]
    fn a_tiny_budget_still_keeps_opener_and_the_single_newest() {
        let messages = vec![
            msg(&"o".repeat(400)),
            msg(&"1".repeat(400)),
            msg(&"2".repeat(400)),
            msg(&"3".repeat(400)),
        ];
        let (kept, elision) = fold_messages_to_budget(messages.clone(), 1);
        // Never empty: framing + the newest survive even under an impossible budget.
        assert_eq!(kept.first().unwrap().id, messages[0].id);
        assert_eq!(kept.last().unwrap().id, messages[3].id);
        assert_eq!(kept.len(), 2);
        let elision = elision.expect("folded");
        assert_eq!(elision.elided_message_count, 2);
        assert_eq!(elision.first_elided_id, messages[1].id);
        assert_eq!(elision.last_elided_id, messages[2].id);
    }
}

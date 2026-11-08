//! Second-family review routing (S4): the SQL twin of
//! [`maidan_types::provider_family`], and the claim rule built on it.

use maidan_types::PROVIDER_FAMILIES;

/// SQL naming the provider family of a usage record from its provider and
/// model columns, built from [`PROVIDER_FAMILIES`] so it cannot drift from
/// [`maidan_types::provider_family`]; NULL when neither names one.
pub(crate) fn family_sql(provider: &str, model: &str) -> String {
    let named = format!("LOWER(COALESCE({provider}, '') || ' ' || {model})");
    let arms: String = PROVIDER_FAMILIES
        .iter()
        .map(|(family, words, prefixes)| {
            let model = format!("LOWER({model})");
            let any = words
                .iter()
                .map(|word| format!("{named} LIKE '%{word}%'"))
                .chain(
                    prefixes
                        .iter()
                        .map(|prefix| format!("{model} = '{prefix}' OR {model} LIKE '{prefix}-%'")),
                )
                .collect::<Vec<_>>()
                .join(" OR ");
            format!(" WHEN {any} THEN '{family}'")
        })
        .collect();
    format!("(CASE{arms} END)")
}

/// The provider family `member` shares with the author of the change
/// `thread` reviews, as a scalar SQL subquery: NULL when the thread names no
/// author family, or `member`'s usage evidence in the thread's workspace
/// never names it. Any such record counts, however old, so a seat that ever
/// reported under the author's family is refused that review for good. Every
/// path that hands a thread to a member applies it: `claim_next` through
/// [`refused_to`], a direct claim or assign through [`not_same_family`] and
/// the backends' `refuse_same_family_in_tx`.
pub(crate) fn shared_family(member: &str, thread: &str) -> String {
    let family = family_sql("ul.provider", "ul.model");
    format!(
        "(SELECT ra.author_family FROM maidan_thread_attributes ra
          JOIN maidan_threads rt ON rt.id = ra.thread_id
          JOIN maidan_channels rc ON rc.id = rt.channel_id
          JOIN maidan_usage_ledger ul ON ul.reporter_id = {member}
           AND ul.workspace_id = rc.workspace_id
          WHERE ra.thread_id = {thread} AND ra.author_family IS NOT NULL
            AND {family} = ra.author_family
          LIMIT 1)"
    )
}

/// True when `member` may not claim `cand`: see [`shared_family`].
pub(crate) fn refused_to(member: &str) -> String {
    format!("({} IS NOT NULL)", shared_family(member, "cand.id"))
}

/// True when `member` declined `cand` ([`maidan_types::ClaimDecline`]): it
/// released the thread saying the review's change was written by its own
/// family. [`refused_to`] reads a member's family from its usage evidence, and
/// a seat with none yet would otherwise be handed the same review on every
/// pass. Only `claim_next` reads it, and only for that member and thread:
/// everyone else is still handed the review, a direct claim or assign is the
/// caller's explicit choice, and a later review round is a new thread. A
/// review's author family is set once, so the decline cannot outlive what it
/// declined.
pub(crate) fn declined_by(member: &str) -> String {
    format!(
        "EXISTS (SELECT 1 FROM maidan_claim_declines cd
                 WHERE cd.thread_id = cand.id AND cd.member_id = {member})"
    )
}

/// The refusal an `own_family` decline answers with on a thread that is not a
/// review naming its author's family: there is no family to have shared.
pub(crate) fn decline_without_family() -> crate::error::StoreError {
    crate::error::StoreError::Conflict(
        "decline own_family: this thread is not a review that names its author_family; release it without decline"
            .to_owned(),
    )
}

/// The `AND` clause an `UPDATE maidan_threads` that hands the row to
/// `member` carries, so the write itself refuses a seat of the author's
/// family even if the read before it raced; see [`shared_family`].
pub(crate) fn not_same_family(member: &str) -> String {
    format!("AND {} IS NULL", shared_family(member, "maidan_threads.id"))
}

/// The refusal a direct claim or assign of a review answers with, naming
/// the family.
pub(crate) fn same_family_refusal(family: &str) -> crate::error::StoreError {
    crate::error::StoreError::Conflict(format!(
        "this review's change was written by the {family} family, and this member's usage names {family}; a review goes to a member of another family"
    ))
}

/// A query reading what a result write on a review needs to judge its
/// producer (#1703), with `member`, `thread` and `lease` as its placeholders.
/// It answers a row only while `member` holds `thread` under `lease` and the
/// thread is a review naming its author's family; otherwise the write's own
/// claim fence answers, so a caller that does not hold the claim learns
/// nothing about the review's family. Every usage read is the thread's own
/// workspace's.
///
/// Every claim-time check reads a member's family from usage it reported
/// before it was handed the review, and a seat with none passes them all.
/// The result write is the last point before a verdict is stored and
/// delivered, so the holder is judged again there, on everything it has
/// reported since: see [`result_refusal`].
pub(crate) fn result_judgement(member: &str, thread: &str, lease: &str) -> String {
    format!(
        "SELECT a.author_family AS author_family,
                {shared} AS shared_family,
                m.kind AS producer_kind,
                EXISTS (SELECT 1 FROM maidan_usage_ledger u
                        WHERE u.thread_id = t.id AND u.reporter_id = {member}
                          AND u.workspace_id = c.workspace_id) AS reported
         FROM maidan_threads t
         JOIN maidan_channels c ON c.id = t.channel_id
         JOIN maidan_thread_attributes a ON a.thread_id = t.id
         JOIN maidan_members m ON m.id = {member}
         WHERE t.id = {thread} AND t.assignee_id = {member}
           AND t.claim_lease_id = {lease} AND t.tombstoned_at IS NULL
           AND a.kind = 'review' AND a.author_family IS NOT NULL",
        shared = shared_family(member, thread),
    )
}

/// Whether a review's result may be stored from its holder, given what
/// [`result_judgement`] read: `None` stores it.
///
/// - A holder whose usage in the workspace names the author's family, before
///   or during this claim, is refused: its verdict would be stored and
///   delivered as a second family's when it is the author's own. The claim
///   stays held, so the holder can release it with `decline: own_family`.
/// - An agent that has reported no usage on the review is refused too. Its
///   usage is the only evidence of the family that did the review, and a
///   verdict stored before that usage would be delivered before anything
///   could judge it. A person is no model family,
///   and reports no usage, so a person's review is stored.
///
/// The usage report itself is never refused: refusing it would keep the
/// spend off the ledger and the budget, and would erase the very evidence
/// this rule reads.
pub(crate) fn result_refusal(
    author_family: &str,
    shared: Option<&str>,
    producer_kind: &str,
    reported: bool,
) -> Option<crate::error::StoreError> {
    if let Some(family) = shared {
        return Some(crate::error::StoreError::Conflict(format!(
            "this review's change was written by the {family} family, and this member's usage names {family}: a {family} model may not review it, and nothing was stored; release the claim with decline own_family"
        )));
    }
    if producer_kind != maidan_types::MemberKind::Human.as_str() && !reported {
        return Some(crate::error::StoreError::Conflict(format!(
            "this review's change was written by the {author_family} family, and this member has reported no usage on it; report usage naming the model that did the review before its result, so the result is judged against that family; nothing was stored"
        )));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sql_twin_names_every_family_word() {
        let sql = family_sql("p", "m");
        for (family, words, prefixes) in PROVIDER_FAMILIES {
            assert!(sql.contains(&format!("THEN '{family}'")));
            for word in *words {
                assert!(sql.contains(&format!("LIKE '%{word}%'")));
            }
            for prefix in *prefixes {
                assert!(sql.contains(&format!("LIKE '{prefix}-%'")));
            }
        }
    }

    #[test]
    fn a_review_result_is_refused_to_a_holder_whose_usage_names_the_authors_family() {
        let refused = result_refusal("anthropic", Some("anthropic"), "agent", true);
        assert!(
            matches!(&refused, Some(crate::error::StoreError::Conflict(m))
                if m.contains("anthropic family") && m.contains("decline own_family")),
            "{refused:?}"
        );
        let person = result_refusal("anthropic", Some("anthropic"), "human", true);
        assert!(person.is_some(), "usage naming the family refuses anyone");
    }

    #[test]
    fn a_review_result_waits_for_an_agents_usage_but_not_a_persons() {
        let unreported = result_refusal("openai", None, "agent", false);
        assert!(
            matches!(&unreported, Some(crate::error::StoreError::Conflict(m))
                if m.contains("reported no usage")),
            "{unreported:?}"
        );
        assert!(result_refusal("openai", None, "agent", true).is_none());
        assert!(result_refusal("openai", None, "human", false).is_none());
    }
}

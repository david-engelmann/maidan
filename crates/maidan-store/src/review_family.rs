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
}

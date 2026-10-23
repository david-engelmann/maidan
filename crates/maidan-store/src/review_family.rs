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

/// True when `cand` is a review routed away from its author's family and
/// `member`'s usage evidence in the review's workspace names that family: such
/// a seat may not claim it. Any such record counts, however old, so a seat
/// that ever reported under the author's family is refused that review for
/// good.
pub(crate) fn refused_to(member: &str) -> String {
    let family = family_sql("ul.provider", "ul.model");
    format!(
        "EXISTS (SELECT 1 FROM maidan_thread_attributes ra
                 JOIN maidan_usage_ledger ul ON ul.reporter_id = {member}
                  AND ul.workspace_id = ch.workspace_id
                 WHERE ra.thread_id = cand.id AND ra.author_family IS NOT NULL
                   AND {family} = ra.author_family)"
    )
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

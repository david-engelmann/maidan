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
        .map(|(family, words)| {
            let any = words
                .iter()
                .map(|word| format!("{named} LIKE '%{word}%'"))
                .collect::<Vec<_>>()
                .join(" OR ");
            format!(" WHEN {any} THEN '{family}'")
        })
        .collect();
    format!("(CASE{arms} END)")
}

/// True when `cand` is a review routed away from its author's family and
/// `member`'s usage evidence names that family: such a seat may not claim it.
pub(crate) fn refused_to(member: &str) -> String {
    let family = family_sql("ul.provider", "ul.model");
    format!(
        "EXISTS (SELECT 1 FROM maidan_thread_attributes ra
                 JOIN maidan_usage_ledger ul ON ul.reporter_id = {member}
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
        for (family, words) in PROVIDER_FAMILIES {
            assert!(sql.contains(&format!("THEN '{family}'")));
            for word in *words {
                assert!(sql.contains(&format!("LIKE '%{word}%'")));
            }
        }
    }
}

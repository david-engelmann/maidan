//! Change notices, the backend-neutral half: what a merged pull request
//! touched, from its rounds. The footprint matching is in
//! [`crate::footprints`].

/// The paths a merged pull request touched, across its rounds in order,
/// first seen first and deduplicated, with the base branch its latest round
/// named. A later round can drop a file an earlier one touched, but the
/// earlier round's commit is still in what merged.
pub(crate) fn merged_paths(
    rounds: impl IntoIterator<Item = (Vec<String>, Option<String>)>,
) -> (Vec<String>, Option<String>) {
    let mut seen = std::collections::HashSet::new();
    let mut paths = Vec::new();
    let mut base_branch = None;
    for (round_paths, round_base) in rounds {
        for path in round_paths {
            if seen.insert(path.clone()) {
                paths.push(path);
            }
        }
        if round_base.is_some() {
            base_branch = round_base;
        }
    }
    (paths, base_branch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_round_counts_and_the_latest_base_wins() {
        let (paths, base) = merged_paths([
            (vec!["a".into(), "b".into()], Some("main".into())),
            (vec!["b".into(), "c".into()], None),
            (vec![], Some("dev".into())),
        ]);
        assert_eq!(paths, vec!["a", "b", "c"]);
        assert_eq!(base.as_deref(), Some("dev"));
    }
}

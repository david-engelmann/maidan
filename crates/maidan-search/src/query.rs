//! Lexical query parsing hints (Postgres full-text search).

/// When true, Postgres search uses `websearch_to_tsquery` so `q` can carry
/// web-style operators (`"phrase"`, `-exclude`, `or`). Otherwise
/// `plainto_tsquery` ANDs plain words (existing behavior).
pub fn use_websearch_to_tsquery(query: &str) -> bool {
    let q = query.trim();
    if q.is_empty() {
        return false;
    }
    if q.contains('"') {
        return true;
    }
    if q.starts_with('-') {
        return true;
    }
    if q.as_bytes()
        .windows(2)
        .any(|w| w[0].is_ascii_whitespace() && w[1] == b'-')
    {
        return true;
    }
    q.split_whitespace().any(|t| t.eq_ignore_ascii_case("or"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_words_use_plainto() {
        assert!(!use_websearch_to_tsquery("rust tokio"));
    }

    #[test]
    fn quoted_phrase_uses_websearch() {
        assert!(use_websearch_to_tsquery("\"systems programming\""));
    }

    #[test]
    fn negation_uses_websearch() {
        assert!(use_websearch_to_tsquery("rust -deployment"));
    }

    #[test]
    fn or_operator_uses_websearch() {
        assert!(use_websearch_to_tsquery("rust or go"));
    }

    #[test]
    fn empty_or_whitespace_uses_plainto() {
        assert!(!use_websearch_to_tsquery(""));
        assert!(!use_websearch_to_tsquery("   "));
    }
}

//! Normalized search scores comparable across Postgres and SQLite within a mode.

use crate::hit::SearchHit;

/// Semantic `rank` is already `1.0 - cosine_distance` in `[0, 1]`.
pub fn semantic_score(rank: f64) -> f64 {
    rank.clamp(0.0, 1.0)
}

/// Min-max normalize lexical `rank` values within one response to `[0, 1]`.
pub fn normalize_lexical_scores(hits: &mut [SearchHit]) {
    if hits.is_empty() {
        return;
    }
    let min = hits.iter().map(|h| h.rank).fold(f64::INFINITY, f64::min);
    let max = hits
        .iter()
        .map(|h| h.rank)
        .fold(f64::NEG_INFINITY, f64::max);
    for hit in hits.iter_mut() {
        hit.score = if (max - min).abs() < f64::EPSILON {
            1.0
        } else {
            (hit.rank - min) / (max - min)
        };
    }
}

pub fn apply_semantic_scores(hits: &mut [SearchHit]) {
    for hit in hits.iter_mut() {
        hit.score = semantic_score(hit.rank);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use maidan_types::*;
    use uuid::Uuid;

    fn hit(rank: f64) -> SearchHit {
        let id = Uuid::new_v4();
        SearchHit {
            message_id: MessageId(id),
            thread_id: ThreadId(id),
            channel_id: ChannelId(id),
            workspace_id: WorkspaceId(id),
            author_id: MemberId(id),
            posted_at: Utc::now(),
            body: String::new(),
            snippet: String::new(),
            rank,
            score: 0.0,
            embedding_model: None,
        }
    }

    #[test]
    fn lexical_normalization_spans_zero_to_one() {
        let mut hits = vec![hit(1.0), hit(3.0), hit(2.0)];
        normalize_lexical_scores(&mut hits);
        assert!((hits[0].score - 0.0).abs() < 1e-9);
        assert!((hits[1].score - 1.0).abs() < 1e-9);
        assert!((hits[2].score - 0.5).abs() < 1e-9);
    }

    #[test]
    fn single_lexical_hit_scores_one() {
        let mut hits = vec![hit(0.42)];
        normalize_lexical_scores(&mut hits);
        assert!((hits[0].score - 1.0).abs() < 1e-9);
    }
}

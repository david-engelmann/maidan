//! Normalized search scores comparable across Postgres and SQLite within a mode.

use std::collections::hash_map::Entry;
use std::collections::HashMap;

use maidan_types::MessageId;

use crate::hit::SearchHit;

/// Default semantic weight for hybrid fusion (`0.5` = equal parts).
pub const DEFAULT_HYBRID_WEIGHT: f64 = 0.5;

/// Fuse normalized lexical and semantic results into one ranking by a weighted
/// sum of their `[0, 1]` scores: `combined = weight*semantic + (1-weight)*lexical`.
/// A result present in only one side contributes `0` on the other. `weight` is
/// the semantic weight, clamped to `[0, 1]`. Returns the top `limit` by combined
/// score; ties break by `posted_at` descending for deterministic ordering.
///
/// Inputs must already be normalized (`SearchHit::score` in `[0, 1]`) — i.e.
/// the output of [`normalize_lexical_scores`] / [`apply_semantic_scores`].
pub fn fuse_hybrid(
    lexical: Vec<SearchHit>,
    semantic: Vec<SearchHit>,
    weight: f64,
    limit: usize,
) -> Vec<SearchHit> {
    let weight = weight.clamp(0.0, 1.0);
    // message_id -> (representative hit, lexical_score, semantic_score).
    let mut merged: HashMap<MessageId, (SearchHit, f64, f64)> = HashMap::new();

    for hit in lexical {
        let score = hit.score;
        let id = hit.message_id;
        // The lexical hit is the representative: it carries the FTS snippet.
        merged.entry(id).or_insert((hit, 0.0, 0.0)).1 = score;
    }
    for hit in semantic {
        let score = hit.score;
        match merged.entry(hit.message_id) {
            Entry::Occupied(mut e) => {
                let slot = e.get_mut();
                slot.2 = score;
                // Tag the hybrid hit with the model that matched semantically.
                slot.0.embedding_model = hit.embedding_model;
            }
            Entry::Vacant(e) => {
                e.insert((hit, 0.0, score));
            }
        }
    }

    let mut out: Vec<SearchHit> = merged
        .into_values()
        .map(|(mut hit, lex, sem)| {
            hit.score = weight * sem + (1.0 - weight) * lex;
            hit
        })
        .collect();
    out.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.posted_at.cmp(&a.posted_at))
    });
    out.truncate(limit);
    out
}

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

    fn scored(score: f64, model: Option<&str>) -> SearchHit {
        let mut h = hit(score);
        h.score = score;
        h.embedding_model = model.map(str::to_string);
        h
    }

    #[test]
    fn fuse_weights_overlapping_hit_above_single_sided_ones() {
        let lex = scored(1.0, None); // strong lexical only
        let sem = scored(1.0, Some("m")); // strong semantic only
        let mut both_lex = scored(0.6, None);
        let mut both_sem = scored(0.6, Some("m"));
        both_lex.message_id = MessageId(uuid::Uuid::nil());
        both_sem.message_id = both_lex.message_id; // same message in both sides

        let fused = fuse_hybrid(
            vec![lex, both_lex],
            vec![sem, both_sem],
            DEFAULT_HYBRID_WEIGHT,
            10,
        );

        // The overlapping message scores 0.5*0.6 + 0.5*0.6 = 0.6; each
        // single-sided hit scores 0.5*1.0 = 0.5. Overlap wins.
        assert_eq!(fused.len(), 3);
        assert_eq!(fused[0].message_id, MessageId(uuid::Uuid::nil()));
        assert!((fused[0].score - 0.6).abs() < 1e-9);
        assert!((fused[1].score - 0.5).abs() < 1e-9);
        // Overlapping hit is tagged with the semantic model.
        assert_eq!(fused[0].embedding_model.as_deref(), Some("m"));
    }

    #[test]
    fn fuse_weight_extremes_reduce_to_single_mode() {
        let lex = scored(0.9, None);
        let sem = scored(0.2, Some("m"));

        let semantic_only = fuse_hybrid(vec![lex.clone()], vec![sem.clone()], 1.0, 10);
        // weight=1.0 → pure semantic: the lexical-only hit scores 0.
        let lex_hit = semantic_only
            .iter()
            .find(|h| h.message_id == lex.message_id)
            .unwrap();
        assert!((lex_hit.score - 0.0).abs() < 1e-9);

        let lexical_only = fuse_hybrid(vec![lex.clone()], vec![sem.clone()], 0.0, 10);
        let lex_hit = lexical_only
            .iter()
            .find(|h| h.message_id == lex.message_id)
            .unwrap();
        assert!((lex_hit.score - 0.9).abs() < 1e-9);
    }

    #[test]
    fn fuse_truncates_to_limit() {
        let lex: Vec<SearchHit> = (0..5).map(|i| scored(0.1 * i as f64, None)).collect();
        let fused = fuse_hybrid(lex, vec![], DEFAULT_HYBRID_WEIGHT, 3);
        assert_eq!(fused.len(), 3);
        // Sorted by combined score descending.
        assert!(fused[0].score >= fused[1].score && fused[1].score >= fused[2].score);
    }
}

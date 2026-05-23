//! Deterministic embedding generation for v0.3.0 (no ML model).
//!
//! Produces a stable 1024-d vector from message body bytes so the indexer
//! pipeline and semantic search can be exercised end-to-end. Replace with
//! a real model in a later cluster.

use sha2::{Digest, Sha256};

use crate::postgres::EMBEDDING_DIM;

const MODEL_NAME: &str = "hash-v1";

pub fn model_name() -> &'static str {
    MODEL_NAME
}

/// SHA-256–derived pseudo-embedding normalized to `[0, 1]` per dimension.
pub fn hash_embedding(body: &str) -> Vec<f32> {
    let digest = Sha256::digest(body.as_bytes());
    digest
        .iter()
        .cycle()
        .take(EMBEDDING_DIM)
        .map(|b| f32::from(*b) / 255.0)
        .collect()
}

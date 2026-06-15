//! HNSW (pgvector) index/query tuning knobs (Cluster 109).
//!
//! All three are optional; `None` means "leave pgvector's default", so the
//! defaults preserve current behavior. `m` and `ef_construction` apply at index
//! build time (changing them only affects indexes built afterward — rebuild via
//! the reindex job to apply); `ef_search` is a per-query GUC set with
//! `SET LOCAL hnsw.ef_search`.

/// pgvector HNSW parameters, sourced from `MAIDAN_HNSW_M`,
/// `MAIDAN_HNSW_EF_CONSTRUCTION`, and `MAIDAN_HNSW_EF_SEARCH`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HnswParams {
    /// Max connections per layer (pgvector default 16).
    pub m: Option<u32>,
    /// Candidate list size during build (pgvector default 64).
    pub ef_construction: Option<u32>,
    /// Candidate list size at query time (pgvector default 40).
    pub ef_search: Option<u32>,
}

impl HnswParams {
    pub fn from_env() -> Self {
        Self {
            m: env_u32("MAIDAN_HNSW_M"),
            ef_construction: env_u32("MAIDAN_HNSW_EF_CONSTRUCTION"),
            ef_search: env_u32("MAIDAN_HNSW_EF_SEARCH"),
        }
    }

    /// The `WITH (...)` clause for `CREATE INDEX … USING hnsw`, or an empty
    /// string when neither build param is set (pgvector defaults apply).
    pub fn build_with_clause(&self) -> String {
        let mut opts = Vec::new();
        if let Some(m) = self.m {
            opts.push(format!("m = {m}"));
        }
        if let Some(efc) = self.ef_construction {
            opts.push(format!("ef_construction = {efc}"));
        }
        if opts.is_empty() {
            String::new()
        } else {
            format!(" WITH ({})", opts.join(", "))
        }
    }
}

fn env_u32(name: &str) -> Option<u32> {
    std::env::var(name)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .filter(|&n| n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_params_emit_no_with_clause() {
        assert_eq!(HnswParams::default().build_with_clause(), "");
    }

    #[test]
    fn build_clause_includes_only_set_params() {
        let p = HnswParams {
            m: Some(32),
            ef_construction: None,
            ef_search: Some(100),
        };
        assert_eq!(p.build_with_clause(), " WITH (m = 32)");

        let both = HnswParams {
            m: Some(24),
            ef_construction: Some(128),
            ef_search: None,
        };
        assert_eq!(
            both.build_with_clause(),
            " WITH (m = 24, ef_construction = 128)"
        );
    }
}

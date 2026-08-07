//! Relevance eval harness (Cluster 118): a small labeled corpus + a controlled
//! embedding so lexical / semantic / hybrid rankings are deterministic and
//! comparable. Guards against ranking regressions — especially that hybrid
//! never recalls *fewer* relevant docs than either single mode, and recovers
//! synonym matches that pure lexical search misses.
//!
//! The `SynonymProvider` embeds text as an L2-normalized bag-of-concepts over a
//! tiny vocabulary with synonym folding (car/automobile/sedan → one dim), so
//! cosine similarity reflects shared concepts rather than shared tokens. This
//! is what lets semantic search match "automobile" for the query "car" while
//! FTS (lexical) cannot.

use std::collections::HashSet;
use std::sync::Arc;

use maidan_search::{
    embedding_provider::{EmbeddingProvider, EmbeddingProviderError},
    sqlite_pool_options, Search, SearchFilters, SqliteSearch,
};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{
    MemberKind, MessageId, NewChannel, NewMember, NewMessage, NewThread, NewWorkspace, WorkspaceId,
};

const DIM: usize = 5;
const OTHER: usize = 4; // catch-all for tokens outside the concept vocab

/// Map a token to its concept dimension (synonyms share a dim).
fn concept_dim(token: &str) -> usize {
    match token {
        "car" | "automobile" | "sedan" | "vehicle" | "truck" => 0,
        "dog" | "canine" | "puppy" | "hound" => 1,
        "payment" | "invoice" | "billing" | "money" | "loan" => 2,
        "satellite" | "orbit" | "galaxy" | "planet" | "planets" => 3,
        _ => OTHER,
    }
}

struct SynonymProvider;

impl EmbeddingProvider for SynonymProvider {
    fn model_name(&self) -> &str {
        "synonym-eval"
    }
    fn dimension(&self) -> usize {
        DIM
    }
    fn embed(&self, body: &str) -> Result<Vec<f32>, EmbeddingProviderError> {
        let mut v = vec![0.0f32; DIM];
        for token in body
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|t| !t.is_empty())
        {
            v[concept_dim(&token.to_ascii_lowercase())] += 1.0;
        }
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for x in &mut v {
                *x /= norm;
            }
        }
        Ok(v)
    }
}

struct Corpus {
    search: SqliteSearch,
    provider: SynonymProvider,
    workspace_id: WorkspaceId,
    /// message bodies in insertion order; index → MessageId.
    ids: Vec<MessageId>,
}

/// `recall@k` = fraction of the relevant set that appears in the hits.
fn recall(hits: &[maidan_search::SearchHit], relevant: &HashSet<MessageId>) -> f64 {
    if relevant.is_empty() {
        return 1.0;
    }
    let found = hits
        .iter()
        .filter(|h| relevant.contains(&h.message_id))
        .count();
    found as f64 / relevant.len() as f64
}

/// Reciprocal rank of the first relevant hit (0 if none).
fn reciprocal_rank(hits: &[maidan_search::SearchHit], relevant: &HashSet<MessageId>) -> f64 {
    for (i, h) in hits.iter().enumerate() {
        if relevant.contains(&h.message_id) {
            return 1.0 / (i as f64 + 1.0);
        }
    }
    0.0
}

async fn build_corpus(bodies: &[&str]) -> Corpus {
    let pool = sqlite_pool_options()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
    let search = SqliteSearch::new(pool.clone());
    let provider = SynonymProvider;

    let ws = store
        .create_workspace(NewWorkspace {
            name: "eval".into(),
        })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "a".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let ch = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "general".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let th = store
        .create_thread(NewThread {
            channel_id: ch.id,
            parent_thread_id: None,
            title: None,
        })
        .await
        .unwrap();

    let mut ids = Vec::new();
    for body in bodies {
        let m = store
            .post_message(NewMessage {
                thread_id: th.id,
                author_id: member.id,
                body: (*body).into(),
                metadata: serde_json::json!({}),
                content: None,
            })
            .await
            .unwrap();
        let embedding = provider.embed(body).unwrap();
        search
            .upsert_embedding(m.id, provider.model_name(), &embedding)
            .await
            .unwrap();
        ids.push(m.id);
    }

    Corpus {
        search,
        provider,
        workspace_id: ws.id,
        ids,
    }
}

impl Corpus {
    async fn lexical(&self, query: &str, k: i64) -> Vec<maidan_search::SearchHit> {
        self.search
            .search_messages(self.workspace_id, query, k, &SearchFilters::default())
            .await
            .unwrap()
    }
    async fn semantic(&self, query: &str, k: i64) -> Vec<maidan_search::SearchHit> {
        let e = self.provider.embed(query).unwrap();
        self.search
            .semantic_search(
                self.workspace_id,
                &e,
                k,
                &SearchFilters::default(),
                self.provider.model_name(),
            )
            .await
            .unwrap()
    }
    async fn hybrid(&self, query: &str, k: i64) -> Vec<maidan_search::SearchHit> {
        let e = self.provider.embed(query).unwrap();
        self.search
            .hybrid_search(
                self.workspace_id,
                query,
                &e,
                k,
                &SearchFilters::default(),
                self.provider.model_name(),
                maidan_search::DEFAULT_HYBRID_WEIGHT,
            )
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn hybrid_dominates_lexical_and_semantic_recall() {
    // 0:car(literal) 1:automobile(synonym) 2:sedan/vehicle(synonym)
    // 3:dog(literal) 4:puppy/hound(synonym) 5:payment(literal)
    // 6:satellite/orbit 7:galaxy/planets
    let bodies = [
        "I bought a new car yesterday",
        "The automobile broke down on the highway",
        "This sedan is a reliable vehicle",
        "My dog loves the park",
        "The puppy and the hound played",
        "Send the payment invoice by friday",
        "The satellite reached orbit safely",
        "A galaxy far away has many planets",
    ];
    let c = build_corpus(&bodies).await;
    let k = 10;

    // (query, relevant indices)
    let queries: [(&str, &[usize]); 3] = [
        ("car", &[0, 1, 2]), // vehicle concept; synonyms 1,2 are lexical misses
        ("dog", &[3, 4]),    // animal concept; synonym 4 is a lexical miss
        ("payment", &[5]),   // finance concept; literal match
    ];

    let mut lex_recall = 0.0;
    let mut sem_recall = 0.0;
    let mut hyb_recall = 0.0;
    let mut hyb_rr = 0.0;
    let n = queries.len() as f64;

    for (query, rel_idx) in queries {
        let relevant: HashSet<MessageId> = rel_idx.iter().map(|&i| c.ids[i]).collect();
        let lex = c.lexical(query, k).await;
        let sem = c.semantic(query, k).await;
        let hyb = c.hybrid(query, k).await;

        let (lr, sr, hr) = (
            recall(&lex, &relevant),
            recall(&sem, &relevant),
            recall(&hyb, &relevant),
        );

        // Per-query dominance: hybrid (a re-ranked union) never recalls fewer
        // relevant docs than either single mode.
        assert!(
            hr >= lr - 1e-9 && hr >= sr - 1e-9,
            "hybrid recall {hr} should dominate lexical {lr} / semantic {sr} for {query:?}"
        );
        // Hybrid's top hit is relevant (ranking-quality guard).
        assert!(
            reciprocal_rank(&hyb, &relevant) >= 1.0 - 1e-9,
            "hybrid top hit should be relevant for {query:?}"
        );

        lex_recall += lr / n;
        sem_recall += sr / n;
        hyb_recall += hr / n;
        hyb_rr += reciprocal_rank(&hyb, &relevant) / n;
    }

    // Lexical misses synonym docs, so its aggregate recall trails; hybrid
    // recovers them and ties/beats semantic.
    assert!(
        hyb_recall >= sem_recall - 1e-9,
        "hybrid {hyb_recall} >= semantic {sem_recall}"
    );
    assert!(
        hyb_recall > lex_recall + 1e-9,
        "hybrid {hyb_recall} should strictly beat lexical {lex_recall} (synonym recall)"
    );
    assert!(
        hyb_recall >= 0.99,
        "hybrid should recall ~all relevant docs, got {hyb_recall}"
    );
    assert!(hyb_rr >= 0.99, "hybrid MRR floor, got {hyb_rr}");
}

#[tokio::test]
async fn hybrid_recovers_synonym_doc_that_lexical_misses() {
    let bodies = [
        "I bought a new car yesterday", // literal "car"
        "The automobile broke down",    // synonym only — lexical "car" misses this
    ];
    let c = build_corpus(&bodies).await;
    let automobile = c.ids[1];

    let lex = c.lexical("car", 10).await;
    assert!(
        !lex.iter().any(|h| h.message_id == automobile),
        "lexical search for 'car' must NOT match the 'automobile' doc"
    );

    let hyb = c.hybrid("car", 10).await;
    assert!(
        hyb.iter().any(|h| h.message_id == automobile),
        "hybrid search must recover the synonym 'automobile' doc"
    );
}

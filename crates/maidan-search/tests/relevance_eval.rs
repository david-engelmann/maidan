//! Relevance eval harness: a small labeled corpus + a controlled embedding so
//! lexical / semantic / hybrid rankings are deterministic and comparable.
//! Guards against ranking regressions — especially that hybrid never recalls
//! *fewer* relevant docs than either single mode, and recovers synonym matches
//! that pure lexical search misses.
//!
//! The gate test (`the_gate_holds`) scores every mode against graded
//! judgments (recall, MRR and nDCG at k=5 and k=10) and fails when any score drops
//! below `relevance_baseline.json` — a ranking change that quietly costs
//! quality goes red here instead of shipping. Raise the baseline only
//! deliberately, with `MAIDAN_WRITE_BASELINE=1`, and say why in the PR.
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
use maidan_store::{prelude::*, run_sqlite_migrations};
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
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
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
            description: None,
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

// ---------------------------------------------------------------------------
// The quality gate: a fixed corpus, graded judgments, recorded baselines.
// ---------------------------------------------------------------------------

/// The gate corpus. Indices are load-bearing — `GATE_QUERIES` judges them.
/// Mixed-concept documents (a truck carrying loan money, billing for a
/// satellite) are the interesting cases: each mode ranks them differently,
/// which is what gives the metrics texture below 1.0.
const GATE_BODIES: [&str; 20] = [
    "I bought a new car yesterday",                 // 0  vehicle, literal
    "The automobile broke down on the highway",     // 1  vehicle, synonym
    "This sedan is a reliable vehicle",             // 2  vehicle, synonyms
    "My dog loves the park",                        // 3  dog, literal
    "The puppy and the hound played",               // 4  dog, synonyms
    "Send the payment invoice by friday",           // 5  money, literals
    "The satellite reached orbit safely",           // 6  space, literals
    "A galaxy far away has many planets",           // 7  space, literals
    "The truck carried the loan money to the bank", // 8  vehicle + money
    "Billing for the satellite launch is due",      // 9  money + space
    "A canine companion for the long drive",        // 10 dog, synonym
    "The car needs new tires and the dog needs a walk", // 11 vehicle + dog
    "Orbit the planet in a small vehicle",          // 12 space + vehicle
    "Money money money, the invoice is all money",  // 13 money, heavy
    "He drove the carpool route past the observatory", // 14 distractor: "carpool" is neither token nor concept "car"
    "The hound chased the truck down the road",        // 15 dog + vehicle, synonyms
    "A loan for a satellite to study the galaxy",      // 16 money + space
    "Three puppy brothers and one puppy sister",       // 17 dog, synonym repeated
    "Payment plans for every vehicle on the lot",      // 18 money + vehicle
    "The invoice for the automobile repairs",          // 19 money + vehicle, synonym
];

/// Graded judgments: (query, [(document index, grade)]). Grade 2 is squarely
/// about the query, grade 1 mentions it alongside something else. Recall and
/// MRR count grade >= 1 as relevant; nDCG uses the grades.
const GATE_QUERIES: [(&str, &[(usize, u32)]); 5] = [
    (
        "car",
        &[
            (0, 2),
            (1, 2),
            (2, 2),
            (8, 1),
            (11, 1),
            (12, 1),
            (15, 1),
            (18, 1),
            (19, 1),
        ],
    ),
    ("dog", &[(3, 2), (4, 2), (10, 2), (17, 2), (11, 1), (15, 1)]),
    (
        "payment",
        &[(5, 2), (13, 2), (8, 1), (9, 1), (16, 1), (18, 1), (19, 1)],
    ),
    ("satellite", &[(6, 2), (7, 1), (9, 1), (12, 1), (16, 1)]),
    ("invoice", &[(5, 2), (13, 2), (19, 2), (9, 1), (16, 1)]),
];

/// One mode's gate scores, averaged over the query set, at both cutoffs.
/// At k=10 a 20-document corpus saturates recall for the concept embedder;
/// k=5 is where the ordering quality shows.
#[derive(Clone, Copy, Default)]
struct ModeScores {
    recall5: f64,
    mrr5: f64,
    ndcg5: f64,
    recall10: f64,
    mrr10: f64,
    ndcg10: f64,
}

impl ModeScores {
    fn fields(&self) -> [(&'static str, f64); 6] {
        [
            ("recall@5", self.recall5),
            ("mrr@5", self.mrr5),
            ("ndcg@5", self.ndcg5),
            ("recall@10", self.recall10),
            ("mrr@10", self.mrr10),
            ("ndcg@10", self.ndcg10),
        ]
    }

    fn fields_mut(&mut self) -> [(&'static str, &mut f64); 6] {
        [
            ("recall@5", &mut self.recall5),
            ("mrr@5", &mut self.mrr5),
            ("ndcg@5", &mut self.ndcg5),
            ("recall@10", &mut self.recall10),
            ("mrr@10", &mut self.mrr10),
            ("ndcg@10", &mut self.ndcg10),
        ]
    }
}

/// nDCG@k over graded judgments: unjudged documents gain 0.
fn ndcg(
    hits: &[maidan_search::SearchHit],
    grades: &std::collections::HashMap<MessageId, u32>,
) -> f64 {
    let gain = |rel: u32| 2f64.powi(rel as i32) - 1.0;
    let dcg: f64 = hits
        .iter()
        .enumerate()
        .map(|(i, h)| {
            gain(grades.get(&h.message_id).copied().unwrap_or(0)) / (i as f64 + 2.0).log2()
        })
        .sum();
    let mut ideal: Vec<u32> = grades.values().copied().collect();
    ideal.sort_unstable_by(|a, b| b.cmp(a));
    let idcg: f64 = ideal
        .iter()
        .enumerate()
        .map(|(i, &rel)| gain(rel) / (i as f64 + 2.0).log2())
        .sum();
    if idcg == 0.0 {
        1.0
    } else {
        dcg / idcg
    }
}

/// Score all three modes over the gate corpus and judgments.
async fn evaluate(c: &Corpus) -> [(&'static str, ModeScores); 3] {
    let mut totals = [
        ("lexical", ModeScores::default()),
        ("semantic", ModeScores::default()),
        ("hybrid", ModeScores::default()),
    ];
    for (query, judged) in GATE_QUERIES {
        let grades: std::collections::HashMap<MessageId, u32> =
            judged.iter().map(|&(i, g)| (c.ids[i], g)).collect();
        let relevant: HashSet<MessageId> = grades.keys().copied().collect();
        for k in [5_i64, 10] {
            let modes = [
                c.lexical(query, k).await,
                c.semantic(query, k).await,
                c.hybrid(query, k).await,
            ];
            for (slot, hits) in totals.iter_mut().zip(modes.iter()) {
                let (r, m, n) = (
                    recall(hits, &relevant),
                    reciprocal_rank(hits, &relevant),
                    ndcg(hits, &grades),
                );
                if k == 5 {
                    slot.1.recall5 += r;
                    slot.1.mrr5 += m;
                    slot.1.ndcg5 += n;
                } else {
                    slot.1.recall10 += r;
                    slot.1.mrr10 += m;
                    slot.1.ndcg10 += n;
                }
            }
        }
    }
    let n = GATE_QUERIES.len() as f64;
    for slot in totals.iter_mut() {
        for (_, v) in slot.1.fields_mut() {
            *v /= n;
        }
    }
    totals
}

fn baseline_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/relevance_baseline.json")
}

fn baseline_json(scores: &[(&'static str, ModeScores); 3]) -> serde_json::Value {
    let mut modes = serde_json::Map::new();
    for (mode, s) in scores {
        let mut fields = serde_json::Map::new();
        for (metric, actual) in s.fields() {
            // Full precision on purpose: a rounded floor can sit above the
            // value it was recorded from, and the gate would then fail on
            // its own baseline.
            fields.insert(metric.to_string(), serde_json::json!(actual));
        }
        modes.insert((*mode).to_string(), serde_json::Value::Object(fields));
    }
    serde_json::Value::Object(modes)
}

#[tokio::test]
async fn the_gate_holds() {
    let c = build_corpus(&GATE_BODIES).await;
    let scores = evaluate(&c).await;

    let mut report = String::from("search quality gate (5 judged queries):\n");
    for (mode, s) in &scores {
        report.push_str(&format!("  {mode:<9}"));
        for (metric, actual) in s.fields() {
            report.push_str(&format!(" {metric} {actual:.4}"));
        }
        report.push('\n');
    }
    println!("{report}");

    if std::env::var_os("MAIDAN_WRITE_BASELINE").is_some() {
        let text = serde_json::to_string_pretty(&baseline_json(&scores)).unwrap();
        std::fs::write(baseline_path(), format!("{text}\n")).unwrap();
        println!("baseline rewritten from the scores above");
        return;
    }

    let baseline: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(baseline_path()).unwrap()).unwrap();
    let mut failures = Vec::new();
    for (mode, s) in &scores {
        for (metric, actual) in s.fields() {
            let floor = baseline[mode][metric].as_f64().unwrap_or_else(|| {
                panic!("baseline has no {mode}.{metric}; regenerate with MAIDAN_WRITE_BASELINE=1")
            });
            if actual < floor - 1e-9 {
                failures.push(format!(
                    "{mode}.{metric} regressed: {actual:.4} < baseline {floor:.4}"
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "search quality gate failed:\n{}\n\n{report}\nIf this drop is the intended cost of a change, say so in the PR and regenerate the baseline with MAIDAN_WRITE_BASELINE=1.",
        failures.join("\n")
    );
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

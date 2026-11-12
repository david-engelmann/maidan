//! Message search (lexical + semantic) tool handler.

use std::sync::Arc;

use maidan_auth::AuthContext;
use maidan_store::Store;
use serde::Deserialize;
use serde_json::Value;

use maidan_types::*;

use super::content_json;
use crate::error::McpError;

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SearchMessagesMode {
    #[default]
    Lexical,
    Semantic,
    Hybrid,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchMessagesArgs {
    workspace_id: uuid::Uuid,
    query: String,
    #[serde(default)]
    mode: SearchMessagesMode,
    #[serde(default = "default_search_limit")]
    limit: i64,
    author_id: Option<uuid::Uuid>,
    channel_id: Option<uuid::Uuid>,
    kind: Option<maidan_types::MemberKind>,
    /// Date-range facet: RFC 3339 lower/upper bounds on `posted_at` — a
    /// half-open `[after, before)` window.
    after: Option<chrono::DateTime<chrono::Utc>>,
    before: Option<chrono::DateTime<chrono::Utc>>,
    embedding_model: Option<String>,
    hybrid_weight: Option<f64>,
    /// Drop full `body` from each hit, keeping only the snippet — parity with
    /// the REST `snippet_only` param.
    #[serde(default)]
    snippet_only: bool,
}

fn default_search_limit() -> i64 {
    25
}

fn clamp_search_limit(limit: i64) -> i64 {
    limit.clamp(1, 500)
}

pub(super) async fn search_messages(
    search: &Arc<dyn maidan_search::Search>,
    embedding_provider: &Arc<dyn maidan_search::EmbeddingProvider>,
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
    meters: Option<&Arc<maidan_store::usage_meters::UsageMeterBuffer>>,
) -> Result<Value, McpError> {
    let a: SearchMessagesArgs = crate::tools::parse_args(args)?;
    let workspace_id = WorkspaceId(a.workspace_id);
    let limit = clamp_search_limit(a.limit);
    // Exclude private channels the caller can't see at the query level so they
    // don't crowd out `limit`. The thread-level `reader_id` filter is the
    // authoritative, DM-aware check, applied in SQL.
    let deny_channels =
        maidan_auth::private_channel_deny_set(store.as_ref(), auth, workspace_id).await?;
    let reader_id = if auth.bypass {
        None
    } else {
        Some(auth.member_id())
    };
    let filters = maidan_search::SearchFilters {
        author_id: a.author_id.map(maidan_types::MemberId),
        channel_id: a.channel_id.map(maidan_types::ChannelId),
        author_kind: a.kind,
        after: a.after,
        before: a.before,
        deny_channels,
        reader_id,
    };
    let hits = match a.mode {
        SearchMessagesMode::Lexical => {
            search
                .search_messages(workspace_id, &a.query, limit, &filters)
                .await?
        }
        SearchMessagesMode::Semantic => {
            if let (Some(meters), false) = (meters, auth.bypass)
                && meters.is_cut_off(workspace_id, maidan_types::UsageMeter::EmbeddingCalls)
            {
                return Err(crate::error::McpError::OverAllowance(
                    maidan_types::UsageMeter::EmbeddingCalls,
                ));
            }
            let embedding = maidan_search::embed_off_worker(embedding_provider, &a.query)
                .await
                .map_err(|e| McpError::Internal(format!("embedding generation failed: {e}")))?;
            if let (Some(meters), false) = (meters, auth.bypass) {
                meters.record(workspace_id, maidan_types::UsageMeter::EmbeddingCalls, 1);
            }
            let model = a
                .embedding_model
                .as_deref()
                .unwrap_or_else(|| embedding_provider.model_name());
            search
                .semantic_search(workspace_id, &embedding, limit, &filters, model)
                .await?
        }
        SearchMessagesMode::Hybrid => {
            if let (Some(meters), false) = (meters, auth.bypass)
                && meters.is_cut_off(workspace_id, maidan_types::UsageMeter::EmbeddingCalls)
            {
                return Err(crate::error::McpError::OverAllowance(
                    maidan_types::UsageMeter::EmbeddingCalls,
                ));
            }
            let embedding = maidan_search::embed_off_worker(embedding_provider, &a.query)
                .await
                .map_err(|e| McpError::Internal(format!("embedding generation failed: {e}")))?;
            if let (Some(meters), false) = (meters, auth.bypass) {
                meters.record(workspace_id, maidan_types::UsageMeter::EmbeddingCalls, 1);
            }
            let model = a
                .embedding_model
                .as_deref()
                .unwrap_or_else(|| embedding_provider.model_name());
            let weight = a
                .hybrid_weight
                .unwrap_or(maidan_search::DEFAULT_HYBRID_WEIGHT);
            search
                .hybrid_search(
                    workspace_id,
                    &a.query,
                    &embedding,
                    limit,
                    &filters,
                    model,
                    weight,
                )
                .await?
        }
    };
    // The SQL filters by readable_thread in-query (via SearchFilters.reader_id),
    // so no post-filter is needed. Bypass mode skips the filter entirely.
    let hits = if a.snippet_only {
        hits.into_iter()
            .map(maidan_search::SearchHit::into_snippet_only)
            .collect()
    } else {
        hits
    };
    Ok(content_json(
        &with_names(store.as_ref(), workspace_id, hits).await?,
    ))
}

/// Each hit with the names a reader asks for: its channel's name, its thread's
/// title and its author's handle. With ids alone, a model answering "what did
/// search find?" reports UUIDs. Every hit here already passed the access
/// check, so naming its channel, thread and author tells the caller nothing it
/// cannot read. A name that no longer resolves (an erased member) is left out
/// rather than failing the search. The names come from one batched store read
/// bounded to the searched workspace, so the cost stays flat in hit count.
async fn with_names(
    store: &dyn Store,
    workspace_id: WorkspaceId,
    hits: Vec<maidan_search::SearchHit>,
) -> Result<Vec<Value>, McpError> {
    let channel_ids: Vec<ChannelId> = distinct(hits.iter().map(|h| h.channel_id));
    let thread_ids: Vec<ThreadId> = distinct(hits.iter().map(|h| h.thread_id));
    let author_ids: Vec<MemberId> = distinct(hits.iter().map(|h| h.author_id));
    let names = store
        .display_names(workspace_id, &channel_ids, &thread_ids, &author_ids)
        .await?;
    let mut named = Vec::with_capacity(hits.len());
    for hit in hits {
        let mut value = serde_json::to_value(&hit)?;
        for (key, name) in [
            ("channel_name", names.channels.get(&hit.channel_id)),
            ("thread_title", names.threads.get(&hit.thread_id)),
            ("author_handle", names.members.get(&hit.author_id)),
        ] {
            if let Some(name) = name {
                value[key] = Value::String(name.clone());
            }
        }
        named.push(value);
    }
    Ok(named)
}

fn distinct<T: Copy + Eq + std::hash::Hash>(ids: impl Iterator<Item = T>) -> Vec<T> {
    ids.collect::<std::collections::HashSet<T>>()
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use maidan_auth::AuthContext;
    use maidan_store::test_support::statements;
    use maidan_store::Store;
    use maidan_types::*;
    use serde_json::{json, Value};

    use super::{clamp_search_limit, with_names};

    fn hit(
        workspace_id: WorkspaceId,
        channel_id: ChannelId,
        thread_id: ThreadId,
        author_id: MemberId,
    ) -> maidan_search::SearchHit {
        maidan_search::SearchHit {
            message_id: MessageId(uuid::Uuid::now_v7()),
            thread_id,
            channel_id,
            workspace_id,
            author_id,
            posted_at: chrono::Utc::now(),
            body: "b".into(),
            snippet: "b".into(),
            rank: 1.0,
            score: 1.0,
            embedding_model: None,
        }
    }

    /// Names for `count` hits, each in its own private channel and thread, plus
    /// one hit naming another workspace's channel, thread and member: the
    /// output and the statements it took.
    async fn named_hits(store: &Arc<dyn Store>, count: usize) -> (Vec<Value>, usize) {
        let ws = store
            .create_workspace(NewWorkspace {
                name: format!("names-{count}"),
            })
            .await
            .unwrap();
        let other = store
            .create_workspace(NewWorkspace {
                name: format!("other-{count}"),
            })
            .await
            .unwrap();
        let make = |wsid: WorkspaceId, tag: String| {
            let store = Arc::clone(store);
            async move {
                let member = store
                    .create_member(NewMember {
                        workspace_id: wsid,
                        handle: format!("m-{tag}"),
                        display_name: None,
                        kind: MemberKind::Human,
                    })
                    .await
                    .unwrap();
                let channel = store
                    .create_channel(NewChannel {
                        workspace_id: wsid,
                        name: format!("c-{tag}"),
                        topic: None,
                        private: true,
                    })
                    .await
                    .unwrap();
                let thread = store
                    .create_thread(NewThread {
                        channel_id: channel.id,
                        parent_thread_id: None,
                        title: Some(format!("t-{tag}")),
                        description: None,
                        created_by: None,
                    })
                    .await
                    .unwrap();
                hit(wsid, channel.id, thread.id, member.id)
            }
        };
        let mut hits = Vec::new();
        for i in 0..count {
            hits.push(make(ws.id, format!("{i}")).await);
        }
        hits.push(make(other.id, "x".into()).await);
        let (named, statements) = statements::count(with_names(store.as_ref(), ws.id, hits)).await;
        (named.unwrap(), statements)
    }

    async fn run_names(store: Arc<dyn Store>) -> (usize, usize) {
        let (named, few) = named_hits(&store, 2).await;
        assert_eq!(named.len(), 3);
        assert_eq!(named[0]["channel_name"], json!("c-0"));
        assert_eq!(named[0]["thread_title"], json!("t-0"));
        assert_eq!(named[0]["author_handle"], json!("m-0"));
        assert_eq!(named[1]["author_handle"], json!("m-1"));
        // A hit naming ids of another workspace gets no names from it.
        for key in ["channel_name", "thread_title", "author_handle"] {
            assert!(named[2].get(key).is_none(), "{key} leaked: {}", named[2]);
        }
        let (named, many) = named_hits(&store, 15).await;
        assert_eq!(named[14]["thread_title"], json!("t-14"));
        (few, many)
    }

    #[tokio::test]
    async fn search_hits_are_named_only_from_the_searched_workspace_sqlite() {
        let pool = maidan_search::sqlite_pool_options()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        maidan_store::run_sqlite_migrations(&pool).await.unwrap();
        run_names(Arc::new(maidan_store::SqliteStore::for_tests(pool))).await;
    }

    #[tokio::test]
    async fn search_names_and_channel_listing_cost_one_statement_postgres() {
        use sqlx::postgres::PgPoolOptions;
        use testcontainers::{runners::AsyncRunner, ImageExt};
        use testcontainers_modules::postgres::Postgres;

        let container = match Postgres::default()
            .with_name("pgvector/pgvector")
            .with_tag("pg17")
            .start()
            .await
        {
            Ok(c) => c,
            Err(err) => {
                maidan_store::test_support::docker::skip_start_failure(err).await;
                return;
            }
        };
        let host = container.get_host().await.unwrap();
        let port = container.get_host_port_ipv4(5432).await.unwrap();
        let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .acquire_timeout(std::time::Duration::from_secs(15))
            .connect(&url)
            .await
            .unwrap();
        maidan_store::run_postgres_migrations(&pool).await.unwrap();
        let store: Arc<dyn Store> = Arc::new(maidan_store::PostgresStore::for_tests(pool));

        let (few, many) = run_names(Arc::clone(&store)).await;
        assert_eq!(few, 1, "naming 3 hits is one statement");
        assert_eq!(many, 1, "naming 16 hits is one statement");

        // MCP list_channels over a workspace of private channels.
        let ws = store
            .create_workspace(NewWorkspace {
                name: "mcp-list".into(),
            })
            .await
            .unwrap();
        let me = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: "me".into(),
                display_name: None,
                kind: MemberKind::Human,
            })
            .await
            .unwrap();
        for i in 0..10 {
            let channel = store
                .create_channel(NewChannel {
                    workspace_id: ws.id,
                    name: format!("p{i}"),
                    topic: None,
                    private: true,
                })
                .await
                .unwrap();
            if i < 3 {
                store
                    .add_channel_member(channel.id, me.id, ChannelMemberRole::Member)
                    .await
                    .unwrap();
            }
        }
        let auth = AuthContext::from_session(me.id, ws.id, Vec::new());
        let args = json!({ "workspace_id": ws.id.0 });
        let (listed, listing) =
            statements::count(crate::tools::channel::list_channels(&store, &auth, &args)).await;
        assert!(listed.is_ok());
        assert_eq!(
            listing, 1,
            "MCP list_channels over 10 private channels is one statement"
        );
    }

    #[test]
    fn search_limit_matches_the_rest_surface() {
        assert_eq!(clamp_search_limit(i64::MIN), 1);
        assert_eq!(clamp_search_limit(25), 25);
        assert_eq!(clamp_search_limit(i64::MAX), 500);
    }
}

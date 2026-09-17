//! Operator CLI for Maidan.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use clap::{Parser, Subcommand};
use maidan_artifacts::LocalFsStore;
use maidan_auth::{resolve_bearer, AuthContext};
use maidan_bus::InMemoryBus;
use maidan_mcp::{run_stdio, McpServer};
use maidan_search::{
    EmbeddingHandler, Indexer, LoggingHandler, PostgresSearch, Search, SqliteSearch,
};
use maidan_store::{
    run_postgres_migrations, run_sqlite_migrations, Dialect, PostgresStore, SqliteStore, Store,
};
use sqlx::{postgres::PgPoolOptions, sqlite::SqlitePoolOptions};

#[derive(Parser)]
#[command(name = "maidan", version, about = "Maidan operator CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// MCP JSON-RPC over stdin/stdout (line-delimited).
    #[command(name = "mcp-stdio")]
    McpStdio {
        #[arg(long, env = "DATABASE_URL", default_value = "sqlite::memory:")]
        database_url: String,
        #[arg(
            long,
            env = "ARTIFACT_LOCALFS_ROOT",
            default_value = "./.local/artifacts"
        )]
        artifact_root: PathBuf,
        /// Serve every tool with unrestricted authority because no
        /// `MAIDAN_MCP_TOKEN` was given. Required to start without one. Never
        /// use this against a database holding real work.
        ///
        /// `MAIDAN_ALLOW_INSECURE_NO_AUTH=1` does the same. It is read by value
        /// rather than by presence, matching the server's acknowledgement, so
        /// setting it to `0` turns it off instead of quietly turning it on.
        #[arg(long)]
        allow_insecure_no_auth: bool,
    },
    /// Re-embed all live messages for the configured provider model.
    #[command(name = "reindex-embeddings")]
    ReindexEmbeddings {
        #[arg(long, env = "DATABASE_URL", default_value = "sqlite::memory:")]
        database_url: String,
        #[arg(long, env = "MAIDAN_EMBEDDING_PROVIDER", default_value = "hash-v1")]
        embedding_provider: String,
        #[arg(long)]
        workspace_id: Option<uuid::Uuid>,
    },
    /// One-time first-admin bootstrap: create the initial workspace, an admin
    /// member, and an admin bearer token (printed once). Refuses if the database
    /// already has a workspace. Removes the need for public bootstrap HTTP routes
    /// or `AUTH_DISABLED` on a production deployment.
    Init {
        #[arg(long, env = "DATABASE_URL", default_value = "sqlite::memory:")]
        database_url: String,
        /// Name for the initial workspace.
        #[arg(long, default_value = "maidan")]
        workspace: String,
        /// Handle for the initial admin member.
        #[arg(long, default_value = "admin")]
        admin_handle: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("MAIDAN_LOG").unwrap_or_else(|_| "info,sqlx=warn".into()))
        .with_target(false)
        .init();

    let cli = Cli::parse();
    match cli.command {
        Commands::McpStdio {
            database_url,
            artifact_root,
            allow_insecure_no_auth,
        } => run_mcp_stdio(&database_url, &artifact_root, allow_insecure_no_auth).await,
        Commands::ReindexEmbeddings {
            database_url,
            embedding_provider,
            workspace_id,
        } => run_reindex_embeddings(&database_url, &embedding_provider, workspace_id).await,
        Commands::Init {
            database_url,
            workspace,
            admin_handle,
        } => run_init(&database_url, &workspace, &admin_handle).await,
    }
}

async fn run_init(
    database_url: &str,
    workspace_name: &str,
    admin_handle: &str,
) -> anyhow::Result<()> {
    let dialect = Dialect::from_url(database_url).context("detect dialect")?;
    let store: Arc<dyn Store> = match dialect {
        Dialect::Sqlite => {
            let pool = SqlitePoolOptions::new()
                .max_connections(maidan_store::DEFAULT_SQLITE_MAX_CONNECTIONS)
                .connect(database_url)
                .await
                .context("connect sqlite")?;
            maidan_store::configure_sqlite_pool(&pool)
                .await
                .context("configure sqlite pragmas")?;
            run_sqlite_migrations(&pool)
                .await
                .context("migrate sqlite")?;
            Arc::new(SqliteStore::new(pool))
        }
        Dialect::Postgres => {
            let pool = PgPoolOptions::new()
                .max_connections(4)
                .connect(database_url)
                .await
                .context("connect postgres")?;
            run_postgres_migrations(&pool)
                .await
                .context("migrate postgres")?;
            Arc::new(PostgresStore::new(pool))
        }
    };

    // Init is a one-time bootstrap: refuse if the store is already populated so it
    // can never clobber an existing deployment or mint a second root token.
    let existing = store.count_workspaces().await.context("count workspaces")?;
    if existing > 0 {
        anyhow::bail!(
            "refusing to init: the database already has {existing} workspace(s). \
             `maidan init` is a one-time first-admin bootstrap; mint further tokens via the API."
        );
    }

    let (workspace, _) = store
        .create_workspace_with_event(maidan_types::NewWorkspace {
            name: workspace_name.to_string(),
        })
        .await
        .context("create workspace")?;
    let (member, _) = store
        .create_member_with_event(maidan_types::NewMember {
            workspace_id: workspace.id,
            handle: admin_handle.to_string(),
            display_name: None,
            kind: maidan_types::MemberKind::Human,
        })
        .await
        .context("create admin member")?;

    let secret = maidan_auth::TokenSecret::generate();
    store
        .create_api_token(maidan_types::NewApiToken {
            workspace_id: workspace.id,
            member_id: member.id,
            app_installation_id: None,
            token_hash: maidan_auth::hash_secret(secret.as_str()),
            label: Some("maidan init (admin)".to_string()),
            capabilities: maidan_auth::capability::all(),
            expires_at: None,
        })
        .await
        .context("mint admin token")?;

    // The secret goes to stdout (logs go to stderr); it is shown exactly once.
    println!();
    println!("Maidan initialized.");
    println!("  workspace: {}  ({})", workspace.name, workspace.id.0);
    println!("  admin:     {}  ({})", member.handle, member.id.0);
    println!();
    println!("Admin bearer token (shown once — save it now):");
    println!();
    println!("    {}", secret.as_str());
    println!();
    println!("Send it as `Authorization: Bearer <token>`. It holds all capabilities;");
    println!("mint narrower per-agent tokens from it via the API. Do not run init again.");
    Ok(())
}

/// A boolean environment variable, read by value. Presence alone is not consent:
/// `MAIDAN_ALLOW_INSECURE_NO_AUTH=0` has to mean no.
fn env_truthy(name: &str) -> bool {
    matches!(
        std::env::var(name).as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE")
    )
}

/// What `mcp-stdio` will serve with, once the environment has had its say.
#[derive(Debug, PartialEq, Eq)]
enum StdioAuth {
    /// Resolve this bearer and serve with exactly its capabilities.
    Token(String),
    /// Serve every tool with unrestricted authority — asked for, not defaulted to.
    Unrestricted,
    /// Neither was given. Refuse rather than pick one.
    Refused,
}

/// Why serving MCP without a token has to be asked for.
///
/// `mcp-stdio` hosts the server itself: it opens the database and answers tool
/// calls over the pipe. Per-tool capability checks are made against one ambient
/// [`AuthContext`], so that context *is* the authorization. With no
/// `MAIDAN_MCP_TOKEN` the only context left to serve is a bypass, which every
/// check passes — and a bypass reached by leaving a variable unset is reached in
/// silence, against whatever database the caller happened to point at.
///
/// So it stays available and stops being the default. `--allow-insecure-no-auth`
/// turns it on, mirroring the server's `MAIDAN_ALLOW_INSECURE_NO_AUTH`
/// acknowledgement for `AUTH_DISABLED`.
fn plan_stdio_auth(token: Option<String>, allow_insecure_no_auth: bool) -> StdioAuth {
    match token {
        Some(token) if !token.is_empty() => StdioAuth::Token(token),
        _ if allow_insecure_no_auth => StdioAuth::Unrestricted,
        _ => StdioAuth::Refused,
    }
}

async fn resolve_stdio_auth(
    store: &dyn Store,
    allow_insecure_no_auth: bool,
) -> anyhow::Result<AuthContext> {
    match plan_stdio_auth(
        std::env::var("MAIDAN_MCP_TOKEN").ok(),
        allow_insecure_no_auth,
    ) {
        StdioAuth::Token(token) => resolve_bearer(store, &token)
            .await
            .context("resolve MAIDAN_MCP_TOKEN"),
        StdioAuth::Unrestricted => {
            tracing::warn!(
                "serving MCP with no token: every tool runs with unrestricted authority over \
                 this database. Set MAIDAN_MCP_TOKEN to scope it."
            );
            Ok(AuthContext::bypass())
        }
        StdioAuth::Refused => anyhow::bail!(
            "no MAIDAN_MCP_TOKEN. Mint one with `maidan init` (or the token API) and set it, \
             or pass --allow-insecure-no-auth to serve every tool with unrestricted authority."
        ),
    }
}

async fn run_mcp_stdio(
    database_url: &str,
    artifact_root: &Path,
    allow_insecure_no_auth: bool,
) -> anyhow::Result<()> {
    let dialect = Dialect::from_url(database_url).context("detect dialect")?;
    let (store, search) = match dialect {
        Dialect::Sqlite => {
            let pool = SqlitePoolOptions::new()
                .max_connections(4)
                .connect(database_url)
                .await
                .context("connect sqlite")?;
            maidan_store::configure_sqlite_pool(&pool)
                .await
                .context("configure sqlite pragmas")?;
            run_sqlite_migrations(&pool)
                .await
                .context("migrate sqlite")?;
            let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
            let search: Arc<dyn Search> = Arc::new(SqliteSearch::new(pool));
            (store, search)
        }
        Dialect::Postgres => {
            let pool = PgPoolOptions::new()
                .max_connections(4)
                .connect(database_url)
                .await
                .context("connect postgres")?;
            run_postgres_migrations(&pool)
                .await
                .context("migrate postgres")?;
            let store: Arc<dyn Store> = Arc::new(PostgresStore::new(pool.clone()));
            let search: Arc<dyn Search> = Arc::new(PostgresSearch::new(pool));
            (store, search)
        }
    };

    let artifacts = Arc::new(LocalFsStore::new(artifact_root.to_path_buf()));

    let auth = resolve_stdio_auth(
        store.as_ref(),
        allow_insecure_no_auth || env_truthy("MAIDAN_ALLOW_INSECURE_NO_AUTH"),
    )
    .await?;

    let embedding_provider: Arc<dyn maidan_search::EmbeddingProvider> =
        Arc::new(maidan_search::HashV1Provider);
    let bus = Arc::new(InMemoryBus::with_capacity(256));
    let indexer_handler: Arc<dyn maidan_search::EventHandler> = match dialect {
        Dialect::Sqlite => Arc::new(LoggingHandler::default()),
        Dialect::Postgres => Arc::new(EmbeddingHandler::new(
            store.clone(),
            search.clone(),
            embedding_provider.clone(),
        )),
    };
    let _indexer = Indexer::new(bus.clone(), indexer_handler).spawn();
    let server = McpServer::new(store, artifacts, search, embedding_provider).with_event_bus(bus);
    tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("stdio runtime")?;
        rt.block_on(run_stdio(&server, &auth))
            .map_err(anyhow::Error::from)
    })
    .await
    .context("stdio task join")?
    .context("mcp stdio")?;
    Ok(())
}

async fn run_reindex_embeddings(
    database_url: &str,
    provider_name: &str,
    workspace_id: Option<uuid::Uuid>,
) -> anyhow::Result<()> {
    let provider =
        maidan_search::provider_from_name(provider_name).context("embedding provider")?;
    let workspace = workspace_id.map(maidan_types::WorkspaceId);
    let dialect = Dialect::from_url(database_url).context("detect dialect")?;
    match dialect {
        Dialect::Sqlite => {
            let pool = SqlitePoolOptions::new()
                .max_connections(4)
                .connect(database_url)
                .await
                .context("connect sqlite")?;
            maidan_store::configure_sqlite_pool(&pool)
                .await
                .context("configure sqlite pragmas")?;
            run_sqlite_migrations(&pool)
                .await
                .context("migrate sqlite")?;
            let search: Arc<dyn Search> = Arc::new(SqliteSearch::new(pool.clone()));
            let report =
                maidan_search::reindex_sqlite(&pool, search.as_ref(), provider.as_ref(), workspace)
                    .await
                    .context("reindex")?;
            println!(
                "reindex complete: processed={} failed={}",
                report.processed, report.failed
            );
        }
        Dialect::Postgres => {
            let pool = PgPoolOptions::new()
                .max_connections(4)
                .connect(database_url)
                .await
                .context("connect postgres")?;
            run_postgres_migrations(&pool)
                .await
                .context("migrate postgres")?;
            let search: Arc<dyn Search> = Arc::new(PostgresSearch::new(pool.clone()));
            let report = maidan_search::reindex_postgres(
                &pool,
                search.as_ref(),
                provider.as_ref(),
                workspace,
            )
            .await
            .context("reindex")?;
            println!(
                "reindex complete: processed={} failed={}",
                report.processed, report.failed
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod stdio_auth_tests {
    use super::*;

    #[test]
    fn a_token_is_used_whether_or_not_the_insecure_flag_is_set() {
        assert_eq!(
            plan_stdio_auth(Some("t".into()), false),
            StdioAuth::Token("t".into())
        );
        assert_eq!(
            plan_stdio_auth(Some("t".into()), true),
            StdioAuth::Token("t".into())
        );
    }

    #[test]
    fn no_token_and_no_flag_is_refused_rather_than_silently_unrestricted() {
        assert_eq!(plan_stdio_auth(None, false), StdioAuth::Refused);
    }

    #[test]
    fn an_empty_token_is_no_token() {
        // `MAIDAN_MCP_TOKEN=` reads as set. Treating it as a token would send an
        // empty bearer to `resolve_bearer`; treating it as absent keeps the
        // refuse-or-acknowledge decision where it belongs.
        assert_eq!(
            plan_stdio_auth(Some(String::new()), false),
            StdioAuth::Refused
        );
        assert_eq!(
            plan_stdio_auth(Some(String::new()), true),
            StdioAuth::Unrestricted
        );
    }

    #[test]
    fn the_flag_alone_serves_unrestricted() {
        assert_eq!(plan_stdio_auth(None, true), StdioAuth::Unrestricted);
    }
}

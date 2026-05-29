//! Operator CLI for Maidan.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use clap::{Parser, Subcommand};
use maidan_artifacts::LocalFsStore;
use maidan_auth::{resolve_bearer, AuthContext};
use maidan_mcp::{run_stdio, McpServer};
use maidan_search::{PostgresSearch, Search, SqliteSearch};
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
        } => run_mcp_stdio(&database_url, &artifact_root).await,
    }
}

async fn run_mcp_stdio(database_url: &str, artifact_root: &Path) -> anyhow::Result<()> {
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

    let auth = if let Ok(token) = std::env::var("MAIDAN_MCP_TOKEN") {
        resolve_bearer(store.as_ref(), &token)
            .await
            .context("resolve MAIDAN_MCP_TOKEN")?
    } else {
        AuthContext::bypass()
    };

    let embedding_provider: Arc<dyn maidan_search::EmbeddingProvider> =
        Arc::new(maidan_search::HashV1Provider);
    let server = McpServer::new(store, artifacts, search, embedding_provider);
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

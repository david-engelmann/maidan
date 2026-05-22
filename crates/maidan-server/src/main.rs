//! Maidan server entrypoint. Real /health + routes land in PR #5.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        "maidan-server starting"
    );
    println!(
        "maidan-server {} (stub; /health lands in PR #5)",
        env!("CARGO_PKG_VERSION")
    );
    Ok(())
}

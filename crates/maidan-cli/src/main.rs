//! Operator CLI for Maidan. Real subcommands land in Cluster F.

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    println!("maidan-cli {}", env!("CARGO_PKG_VERSION"));
    Ok(())
}

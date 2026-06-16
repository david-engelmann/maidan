//! Write MCP reference markdown for mdBook (`book/src/mcp-reference.md`).

use std::{env, fs, path::PathBuf};

fn main() -> std::io::Result<()> {
    let path = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("book/src/mcp-reference.md"));

    let markdown = maidan_mcp::reference::markdown();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, markdown)?;
    eprintln!("wrote {}", path.display());
    Ok(())
}

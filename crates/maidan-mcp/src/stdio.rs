//! Line-delimited JSON-RPC on stdin/stdout (MCP stdio transport).

use std::io::{self, BufRead, Write};

use maidan_auth::AuthContext;

use crate::{JsonRpcRequest, JsonRpcResponse, McpServer};

/// Run the MCP dispatcher until stdin EOF.
pub async fn run_stdio(server: &McpServer, auth: &AuthContext) -> io::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let request: JsonRpcRequest = match serde_json::from_str(trimmed) {
            Ok(r) => r,
            Err(_) => {
                write_response(&mut stdout, JsonRpcResponse::parse_error())?;
                continue;
            }
        };
        let response = server.handle(request, auth).await;
        write_response(&mut stdout, response)?;
    }
    Ok(())
}

fn write_response(stdout: &mut impl Write, response: JsonRpcResponse) -> io::Result<()> {
    let line = serde_json::to_string(&response).map_err(|e| io::Error::other(e.to_string()))?;
    writeln!(stdout, "{line}")?;
    stdout.flush()?;
    Ok(())
}

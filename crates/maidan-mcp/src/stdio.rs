//! Line-delimited JSON-RPC on stdin/stdout (MCP stdio transport).

use std::io::{self, BufRead, Write};

use maidan_auth::AuthContext;

use crate::{JsonRpcNotification, JsonRpcResponse, McpServer, McpSession};

/// Run the MCP dispatcher until stdin EOF. The process is one session: what
/// it subscribes to is written after each response, and nothing else is.
pub async fn run_stdio(server: &McpServer, auth: &AuthContext) -> io::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut listener = server.listen(auth, McpSession::Stdio).await;
    for line in stdin.lock().lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let request = match crate::protocol::parse_request(trimmed.as_bytes()) {
            Ok(r) => r,
            Err(rejected) => {
                write_response(&mut stdout, JsonRpcResponse::rejected(rejected))?;
                continue;
            }
        };
        let response = server.handle_in(request, auth, &McpSession::Stdio).await;
        write_response(&mut stdout, response)?;
        for notification in listener.drain() {
            write_notification(&mut stdout, notification)?;
        }
    }
    Ok(())
}

fn write_response(stdout: &mut impl Write, response: JsonRpcResponse) -> io::Result<()> {
    let line = serde_json::to_string(&response).map_err(|e| io::Error::other(e.to_string()))?;
    writeln!(stdout, "{line}")?;
    stdout.flush()?;
    Ok(())
}

fn write_notification(
    stdout: &mut impl Write,
    notification: JsonRpcNotification,
) -> io::Result<()> {
    let line = serde_json::to_string(&notification).map_err(|e| io::Error::other(e.to_string()))?;
    writeln!(stdout, "{line}")?;
    stdout.flush()?;
    Ok(())
}

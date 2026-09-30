//! Line-delimited JSON-RPC on stdin/stdout (MCP stdio transport).

use std::io::{self, BufRead, Write};

use maidan_auth::AuthContext;

use crate::{JsonRpcNotification, JsonRpcResponse, McpServer, McpSession};

/// Run the MCP dispatcher until stdin EOF. The process is one session: what
/// it subscribes to is written after each response, and nothing else is.
pub async fn run_stdio(server: &McpServer, auth: &AuthContext) -> io::Result<()> {
    serve_lines(server, auth, io::stdin().lock(), &mut io::stdout()).await
}

/// One request per line of `input`, answered on `output`. A request with no
/// `id` is a notification: it runs, and JSON-RPC forbids answering it, even
/// with an error. A line that is not a request has no id to tell, so it is
/// answered with a null one.
async fn serve_lines(
    server: &McpServer,
    auth: &AuthContext,
    input: impl BufRead,
    output: &mut impl Write,
) -> io::Result<()> {
    let mut listener = server.listen(auth, McpSession::Stdio).await;
    for line in input.lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let request = match crate::protocol::parse_request(trimmed.as_bytes()) {
            Ok(r) => r,
            Err(rejected) => {
                write_response(output, JsonRpcResponse::rejected(rejected))?;
                continue;
            }
        };
        let notification = request.id.is_none();
        let response = server.handle_in(request, auth, &McpSession::Stdio).await;
        if !notification {
            write_response(output, response)?;
        }
        for notification in listener.drain() {
            write_notification(output, notification)?;
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use maidan_artifacts::LocalFsStore;
    use maidan_search::HashV1Provider;
    use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
    use serde_json::Value;
    use sqlx::sqlite::SqlitePoolOptions;

    use super::*;

    #[tokio::test]
    async fn a_notification_on_stdio_gets_no_response() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        run_sqlite_migrations(&pool).await.unwrap();
        let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
        let dir = tempfile::tempdir().unwrap();
        let server = McpServer::new(
            store,
            Arc::new(LocalFsStore::new(dir.path())),
            Arc::new(maidan_search::SqliteSearch::new(pool)),
            Arc::new(HashV1Provider),
        );

        let input = [
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            r#"{"jsonrpc":"2.0","method":"no/such/method"}"#,
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
            r#"{"jsonrpc":"2.0","method":"tools/call","params":{"name":"no_such_tool"}}"#,
            r#"{not json"#,
            r#"{"jsonrpc":"2.0","id":"two","method":"no/such/method"}"#,
        ]
        .join("\n");
        let mut output = Vec::new();
        serve_lines(
            &server,
            &AuthContext::bypass(),
            input.as_bytes(),
            &mut output,
        )
        .await
        .unwrap();

        let answers: Vec<Value> = String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let ids: Vec<&Value> = answers.iter().map(|a| &a["id"]).collect();
        assert_eq!(
            ids,
            [&Value::from(1), &Value::Null, &Value::from("two")],
            "{answers:?}"
        );
        assert!(answers[0]["result"]["tools"].is_array());
        assert_eq!(answers[1]["error"]["code"], -32700);
        assert_eq!(answers[2]["error"]["code"], -32601);
    }
}

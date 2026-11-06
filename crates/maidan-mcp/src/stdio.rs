//! Line-delimited JSON-RPC on stdin/stdout (MCP stdio transport).

use std::io::{self, BufRead, Write};

use maidan_auth::AuthContext;

use crate::{JsonRpcError, JsonRpcNotification, JsonRpcResponse, McpServer, McpSession};

/// The largest stdin line `mcp-stdio` will buffer. A request past this is
/// rejected and the rest of the line discarded, so one stream without a
/// newline cannot grow the process without bound. 16 MiB still covers a
/// 10 MiB base64 artifact upload.
pub const MAX_STDIO_LINE_BYTES: usize = 16 * 1024 * 1024;

const LINE_BUF_IDLE_CAP: usize = 64 * 1024;

enum LineRead {
    Eof,
    Line,
    TooLong,
}

/// Read one `\n`-terminated line into `buf`, or stop once `buf` would pass
/// `max_bytes`. An oversized line is drained through its newline (or EOF) and
/// reported as [`LineRead::TooLong`] with `buf` cleared, so the next call
/// starts at the following line.
fn read_bounded_line(
    input: &mut impl BufRead,
    buf: &mut Vec<u8>,
    max_bytes: usize,
) -> io::Result<LineRead> {
    buf.clear();
    loop {
        let available = input.fill_buf()?;
        if available.is_empty() {
            return Ok(if buf.is_empty() {
                LineRead::Eof
            } else {
                LineRead::Line
            });
        }
        if let Some(pos) = available.iter().position(|byte| *byte == b'\n') {
            let take = pos + 1;
            if buf.len() + take > max_bytes {
                input.consume(take);
                buf.clear();
                return Ok(LineRead::TooLong);
            }
            buf.extend_from_slice(&available[..take]);
            input.consume(take);
            return Ok(LineRead::Line);
        }
        if buf.len() + available.len() > max_bytes {
            let n = available.len();
            input.consume(n);
            buf.clear();
            loop {
                let available = input.fill_buf()?;
                if available.is_empty() {
                    return Ok(LineRead::TooLong);
                }
                if let Some(pos) = available.iter().position(|byte| *byte == b'\n') {
                    input.consume(pos + 1);
                    return Ok(LineRead::TooLong);
                }
                let n = available.len();
                input.consume(n);
            }
        }
        buf.extend_from_slice(available);
        let n = available.len();
        input.consume(n);
    }
}

fn shrink_line_buf(buf: &mut Vec<u8>) {
    if buf.capacity() > LINE_BUF_IDLE_CAP {
        buf.clear();
        buf.shrink_to(LINE_BUF_IDLE_CAP);
    }
}

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
    mut input: impl BufRead,
    output: &mut impl Write,
) -> io::Result<()> {
    let mut listener = server.listen(auth, McpSession::Stdio).await;
    let mut buf = Vec::new();
    loop {
        match read_bounded_line(&mut input, &mut buf, MAX_STDIO_LINE_BYTES)? {
            LineRead::Eof => break,
            LineRead::TooLong => {
                write_response(
                    output,
                    JsonRpcResponse::rejected(JsonRpcError::invalid_request(
                        "request line exceeds maximum length",
                    )),
                )?;
                for notification in listener.drain() {
                    write_notification(output, notification)?;
                }
                shrink_line_buf(&mut buf);
            }
            LineRead::Line => {
                let trimmed = buf.trim_ascii();
                if trimmed.is_empty() {
                    shrink_line_buf(&mut buf);
                    continue;
                }
                // Bytes, not a `String`: invalid UTF-8 is a JSON-RPC parse
                // error for this line, and the session keeps reading.
                let request = match crate::protocol::parse_request(trimmed) {
                    Ok(r) => r,
                    Err(rejected) => {
                        write_response(output, JsonRpcResponse::rejected(rejected))?;
                        shrink_line_buf(&mut buf);
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
                shrink_line_buf(&mut buf);
            }
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
    use maidan_auth::capability;
    use maidan_search::HashV1Provider;
    use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
    use maidan_types::{MemberKind, NewChannel, NewMember, NewThread, NewWorkspace};
    use serde_json::{json, Value};

    use super::*;

    async fn sqlite_server() -> (McpServer, Arc<dyn Store>, tempfile::TempDir) {
        let pool = maidan_search::sqlite_pool_options()
            .max_connections(4)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        run_sqlite_migrations(&pool).await.unwrap();
        let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
        let dir = tempfile::tempdir().unwrap();
        let server = McpServer::new(
            store.clone(),
            Arc::new(LocalFsStore::new(dir.path())),
            Arc::new(maidan_search::SqliteSearch::new(pool)),
            Arc::new(HashV1Provider),
        );
        (server, store, dir)
    }

    fn answers(output: Vec<u8>) -> Vec<Value> {
        String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[tokio::test]
    async fn a_notification_on_stdio_gets_no_response() {
        let (server, _store, _dir) = sqlite_server().await;

        let input = [
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            r#"{"jsonrpc":"2.0","method":"no/such/method"}"#,
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
            r#"{"jsonrpc":"2.0","method":"tools/call","params":{"name":"no_such_tool"}}"#,
            r#"{not json"#,
            r#"{"jsonrpc":"1.0","method":"tools/list"}"#,
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

        let answers = answers(output);
        let ids: Vec<&Value> = answers.iter().map(|a| &a["id"]).collect();
        assert_eq!(
            ids,
            [
                &Value::from(1),
                &Value::Null,
                &Value::Null,
                &Value::from("two")
            ],
            "{answers:?}"
        );
        assert!(answers[0]["result"]["tools"].is_array());
        assert_eq!(answers[1]["error"]["code"], -32700);
        assert_eq!(answers[2]["error"]["code"], -32600);
        assert_eq!(answers[3]["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn a_subscribed_update_is_written_after_the_response_that_caused_it() {
        let (server, store, _dir) = sqlite_server().await;
        let workspace = store
            .create_workspace(NewWorkspace {
                name: "stdio".into(),
            })
            .await
            .unwrap();
        let member = store
            .create_member(NewMember {
                workspace_id: workspace.id,
                handle: "alice".into(),
                display_name: None,
                kind: MemberKind::Human,
            })
            .await
            .unwrap();
        let channel = store
            .create_channel(NewChannel {
                workspace_id: workspace.id,
                name: "general".into(),
                topic: None,
                private: false,
            })
            .await
            .unwrap();
        let thread = store
            .create_thread(NewThread {
                channel_id: channel.id,
                parent_thread_id: None,
                title: Some("t".into()),
                description: None,
                created_by: None,
            })
            .await
            .unwrap();
        let auth = AuthContext::from_session(
            member.id,
            workspace.id,
            vec![
                capability::MESSAGE_POST.to_string(),
                capability::WORKSPACE_READ.to_string(),
            ],
        );
        let uri = format!("maidan://threads/{}", thread.id.0);
        let input = [
            json!({"jsonrpc": "2.0", "id": 1, "method": "resources/subscribe", "params": {"uri": uri}}),
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {
                "name": "post_message",
                "arguments": {"thread_id": thread.id.0, "body": "hello"}
            }}),
        ]
        .map(|line| line.to_string())
        .join("\n");
        let mut output = Vec::new();
        serve_lines(&server, &auth, input.as_bytes(), &mut output)
            .await
            .unwrap();

        let answers = answers(output);
        assert_eq!(answers.len(), 3, "{answers:?}");
        assert_eq!(answers[0]["id"], 1);
        assert!(answers[0].get("error").is_none(), "{answers:?}");
        assert_eq!(answers[1]["id"], 2);
        assert!(answers[1].get("error").is_none(), "{answers:?}");
        assert_ne!(answers[1]["result"]["isError"], true, "{answers:?}");
        assert_eq!(answers[2]["method"], "notifications/resources/updated");
        assert_eq!(answers[2]["params"]["uri"], uri);
        assert!(answers[2].get("id").is_none());
    }

    #[tokio::test]
    async fn an_oversized_line_is_rejected_and_the_next_line_still_runs() {
        let (server, _store, _dir) = sqlite_server().await;
        let mut input = vec![b'x'; MAX_STDIO_LINE_BYTES + 1];
        input.push(b'\n');
        input.extend(br#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#);
        input.push(b'\n');
        let mut output = Vec::new();
        serve_lines(
            &server,
            &AuthContext::bypass(),
            input.as_slice(),
            &mut output,
        )
        .await
        .unwrap();
        let answers = answers(output);
        assert_eq!(answers.len(), 2, "{answers:?}");
        assert_eq!(answers[0]["id"], Value::Null);
        assert_eq!(answers[0]["error"]["code"], -32600);
        assert_eq!(answers[1]["id"], 1);
        assert!(answers[1].get("error").is_none(), "{answers:?}");
    }

    #[tokio::test]
    async fn invalid_utf8_is_a_parse_error_and_the_next_line_still_runs() {
        let (server, _store, _dir) = sqlite_server().await;
        let mut input = vec![0xff, b'\n'];
        input.extend(br#"{"jsonrpc":"2.0","id":7,"method":"ping"}"#);
        input.push(b'\n');
        let mut output = Vec::new();
        serve_lines(
            &server,
            &AuthContext::bypass(),
            input.as_slice(),
            &mut output,
        )
        .await
        .unwrap();
        let answers = answers(output);
        assert_eq!(answers.len(), 2, "{answers:?}");
        assert_eq!(answers[0]["id"], Value::Null);
        assert_eq!(answers[0]["error"]["code"], -32700);
        assert_eq!(answers[1]["id"], 7);
        assert!(answers[1].get("error").is_none(), "{answers:?}");
    }

    #[tokio::test]
    async fn a_crlf_line_is_one_request() {
        let (server, _store, _dir) = sqlite_server().await;
        let input = "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"ping\"}\r\n";
        let mut output = Vec::new();
        serve_lines(
            &server,
            &AuthContext::bypass(),
            input.as_bytes(),
            &mut output,
        )
        .await
        .unwrap();
        let answers = answers(output);
        assert_eq!(answers.len(), 1, "{answers:?}");
        assert_eq!(answers[0]["id"], 3);
        assert!(answers[0].get("error").is_none(), "{answers:?}");
    }
}

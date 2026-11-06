//! A minimal A2A v1.0 JSON-RPC client (`{base}/a2a/v1/rpc`).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures::{Stream, StreamExt};
use serde::de::DeserializeOwned;

use crate::error::A2aClientError;
use crate::protocol::{
    GetTaskRequest, JsonRpcId, JsonRpcRequest, JsonRpcResponse, SendMessageRequest,
    SendMessageResponse, StreamResponse, Task, A2A_PROTOCOL_VERSION, A2A_VERSION_HEADER,
    JSONRPC_VERSION, METHOD_GET_TASK, METHOD_SEND_MESSAGE, METHOD_SEND_STREAMING_MESSAGE,
};

/// An incomplete SSE event larger than this is refused instead of growing
/// without a frame delimiter.
const MAX_SSE_EVENT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct A2aClient {
    base_url: String,
    bearer: Option<String>,
    http: reqwest::Client,
    next_id: Arc<AtomicU64>,
}

impl A2aClient {
    pub fn new(base_url: impl Into<String>) -> Result<Self, A2aClientError> {
        let base_url = normalize_base_url(&base_url.into())?;
        Ok(Self {
            base_url,
            bearer: None,
            // `read_timeout` bounds a dead socket between chunks. It does not
            // cap how long an active stream may run. Unary `call` adds its own
            // overall deadline. Redirects are off so a POST cannot be steered
            // onto another origin or down to cleartext.
            http: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .read_timeout(Duration::from_secs(60))
                .redirect(reqwest::redirect::Policy::none())
                .user_agent(concat!("maidan-a2a/", env!("CARGO_PKG_VERSION")))
                .build()
                .map_err(|e| A2aClientError::Http(e.to_string()))?,
            next_id: Arc::new(AtomicU64::new(1)),
        })
    }

    /// Remember `token` for later requests. An empty token is refused here so
    /// the client never sends `Authorization: Bearer ` with nothing after it.
    /// A token over cleartext is refused unless the host is this machine.
    pub fn with_bearer(mut self, token: impl Into<String>) -> Result<Self, A2aClientError> {
        let token = token.into();
        if token.trim().is_empty() {
            return Err(A2aClientError::Http(
                "bearer token must not be empty".into(),
            ));
        }
        self.bearer = Some(token);
        self.ensure_bearer_transport()?;
        Ok(self)
    }

    pub async fn send_message(
        &self,
        params: SendMessageRequest,
    ) -> Result<SendMessageResponse, A2aClientError> {
        self.call(METHOD_SEND_MESSAGE, serde_json::to_value(params)?)
            .await
    }

    /// Send a message over SSE and collect every event until the server
    /// closes the stream. An error frame ends the call with that error.
    pub async fn send_streaming_message(
        &self,
        params: SendMessageRequest,
    ) -> Result<Vec<StreamResponse>, A2aClientError> {
        let id = self.next_id();
        let body = request(
            METHOD_SEND_STREAMING_MESSAGE,
            id.clone(),
            serde_json::to_value(params)?,
        );
        let resp = self
            .post(&body)?
            .send()
            .await
            .map_err(|e| A2aClientError::Http(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(http_status(resp).await);
        }
        let frames = parse_sse_stream(
            resp.bytes_stream()
                .map(|chunk| chunk.map_err(|e| A2aClientError::Http(e.to_string()))),
        )
        .await?;
        let mut events = Vec::with_capacity(frames.len());
        for data in frames {
            let rpc: JsonRpcResponse =
                serde_json::from_str(&data).map_err(|e| A2aClientError::Decode(e.to_string()))?;
            events.push(decode_result(rpc, &id)?);
        }
        Ok(events)
    }

    pub async fn get_task(&self, task_id: &str) -> Result<Task, A2aClientError> {
        let params = GetTaskRequest {
            id: task_id.to_string(),
            history_length: None,
        };
        self.call(METHOD_GET_TASK, serde_json::to_value(params)?)
            .await
    }

    fn next_id(&self) -> JsonRpcId {
        let n = self.next_id.fetch_add(1, Ordering::Relaxed);
        JsonRpcId::Number(i64::try_from(n).unwrap_or(i64::MAX))
    }

    fn post(&self, body: &JsonRpcRequest) -> Result<reqwest::RequestBuilder, A2aClientError> {
        self.ensure_bearer_transport()?;
        let url = format!("{}/a2a/v1/rpc", self.base_url);
        let mut req = self
            .http
            .post(url)
            .header(A2A_VERSION_HEADER, A2A_PROTOCOL_VERSION)
            .json(body);
        if let Some(token) = &self.bearer {
            req = req.bearer_auth(token);
        }
        Ok(req)
    }

    fn ensure_bearer_transport(&self) -> Result<(), A2aClientError> {
        let Some(token) = &self.bearer else {
            return Ok(());
        };
        if token.trim().is_empty() {
            return Err(A2aClientError::Http(
                "bearer token must not be empty".into(),
            ));
        }
        let url = reqwest::Url::parse(&self.base_url)
            .map_err(|e| A2aClientError::Http(format!("invalid base url: {e}")))?;
        if url.scheme() == "https" {
            return Ok(());
        }
        if url.scheme() == "http" && host_is_loopback(url.host_str().unwrap_or("")) {
            return Ok(());
        }
        Err(A2aClientError::Http(format!(
            "refusing to send a bearer token over cleartext to {}",
            self.base_url
        )))
    }

    async fn call<T: DeserializeOwned>(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<T, A2aClientError> {
        let id = self.next_id();
        let resp = self
            .post(&request(method, id.clone(), params))?
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| A2aClientError::Http(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(http_status(resp).await);
        }
        let rpc: JsonRpcResponse = resp
            .json()
            .await
            .map_err(|e| A2aClientError::Decode(e.to_string()))?;
        decode_result(rpc, &id)
    }
}

fn normalize_base_url(raw: &str) -> Result<String, A2aClientError> {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(A2aClientError::Http("base url must not be empty".into()));
    }
    // The URL parser turns `https:///missing-host` into a host named
    // `missing-host`. Reject an empty authority before that rewrite.
    if raw_host(trimmed).is_none() {
        return Err(A2aClientError::Http("base url must name a host".into()));
    }
    let url = reqwest::Url::parse(trimmed)
        .map_err(|e| A2aClientError::Http(format!("invalid base url: {e}")))?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err(A2aClientError::Http(
            "base url must use http or https".into(),
        ));
    }
    if url.host_str().map(str::is_empty).unwrap_or(true) {
        return Err(A2aClientError::Http("base url must name a host".into()));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(A2aClientError::Http(
            "base url must not include userinfo".into(),
        ));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(A2aClientError::Http(
            "base url must not include a query or fragment".into(),
        ));
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

fn host_is_loopback(host: &str) -> bool {
    // `host_str` for an IPv6 address includes the brackets.
    let host = host.trim_matches(['[', ']']);
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<std::net::IpAddr>()
        .is_ok_and(|ip| ip.is_loopback())
}

/// Host from the authority, before a parser repairs an empty one. `None` when
/// the authority has no host (`https:///missing-host`, `https://:8080`).
fn raw_host(url: &str) -> Option<&str> {
    let (_, after) = url.split_once("://")?;
    let authority = after.split(['/', '?', '#']).next().unwrap_or("");
    let hostport = authority
        .rsplit_once('@')
        .map(|(_, hostport)| hostport)
        .unwrap_or(authority);
    let host = if let Some(rest) = hostport.strip_prefix('[') {
        rest.split_once(']')?.0
    } else {
        hostport
            .split_once(':')
            .map(|(host, _)| host)
            .unwrap_or(hostport)
    };
    if host.is_empty() {
        None
    } else {
        Some(host)
    }
}

async fn http_status(resp: reqwest::Response) -> A2aClientError {
    let status = resp.status();
    let snippet = error_body_snippet(resp).await;
    if snippet.is_empty() {
        A2aClientError::Http(format!("HTTP {status}"))
    } else {
        A2aClientError::Http(format!("HTTP {status}: {snippet}"))
    }
}

/// At most 512 bytes, and at most eight chunks. A non-2xx body is a diagnostic,
/// not a download: `bytes()` would buffer a multi-megabyte or dripping reply.
async fn error_body_snippet(mut resp: reqwest::Response) -> String {
    let mut buf = Vec::with_capacity(512);
    for _ in 0..8 {
        if buf.len() >= 512 {
            break;
        }
        match resp.chunk().await {
            Ok(Some(chunk)) if !chunk.is_empty() => {
                let take = (512 - buf.len()).min(chunk.len());
                buf.extend_from_slice(&chunk[..take]);
            }
            _ => break,
        }
    }
    drop(resp);
    String::from_utf8_lossy(&buf).trim().to_string()
}

fn request(method: &str, id: JsonRpcId, params: serde_json::Value) -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: JSONRPC_VERSION.to_string(),
        id,
        method: method.to_string(),
        params,
    }
}

fn decode_result<T: DeserializeOwned>(
    rpc: JsonRpcResponse,
    expected_id: &JsonRpcId,
) -> Result<T, A2aClientError> {
    if rpc.jsonrpc != JSONRPC_VERSION {
        return Err(A2aClientError::Decode(format!(
            "jsonrpc version {:?}, expected {JSONRPC_VERSION}",
            rpc.jsonrpc
        )));
    }
    if let Some(err) = rpc.error {
        return Err(A2aClientError::Rpc {
            code: err.code,
            message: err.message,
        });
    }
    if &rpc.id != expected_id {
        return Err(A2aClientError::Decode(format!(
            "response id {:?} does not match request {expected_id:?}",
            rpc.id
        )));
    }
    let result = rpc
        .result
        .ok_or_else(|| A2aClientError::Decode("missing result".into()))?;
    serde_json::from_value(result).map_err(|e| A2aClientError::Decode(e.to_string()))
}

/// Join SSE `data:` lines from a byte stream. Chunk boundaries are not
/// character boundaries, and a frame may end with `\n\n` or `\r\n\r\n`.
async fn parse_sse_stream<S, B>(mut stream: S) -> Result<Vec<String>, A2aClientError>
where
    S: Stream<Item = Result<B, A2aClientError>> + Unpin,
    B: AsRef<[u8]>,
{
    let mut parser = SseParser::default();
    let mut out = Vec::new();
    while let Some(chunk) = stream.next().await {
        out.extend(parser.push(chunk?.as_ref())?);
    }
    out.extend(parser.finish()?);
    Ok(out)
}

#[derive(Default)]
struct SseParser {
    buf: Vec<u8>,
    pos: usize,
}

impl SseParser {
    fn pending_len(&self) -> usize {
        self.buf.len() - self.pos
    }

    fn push(&mut self, chunk: &[u8]) -> Result<Vec<String>, A2aClientError> {
        if self.pending_len().saturating_add(chunk.len()) > MAX_SSE_EVENT_BYTES {
            return Err(A2aClientError::Decode(format!(
                "sse event exceeds {MAX_SSE_EVENT_BYTES} bytes without a frame delimiter"
            )));
        }
        self.buf.extend_from_slice(chunk);
        self.drain_events()
    }

    fn finish(&mut self) -> Result<Vec<String>, A2aClientError> {
        if self.pending_len() == 0 {
            return Ok(Vec::new());
        }
        let block = self.buf[self.pos..].to_vec();
        self.pos = self.buf.len();
        Ok(event_data(&block)?.into_iter().collect())
    }

    fn drain_events(&mut self) -> Result<Vec<String>, A2aClientError> {
        let mut out = Vec::new();
        loop {
            let pending = &self.buf[self.pos..];
            let Some((block_end, consumed)) = find_event(pending) else {
                break;
            };
            let block = pending[..block_end].to_vec();
            self.pos += consumed;
            if let Some(data) = event_data(&block)? {
                out.push(data);
            }
        }
        if self.pos > 8192 && self.pos * 2 > self.buf.len() {
            self.buf.drain(..self.pos);
            self.pos = 0;
        }
        Ok(out)
    }
}

/// `(bytes of the event before the blank line, bytes to drop including it)`.
fn find_event(buf: &[u8]) -> Option<(usize, usize)> {
    let mut i = 0;
    let mut line_start = 0;
    while i < buf.len() {
        let (line_end, next) = match buf[i] {
            b'\n' => (i, i + 1),
            b'\r' if i + 1 < buf.len() && buf[i + 1] == b'\n' => (i, i + 2),
            b'\r' if i + 1 < buf.len() => (i, i + 1),
            b'\r' => return None,
            _ => {
                i += 1;
                continue;
            }
        };
        if line_start == line_end {
            return Some((line_start, next));
        }
        line_start = next;
        i = next;
    }
    None
}

fn event_data(block: &[u8]) -> Result<Option<String>, A2aClientError> {
    let text = std::str::from_utf8(block)
        .map_err(|e| A2aClientError::Decode(format!("sse frame is not utf-8: {e}")))?;
    let mut data_lines = Vec::new();
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() || line.starts_with(':') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("data:") {
            data_lines.push(rest.strip_prefix(' ').unwrap_or(rest));
        }
    }
    if data_lines.is_empty() {
        return Ok(None);
    }
    Ok(Some(data_lines.join("\n")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::JsonRpcError;
    use futures::stream;

    fn collect(chunks: Vec<Vec<u8>>) -> Result<Vec<String>, A2aClientError> {
        futures::executor::block_on(parse_sse_stream(stream::iter(
            chunks.into_iter().map(Ok::<_, A2aClientError>),
        )))
    }

    #[test]
    fn sse_reassembles_a_character_split_across_chunks() {
        let mut chunks = vec![b"data: hi ".to_vec()];
        for byte in "—😀".as_bytes() {
            chunks.push(vec![*byte]);
        }
        chunks.push(b"\n\n".to_vec());
        let frames = collect(chunks).unwrap();
        assert_eq!(frames, vec!["hi —😀".to_string()]);
    }

    #[test]
    fn sse_accepts_crlf_frames_multiline_data_comments_and_a_trailing_block() {
        let frames = collect(vec![
            b": keep-alive\r\n\r\ndata: {\"a\":\r\ndata: 1}\r\n\r\ndata: {\"tail\":true}".to_vec(),
        ])
        .unwrap();
        assert_eq!(
            frames,
            vec!["{\"a\":\n1}".to_string(), "{\"tail\":true}".to_string()]
        );
    }

    #[test]
    fn sse_strips_only_one_leading_space_after_data() {
        let frames = collect(vec![b"data:  hi\n\n".to_vec()]).unwrap();
        assert_eq!(frames, vec![" hi".to_string()]);
    }

    #[test]
    fn sse_rejects_an_event_larger_than_the_cap() {
        let chunk = vec![b'x'; MAX_SSE_EVENT_BYTES + 1];
        let err = collect(vec![chunk]).unwrap_err();
        assert!(matches!(err, A2aClientError::Decode(_)), "{err}");
    }

    #[test]
    fn bearer_over_cleartext_is_limited_to_loopback() {
        let refuse = |url: &str| {
            A2aClient::new(url)
                .unwrap()
                .with_bearer("tok")
                .expect_err(url)
        };
        for url in [
            "http://example.com",
            "HTTP://example.com",
            "http://127.evil.com",
        ] {
            let err = refuse(url);
            assert!(err.to_string().contains("cleartext"), "{url}: {err}");
        }
        for url in ["http://127.0.0.1", "http://[::1]:9", "http://localhost"] {
            A2aClient::new(url).unwrap().with_bearer("tok").expect(url);
        }
        A2aClient::new("https://example.com")
            .unwrap()
            .with_bearer("tok")
            .expect("https");
        assert!(A2aClient::new("http://127.0.0.1")
            .unwrap()
            .with_bearer("   ")
            .is_err());
        assert!(A2aClient::new("http://127.0.0.1")
            .unwrap()
            .with_bearer("")
            .is_err());
    }

    #[test]
    fn base_url_rejects_userinfo_query_fragment_and_a_missing_host() {
        assert!(A2aClient::new("https://user:pass@peer.example").is_err());
        assert!(A2aClient::new("https://peer.example?x=1").is_err());
        assert!(A2aClient::new("https://peer.example#frag").is_err());
        assert!(A2aClient::new("https:///missing-host").is_err());
        assert!(A2aClient::new("ftp://peer.example").is_err());
        let client = A2aClient::new("  HTTPS://Peer.Example/prefix/  ").unwrap();
        assert_eq!(client.base_url, "https://peer.example/prefix");
    }

    #[test]
    fn request_ids_increase_and_a_result_must_echo_its_id() {
        let client = A2aClient::new("http://127.0.0.1").unwrap();
        assert_eq!(client.next_id(), JsonRpcId::Number(1));
        assert_eq!(client.next_id(), JsonRpcId::Number(2));

        let ok = JsonRpcResponse::success(JsonRpcId::Number(1), serde_json::json!({"n": 1}));
        let value: serde_json::Value =
            decode_result(ok, &JsonRpcId::Number(1)).expect("matching id");
        assert_eq!(value["n"], 1);

        let wrong = JsonRpcResponse::success(JsonRpcId::Number(9), serde_json::json!({}));
        assert!(decode_result::<serde_json::Value>(wrong, &JsonRpcId::Number(1)).is_err());

        let version = JsonRpcResponse {
            jsonrpc: "1.0".into(),
            id: JsonRpcId::Number(1),
            result: Some(serde_json::json!({})),
            error: None,
        };
        assert!(decode_result::<serde_json::Value>(version, &JsonRpcId::Number(1)).is_err());

        let rpc_err = JsonRpcResponse::failure(
            JsonRpcId::Number(9),
            JsonRpcError {
                code: -32600,
                message: "no".into(),
                data: None,
            },
        );
        assert!(matches!(
            decode_result::<serde_json::Value>(rpc_err, &JsonRpcId::Number(1)),
            Err(A2aClientError::Rpc { code: -32600, .. })
        ));
    }
}

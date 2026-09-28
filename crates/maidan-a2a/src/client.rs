//! A minimal A2A v1.0 JSON-RPC client (`{base}/a2a/v1/rpc`).

use futures::StreamExt;
use serde::de::DeserializeOwned;

use crate::error::A2aClientError;
use crate::protocol::{
    GetTaskRequest, JsonRpcId, JsonRpcRequest, JsonRpcResponse, SendMessageRequest,
    SendMessageResponse, StreamResponse, Task, A2A_PROTOCOL_VERSION, A2A_VERSION_HEADER,
    JSONRPC_VERSION, METHOD_GET_TASK, METHOD_SEND_MESSAGE, METHOD_SEND_STREAMING_MESSAGE,
};

#[derive(Debug, Clone)]
pub struct A2aClient {
    base_url: String,
    bearer: Option<String>,
    http: reqwest::Client,
}

impl A2aClient {
    pub fn new(base_url: impl Into<String>) -> Result<Self, A2aClientError> {
        Ok(Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            bearer: None,
            // A connect timeout bounds the indefinite-hang risk for every request
            // (streaming included) without capping a legitimately long streaming
            // response; non-streaming `call` adds an overall per-request timeout.
            http: reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(10))
                .build()
                .map_err(|e| A2aClientError::Http(e.to_string()))?,
        })
    }

    pub fn with_bearer(mut self, token: impl Into<String>) -> Self {
        self.bearer = Some(token.into());
        self
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
        let body = request(METHOD_SEND_STREAMING_MESSAGE, serde_json::to_value(params)?);
        let resp = self
            .post(&body)
            .send()
            .await
            .map_err(|e| A2aClientError::Http(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(A2aClientError::Http(format!("HTTP {}", resp.status())));
        }
        let mut events = Vec::new();
        let mut buf = String::new();
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| A2aClientError::Http(e.to_string()))?;
            buf.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(pos) = buf.find("\n\n") {
                let block = buf[..pos].to_string();
                buf = buf[pos + 2..].to_string();
                for line in block.lines() {
                    let Some(data) = line.strip_prefix("data:") else {
                        continue;
                    };
                    let rpc: JsonRpcResponse = serde_json::from_str(data.trim())
                        .map_err(|e| A2aClientError::Decode(e.to_string()))?;
                    events.push(decode_result(rpc)?);
                }
            }
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

    fn post(&self, body: &JsonRpcRequest) -> reqwest::RequestBuilder {
        let url = format!("{}/a2a/v1/rpc", self.base_url);
        let mut req = self
            .http
            .post(url)
            .header(A2A_VERSION_HEADER, A2A_PROTOCOL_VERSION)
            .json(body);
        if let Some(token) = &self.bearer {
            req = req.bearer_auth(token);
        }
        req
    }

    async fn call<T: DeserializeOwned>(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<T, A2aClientError> {
        let resp = self
            .post(&request(method, params))
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| A2aClientError::Http(e.to_string()))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(A2aClientError::Http(format!("HTTP {status}")));
        }
        let rpc: JsonRpcResponse = resp
            .json()
            .await
            .map_err(|e| A2aClientError::Decode(e.to_string()))?;
        decode_result(rpc)
    }
}

fn request(method: &str, params: serde_json::Value) -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: JSONRPC_VERSION.to_string(),
        id: JsonRpcId::Number(1),
        method: method.to_string(),
        params,
    }
}

fn decode_result<T: DeserializeOwned>(rpc: JsonRpcResponse) -> Result<T, A2aClientError> {
    if let Some(err) = rpc.error {
        return Err(A2aClientError::Rpc {
            code: err.code,
            message: err.message,
        });
    }
    let result = rpc
        .result
        .ok_or_else(|| A2aClientError::Decode("missing result".into()))?;
    serde_json::from_value(result).map_err(|e| A2aClientError::Decode(e.to_string()))
}

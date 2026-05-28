use crate::error::A2aClientError;
use crate::protocol::{GetTaskRequest, Task};
use crate::protocol::{
    JsonRpcId, JsonRpcRequest, JsonRpcResponse, SendMessageRequest, JSONRPC_VERSION,
    METHOD_GET_TASK, METHOD_SEND_MESSAGE,
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
            http: reqwest::Client::builder()
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
    ) -> Result<serde_json::Value, A2aClientError> {
        self.call(METHOD_SEND_MESSAGE, serde_json::to_value(params)?)
            .await
    }

    pub async fn get_task(&self, task_id: &str) -> Result<Task, A2aClientError> {
        let params = GetTaskRequest {
            id: task_id.to_string(),
        };
        let value = self
            .call(METHOD_GET_TASK, serde_json::to_value(params)?)
            .await?;
        serde_json::from_value(value).map_err(|e| A2aClientError::Decode(e.to_string()))
    }

    async fn call(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, A2aClientError> {
        let id = JsonRpcId::Number(1);
        let body = JsonRpcRequest {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id: id.clone(),
            method: method.to_string(),
            params,
        };
        let url = format!("{}/a2a/v1/rpc", self.base_url);
        let mut req = self.http.post(&url).json(&body);
        if let Some(token) = &self.bearer {
            req = req.bearer_auth(token);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| A2aClientError::Http(e.to_string()))?;
        let status = resp.status();
        let rpc: JsonRpcResponse = resp
            .json()
            .await
            .map_err(|e| A2aClientError::Decode(e.to_string()))?;
        if !status.is_success() {
            return Err(A2aClientError::Http(format!("HTTP {status}")));
        }
        if let Some(err) = rpc.error {
            return Err(A2aClientError::Rpc {
                code: err.code,
                message: err.message,
            });
        }
        rpc.result
            .ok_or_else(|| A2aClientError::Decode("missing result".into()))
    }
}

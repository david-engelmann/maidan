//! MCP dispatcher. Takes JSON-RPC requests and returns responses.
//! Transport-agnostic; `maidan-server` wraps it behind `POST /mcp`.

use std::collections::HashSet;
use std::sync::Arc;

use maidan_artifacts::ArtifactStore;
use maidan_auth::AuthContext;
use maidan_search::{EmbeddingProvider, Search};
use maidan_store::Store;
use serde_json::{json, Value};
use tokio::sync::{broadcast, Mutex};

const NOTIFICATION_BROADCAST_CAPACITY: usize = 64;

use crate::error::McpError;
use crate::protocol::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};
use crate::{prompts, resources, tools};

const MCP_VERSION: &str = "2024-11-05";
const NOTIFY_RESOURCE_UPDATED: &str = "notifications/resources/updated";

#[derive(Clone)]
pub struct McpServer {
    store: Arc<dyn Store>,
    artifacts: Arc<dyn ArtifactStore>,
    search: Arc<dyn Search>,
    embedding_provider: Arc<dyn EmbeddingProvider>,
    server_name: String,
    server_version: String,
    subscriptions: Arc<Mutex<HashSet<String>>>,
    pending_notifications: Arc<Mutex<Vec<JsonRpcNotification>>>,
    notification_tx: broadcast::Sender<JsonRpcNotification>,
}

impl McpServer {
    pub fn new(
        store: Arc<dyn Store>,
        artifacts: Arc<dyn ArtifactStore>,
        search: Arc<dyn Search>,
        embedding_provider: Arc<dyn EmbeddingProvider>,
    ) -> Self {
        let (notification_tx, _) = broadcast::channel(NOTIFICATION_BROADCAST_CAPACITY);
        Self {
            store,
            artifacts,
            search,
            embedding_provider,
            server_name: "maidan".into(),
            server_version: env!("CARGO_PKG_VERSION").into(),
            subscriptions: Arc::new(Mutex::new(HashSet::new())),
            pending_notifications: Arc::new(Mutex::new(Vec::new())),
            notification_tx,
        }
    }

    /// Live stream of MCP JSON-RPC notifications (HTTP SSE transport).
    pub fn subscribe_notifications(&self) -> broadcast::Receiver<JsonRpcNotification> {
        self.notification_tx.subscribe()
    }

    pub async fn handle(&self, request: JsonRpcRequest, auth: &AuthContext) -> JsonRpcResponse {
        let id = request.id.clone().unwrap_or(Value::Null);
        match self.dispatch(&request, auth).await {
            Ok(result) => JsonRpcResponse::success(id, result),
            Err(err) => {
                tracing::debug!(method = %request.method, error = %err, "mcp dispatch error");
                JsonRpcResponse::failure(id, err.to_jsonrpc())
            }
        }
    }

    async fn dispatch(
        &self,
        request: &JsonRpcRequest,
        auth: &AuthContext,
    ) -> Result<Value, McpError> {
        match request.method.as_str() {
            "initialize" => self.initialize().await,
            "tools/list" => Ok(json!({ "tools": tools::catalog() })),
            "tools/call" => self.tools_call(&request.params, auth).await,
            "resources/list" => Ok(json!({ "resources": resources::catalog() })),
            "resources/read" => self.resources_read(&request.params, auth).await,
            "resources/subscribe" => self.resources_subscribe(&request.params, auth).await,
            "resources/unsubscribe" => self.resources_unsubscribe(&request.params, auth).await,
            "prompts/list" => Ok(json!({ "prompts": prompts::catalog() })),
            "prompts/get" => self.prompts_get(&request.params, auth).await,
            other => Err(McpError::MethodNotFound(other.into())),
        }
    }

    async fn initialize(&self) -> Result<Value, McpError> {
        Ok(json!({
            "protocolVersion": MCP_VERSION,
            "capabilities": {
                "tools": {},
                "resources": { "subscribe": true },
                "prompts": {}
            },
            "serverInfo": {
                "name": self.server_name,
                "version": self.server_version
            }
        }))
    }

    async fn tools_call(&self, params: &Value, auth: &AuthContext) -> Result<Value, McpError> {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::InvalidParams("missing tool name".into()))?;
        if !auth.bypass {
            let cap = tools::required_capability(name)?;
            auth.require_capability(cap).map_err(McpError::from)?;
        }
        let args = params.get("arguments").cloned().unwrap_or(json!({}));
        let result = tools::dispatch(
            &self.store,
            &self.artifacts,
            &self.search,
            &self.embedding_provider,
            auth,
            name,
            &args,
        )
        .await?;
        self.queue_resource_updates(name, &args, &result).await;
        Ok(result)
    }

    async fn prompts_get(&self, params: &Value, auth: &AuthContext) -> Result<Value, McpError> {
        if !auth.bypass {
            auth.require_capability(maidan_auth::capability::WORKSPACE_READ)
                .map_err(McpError::from)?;
        }
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::InvalidParams("missing prompt name".into()))?;
        let args = params.get("arguments").cloned().unwrap_or(json!({}));
        prompts::get(&self.store, name, &args).await
    }

    async fn resources_read(&self, params: &Value, auth: &AuthContext) -> Result<Value, McpError> {
        if !auth.bypass {
            auth.require_capability(maidan_auth::capability::WORKSPACE_READ)
                .map_err(McpError::from)?;
        }
        let uri = params
            .get("uri")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::InvalidParams("missing uri".into()))?;
        resources::read(&self.store, &self.artifacts, uri).await
    }

    async fn resources_subscribe(
        &self,
        params: &Value,
        auth: &AuthContext,
    ) -> Result<Value, McpError> {
        if !auth.bypass {
            auth.require_capability(maidan_auth::capability::WORKSPACE_READ)
                .map_err(McpError::from)?;
        }
        let uri = params
            .get("uri")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::InvalidParams("missing uri".into()))?;
        resources::validate_uri(uri)?;
        let mut subscriptions = self.subscriptions.lock().await;
        subscriptions.insert(uri.to_string());
        Ok(json!({
            "ok": true,
            "uri": uri
        }))
    }

    async fn resources_unsubscribe(
        &self,
        params: &Value,
        auth: &AuthContext,
    ) -> Result<Value, McpError> {
        if !auth.bypass {
            auth.require_capability(maidan_auth::capability::WORKSPACE_READ)
                .map_err(McpError::from)?;
        }
        let uri = params
            .get("uri")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::InvalidParams("missing uri".into()))?;
        resources::validate_uri(uri)?;
        let mut subscriptions = self.subscriptions.lock().await;
        let removed = subscriptions.remove(uri);
        Ok(json!({
            "ok": true,
            "uri": uri,
            "removed": removed
        }))
    }

    async fn queue_resource_updates(&self, tool_name: &str, args: &Value, result: &Value) {
        let uris = crate::resource_updates::uris_for_tool_mutation(
            self.store.as_ref(),
            tool_name,
            args,
            result,
        )
        .await;
        if uris.is_empty() {
            return;
        }
        let subscriptions = self.subscriptions.lock().await;
        let matching: Vec<String> = uris
            .into_iter()
            .filter(|uri| subscriptions.contains(uri))
            .collect();
        drop(subscriptions);
        if matching.is_empty() {
            return;
        }
        let mut pending = self.pending_notifications.lock().await;
        for uri in matching {
            let notification =
                JsonRpcNotification::new(NOTIFY_RESOURCE_UPDATED, json!({ "uri": uri }));
            pending.push(notification.clone());
            let _ = self.notification_tx.send(notification);
        }
    }

    pub async fn take_pending_notifications(&self) -> Vec<JsonRpcNotification> {
        let mut pending = self.pending_notifications.lock().await;
        std::mem::take(&mut *pending)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use maidan_artifacts::LocalFsStore;
    use maidan_auth::AuthContext;
    use maidan_search::HashV1Provider;
    use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
    use maidan_types::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn mk_server() -> (McpServer, ThreadId, MemberId) {
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        run_sqlite_migrations(&pool).await.unwrap();

        let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
        let workspace = store
            .create_workspace(NewWorkspace {
                name: "mcp-subscribe".into(),
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
            })
            .await
            .unwrap();

        let server = McpServer::new(
            store,
            Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
            Arc::new(maidan_search::SqliteSearch::new(pool)),
            Arc::new(HashV1Provider),
        );
        (server, thread.id, member.id)
    }

    fn request(id: i64, method: &str, params: Value) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(json!(id)),
            method: method.into(),
            params,
        }
    }

    #[tokio::test]
    async fn subscribe_and_post_message_emits_resource_updated_notification() {
        let (server, thread_id, member_id) = mk_server().await;
        let auth = AuthContext::bypass();
        let uri = format!("maidan://threads/{}", thread_id.0);

        let subscribe = server
            .handle(
                request(1, "resources/subscribe", json!({ "uri": uri })),
                &auth,
            )
            .await;
        assert!(subscribe.error.is_none());

        let _ = server
            .handle(
                request(
                    2,
                    "tools/call",
                    json!({
                        "name": "post_message",
                        "arguments": {
                            "thread_id": thread_id.0,
                            "author_id": member_id.0,
                            "body": "hello"
                        }
                    }),
                ),
                &auth,
            )
            .await;

        let notifications = server.take_pending_notifications().await;
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].method, NOTIFY_RESOURCE_UPDATED);
        assert_eq!(
            notifications[0].params["uri"],
            format!("maidan://threads/{}", thread_id.0)
        );
    }

    #[tokio::test]
    async fn unsubscribe_suppresses_future_notifications() {
        let (server, thread_id, member_id) = mk_server().await;
        let auth = AuthContext::bypass();
        let uri = format!("maidan://threads/{}", thread_id.0);

        let _ = server
            .handle(
                request(1, "resources/subscribe", json!({ "uri": uri.clone() })),
                &auth,
            )
            .await;
        let _ = server
            .handle(
                request(2, "resources/unsubscribe", json!({ "uri": uri })),
                &auth,
            )
            .await;

        let _ = server
            .handle(
                request(
                    3,
                    "tools/call",
                    json!({
                        "name": "post_message",
                        "arguments": {
                            "thread_id": thread_id.0,
                            "author_id": member_id.0,
                            "body": "hello"
                        }
                    }),
                ),
                &auth,
            )
            .await;

        assert!(server.take_pending_notifications().await.is_empty());
    }
}

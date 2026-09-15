//! Room URIs, named capability sets, handle aliases, and holder-side
//! token attenuation (Cluster 395, Wave 3 #35).

use std::sync::Arc;

use chrono::{DateTime, Utc};
use maidan_auth::{capability, hash_secret, AuthContext, TokenSecret};
use maidan_store::Store;
use maidan_types::{NewApiToken, RoomCard, RoomUri, WorkspaceId};
use serde::Deserialize;
use serde_json::{json, Value};

use super::content_json;
use crate::error::McpError;

fn holder_grant(auth: &AuthContext) -> Vec<String> {
    if auth.bypass {
        capability::all()
    } else {
        auth.capabilities().to_vec()
    }
}

/// Catalog of named sets and the atomics they expand to.
pub(super) async fn list_capability_sets() -> Result<Value, McpError> {
    let sets: Vec<Value> = maidan_auth::named_sets()
        .into_iter()
        .map(|s| {
            json!({
                "name": s.name,
                "capabilities": s.capabilities,
            })
        })
        .collect();
    Ok(content_json(&json!({ "sets": sets })))
}

#[derive(Deserialize)]
struct ParseUriArgs {
    uri: String,
}

/// Parse a hierarchical `maidan://{workspace_id}/…` room URI. MCP
/// `maidan://threads/{id}` and Cluster 392 `maidan:event/{id}` pins fail.
pub(super) async fn parse_maidan_uri(args: &Value) -> Result<Value, McpError> {
    let a: ParseUriArgs = serde_json::from_value(args.clone())?;
    let uri = RoomUri::parse(&a.uri).map_err(|e| McpError::InvalidParams(e.to_string()))?;
    Ok(content_json(&uri))
}

#[derive(Deserialize)]
struct GetRoomArgs {
    workspace_id: uuid::Uuid,
}

pub(super) async fn get_room(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: GetRoomArgs = serde_json::from_value(args.clone())?;
    let workspace_id = WorkspaceId(a.workspace_id);
    auth.ensure_workspace(workspace_id)?;
    let _ = store.get_workspace(workspace_id).await?;
    let handle = store
        .get_workspace_handle(workspace_id)
        .await?
        .map(|h| h.handle);
    Ok(content_json(&RoomCard::new(workspace_id, handle)))
}

#[derive(Deserialize)]
struct SetHandleArgs {
    workspace_id: uuid::Uuid,
    handle: String,
}

pub(super) async fn set_workspace_handle(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: SetHandleArgs = serde_json::from_value(args.clone())?;
    let workspace_id = WorkspaceId(a.workspace_id);
    auth.ensure_workspace(workspace_id)?;
    let row = store.set_workspace_handle(workspace_id, &a.handle).await?;
    Ok(content_json(&row))
}

#[derive(Deserialize)]
struct AttenuateArgs {
    capabilities: Vec<String>,
    #[serde(default)]
    expires_at: Option<DateTime<Utc>>,
    #[serde(default)]
    label: Option<String>,
}

/// Holder-side attenuation: derive a weaker token without `token:admin`.
/// Amplification is rejected. A derived expiry cannot outlive the parent.
pub(super) async fn attenuate_token(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: AttenuateArgs = serde_json::from_value(args.clone())?;
    let capabilities = maidan_auth::attenuate(&holder_grant(auth), &a.capabilities)
        .map_err(McpError::InvalidParams)?;
    let parent = match auth.token_id {
        Some(id) => store.get_api_token(id).await?.expires_at,
        None => None,
    };
    let expires_at = maidan_auth::attenuate_expiry(parent, a.expires_at, Utc::now())
        .map_err(McpError::InvalidParams)?;
    let secret = TokenSecret::generate();
    let record = store
        .create_api_token(NewApiToken {
            workspace_id: auth.workspace_id,
            member_id: auth.member_id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: a.label,
            capabilities: capabilities.clone(),
            expires_at,
        })
        .await?;
    Ok(content_json(&json!({
        "id": record.id.0,
        "secret": secret.as_str(),
        "workspace_id": record.workspace_id.0,
        "member_id": record.member_id.0,
        "capabilities": record.capabilities,
        "expires_at": record.expires_at,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use maidan_artifacts::LocalFsStore;
    use maidan_auth::capability::{MESSAGE_POST, TOKEN_ADMIN, WORKSPACE_READ, WORKSPACE_WRITE};
    use maidan_auth::{AGENT_WORKER, HUMAN_ADMIN};
    use maidan_search::HashV1Provider;
    use maidan_store::{run_sqlite_migrations, SqliteStore};
    use maidan_types::{MemberKind, NewMember, NewWorkspace, ROOM_DISCOVERY_TYPE, ROOM_TYPE};
    use sqlx::sqlite::SqlitePoolOptions;

    use crate::server::McpServer;

    fn content(v: &Value) -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    }

    async fn blank_store() -> (Arc<dyn Store>, sqlx::SqlitePool) {
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        run_sqlite_migrations(&pool).await.unwrap();
        (Arc::new(SqliteStore::new(pool.clone())), pool)
    }

    fn mcp(store: Arc<dyn Store>, pool: sqlx::SqlitePool) -> McpServer {
        McpServer::new(
            store,
            Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
            Arc::new(maidan_search::SqliteSearch::new(pool)),
            Arc::new(HashV1Provider),
        )
    }

    async fn seed(store: &dyn Store) -> (WorkspaceId, maidan_types::MemberId) {
        let (ws, _) = store
            .create_workspace_with_event(NewWorkspace {
                name: "room".into(),
            })
            .await
            .unwrap();
        let (member, _) = store
            .create_member_with_event(NewMember {
                workspace_id: ws.id,
                handle: "agent".into(),
                display_name: None,
                kind: MemberKind::Human,
            })
            .await
            .unwrap();
        (ws.id, member.id)
    }

    #[tokio::test]
    async fn room_tools_parse_handle_sets_and_attenuation() {
        let (store, pool) = blank_store().await;
        let (ws, member) = seed(store.as_ref()).await;
        let server = mcp(store.clone(), pool);
        let worker = maidan_auth::expand_set(AGENT_WORKER).unwrap();
        let reader = AuthContext::from_session(member, ws, vec![WORKSPACE_READ.to_string()]);
        let writer = AuthContext::from_session(
            member,
            ws,
            vec![WORKSPACE_READ.to_string(), WORKSPACE_WRITE.to_string()],
        );
        let holder = AuthContext::from_session(member, ws, worker.clone());

        let catalog = content(
            &server
                .call_tool(&reader, "list_capability_sets", &json!({}))
                .await
                .unwrap(),
        );
        let names: Vec<&str> = catalog["sets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec![AGENT_WORKER, HUMAN_ADMIN]);

        let parsed = content(
            &server
                .call_tool(
                    &reader,
                    "parse_maidan_uri",
                    &json!({ "uri": format!("maidan://{}", ws.0) }),
                )
                .await
                .unwrap(),
        );
        assert_eq!(parsed["workspace_id"], json!(ws.0));
        let mcp_form = server
            .call_tool(
                &reader,
                "parse_maidan_uri",
                &json!({ "uri": "maidan://threads/33333333-3333-4333-8333-333333333333" }),
            )
            .await
            .unwrap_err();
        assert!(mcp_form.to_string().contains("authority"), "{mcp_form}");

        let empty = content(
            &server
                .call_tool(&reader, "get_room", &json!({ "workspace_id": ws.0 }))
                .await
                .unwrap(),
        );
        assert_eq!(empty["$type"], ROOM_TYPE);
        assert_eq!(empty["uri"], format!("maidan://{}", ws.0));
        assert!(empty.get("handle").is_none());

        let set = content(
            &server
                .call_tool(
                    &writer,
                    "set_workspace_handle",
                    &json!({ "workspace_id": ws.0, "handle": "acme" }),
                )
                .await
                .unwrap(),
        );
        assert_eq!(set["handle"], "acme");
        let renamed = content(
            &server
                .call_tool(
                    &writer,
                    "set_workspace_handle",
                    &json!({ "workspace_id": ws.0, "handle": "renamed" }),
                )
                .await
                .unwrap(),
        );
        assert_eq!(renamed["handle"], "renamed");
        let card = content(
            &server
                .call_tool(&reader, "get_room", &json!({ "workspace_id": ws.0 }))
                .await
                .unwrap(),
        );
        assert_eq!(card["handle"], "renamed");
        assert_eq!(card["uri"], format!("maidan://{}", ws.0));

        let derived = content(
            &server
                .call_tool(
                    &holder,
                    "attenuate_token",
                    &json!({
                        "capabilities": [WORKSPACE_READ, MESSAGE_POST],
                        "label": "weaker"
                    }),
                )
                .await
                .unwrap(),
        );
        assert_eq!(
            derived["capabilities"],
            json!([WORKSPACE_READ, MESSAGE_POST])
        );
        assert!(derived["secret"].as_str().unwrap().starts_with("maid_"));

        let amp = server
            .call_tool(
                &holder,
                "attenuate_token",
                &json!({ "capabilities": [TOKEN_ADMIN] }),
            )
            .await
            .unwrap_err();
        assert!(amp.to_string().contains("exceeds holder grant"), "{amp}");

        // Discovery type is the public document; tools never leak a tenant list.
        let doc = maidan_types::RoomDiscovery::document();
        assert_eq!(doc.type_id, ROOM_DISCOVERY_TYPE);
    }
}

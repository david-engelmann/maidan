//! Room URIs, named capability sets, handle aliases, and holder-side token
//! attenuation.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use maidan_auth::{capability, hash_secret, AuthContext, TokenSecret};
use maidan_store::Store;
use maidan_types::{
    DelegationGrantId, MemberId, NewApiToken, NewAuditEvent, NewDelegationGrant, RoomCard, RoomUri,
    WorkspaceId,
};
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
#[serde(deny_unknown_fields)]
struct ParseUriArgs {
    uri: String,
}

/// Parse a hierarchical `maidan://{workspace_id}/…` room URI. MCP
/// `maidan://threads/{id}` and `maidan:event/{id}` pins fail.
pub(super) async fn parse_maidan_uri(args: &Value) -> Result<Value, McpError> {
    let a: ParseUriArgs = serde_json::from_value(args.clone())?;
    let uri = RoomUri::parse(&a.uri).map_err(|e| McpError::InvalidParams(e.to_string()))?;
    Ok(content_json(&uri))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
struct AttenuateArgs {
    capabilities: Vec<String>,
    #[serde(default)]
    expires_at: Option<DateTime<Utc>>,
    #[serde(default)]
    label: Option<String>,
}

/// Holder-side attenuation: derive a weaker token without `token:admin`.
/// Amplification is rejected. A derived expiry cannot outlive the parent.
///
/// Inherits the parent's `app_installation_id` and per-token quotas, for the
/// reason the REST twin does: attenuation may be a no-op re-issue, so any bound
/// the parent carried and the child did not was a way to shed it by asking.
/// Also writes the `token.mint` audit row this path was missing entirely —
/// minting a bearer is an audited mutation everywhere else, and the REST twin
/// already recorded it.
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
    let inherited_quotas = match auth.token_id {
        Some(parent_id) => store.list_token_quotas(parent_id).await?,
        None => Vec::new(),
    };
    let secret = TokenSecret::generate();
    let derived = NewApiToken {
        workspace_id: auth.workspace_id,
        member_id: auth.member_id,
        app_installation_id: auth.app_installation_id,
        token_hash: hash_secret(secret.as_str()),
        label: a.label,
        capabilities: capabilities.clone(),
        expires_at,
    };
    // Record the parent so revoking it reaches this token.
    let record = match auth.token_id {
        Some(parent) => store.create_attenuated_api_token(derived, parent).await?,
        None => store.create_api_token(derived).await?,
    };
    if !inherited_quotas.is_empty() {
        store
            .replace_token_quotas(record.id, &inherited_quotas)
            .await?;
    }
    // Best-effort, like `crate::audit::record`: a mint must not lose its
    // response-only secret to an audit hiccup.
    if let Err(err) = store
        .append_audit(maidan_types::NewAuditEvent {
            actor_id: Some(auth.actor_id),
            action: "token.mint".into(),
            target_kind: Some("api_token".into()),
            target_id: Some(record.id.0),
            metadata: json!({
                "workspace_id": record.workspace_id.0,
                "subject_member_id": record.member_id.0,
                "capabilities": record.capabilities.clone(),
                "expires_at": record.expires_at,
                "attenuated": true,
                "surface": "mcp",
                "parent_token_id": auth.token_id.map(|t| t.0),
                "app_installation_id": record.app_installation_id.map(|i| i.0),
                "inherited_quotas": inherited_quotas.len(),
            }),
        })
        .await
    {
        tracing::warn!(error = %err, "audit.write_failed");
    }
    Ok(content_json(&json!({
        "id": record.id.0,
        "secret": secret.as_str(),
        "workspace_id": record.workspace_id.0,
        "member_id": record.member_id.0,
        "capabilities": record.capabilities,
        "expires_at": record.expires_at,
    })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DelegateTokenArgs {
    grant_id: uuid::Uuid,
    #[serde(default)]
    capabilities: Vec<String>,
    #[serde(default)]
    expires_at: Option<DateTime<Utc>>,
    #[serde(default)]
    label: Option<String>,
}

pub(super) async fn delegate_token(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: DelegateTokenArgs = serde_json::from_value(args.clone())?;
    // One hop — see the REST twin. A borrowed token exchanging would use its
    // subject's grants and erase the real actor from the chain.
    if auth.delegation_grant_id.is_some() {
        return Err(McpError::Forbidden(
            "a delegated token cannot exchange a grant; delegation is one hop".into(),
        ));
    }
    let grant = store
        .get_delegation_grant(maidan_types::DelegationGrantId(a.grant_id))
        .await?;
    // As on REST: another workspace's grant reads as absent.
    if !auth.bypass && grant.workspace_id != auth.workspace_id {
        return Err(McpError::NotFound);
    }
    if grant.delegate_id != auth.actor_id {
        return Err(McpError::Forbidden(
            "delegation grant belongs to a different delegate".into(),
        ));
    }
    let now = Utc::now();
    if grant.revoked_at.is_some() || grant.expires_at <= now {
        return Err(McpError::Unauthorized);
    }
    let held = holder_grant(auth);
    let requested = if a.capabilities.is_empty() {
        grant
            .capabilities
            .iter()
            .filter(|capability| held.contains(capability))
            .cloned()
            .collect()
    } else {
        a.capabilities
    };
    let capabilities = maidan_auth::attenuate(&grant.capabilities, &requested)
        .and_then(|caps| maidan_auth::attenuate(&held, &caps))
        .map_err(McpError::InvalidParams)?;
    let parent_expiry = match auth.token_id {
        Some(id) => store.get_api_token(id).await?.expires_at,
        None => None,
    };
    let expires_at =
        maidan_auth::delegated_expiry(grant.expires_at, parent_expiry, a.expires_at, now)
            .map_err(McpError::InvalidParams)?;
    let secret = TokenSecret::generate();
    let record = store
        .create_delegated_api_token(
            NewApiToken {
                workspace_id: grant.workspace_id,
                member_id: grant.subject_id,
                app_installation_id: None,
                token_hash: hash_secret(secret.as_str()),
                label: a.label,
                capabilities: capabilities.clone(),
                expires_at: Some(expires_at),
            },
            grant.id,
            auth.member_id,
            auth.token_id,
        )
        .await?;
    if let Err(err) = store
        .append_audit(maidan_types::NewAuditEvent {
            actor_id: Some(auth.actor_id),
            action: "token.delegate".into(),
            target_kind: Some("api_token".into()),
            target_id: Some(record.id.0),
            metadata: json!({
                "workspace_id": record.workspace_id.0,
                "delegate_id": auth.member_id.0,
                "subject_member_id": record.member_id.0,
                "grant_id": grant.id.0,
                "capabilities": record.capabilities.clone(),
                "expires_at": record.expires_at,
                "surface": "mcp",
                "parent_token_id": auth.token_id.map(|id| id.0),
            }),
        })
        .await
    {
        tracing::warn!(error = %err, "audit.write_failed");
    }
    Ok(content_json(&json!({
        "grant_id": grant.id.0,
        "delegate_id": auth.member_id.0,
        "id": record.id.0,
        "secret": secret.as_str(),
        "workspace_id": record.workspace_id.0,
        "member_id": record.member_id.0,
        "capabilities": record.capabilities,
        "expires_at": record.expires_at,
    })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateDelegationGrantArgs {
    workspace_id: uuid::Uuid,
    subject_id: uuid::Uuid,
    delegate_id: uuid::Uuid,
    capabilities: Vec<String>,
    purpose: String,
    expires_at: DateTime<Utc>,
}

pub(super) async fn create_delegation_grant(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: CreateDelegationGrantArgs = serde_json::from_value(args.clone())?;
    let workspace_id = WorkspaceId(a.workspace_id);
    auth.ensure_workspace(workspace_id)?;
    if let Some(unknown) = a
        .capabilities
        .iter()
        .find(|capability| !capability::is_known(capability))
    {
        return Err(McpError::InvalidParams(format!(
            "unknown delegated capability: {unknown}"
        )));
    }
    if let Some(authority) = a
        .capabilities
        .iter()
        .find(|capability| !capability::is_delegatable(capability))
    {
        return Err(McpError::InvalidParams(format!(
            "{authority} cannot be delegated: a grant lends the ability to do work, \
             never the means to hand out more authority"
        )));
    }
    let subject_id = MemberId(a.subject_id);
    let delegate_id = MemberId(a.delegate_id);
    for member_id in [subject_id, delegate_id] {
        let member = store.get_member(member_id).await?;
        if member.workspace_id != workspace_id {
            return Err(McpError::InvalidParams(
                "subject and delegate must belong to the workspace".into(),
            ));
        }
    }
    let grant = store
        .create_delegation_grant(NewDelegationGrant {
            workspace_id,
            subject_id,
            delegate_id,
            capabilities: a.capabilities,
            purpose: a.purpose,
            authorized_by: auth.actor_id,
            expires_at: a.expires_at,
        })
        .await?;
    if let Err(err) = store
        .append_audit(NewAuditEvent {
            actor_id: Some(auth.actor_id),
            action: "delegation_grant.create".into(),
            target_kind: Some("delegation_grant".into()),
            target_id: Some(grant.id.0),
            metadata: json!({
                "workspace_id": workspace_id.0,
                "subject_id": grant.subject_id.0,
                "delegate_id": grant.delegate_id.0,
                "capabilities": grant.capabilities.clone(),
                "expires_at": grant.expires_at,
                "purpose": grant.purpose.clone(),
                "surface": "mcp",
            }),
        })
        .await
    {
        tracing::warn!(error = %err, "audit.write_failed");
    }
    Ok(content_json(&grant))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListDelegationGrantsArgs {
    workspace_id: uuid::Uuid,
}

pub(super) async fn list_delegation_grants(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ListDelegationGrantsArgs = serde_json::from_value(args.clone())?;
    let workspace_id = WorkspaceId(a.workspace_id);
    auth.ensure_workspace(workspace_id)?;
    Ok(content_json(
        &store.list_delegation_grants(workspace_id).await?,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RevokeDelegationGrantArgs {
    workspace_id: uuid::Uuid,
    grant_id: uuid::Uuid,
}

pub(super) async fn revoke_delegation_grant(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: RevokeDelegationGrantArgs = serde_json::from_value(args.clone())?;
    let workspace_id = WorkspaceId(a.workspace_id);
    let grant_id = DelegationGrantId(a.grant_id);
    auth.ensure_workspace(workspace_id)?;
    let existing = store.get_delegation_grant(grant_id).await?;
    if existing.workspace_id != workspace_id {
        return Err(McpError::NotFound);
    }
    store
        .revoke_delegation_grant(workspace_id, grant_id)
        .await?;
    let grant = store.get_delegation_grant(grant_id).await?;
    if let Err(err) = store
        .append_audit(NewAuditEvent {
            actor_id: Some(auth.actor_id),
            action: "delegation_grant.revoke".into(),
            target_kind: Some("delegation_grant".into()),
            target_id: Some(grant.id.0),
            metadata: json!({
                "workspace_id": workspace_id.0,
                "subject_id": grant.subject_id.0,
                "delegate_id": grant.delegate_id.0,
                "surface": "mcp",
            }),
        })
        .await
    {
        tracing::warn!(error = %err, "audit.write_failed");
    }
    Ok(content_json(&grant))
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
        let admin = AuthContext::from_session(member, ws, vec![TOKEN_ADMIN.to_string()]);
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

        let subject = store
            .create_member(NewMember {
                workspace_id: ws,
                handle: "subject".into(),
                display_name: None,
                kind: MemberKind::Agent,
            })
            .await
            .unwrap();
        let grant = content(
            &server
                .call_tool(
                    &admin,
                    "create_delegation_grant",
                    &json!({
                        "workspace_id": ws.0,
                        "subject_id": subject.id.0,
                        "delegate_id": member.0,
                        "capabilities": [WORKSPACE_READ, MESSAGE_POST],
                        "purpose": "mcp exchange",
                        "expires_at": Utc::now() + chrono::Duration::hours(2),
                    }),
                )
                .await
                .unwrap(),
        );
        let grant_id = uuid::Uuid::parse_str(grant["id"].as_str().unwrap()).unwrap();
        let listed = content(
            &server
                .call_tool(
                    &admin,
                    "list_delegation_grants",
                    &json!({ "workspace_id": ws.0 }),
                )
                .await
                .unwrap(),
        );
        assert_eq!(listed.as_array().unwrap().len(), 1);
        let delegated = content(
            &server
                .call_tool(&reader, "delegate_token", &json!({ "grant_id": grant_id }))
                .await
                .unwrap(),
        );
        assert_eq!(delegated["member_id"], json!(subject.id.0));
        assert_eq!(delegated["capabilities"], json!([WORKSPACE_READ]));
        let delegated_secret = delegated["secret"].as_str().unwrap();
        let resolved = maidan_auth::resolve_bearer(store.as_ref(), delegated_secret)
            .await
            .unwrap();
        assert_eq!(resolved.member_id, subject.id);
        assert_eq!(resolved.actor_id, member);
        assert_eq!(resolved.delegation_grant_id.map(|id| id.0), Some(grant_id));
        server
            .call_tool(&resolved, "whoami", &json!({}))
            .await
            .unwrap();
        assert!(server
            .call_tool(
                &resolved,
                "create_delegation_grant",
                &json!({ "workspace_id": ws.0 }),
            )
            .await
            .is_err());
        let decisions: Vec<_> = store
            .list_audit_for_workspace(ws, 50)
            .await
            .unwrap()
            .into_iter()
            .filter(|event| event.action == "authorization.decision")
            .collect();
        assert!(decisions.iter().any(|event| {
            event.actor_id == Some(member)
                && event.metadata["subject_id"] == json!(subject.id.0)
                && event.metadata["grant_id"] == json!(grant_id)
                && event.metadata["outcome"] == "allowed"
        }));
        assert!(decisions
            .iter()
            .any(|event| event.metadata["outcome"] == "denied"));
        let revoked = content(
            &server
                .call_tool(
                    &admin,
                    "revoke_delegation_grant",
                    &json!({ "workspace_id": ws.0, "grant_id": grant_id }),
                )
                .await
                .unwrap(),
        );
        assert!(revoked["revoked_at"].is_string());
        assert!(
            maidan_auth::resolve_bearer(store.as_ref(), delegated_secret)
                .await
                .is_err()
        );

        // Discovery type is the public document; tools never leak a tenant list.
        let doc = maidan_types::RoomDiscovery::document();
        assert_eq!(doc.type_id, ROOM_DISCOVERY_TYPE);
    }
}

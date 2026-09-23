//! Operator tools for issuing and revoking read-only cross-organization share
//! tickets. Consumer reads stay on the dedicated REST surface.

use chrono::{DateTime, Utc};
use maidan_auth::{hash_secret, AuthContext, ShareTicketSecret};
use maidan_types::{ChannelId, NewAuditEvent, NewShareTicket, ShareTicketId};
use serde::Deserialize;
use serde_json::{json, Value};

use super::content_json;
use crate::error::McpError;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateArgs {
    channel_id: uuid::Uuid,
    expires_at: DateTime<Utc>,
    #[serde(default)]
    artifact_shas: Vec<String>,
}

pub(super) async fn create_share_ticket(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: CreateArgs = serde_json::from_value(args.clone())?;
    let secret = ShareTicketSecret::generate();
    let ticket = server
        .store
        .create_share_ticket(NewShareTicket {
            workspace_id: auth.workspace_id,
            channel_id: ChannelId(a.channel_id),
            owner_id: auth.member_id,
            created_by: auth.member_id,
            token_hash: hash_secret(secret.as_str()),
            expires_at: a.expires_at,
            artifact_shas: a.artifact_shas,
        })
        .await?;
    let artifact_shas = server.store.list_share_ticket_artifacts(ticket.id).await?;
    if let Err(err) = server
        .store
        .append_audit(NewAuditEvent {
            actor_id: Some(auth.member_id),
            action: "share_ticket.create".into(),
            target_kind: Some("share_ticket".into()),
            target_id: Some(ticket.id.0),
            metadata: json!({
                "workspace_id": auth.workspace_id.0,
                "channel_id": ticket.channel_id.0,
                "owner_id": ticket.owner_id.0,
                "expires_at": ticket.expires_at,
                "artifact_count": artifact_shas.len(),
            }),
        })
        .await
    {
        tracing::warn!(error = %err, "audit.write_failed");
    }
    Ok(content_json(&json!({
        "ticket": ticket,
        "artifact_shas": artifact_shas,
        "secret": secret.as_str(),
    })))
}

pub(super) async fn list_share_tickets(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    _args: &Value,
) -> Result<Value, McpError> {
    let tickets = server.store.list_share_tickets(auth.workspace_id).await?;
    let mut response = Vec::with_capacity(tickets.len());
    for ticket in tickets {
        let artifact_shas = server.store.list_share_ticket_artifacts(ticket.id).await?;
        response.push(json!({ "ticket": ticket, "artifact_shas": artifact_shas }));
    }
    Ok(content_json(&response))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RevokeArgs {
    ticket_id: uuid::Uuid,
}

pub(super) async fn revoke_share_ticket(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: RevokeArgs = serde_json::from_value(args.clone())?;
    let ticket_id = ShareTicketId(a.ticket_id);
    if !server
        .store
        .revoke_share_ticket(auth.workspace_id, ticket_id)
        .await?
    {
        return Err(McpError::InvalidParams("share ticket not found".into()));
    }
    if let Err(err) = server
        .store
        .append_audit(NewAuditEvent {
            actor_id: Some(auth.member_id),
            action: "share_ticket.revoke".into(),
            target_kind: Some("share_ticket".into()),
            target_id: Some(ticket_id.0),
            metadata: json!({ "workspace_id": auth.workspace_id.0 }),
        })
        .await
    {
        tracing::warn!(error = %err, "audit.write_failed");
    }
    Ok(content_json(&json!({ "revoked": true })))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::Duration as ChronoDuration;
    use maidan_artifacts::LocalFsStore;
    use maidan_auth::capability::{TOKEN_ADMIN, WORKSPACE_READ};
    use maidan_search::HashV1Provider;
    use maidan_store::{prelude::*, run_sqlite_migrations};
    use maidan_types::{
        ArtifactKind, MemberKind, NewArtifact, NewChannel, NewMember, NewWorkspace,
    };
    use sqlx::sqlite::SqlitePoolOptions;

    use super::*;

    fn content(value: Value) -> Value {
        serde_json::from_str(value["content"][0]["text"].as_str().unwrap()).unwrap()
    }

    #[tokio::test]
    async fn issuer_tools_disclose_the_secret_once_and_honor_revocation() {
        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        run_sqlite_migrations(&pool).await.unwrap();
        let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
        let ws = store
            .create_workspace(NewWorkspace {
                name: "owner".into(),
            })
            .await
            .unwrap();
        let owner = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: "owner".into(),
                display_name: None,
                kind: MemberKind::Human,
            })
            .await
            .unwrap();
        let channel = store
            .create_channel(NewChannel {
                workspace_id: ws.id,
                name: "incident".into(),
                topic: None,
                private: true,
            })
            .await
            .unwrap();
        let sha = "b".repeat(64);
        store
            .upsert_artifact(NewArtifact {
                sha256: sha.clone(),
                size_bytes: 1,
                mime_type: None,
                kind: ArtifactKind::Attachment,
                uploaded_by: Some(owner.id),
            })
            .await
            .unwrap();
        store.record_artifact_ref(ws.id, &sha).await.unwrap();
        let server = crate::server::McpServer::new(
            store.clone(),
            Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
            Arc::new(maidan_search::SqliteSearch::new(pool)),
            Arc::new(HashV1Provider),
        );
        let admin = AuthContext::from_session(owner.id, ws.id, vec![TOKEN_ADMIN.to_string()]);
        let forged = server
            .call_tool(
                &admin,
                "create_share_ticket",
                &json!({
                    "channel_id": channel.id.0,
                    "owner_id": uuid::Uuid::new_v4(),
                    "expires_at": Utc::now() + ChronoDuration::hours(2),
                }),
            )
            .await
            .expect_err("share-ticket ownership must not come from MCP arguments");
        assert!(matches!(forged, McpError::InvalidParams(_)));

        let created = content(
            server
                .call_tool(
                    &admin,
                    "create_share_ticket",
                    &json!({
                        "channel_id": channel.id.0,
                        "expires_at": Utc::now() + ChronoDuration::hours(2),
                        "artifact_shas": [sha],
                    }),
                )
                .await
                .unwrap(),
        );
        let secret = created["secret"].as_str().unwrap();
        let ticket_id = created["ticket"]["id"].as_str().unwrap();
        assert!(secret.starts_with("maid_share_"));
        assert_eq!(created["ticket"]["owner_id"], json!(owner.id.0));
        assert!(created["ticket"].get("token_hash").is_none());

        let listed = content(
            server
                .call_tool(&admin, "list_share_tickets", &json!({}))
                .await
                .unwrap(),
        );
        assert_eq!(listed.as_array().unwrap().len(), 1);
        assert!(!listed.to_string().contains(secret));
        assert!(!listed.to_string().contains("token_hash"));

        let read_only =
            AuthContext::from_session(owner.id, ws.id, vec![WORKSPACE_READ.to_string()]);
        assert!(server
            .call_tool(&read_only, "list_share_tickets", &json!({}))
            .await
            .is_err());

        let revoked = content(
            server
                .call_tool(
                    &admin,
                    "revoke_share_ticket",
                    &json!({ "ticket_id": ticket_id }),
                )
                .await
                .unwrap(),
        );
        assert_eq!(revoked["revoked"], true);
        assert!(store
            .get_share_ticket(ShareTicketId(ticket_id.parse().unwrap()))
            .await
            .unwrap()
            .revoked_at
            .is_some());

        let audit = store.list_audit_for_workspace(ws.id, 20).await.unwrap();
        let serialized_audit = serde_json::to_string(&audit).unwrap();
        assert!(serialized_audit.contains("share_ticket.create"));
        assert!(serialized_audit.contains("share_ticket.revoke"));
        assert!(!serialized_audit.contains(secret));
        assert!(!serialized_audit.contains(&hash_secret(secret)));
    }
}

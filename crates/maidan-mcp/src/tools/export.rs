//! Signed workspace export / verify / import.
//!
//! Twins of REST `GET /workspaces/:id/export`, `POST
//! /workspaces/export/verify`, and `POST /workspaces/import`. All three are
//! `token:admin`. Signing and verify-key pins live on
//! [`crate::server::McpServer`], set from `main.rs` the same way
//! `set_encryption_key` is.

use maidan_auth::AuthContext;
use maidan_store::{build_workspace_export, StoreError};
use maidan_types::{
    flatten_export, remap_import, SignedExport, TokenPolicy, WorkspaceExport, WorkspaceId,
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use super::content_json;
use crate::error::McpError;
use crate::server::McpServer;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportArgs {
    #[serde(default)]
    workspace_id: Option<Uuid>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportArgs {
    envelope: SignedExport,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    force: bool,
}

fn parse_envelope(args: &Value) -> Result<SignedExport, McpError> {
    if args.get("envelope").is_some() {
        serde_json::from_value(args["envelope"].clone())
            .map_err(|e| McpError::InvalidParams(format!("signed envelope: {e}")))
    } else {
        serde_json::from_value(args.clone())
            .map_err(|e| McpError::InvalidParams(format!("signed envelope: {e}")))
    }
}

fn payload_workspace_id(payload: &Value) -> Option<Uuid> {
    payload
        .get("workspace")
        .and_then(|w| w.get("id"))
        .and_then(|id| id.as_str())
        .and_then(|s| s.parse().ok())
}

fn verify_pin(server: &McpServer) -> Option<&[[u8; 32]]> {
    let keys = server.export_verify_keys();
    if keys.is_empty() {
        None
    } else {
        Some(keys)
    }
}

/// Export the caller's workspace (or `workspace_id`) as a signed envelope.
pub(super) async fn export_workspace(
    server: &McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ExportArgs = serde_json::from_value(args.clone())?;
    let workspace_id = WorkspaceId(a.workspace_id.unwrap_or(auth.workspace_id.0));
    auth.ensure_workspace(workspace_id)?;
    let Some(key) = server.export_signing() else {
        return Err(McpError::InvalidParams(
            "signed export is not configured: set MAIDAN_EXPORT_SIGNING_KEY".into(),
        ));
    };
    let inner = build_workspace_export(server.store.as_ref(), workspace_id).await?;
    let payload = serde_json::to_value(&inner)
        .map_err(|e| McpError::Internal(format!("export serialize: {e}")))?;
    let signed = maidan_auth::sign_export(key, payload)
        .map_err(|e| McpError::InvalidParams(e.to_string()))?;
    // As on REST: a read, but the whole workspace leaves in it. Recorded
    // before the bundle is released, and withheld if it cannot be.
    server
        .store
        .append_audit(maidan_types::NewAuditEvent {
            actor_id: Some(auth.actor_id),
            action: "workspace.export".into(),
            target_kind: Some("workspace".into()),
            target_id: Some(workspace_id.0),
            metadata: json!({ "content_sha256": signed.content_sha256, "surface": "mcp" }),
        })
        .await?;
    Ok(content_json(&signed))
}

/// Verify a signed export without importing it. A blank instance uses this
/// to check the file before `import_workspace`.
pub(super) fn verify_workspace_export(server: &McpServer, args: &Value) -> Result<Value, McpError> {
    let signed = parse_envelope(args)?;
    maidan_auth::verify_export(&signed, verify_pin(server))
        .map_err(|e| McpError::InvalidParams(e.to_string()))?;
    Ok(content_json(&json!({
        "ok": true,
        "token_policy": signed.token_policy,
        "public_key": signed.public_key,
        "content_sha256": signed.content_sha256,
        "workspace_id": payload_workspace_id(&signed.payload),
    })))
}

/// Verify then import a signed envelope. `mode=new` remaps ids; `restore` keeps
/// them (conflict unless `force`).
///
/// Takes `auth` for the reason the REST twin does: the signature proves
/// integrity, never authority, so a `restore` names a caller-supplied workspace
/// that has to be scoped. This handler previously took no [`AuthContext`] at
/// all, which made scoping structurally impossible.
pub(super) async fn import_workspace(
    server: &McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    // Two accepted shapes: the envelope nested under `envelope` (which can also
    // carry `mode`/`force`), or the bare envelope as the whole argument object.
    //
    // Which one the caller meant is decided by the presence of the `envelope`
    // key, NOT by whether the nested shape happens to parse. The old `Err(_) =>
    // defaults` swallowed the reason: a nested envelope with a malformed
    // `mode`, or a typo'd key now that the struct is strict, fell through to
    // the bare path and silently ran with `mode: "new"` — the caller asked for
    // a restore and got a detached workspace, with no error.
    let a = if args.get("envelope").is_some() {
        serde_json::from_value::<ImportArgs>(args.clone())
            .map_err(|e| McpError::InvalidParams(format!("import arguments: {e}")))?
    } else {
        ImportArgs {
            envelope: parse_envelope(args)?,
            mode: None,
            force: false,
        }
    };
    maidan_auth::verify_export(&a.envelope, verify_pin(server))
        .map_err(|e| McpError::InvalidParams(e.to_string()))?;
    let bundle: WorkspaceExport =
        serde_json::from_value(a.envelope.payload.clone()).map_err(|e| {
            McpError::InvalidParams(format!("export payload is not a workspace bundle: {e}"))
        })?;

    let mode = a.mode.as_deref().unwrap_or("new");
    let flat = flatten_export(bundle);
    let (to_write, replace_existing) = match mode {
        "new" => (remap_import(flat, Uuid::new_v4), false),
        "restore" => {
            auth.ensure_workspace(flat.workspace.id)?;
            match server.store.get_workspace(flat.workspace.id).await {
                Ok(_) if !a.force => {
                    return Err(McpError::InvalidParams(format!(
                        "workspace {} already exists; pass force=true to overwrite",
                        flat.workspace.id.0
                    )));
                }
                // As on REST, the store erases in the import's transaction and
                // refuses a workspace under legal hold. This path erased with
                // no hold check before.
                Ok(_) => (flat, true),
                Err(StoreError::NotFound) => (flat, false),
                Err(e) => return Err(e.into()),
            }
        }
        other => {
            return Err(McpError::InvalidParams(format!(
                "unknown import mode {other:?}; expected new or restore"
            )))
        }
    };

    let workspace_id = to_write.workspace.id;
    server
        .store
        .import_workspace_audited(
            &to_write,
            replace_existing,
            maidan_types::NewAuditEvent {
                actor_id: Some(auth.actor_id),
                action: "workspace.import".into(),
                target_kind: Some("workspace".into()),
                target_id: Some(workspace_id.0),
                metadata: json!({
                    "mode": mode,
                    "force": a.force,
                    "replaced_existing": replace_existing,
                    "surface": "mcp",
                }),
            },
        )
        .await?;
    Ok(content_json(&json!({
        "workspace_id": workspace_id,
        "mode": mode,
        "token_policy": TokenPolicy::TokensDieOnExport,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use maidan_artifacts::LocalFsStore;
    use maidan_auth::{capability::TOKEN_ADMIN, AuthContext, ExportSigningKey};
    use maidan_search::HashV1Provider;
    use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
    use maidan_types::*;
    use serde_json::json;
    use sqlx::sqlite::SqlitePoolOptions;

    const SEED: [u8; 32] = [0x11; 32];

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

    async fn seed(store: &dyn Store) -> (WorkspaceId, MemberId) {
        let ws = store
            .create_workspace(NewWorkspace {
                name: "origin".into(),
            })
            .await
            .unwrap();
        let alice = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: "alice".into(),
                display_name: None,
                kind: MemberKind::Human,
            })
            .await
            .unwrap();
        let channel = store
            .create_channel(NewChannel {
                workspace_id: ws.id,
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
        store
            .post_message(NewMessage {
                thread_id: thread.id,
                author_id: alice.id,
                body: "hello signed".into(),
                metadata: json!({}),
                content: None,
            })
            .await
            .unwrap();
        (ws.id, alice.id)
    }

    #[tokio::test]
    async fn export_verify_import_on_a_blank_store_and_reject_tamper() {
        let (origin_store, origin_pool) = blank_store().await;
        let (ws, alice) = seed(origin_store.as_ref()).await;
        let origin = mcp(origin_store.clone(), origin_pool);
        origin.set_export_signing(ExportSigningKey::from_seed(SEED));
        let auth = AuthContext::from_session(alice, ws, vec![TOKEN_ADMIN.to_string()]);

        let exported = content(
            &origin
                .call_tool(&auth, "export_workspace", &json!({}))
                .await
                .unwrap(),
        );
        assert_eq!(exported["$type"], "maidan.workspace.export/1");
        assert_eq!(exported["token_policy"], "tokens_die_on_export");
        assert!(exported["payload"].get("token_hash").is_none());
        assert!(exported["payload"].get("tokens").is_none());

        let (dest_store, dest_pool) = blank_store().await;
        let dest_ws = dest_store
            .create_workspace(NewWorkspace {
                name: "dest-admin".into(),
            })
            .await
            .unwrap();
        let dest_admin = dest_store
            .create_member(NewMember {
                workspace_id: dest_ws.id,
                handle: "ops".into(),
                display_name: None,
                kind: MemberKind::Human,
            })
            .await
            .unwrap();
        let dest = mcp(dest_store.clone(), dest_pool);
        let dest_auth =
            AuthContext::from_session(dest_admin.id, dest_ws.id, vec![TOKEN_ADMIN.to_string()]);

        let verified = content(
            &dest
                .call_tool(&dest_auth, "verify_workspace_export", &exported)
                .await
                .unwrap(),
        );
        assert_eq!(verified["ok"], true);
        assert_eq!(verified["token_policy"], "tokens_die_on_export");
        assert_eq!(verified["workspace_id"], json!(ws.0.to_string()));

        let imported = content(
            &dest
                .call_tool(
                    &dest_auth,
                    "import_workspace",
                    &json!({ "envelope": exported, "mode": "new" }),
                )
                .await
                .unwrap(),
        );
        let new_ws = WorkspaceId(imported["workspace_id"].as_str().unwrap().parse().unwrap());
        assert_ne!(new_ws, ws);
        let members = dest_store.list_members(new_ws).await.unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].handle, "alice");
        let tokens = dest_store
            .list_api_tokens_for_member(new_ws, members[0].id)
            .await
            .unwrap();
        assert!(tokens.is_empty(), "tokens die on export");

        let mut flipped = exported.clone();
        flipped["payload"]["workspace"]["name"] = json!("evil");
        assert!(dest
            .call_tool(&dest_auth, "verify_workspace_export", &flipped)
            .await
            .is_err());

        let mut stuffed = exported.clone();
        stuffed["payload"]["token_hash"] = json!("abc");
        assert!(dest
            .call_tool(&dest_auth, "verify_workspace_export", &stuffed)
            .await
            .is_err());
    }

    /// REST refused a forced restore over a held workspace; this path erased it.
    #[tokio::test]
    async fn a_forced_restore_over_a_held_workspace_is_refused() {
        let (store, pool) = blank_store().await;
        let (ws, alice) = seed(store.as_ref()).await;
        let server = mcp(store.clone(), pool);
        server.set_export_signing(ExportSigningKey::from_seed(SEED));
        let auth = AuthContext::from_session(alice, ws, vec![TOKEN_ADMIN.to_string()]);
        let exported = content(
            &server
                .call_tool(&auth, "export_workspace", &json!({}))
                .await
                .unwrap(),
        );
        store
            .place_legal_hold(ws, "litigation", Some(alice))
            .await
            .unwrap();

        let err = server
            .call_tool(
                &auth,
                "import_workspace",
                &json!({ "envelope": exported, "mode": "restore", "force": true }),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("legal hold"), "{err}");
        assert!(
            store.get_workspace(ws).await.is_ok(),
            "the held workspace survives"
        );
        assert_eq!(store.list_members(ws).await.unwrap().len(), 1);
    }

    /// The whole workspace leaves in an export, so it is withheld when its
    /// record cannot be written; an import that cannot be recorded does not
    /// happen.
    #[tokio::test]
    async fn an_export_or_import_that_cannot_be_recorded_does_not_happen() {
        let (store, pool) = blank_store().await;
        let (ws, alice) = seed(store.as_ref()).await;
        let server = mcp(store.clone(), pool.clone());
        server.set_export_signing(ExportSigningKey::from_seed(SEED));
        let auth = AuthContext::from_session(alice, ws, vec![TOKEN_ADMIN.to_string()]);
        let exported = content(
            &server
                .call_tool(&auth, "export_workspace", &json!({}))
                .await
                .unwrap(),
        );
        let workspaces = store.count_workspaces().await.unwrap();

        sqlx::query(
            "CREATE TRIGGER audit_down BEFORE INSERT ON maidan_audit
             BEGIN SELECT RAISE(ABORT, 'audit down'); END",
        )
        .execute(&pool)
        .await
        .unwrap();

        assert!(server
            .call_tool(&auth, "export_workspace", &json!({}))
            .await
            .is_err());
        assert!(server
            .call_tool(
                &auth,
                "import_workspace",
                &json!({ "envelope": exported, "mode": "new" }),
            )
            .await
            .is_err());
        assert_eq!(store.count_workspaces().await.unwrap(), workspaces);
    }

    #[tokio::test]
    async fn export_without_a_signing_key_fails_closed() {
        let (store, pool) = blank_store().await;
        let (ws, alice) = seed(store.as_ref()).await;
        let server = mcp(store, pool);
        let auth = AuthContext::from_session(alice, ws, vec![TOKEN_ADMIN.to_string()]);
        let err = server
            .call_tool(&auth, "export_workspace", &json!({}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("MAIDAN_EXPORT_SIGNING_KEY"));
    }
}

//! Real callers for tests: a workspace with an admin and a bearer token,
//! minted through the store the way `maidan init` mints the first one.
//!
//! The server has no unauthenticated mode, so a test that drives it over
//! HTTP sends a token like any client. Built only under `cfg(test)` or the
//! `test-support` feature, which only dev-dependencies enable.

use std::time::Duration;

use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_store::Store;
use maidan_types::{MemberId, MemberKind, NewApiToken, NewMember, NewWorkspace, WorkspaceId};

/// A workspace, its first admin, and that admin's bearer secret.
#[derive(Debug, Clone)]
pub struct Tenant {
    pub workspace_id: WorkspaceId,
    pub admin_id: MemberId,
    /// The admin's bearer secret. It holds every capability.
    pub token: String,
}

impl Tenant {
    /// A client that sends the admin's bearer on every request.
    pub fn client(&self) -> reqwest::Client {
        bearer_client(&self.token)
    }

    /// A new member of this workspace and a token for it holding every
    /// capability.
    pub async fn member(&self, store: &dyn Store, handle: &str, kind: MemberKind) -> Caller {
        caller(store, self.workspace_id, handle, kind).await
    }
}

/// A member and its bearer secret.
#[derive(Debug, Clone)]
pub struct Caller {
    pub member_id: MemberId,
    pub token: String,
}

impl Caller {
    pub fn client(&self) -> reqwest::Client {
        bearer_client(&self.token)
    }
}

/// Opens `name` with a human admin `admin` holding every capability.
pub async fn tenant(store: &dyn Store, name: &str) -> Tenant {
    let (workspace, _) = store
        .create_workspace_with_event(NewWorkspace {
            name: name.to_string(),
        })
        .await
        .unwrap_or_else(|err| panic!("create workspace {name}: {err}"));
    let admin = caller(store, workspace.id, "admin", MemberKind::Human).await;
    Tenant {
        workspace_id: workspace.id,
        admin_id: admin.member_id,
        token: admin.token,
    }
}

/// A new member of `workspace_id` and a token for it holding every capability.
pub async fn caller(
    store: &dyn Store,
    workspace_id: WorkspaceId,
    handle: &str,
    kind: MemberKind,
) -> Caller {
    let (member, _) = store
        .create_member_with_event(NewMember {
            workspace_id,
            handle: handle.to_string(),
            display_name: None,
            kind,
        })
        .await
        .unwrap_or_else(|err| panic!("create member {handle}: {err}"));
    let token = token(store, workspace_id, member.id, capability::all()).await;
    Caller {
        member_id: member.id,
        token,
    }
}

/// A bearer secret for an existing member, holding `capabilities`.
pub async fn token(
    store: &dyn Store,
    workspace_id: WorkspaceId,
    member_id: MemberId,
    capabilities: Vec<String>,
) -> String {
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id,
            member_id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: Some("test".into()),
            capabilities,
            expires_at: None,
        })
        .await
        .unwrap_or_else(|err| panic!("mint token: {err}"));
    secret.as_str().to_string()
}

/// A bearer secret holding every capability for an existing member.
pub async fn admin_token(store: &dyn Store, member_id: MemberId) -> String {
    let member = store
        .get_member(member_id)
        .await
        .unwrap_or_else(|err| panic!("member {member_id:?}: {err}"));
    token(store, member.workspace_id, member_id, capability::all()).await
}

/// The `Authorization` value for `token`.
pub fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}

/// A client that sends `token` as its bearer on every request.
pub fn bearer_client(token: &str) -> reqwest::Client {
    let mut headers = reqwest::header::HeaderMap::new();
    let mut value = reqwest::header::HeaderValue::from_str(&bearer(token))
        .unwrap_or_else(|err| panic!("bearer header: {err}"));
    value.set_sensitive(true);
    headers.insert(reqwest::header::AUTHORIZATION, value);
    reqwest::Client::builder()
        .default_headers(headers)
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_else(|err| panic!("client: {err}"))
}

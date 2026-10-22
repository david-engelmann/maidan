use maidan_store::Store;
use maidan_types::{ApiToken, ApiTokenId, Peer, WorkspaceId};

use crate::context::AuthContext;
use crate::error::AuthError;
use crate::token::{hash_secret, hashes_equal};

/// Resolve a bearer secret to an [`AuthContext`] via the store.
///
/// A token minted by the OAuth authorization server is refused here: it is
/// bound to the MCP resource (RFC 8707), and only
/// [`resolve_resource_bearer`] accepts it. Refusing by default means every
/// surface that resolves a bearer (REST, WebSocket, gRPC, a session made
/// from a token) is closed to it without each having to remember.
pub async fn resolve_bearer(store: &dyn Store, bearer: &str) -> Result<AuthContext, AuthError> {
    let token = active_token(store, bearer).await?;
    if token.oauth_grant_id.is_some() {
        return Err(AuthError::Unauthorized);
    }
    active_token_context(store, token).await
}

/// [`resolve_bearer`] for the one route an OAuth token is bound to, the MCP
/// resource: it also accepts a token minted under an OAuth grant. Call it
/// only once the request is known to be for that resource.
pub async fn resolve_resource_bearer(
    store: &dyn Store,
    bearer: &str,
) -> Result<AuthContext, AuthError> {
    let token = active_token(store, bearer).await?;
    active_token_context(store, token).await
}

async fn active_token(store: &dyn Store, bearer: &str) -> Result<ApiToken, AuthError> {
    let computed = hash_secret(bearer);
    let token = store.get_active_api_token_by_hash(&computed).await?;
    if !hashes_equal(&token.token_hash, &computed) {
        return Err(AuthError::Unauthorized);
    }
    Ok(token)
}

/// Resolve a token by id to the [`AuthContext`] its bearer would get, for a
/// browser session made from it. The token must still be live by the same
/// test a bearer passes — not revoked or rotated away, not expired, its grant
/// and app installation live — so the session ends when the token does.
pub async fn resolve_token_id(
    store: &dyn Store,
    token_id: ApiTokenId,
) -> Result<AuthContext, AuthError> {
    let token = store.get_api_token(token_id).await?;
    let active = store
        .get_active_api_token_by_hash(&token.token_hash)
        .await?;
    if active.id != token_id {
        return Err(AuthError::Unauthorized);
    }
    active_token_context(store, active).await
}

async fn active_token_context(
    store: &dyn Store,
    token: ApiToken,
) -> Result<AuthContext, AuthError> {
    ensure_workspace_active(store, token.workspace_id).await?;
    if let Some(grant_id) = token.delegation_grant_id {
        let grant = store.get_delegation_grant(grant_id).await?;
        return Ok(AuthContext::from_delegated_token(
            token.id,
            grant.delegate_id,
            token.member_id,
            token.workspace_id,
            grant_id,
            token.capabilities,
        ));
    }
    Ok(token_to_context(token))
}

/// Refuse every credential of a suspended workspace with
/// [`maidan_types::WORKSPACE_SUSPENDED`] (a 403, not a 401: the credential is
/// good, the workspace is not open). Every token path resolves through here,
/// so bearer, session, WebSocket, stream re-check and A2A refuse alike.
pub async fn ensure_workspace_active(
    store: &dyn Store,
    workspace_id: WorkspaceId,
) -> Result<(), AuthError> {
    match store.get_workspace_suspension(workspace_id).await? {
        Some(_) => Err(AuthError::Forbidden(
            maidan_types::WORKSPACE_SUSPENDED.to_string(),
        )),
        None => Ok(()),
    }
}

/// Resolve a federation peer bearer to a [`Peer`] row.
pub async fn resolve_peer_bearer(store: &dyn Store, bearer: &str) -> Result<Peer, AuthError> {
    let computed = hash_secret(bearer);
    let peer = store.get_peer_by_token_hash(&computed).await?;
    if !hashes_equal(&peer.token_hash, &computed) {
        return Err(AuthError::Unauthorized);
    }
    Ok(peer)
}

fn token_to_context(token: ApiToken) -> AuthContext {
    match token.app_installation_id {
        Some(installation_id) => AuthContext::from_app_token(
            token.id,
            token.member_id,
            token.workspace_id,
            installation_id,
            token.capabilities,
        ),
        None => AuthContext::from_token(
            token.id,
            token.member_id,
            token.workspace_id,
            token.capabilities,
        ),
    }
}

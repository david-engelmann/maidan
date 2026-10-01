use maidan_store::Store;
use maidan_types::{ApiToken, ApiTokenId, Peer};

use crate::context::AuthContext;
use crate::error::AuthError;
use crate::token::{hash_secret, hashes_equal};

/// Resolve a bearer secret to an [`AuthContext`] via the store.
pub async fn resolve_bearer(store: &dyn Store, bearer: &str) -> Result<AuthContext, AuthError> {
    let computed = hash_secret(bearer);
    let token = store.get_active_api_token_by_hash(&computed).await?;
    if !hashes_equal(&token.token_hash, &computed) {
        return Err(AuthError::Unauthorized);
    }
    active_token_context(store, token).await
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

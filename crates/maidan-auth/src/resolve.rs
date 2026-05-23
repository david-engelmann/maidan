use maidan_store::Store;
use maidan_types::ApiToken;

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
    Ok(token_to_context(token))
}

fn token_to_context(token: ApiToken) -> AuthContext {
    AuthContext::from_token(token.member_id, token.workspace_id, token.capabilities)
}

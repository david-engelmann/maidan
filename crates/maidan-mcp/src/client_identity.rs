//! Which client a model decided a gate through, and how far that name can be
//! trusted (Next 17, David's 2a, 2026-10-09).
//!
//! Strongest first:
//!
//! 1. **The credential.** A token issued to a registered client names that
//!    client, and the server vouches for it. On `main` that is a token minted
//!    for an installed app (an app-installation token, from the install or
//!    the installed-app OAuth code flow): the record names the app's
//!    registered name and id. A token issued to a registered OAuth client
//!    (Next 23, #1324/#1337) is the other credential source; see
//!    [`oauth_client`].
//! 2. **`clientInfo`** from the request's `_meta`: what the client called
//!    itself, kept and shown as self-reported.
//! 3. Nothing: "an unidentified MCP client".

use maidan_auth::AuthContext;
use maidan_store::StoreError;
use maidan_types::{ClientIdentitySource, GateDecisionVia};

use crate::call_context::CallContext;
use crate::error::McpError;

/// A client the credential names: its registered name and id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialClient {
    pub id: String,
    pub name: String,
}

/// The decision record for a model's call: who asked through what, and how
/// the name is known. `model_asked` is always true here.
pub async fn decided_via(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    call: &CallContext,
) -> Result<GateDecisionVia, McpError> {
    if let Some(client) = credential_client(server, auth).await? {
        return Ok(GateDecisionVia {
            client_name: Some(client.name),
            client_version: None,
            client_id: Some(client.id),
            client_source: ClientIdentitySource::Credential,
            model_asked: true,
        });
    }
    Ok(from_client_info(call))
}

/// The record when the credential names no client.
pub fn from_client_info(call: &CallContext) -> GateDecisionVia {
    let source = if call.client.name.is_some() {
        ClientIdentitySource::SelfReported
    } else {
        ClientIdentitySource::None
    };
    GateDecisionVia {
        client_name: call.client.name.clone(),
        client_version: call.client.name.as_ref().and(call.client.version.clone()),
        client_id: None,
        client_source: source,
        model_asked: true,
    }
}

/// The client the caller's token was issued to, if it was issued to one.
async fn credential_client(
    server: &crate::server::McpServer,
    auth: &AuthContext,
) -> Result<Option<CredentialClient>, McpError> {
    if let Some(client) = oauth_client(auth) {
        return Ok(Some(client));
    }
    let Some(installation_id) = auth.app_installation_id else {
        return Ok(None);
    };
    let installation = match server.store.get_app_installation(installation_id).await {
        Ok(installation) => installation,
        Err(StoreError::NotFound) => return Ok(None),
        Err(err) => return Err(err.into()),
    };
    // Only an installation in the caller's own workspace names it; anything
    // else is a stale or foreign id and names nobody.
    if installation.workspace_id != auth.workspace_id {
        return Ok(None);
    }
    let app = match server.store.get_app(installation.app_id).await {
        Ok(app) => app,
        Err(StoreError::NotFound) => return Ok(None),
        Err(err) => return Err(err.into()),
    };
    Ok(Some(CredentialClient {
        id: app.id.0.to_string(),
        name: app.name,
    }))
}

/// The registered OAuth client a token was issued to.
///
/// TODO(Next 23, #1324/#1337): `main` has no OAuth client registry yet: the
/// authorization server lives on Muse's branch under
/// `crates/maidan-server/src/oauth/`, which this lane does not touch. When it
/// lands, key this on the token's issuing client (the client id the token row
/// records, reached from `auth.token_id`) and return the client's registered
/// name and `client_id`. Until then no token names an OAuth client, so this
/// branch of the precedence never applies.
fn oauth_client(_auth: &AuthContext) -> Option<CredentialClient> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::call_context::ClientInfo;

    fn call(name: Option<&str>, version: Option<&str>) -> CallContext {
        CallContext {
            client: ClientInfo {
                name: name.map(str::to_string),
                version: version.map(str::to_string),
            },
            ..CallContext::default()
        }
    }

    #[test]
    fn client_info_is_self_reported_and_nothing_is_none() {
        let via = from_client_info(&call(Some("ChatGPT"), Some("1.0")));
        assert_eq!(via.client_source, ClientIdentitySource::SelfReported);
        assert_eq!(via.client_name.as_deref(), Some("ChatGPT"));
        assert_eq!(via.client_id, None);
        let none = from_client_info(&call(None, Some("9")));
        assert_eq!(none.client_source, ClientIdentitySource::None);
        assert_eq!(none.client_version, None, "a version names no client");
        assert!(none.model_asked);
    }
}

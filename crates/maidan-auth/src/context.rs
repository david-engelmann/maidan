use maidan_types::{ApiTokenId, AppInstallationId, MemberId, WorkspaceId};

/// Resolved caller identity after bearer validation.
#[derive(Debug, Clone)]
pub struct AuthContext {
    pub token_id: Option<ApiTokenId>,
    pub member_id: MemberId,
    pub workspace_id: WorkspaceId,
    pub app_installation_id: Option<AppInstallationId>,
    capabilities: Vec<String>,
    pub bypass: bool,
}

impl AuthContext {
    pub fn from_token(
        token_id: ApiTokenId,
        member_id: MemberId,
        workspace_id: WorkspaceId,
        capabilities: Vec<String>,
    ) -> Self {
        Self {
            token_id: Some(token_id),
            member_id,
            workspace_id,
            app_installation_id: None,
            capabilities,
            bypass: false,
        }
    }

    pub fn from_app_token(
        token_id: ApiTokenId,
        member_id: MemberId,
        workspace_id: WorkspaceId,
        app_installation_id: AppInstallationId,
        capabilities: Vec<String>,
    ) -> Self {
        Self {
            token_id: Some(token_id),
            member_id,
            workspace_id,
            app_installation_id: Some(app_installation_id),
            capabilities,
            bypass: false,
        }
    }

    pub fn from_session(
        member_id: MemberId,
        workspace_id: WorkspaceId,
        capabilities: Vec<String>,
    ) -> Self {
        Self {
            token_id: None,
            member_id,
            workspace_id,
            app_installation_id: None,
            capabilities,
            bypass: false,
        }
    }

    pub fn bypass() -> Self {
        Self {
            token_id: None,
            member_id: MemberId(uuid::Uuid::nil()),
            workspace_id: WorkspaceId(uuid::Uuid::nil()),
            app_installation_id: None,
            capabilities: Vec::new(),
            bypass: true,
        }
    }

    pub fn has_capability(&self, cap: &str) -> bool {
        self.bypass || self.capabilities.iter().any(|c| c == cap)
    }

    /// The capabilities granted to this caller (Cluster 336 — `whoami` / `GET /me`
    /// self-discovery). Empty for a bypass caller (auth disabled).
    pub fn capabilities(&self) -> &[String] {
        &self.capabilities
    }

    pub fn require_capability(&self, cap: &str) -> Result<(), crate::AuthError> {
        if self.has_capability(cap) {
            Ok(())
        } else {
            Err(crate::AuthError::Forbidden(format!(
                "missing capability: {cap}"
            )))
        }
    }

    pub fn ensure_workspace(&self, workspace_id: WorkspaceId) -> Result<(), crate::AuthError> {
        if self.bypass || self.workspace_id == workspace_id {
            Ok(())
        } else {
            Err(crate::AuthError::Forbidden(
                "token is not valid for this workspace".into(),
            ))
        }
    }
}

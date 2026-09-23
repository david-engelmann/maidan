use maidan_types::{ApiTokenId, AppInstallationId, DelegationGrantId, MemberId, WorkspaceId};

/// Resolved caller identity after bearer validation.
#[derive(Debug, Clone)]
pub struct AuthContext {
    pub token_id: Option<ApiTokenId>,
    /// The member presenting authority. For ordinary credentials this equals
    /// `member_id`; for a delegated credential it is the grant's delegate.
    pub actor_id: MemberId,
    /// The member actions are attributed to.
    pub member_id: MemberId,
    pub workspace_id: WorkspaceId,
    pub app_installation_id: Option<AppInstallationId>,
    pub delegation_grant_id: Option<DelegationGrantId>,
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
            actor_id: member_id,
            member_id,
            workspace_id,
            app_installation_id: None,
            delegation_grant_id: None,
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
            actor_id: member_id,
            member_id,
            workspace_id,
            app_installation_id: Some(app_installation_id),
            delegation_grant_id: None,
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
            actor_id: member_id,
            member_id,
            workspace_id,
            app_installation_id: None,
            delegation_grant_id: None,
            capabilities,
            bypass: false,
        }
    }

    pub fn bypass() -> Self {
        Self {
            token_id: None,
            actor_id: MemberId(uuid::Uuid::nil()),
            member_id: MemberId(uuid::Uuid::nil()),
            workspace_id: WorkspaceId(uuid::Uuid::nil()),
            app_installation_id: None,
            delegation_grant_id: None,
            capabilities: Vec::new(),
            bypass: true,
        }
    }

    pub fn from_delegated_token(
        token_id: ApiTokenId,
        actor_id: MemberId,
        subject_id: MemberId,
        workspace_id: WorkspaceId,
        delegation_grant_id: DelegationGrantId,
        capabilities: Vec<String>,
    ) -> Self {
        Self {
            token_id: Some(token_id),
            actor_id,
            member_id: subject_id,
            workspace_id,
            app_installation_id: None,
            delegation_grant_id: Some(delegation_grant_id),
            capabilities,
            bypass: false,
        }
    }

    pub fn has_capability(&self, cap: &str) -> bool {
        self.bypass || self.capabilities.iter().any(|c| c == cap)
    }

    /// The capabilities granted to this caller. Empty for a bypass caller (auth
    /// disabled).
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

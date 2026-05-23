use maidan_types::{MemberId, WorkspaceId};

/// Resolved caller identity after bearer validation.
#[derive(Debug, Clone)]
pub struct AuthContext {
    pub member_id: MemberId,
    pub workspace_id: WorkspaceId,
    capabilities: Vec<String>,
    pub bypass: bool,
}

impl AuthContext {
    pub fn from_token(
        member_id: MemberId,
        workspace_id: WorkspaceId,
        capabilities: Vec<String>,
    ) -> Self {
        Self {
            member_id,
            workspace_id,
            capabilities,
            bypass: false,
        }
    }

    pub fn bypass() -> Self {
        Self {
            member_id: MemberId(uuid::Uuid::nil()),
            workspace_id: WorkspaceId(uuid::Uuid::nil()),
            capabilities: Vec::new(),
            bypass: true,
        }
    }

    pub fn has_capability(&self, cap: &str) -> bool {
        self.bypass || self.capabilities.iter().any(|c| c == cap)
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

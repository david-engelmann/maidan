use maidan_types::{ApiTokenId, AppInstallationId, DelegationGrantId, MemberId, WorkspaceId};

/// Constructors only this crate may call in a build that runs. A test build
/// (`cfg(test)`, or the `test-support` feature that only dev-dependencies
/// turn on) sees them as `pub`, so a test can hold any caller it needs without
/// a credential round trip; `cargo build --release` never does.
macro_rules! sealed_constructor {
    ($(#[$meta:meta])* fn $name:ident $($rest:tt)*) => {
        #[cfg(any(test, feature = "test-support"))]
        $(#[$meta])*
        pub fn $name $($rest)*

        #[cfg(not(any(test, feature = "test-support")))]
        $(#[$meta])*
        pub(crate) fn $name $($rest)*
    };
}

/// Resolved caller identity after bearer validation.
///
/// Sealed: every field is private and every constructor is crate-private, so
/// code outside this crate can read a caller but never make or alter one. The
/// ways in are the resolvers ([`crate::resolve_bearer`],
/// [`crate::resolve_token_id`], [`crate::resolve_session`],
/// [`crate::resolve_anonymous_reader`]), each of which checks a credential or
/// a stored record first.
#[derive(Debug, Clone)]
pub struct AuthContext {
    token_id: Option<ApiTokenId>,
    /// The member presenting authority. For ordinary credentials this equals
    /// `member_id`; for a delegated credential it is the grant's delegate.
    actor_id: MemberId,
    /// The member actions are attributed to.
    member_id: MemberId,
    workspace_id: WorkspaceId,
    app_installation_id: Option<AppInstallationId>,
    delegation_grant_id: Option<DelegationGrantId>,
    capabilities: Vec<String>,
    pub bypass: bool,
    anonymous: bool,
}

impl AuthContext {
    /// The token this caller presented, if any.
    pub fn token_id(&self) -> Option<ApiTokenId> {
        self.token_id
    }

    /// The member presenting authority. For ordinary credentials this equals
    /// [`Self::member_id`]; for a delegated credential it is the delegate.
    pub fn actor_id(&self) -> MemberId {
        self.actor_id
    }

    /// The member actions are attributed to.
    pub fn member_id(&self) -> MemberId {
        self.member_id
    }

    pub fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }

    pub fn app_installation_id(&self) -> Option<AppInstallationId> {
        self.app_installation_id
    }

    pub fn delegation_grant_id(&self) -> Option<DelegationGrantId> {
        self.delegation_grant_id
    }

    sealed_constructor! {
        fn from_token(
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
                anonymous: false,
            }
        }
    }

    sealed_constructor! {
        fn from_app_token(
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
                anonymous: false,
            }
        }
    }

    sealed_constructor! {
        fn from_session(
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
                anonymous: false,
            }
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
            anonymous: false,
        }
    }

    sealed_constructor! {
        /// A caller with no credential on a dev instance's MCP endpoint, reading
        /// one synthetic workspace as `member_id`. It holds `workspace:read` and
        /// nothing else, and the MCP server also refuses every tool that writes,
        /// because some writes ask only for `workspace:read`. It is never a bypass,
        /// so every workspace check applies to it.
        fn anonymous_reader(member_id: MemberId, workspace_id: WorkspaceId) -> Self {
            Self {
                token_id: None,
                actor_id: member_id,
                member_id,
                workspace_id,
                app_installation_id: None,
                delegation_grant_id: None,
                capabilities: vec![crate::capability::WORKSPACE_READ.to_string()],
                bypass: false,
                anonymous: true,
            }
        }
    }

    /// Whether this caller presented no credential (see [`Self::anonymous_reader`]).
    pub fn is_anonymous(&self) -> bool {
        self.anonymous
    }

    sealed_constructor! {
        /// A borrowed context acts as `subject_id` on behalf of `actor_id`.
        ///
        /// Authority capabilities are dropped here, not merely refused when a grant
        /// is created: this is the one place a borrowed context is built, so filtering
        /// in the constructor makes it impossible for one to hold `token:admin` or any
        /// other authority capability — whatever the grant or token row says, and
        /// from whichever call site.
        fn from_delegated_token(
            token_id: ApiTokenId,
            actor_id: MemberId,
            subject_id: MemberId,
            workspace_id: WorkspaceId,
            delegation_grant_id: DelegationGrantId,
            capabilities: Vec<String>,
        ) -> Self {
            let capabilities = capabilities
                .into_iter()
                .filter(|capability| crate::capability::is_delegatable(capability))
                .collect();
            Self {
                token_id: Some(token_id),
                actor_id,
                member_id: subject_id,
                workspace_id,
                app_installation_id: None,
                delegation_grant_id: Some(delegation_grant_id),
                capabilities,
                bypass: false,
                anonymous: false,
            }
        }
    }

    /// Who is acting and for whom, for recording against whatever this request
    /// writes. `None` when auth is bypassed: there is no real principal, so
    /// claiming one — the nil member — would be recording a fiction.
    pub fn attribution(&self) -> Option<maidan_types::Attribution> {
        (!self.bypass).then_some(maidan_types::Attribution {
            actor_id: self.actor_id,
            subject_id: self.member_id,
            grant_id: self.delegation_grant_id,
        })
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

//! The outside services a workspace may connect with a credential of its own.
//!
//! A workspace stores one reserved secret per surface (see
//! [`crate::RESERVED_SECRET_PREFIX`]). The store applies the rule that decides
//! whether a workspace is served on a surface, and the server picks the
//! credential that acts for it, so both name a surface the same way.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::egress::EgressSurface;

/// A service a workspace can connect. `GithubBranch` egress uses the GitHub
/// connection, so it has no surface of its own here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum ConnectionSurface {
    Github,
    Slack,
    Linear,
}

impl ConnectionSurface {
    /// Every surface, in the order a listing shows them.
    pub const ALL: [ConnectionSurface; 3] = [Self::Github, Self::Slack, Self::Linear];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Github => "github",
            Self::Slack => "slack",
            Self::Linear => "linear",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "github" => Some(Self::Github),
            "slack" => Some(Self::Slack),
            "linear" => Some(Self::Linear),
            _ => None,
        }
    }

    /// The name a person reads.
    pub fn label(self) -> &'static str {
        match self {
            Self::Github => "GitHub",
            Self::Slack => "Slack",
            Self::Linear => "Linear",
        }
    }

    /// The workspace secret that holds this surface's connection. Reserved:
    /// no generic secret route or tool can create, list, read, substitute or
    /// delete it.
    pub fn secret_name(self) -> &'static str {
        match self {
            Self::Github => "maidan:github_connection",
            Self::Slack => "maidan:slack_connection",
            Self::Linear => "maidan:linear_connection",
        }
    }

    /// The connection an egress surface sends through.
    pub fn for_egress(surface: EgressSurface) -> Self {
        match surface {
            EgressSurface::Github | EgressSurface::GithubBranch => Self::Github,
            EgressSurface::Slack => Self::Slack,
            EgressSurface::Linear => Self::Linear,
        }
    }
}

impl fmt::Display for ConnectionSurface {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

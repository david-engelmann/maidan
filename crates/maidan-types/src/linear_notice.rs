//! Claim and release comments on a task's Linear issue.
//!
//! Where a workspace has turned them on, a task whose attributes name a
//! Linear issue gets one `CLAIM:` comment there when it is first claimed, and
//! one `RELEASED:` comment when it ends without merging. They go out through
//! the Linear delivery path with the same key and the same per-team
//! allowlist as a result's comment: no assignee, no new identity.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::ids::{ThreadId, WorkspaceId};

/// Which notice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinearNoticeKind {
    /// The task was claimed.
    Claim,
    /// The task ended without merging.
    Release,
}

impl LinearNoticeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claim => "claim",
            Self::Release => "release",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "claim" => Some(Self::Claim),
            "release" => Some(Self::Release),
            _ => None,
        }
    }
}

/// A notice due on a task's Linear issue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinearNoticeDue {
    pub workspace_id: WorkspaceId,
    pub thread_id: ThreadId,
    pub kind: LinearNoticeKind,
    /// The issue key from the task's attributes.
    pub issue: String,
    pub title: Option<String>,
    pub engine: Option<crate::TaskEngine>,
    /// The handle of the member holding the claim, for a claim notice.
    pub holder: Option<String>,
    /// Why the task was released, for a release notice.
    pub reason: Option<String>,
}

impl LinearNoticeDue {
    /// The comment: `CLAIM: pi/<task id> …` or `RELEASED: pi/<task id> …`,
    /// prefixed with the engine the task is routed to.
    pub fn body(&self) -> String {
        let task = format!(
            "{}/{}",
            self.engine.map_or("maidan", |engine| engine.as_str()),
            self.thread_id.0
        );
        let title = self
            .title
            .as_deref()
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .map(|title| format!(": {title}"))
            .unwrap_or_default();
        match self.kind {
            LinearNoticeKind::Claim => format!(
                "CLAIM: {task} claimed in Maidan by {}{title}",
                self.holder.as_deref().unwrap_or("an agent")
            ),
            LinearNoticeKind::Release => format!(
                "RELEASED: {task} {}{title}",
                self.reason.as_deref().unwrap_or("ended without merging")
            ),
        }
    }

    /// The outbox key for this notice: negative, so it never meets an event
    /// log id, and fixed per task and kind, so the outbox's own dedup holds
    /// too.
    pub fn outbox_key(&self) -> i64 {
        let mut hasher = Sha256::new();
        hasher.update(self.thread_id.0.as_bytes());
        hasher.update(self.kind.as_str().as_bytes());
        let digest = hasher.finalize();
        let mut first = [0u8; 8];
        first.copy_from_slice(&digest[..8]);
        -(i64::from_be_bytes(first) & i64::MAX) - 1
    }
}

/// Whether a workspace says its tasks' claims and releases on Linear.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct LinearClaimComments {
    pub enabled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn due(kind: LinearNoticeKind) -> LinearNoticeDue {
        LinearNoticeDue {
            workspace_id: WorkspaceId(uuid::Uuid::nil()),
            thread_id: ThreadId(uuid::Uuid::nil()),
            kind,
            issue: "ENG-1".into(),
            title: Some(" Name the venue ".into()),
            engine: Some(crate::TaskEngine::Pi),
            holder: Some("agent0".into()),
            reason: None,
        }
    }

    #[test]
    fn the_comments_name_the_engine_and_task() {
        let nil = uuid::Uuid::nil();
        assert_eq!(
            due(LinearNoticeKind::Claim).body(),
            format!("CLAIM: pi/{nil} claimed in Maidan by agent0: Name the venue")
        );
        let mut release = due(LinearNoticeKind::Release);
        release.engine = None;
        release.title = None;
        assert_eq!(
            release.body(),
            format!("RELEASED: maidan/{nil} ended without merging")
        );
    }

    #[test]
    fn each_notice_has_its_own_negative_outbox_key() {
        let claim = due(LinearNoticeKind::Claim).outbox_key();
        let release = due(LinearNoticeKind::Release).outbox_key();
        assert!(claim < 0 && release < 0);
        assert_ne!(claim, release);
        assert_eq!(claim, due(LinearNoticeKind::Claim).outbox_key());
    }
}

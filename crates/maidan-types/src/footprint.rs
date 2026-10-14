//! Claim footprints: what part of a repository the work on a thread touches,
//! so a swarm of agents claiming from one queue never holds overlapping work.
//!
//! The footprint belongs to the thread. It counts only while the thread has a
//! live claim (an assignee, an unexpired lease, the thread open), so it frees
//! itself when the claim ends by a result, a release or an expiry. Two live
//! footprints conflict when they are in the same workspace and repository and
//! share a key that one of them holds exclusively; a branch is always held
//! exclusively. The claim queue skips a thread whose footprint would conflict,
//! and setting a footprint on a claimed thread is refused when it would.
//!
//! Keys: `lane`, `crate` and `branch` match exactly. `path` and `glob` share
//! one namespace and match by prefix, a glob counting up to its first
//! wildcard, so `src/bridge/**` overlaps `src/bridge/x.rs` and `src/` but not
//! `src/gateway`. A path never conflicts with a crate: nothing here knows
//! which crate a path is in, so a producer that wants that safety names the
//! lane as well.
//!
//! A `slot` item names a shared resource rather than part of the repository,
//! such as a build slot (`rust-build`). Slots are counted across the whole
//! workspace, whatever the repository or mode: when an operator sets a
//! capacity for a slot ([`SlotUsage`]), the claim queue defers a thread whose
//! slot already has that many live claims, so N agents build at once and the
//! rest wait their turn. A slot without a capacity limits nothing.
//!
//! An operator can pause a repository in a workspace ([`RepoPause`]): the
//! claim queue then defers every thread whose footprint names it, as it
//! defers an overlapping one. A pause gates new claims only; work already
//! claimed runs on, and a thread without a footprint is never paused.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{MemberId, ThreadId, WorkspaceId};

/// The most items one footprint may list.
pub const FOOTPRINT_MAX_ITEMS: usize = 200;
const VALUE_MAX_CHARS: usize = 512;
const REPO_MAX_CHARS: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum FootprintMode {
    Exclusive,
    Shared,
}

impl FootprintMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exclusive => "exclusive",
            Self::Shared => "shared",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "exclusive" => Some(Self::Exclusive),
            "shared" => Some(Self::Shared),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum FootprintKind {
    Path,
    Glob,
    Crate,
    Lane,
    Branch,
    Slot,
}

impl FootprintKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Path => "path",
            Self::Glob => "glob",
            Self::Crate => "crate",
            Self::Lane => "lane",
            Self::Branch => "branch",
            Self::Slot => "slot",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "path" => Some(Self::Path),
            "glob" => Some(Self::Glob),
            "crate" => Some(Self::Crate),
            "lane" => Some(Self::Lane),
            "branch" => Some(Self::Branch),
            "slot" => Some(Self::Slot),
            _ => None,
        }
    }

    /// The namespace keys of this kind are compared in: paths and globs share
    /// one.
    pub fn namespace(self) -> &'static str {
        match self {
            Self::Path | Self::Glob => "path",
            other => other.as_str(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FootprintItem {
    pub kind: FootprintKind,
    pub value: String,
}

impl FootprintItem {
    /// The key this item is compared by: the value itself, except that a glob
    /// counts up to its first wildcard, and a path loses a leading `./` or `/`.
    pub fn key(&self) -> String {
        match self.kind {
            FootprintKind::Path => strip_root(&self.value).to_string(),
            FootprintKind::Glob => {
                let value = strip_root(&self.value);
                let end = value.find(['*', '?', '[', '{']).unwrap_or(value.len());
                value[..end].to_string()
            }
            _ => self.value.clone(),
        }
    }
}

fn strip_root(value: &str) -> &str {
    let mut value = value;
    loop {
        if let Some(rest) = value.strip_prefix("./") {
            value = rest;
        } else if let Some(rest) = value.strip_prefix('/') {
            value = rest;
        } else {
            return value;
        }
    }
}

/// A thread's footprint as declared.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Footprint {
    /// `owner/name`, lowercase; a GitHub clone URL is accepted and reduced to it.
    pub repo: String,
    pub mode: FootprintMode,
    pub items: Vec<FootprintItem>,
}

impl Footprint {
    /// Normalize the repository and check every item, or say what is wrong.
    pub fn validated(mut self) -> Result<Self, String> {
        self.repo = normalize_repo(&self.repo)?;
        if self.items.len() > FOOTPRINT_MAX_ITEMS {
            return Err(format!(
                "a footprint lists at most {FOOTPRINT_MAX_ITEMS} items"
            ));
        }
        for item in &mut self.items {
            item.value = item.value.trim().to_string();
            // Slot names compare across the workspace, so one spelling.
            if item.kind == FootprintKind::Slot {
                item.value = item.value.to_ascii_lowercase();
            }
            if item.value.chars().count() > VALUE_MAX_CHARS {
                return Err(format!(
                    "a footprint value is at most {VALUE_MAX_CHARS} characters"
                ));
            }
            if item.value.chars().any(char::is_control) {
                return Err("a footprint value has no control characters".into());
            }
            // A glob may match the whole repository (`**`); every other kind
            // names something.
            if item.value.is_empty() || (item.kind != FootprintKind::Glob && item.key().is_empty())
            {
                return Err(format!("a `{}` item needs a value", item.kind.as_str()));
            }
            if matches!(item.kind, FootprintKind::Path | FootprintKind::Glob)
                && item.value.split('/').any(|part| part == "..")
            {
                return Err("a footprint path stays inside the repository (no `..`)".into());
            }
        }
        // The same item twice says nothing more, and would collide on the
        // stored key.
        let mut seen = std::collections::HashSet::new();
        self.items.retain(|item| seen.insert(item.clone()));
        Ok(self)
    }
}

/// `owner/name`, lowercase. A GitHub clone URL (`https://github.com/o/n.git`,
/// `git@github.com:o/n.git`) is reduced to it, so producers that write the
/// repository differently still compare equal.
pub fn normalize_repo(raw: &str) -> Result<String, String> {
    let mut repo = raw.trim().to_ascii_lowercase();
    for prefix in [
        "https://github.com/",
        "http://github.com/",
        "ssh://git@github.com/",
        "git@github.com:",
        "github.com/",
    ] {
        if let Some(rest) = repo.strip_prefix(prefix) {
            repo = rest.to_string();
            break;
        }
    }
    let repo = repo.trim_end_matches('/');
    let repo = repo.strip_suffix(".git").unwrap_or(repo).to_string();
    let valid = !repo.is_empty()
        && repo.chars().count() <= REPO_MAX_CHARS
        && repo.split_once('/').is_some_and(|(owner, name)| {
            !owner.is_empty() && !name.is_empty() && !name.contains('/')
        })
        && !repo
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '?' | '#' | ':'));
    if valid {
        Ok(repo)
    } else {
        Err("repo must be `owner/name` (or a GitHub clone URL)".into())
    }
}

/// A live claim and the footprint it holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ActiveClaim {
    pub thread_id: ThreadId,
    pub assignee_id: MemberId,
    /// `None` for a claim taken without a lease, which holds until released.
    pub lease_expires_at: Option<DateTime<Utc>>,
    pub repo: String,
    pub mode: FootprintMode,
    pub items: Vec<FootprintItem>,
}

/// One item of a live claim that a footprint would overlap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FootprintConflict {
    pub thread_id: ThreadId,
    pub assignee_id: MemberId,
    pub lease_expires_at: Option<DateTime<Utc>>,
    pub repo: String,
    pub kind: FootprintKind,
    pub value: String,
    pub mode: FootprintMode,
}

/// A slot's name as it is stored and compared: trimmed and lowercase, at most
/// 512 characters, no control characters.
pub fn normalize_slot(raw: &str) -> Result<String, String> {
    let slot = raw.trim().to_ascii_lowercase();
    if slot.is_empty() {
        return Err("a slot needs a name".into());
    }
    if slot.chars().count() > VALUE_MAX_CHARS || slot.chars().any(char::is_control) {
        return Err(format!(
            "a slot name is at most {VALUE_MAX_CHARS} characters, with no control characters"
        ));
    }
    Ok(slot)
}

/// A slot in a workspace: its capacity, when an operator set one, and how
/// many live claims hold it now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SlotUsage {
    pub slot: String,
    /// `None` for a slot in use that has no capacity, which limits nothing.
    pub capacity: Option<i64>,
    pub in_use: i64,
}

/// The body that sets a slot's capacity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SetSlotCapacity {
    pub slot: String,
    /// At least 1: how many live claims may hold the slot at once.
    pub capacity: i64,
}

/// A repository paused in one workspace: no thread whose footprint names it
/// is claimed until the pause is lifted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RepoPause {
    pub workspace_id: WorkspaceId,
    /// Lowercase `owner/name`, as footprints store it.
    pub repo: String,
    pub paused_at: DateTime<Utc>,
    pub paused_by: MemberId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// The body of a pause: the repository, in any spelling [`normalize_repo`]
/// accepts, and an optional note for the audit trail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PauseRepo {
    pub repo: String,
    #[serde(default)]
    pub reason: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: FootprintKind, value: &str) -> FootprintItem {
        FootprintItem {
            kind,
            value: value.into(),
        }
    }

    #[test]
    fn a_repo_is_compared_as_lowercase_owner_and_name() {
        for raw in [
            "Example/App",
            "https://github.com/example/app.git",
            "git@github.com:Example/App.git",
            "github.com/example/app/",
            " example/app ",
        ] {
            assert_eq!(normalize_repo(raw).as_deref(), Ok("example/app"), "{raw}");
        }
        for bad in [
            "app",
            "example/app/extra",
            "",
            "a b/c",
            "https://gitlab.com/x/y",
        ] {
            assert!(normalize_repo(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_glob_is_keyed_by_what_comes_before_its_first_wildcard() {
        assert_eq!(
            item(FootprintKind::Glob, "src/bridge/**").key(),
            "src/bridge/"
        );
        assert_eq!(item(FootprintKind::Glob, "**/*.rs").key(), "");
        assert_eq!(item(FootprintKind::Path, "./src/a.rs").key(), "src/a.rs");
        assert_eq!(item(FootprintKind::Lane, "bridge").key(), "bridge");
        assert_eq!(
            FootprintKind::Glob.namespace(),
            FootprintKind::Path.namespace()
        );
    }

    #[test]
    fn a_footprint_is_checked_and_deduplicated() {
        let ok = Footprint {
            repo: "https://github.com/Example/App".into(),
            mode: FootprintMode::Exclusive,
            items: vec![
                item(FootprintKind::Lane, "bridge"),
                item(FootprintKind::Lane, " bridge "),
                item(FootprintKind::Glob, "**"),
            ],
        }
        .validated()
        .unwrap();
        assert_eq!(ok.repo, "example/app");
        assert_eq!(ok.items.len(), 2);
        for bad in [
            item(FootprintKind::Path, "../outside"),
            item(FootprintKind::Lane, " "),
            item(FootprintKind::Branch, "a\nb"),
        ] {
            let footprint = Footprint {
                repo: "example/app".into(),
                mode: FootprintMode::Shared,
                items: vec![bad.clone()],
            };
            assert!(footprint.validated().is_err(), "{bad:?}");
        }
    }
}

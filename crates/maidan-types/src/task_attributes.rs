//! Task attributes: what kind of work a thread is, who or what should do
//! it, and whether it needs a threat model.
//!
//! Routing and the swarm ramp metrics read these: a claimer can ask the
//! queue only for work meant for its engine, and the metrics can split a
//! workspace's pull requests by the kind and engine of the task behind
//! them. The attributes belong to the thread, are set at intake or replaced
//! later, and are stored beside it; a thread nobody ever described reads as
//! the defaults (a change, no engine, no threat model).

use serde::{Deserialize, Serialize};

/// What kind of work a task is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum TaskKind {
    /// Work that lands a change (the default).
    #[default]
    Change,
    /// Work that plans other work; its fan-out is the planner's children.
    Plan,
    /// Work that reviews a change.
    Review,
}

impl TaskKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Change => "change",
            Self::Plan => "plan",
            Self::Review => "review",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "change" => Some(Self::Change),
            "plan" => Some(Self::Plan),
            "review" => Some(Self::Review),
            _ => None,
        }
    }
}

/// Who or what a task is meant for. `None` on a thread means nobody has
/// routed it yet; an engine-filtered claim never takes a thread routed to
/// another engine, nor one not routed at all. A workspace's default engine
/// routes the threads created there without one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum TaskEngine {
    /// An automated Pi worker.
    Pi,
    /// An interactive agent session.
    Session,
    /// A person.
    Human,
}

impl TaskEngine {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pi => "pi",
            Self::Session => "session",
            Self::Human => "human",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pi" => Some(Self::Pi),
            "session" => Some(Self::Session),
            "human" => Some(Self::Human),
            _ => None,
        }
    }
}

/// How big a task is, as its planner judged it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum TaskSize {
    S,
    M,
    L,
}

impl TaskSize {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::S => "S",
            Self::M => "M",
            Self::L => "L",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "S" => Some(Self::S),
            "M" => Some(Self::M),
            "L" => Some(Self::L),
            _ => None,
        }
    }
}

/// How much review a change needs before it merges: Tier A takes the fast
/// path, Tier B a full review. A thread with no tier stored counts as B in
/// the metrics, while the stored value stays unset, so a later restatement
/// can tell "never said" from "B".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum TaskTier {
    A,
    B,
}

impl TaskTier {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "A" => Some(Self::A),
            "B" => Some(Self::B),
            _ => None,
        }
    }
}

/// The service lane a thread's model calls may run in (Context Economics
/// C7). A thread whose deadline has slack can be marked for a provider's
/// batch API (about half price, results within hours) or its flex tier
/// (discounted, slower, may be refused under load). Unset is the realtime
/// lane, the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum TaskLane {
    Batch,
    Flex,
}

impl TaskLane {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Batch => "batch",
            Self::Flex => "flex",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "batch" => Some(Self::Batch),
            "flex" => Some(Self::Flex),
            _ => None,
        }
    }
}

/// A workspace's default engine: where a thread created there with no
/// engine named is routed. `None` leaves such threads unrouted, and an
/// engine-filtered claim never takes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WorkspaceDefaultEngine {
    pub engine: Option<TaskEngine>,
}

/// Whether a change entering review spawns its second-family review child.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum ReviewChildren {
    /// Spawn a `kind: review` child routed away from the author's family.
    #[default]
    Spawn,
    /// Spawn none; review stays with the pull request's own reviewers.
    Off,
}

impl ReviewChildren {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Spawn => "spawn",
            Self::Off => "off",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "spawn" => Some(Self::Spawn),
            "off" => Some(Self::Off),
            _ => None,
        }
    }
}

/// A workspace's review-children setting, read and set by
/// `/workspaces/{id}/review-children`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WorkspaceReviewChildren {
    pub review_children: ReviewChildren,
}

/// A task attribute a workspace can name in a unique key set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum UniqueAttributeKey {
    /// The Linear issue a task is for.
    LinearIssue,
    /// The repository a task changes.
    Repo,
}

impl UniqueAttributeKey {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LinearIssue => "linear_issue",
            Self::Repo => "repo",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "linear_issue" => Some(Self::LinearIssue),
            "repo" => Some(Self::Repo),
            _ => None,
        }
    }

    /// The `maidan_thread_attributes` column holding this key. Spelled here,
    /// never taken from a request, so it is safe to splice into SQL.
    pub fn column(self) -> &'static str {
        match self {
            Self::LinearIssue => "linear_issue",
            Self::Repo => "repo",
        }
    }

    /// The value `attributes` gives this key, or `None` when it gives none.
    pub fn value_of(self, attributes: &TaskAttributes) -> Option<&str> {
        match self {
            Self::LinearIssue => attributes.linear_issue.as_deref(),
            Self::Repo => attributes.repo.as_deref(),
        }
    }

    /// The values `attributes` gives every key of `set`, in the set's order,
    /// or `None` when it lacks one: a thread missing a key of a set is not
    /// constrained by that set.
    pub fn values_of<'a>(set: &[Self], attributes: &'a TaskAttributes) -> Option<Vec<&'a str>> {
        set.iter().map(|key| key.value_of(attributes)).collect()
    }
}

/// A held key set and its values, as a refusal names them:
/// `linear_issue ENG-6598, repo example/wax`.
pub fn describe_held(held: &[(UniqueAttributeKey, String)]) -> String {
    held.iter()
        .map(|(key, value)| format!("{} {value}", key.as_str()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A workspace's unique thread attributes, read and set by
/// `/workspaces/{id}/unique-thread-attributes`. Each entry of `unique` is a
/// key set: no two open threads in the workspace that both give every key of
/// a set may give the same values for all of them. Empty by default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct WorkspaceUniqueThreadAttributes {
    /// The key sets held unique, such as `[["linear_issue", "repo"]]`: one
    /// open thread per ticket in each repository.
    pub unique: Vec<Vec<UniqueAttributeKey>>,
}

impl WorkspaceUniqueThreadAttributes {
    /// Refuse an empty key set, which would make every thread a holder of
    /// every other's empty tuple.
    pub fn validated(self) -> Result<Self, String> {
        if self.unique.iter().any(Vec::is_empty) {
            return Err("a unique key set names at least one key".into());
        }
        Ok(self.normalized())
    }

    /// Each set sorted and without repeats, empty sets dropped, and the sets
    /// sorted and without repeats, as they are stored. The fixed order also
    /// makes two writers checking several sets take their locks in the same
    /// order.
    pub fn normalized(mut self) -> Self {
        for set in &mut self.unique {
            set.sort();
            set.dedup();
        }
        self.unique.retain(|set| !set.is_empty());
        self.unique.sort();
        self.unique.dedup();
        self
    }

    /// Read the stored form: a JSON array of arrays of key names. A set
    /// naming a key this build does not know is skipped, rather than failing
    /// every thread write or holding a narrower set than was asked for.
    pub fn from_stored(stored: &str) -> Self {
        let sets: Vec<Vec<String>> = serde_json::from_str(stored).unwrap_or_default();
        Self {
            unique: sets
                .iter()
                .filter_map(|set| {
                    set.iter()
                        .map(String::as_str)
                        .map(UniqueAttributeKey::parse)
                        .collect::<Option<Vec<_>>>()
                })
                .collect(),
        }
        .normalized()
    }

    /// The stored form: a JSON array of arrays of key names.
    pub fn to_stored(&self) -> String {
        let sets: Vec<Vec<&str>> = self
            .unique
            .iter()
            .map(|set| set.iter().map(|key| key.as_str()).collect())
            .collect();
        serde_json::json!(sets).to_string()
    }
}

/// The model provider families a review can be routed between, in the
/// order a review family is picked, each with the words that name it in a
/// usage record's provider or model. The SQL twin is built from this table.
pub const PROVIDER_FAMILIES: &[(&str, &[&str], &[&str])] = &[
    // Order matters: the first family that matches wins, so Claude hosted on
    // Vertex or Bedrock resolves to anthropic before google.
    ("anthropic", &["anthropic", "claude"], &[]),
    // OpenAI's reasoning models are named by a bare prefix (`o3`, `o4-mini`).
    ("openai", &["openai", "gpt", "codex"], &["o1", "o3", "o4"]),
    ("google", &["google", "gemini", "vertex"], &[]),
    ("xai", &["xai", "grok"], &[]),
    // A DeepSeek review seat reports `provider: "deepseek"` with models named
    // `deepseek-chat` and `deepseek-reasoner`.
    ("deepseek", &["deepseek"], &[]),
];

/// The provider family a usage record names, from its provider and model
/// (lowercased, first family in [`PROVIDER_FAMILIES`] whose word appears in
/// either, or whose model prefix is the model or starts it before a `-`),
/// or `None` when neither names one.
pub fn provider_family(provider: Option<&str>, model: &str) -> Option<&'static str> {
    let named = format!("{} {}", provider.unwrap_or(""), model).to_lowercase();
    let model = model.to_lowercase();
    PROVIDER_FAMILIES
        .iter()
        .find(|(_, words, prefixes)| {
            words.iter().any(|word| named.contains(word))
                || prefixes
                    .iter()
                    .any(|prefix| model == *prefix || model.starts_with(&format!("{prefix}-")))
        })
        .map(|(family, _, _)| *family)
}

/// The family a change by `author` is reviewed by: the first family that is
/// not the author's. `None` when the author's family is unknown, since a
/// review cannot be routed away from a family nobody named.
pub fn review_family_for(author: Option<&str>) -> Option<&'static str> {
    let author = author?;
    PROVIDER_FAMILIES
        .iter()
        .map(|(family, _, _)| *family)
        .find(|family| *family != author)
}

/// The most characters an acceptance criterion may hold.
pub const ACCEPTANCE_MAX_CHARS: usize = 4000;

/// A thread's task attributes, whole: setting them replaces every field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
// A set replaces every field, so an unknown key is refused, not dropped: a
// body wrapped as `{"attributes": {…}}` would otherwise parse as all defaults
// and wipe the thread's engine, acceptance and Linear issue.
#[serde(deny_unknown_fields)]
pub struct TaskAttributes {
    #[serde(default)]
    pub kind: TaskKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<TaskEngine>,
    #[serde(default)]
    pub threat_model: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<TaskSize>,
    /// How a reviewer verifies the task: what the change must satisfy. Shown
    /// to the worker in the thread context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acceptance: Option<String>,
    /// The Linear issue the task is for (`ENG-1234`). Where the workspace
    /// has turned claim comments on, the task's claim and release are said
    /// on it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linear_issue: Option<String>,
    /// The change's review tier, set at intake and restated when a new head
    /// changes it. Read by the metrics only; nothing routes or gates on it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<TaskTier>,
    /// The service lane the task may run in: batch or flex for work whose
    /// deadline has slack, unset for realtime.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lane: Option<TaskLane>,
    /// The provider family a review child is routed to: one other than the
    /// family of the change's author, picked when the change entered review.
    /// Set by the server only; setting attributes leaves it as it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_family: Option<String>,
    /// The provider family that wrote the change this task reviews
    /// (`anthropic`, `openai`, `google`, `xai`): `claim_next` never hands
    /// the thread to a claimer whose usage names that family. Set by the
    /// producer filing a review, or by the server for a review child. Once
    /// stored it is kept: a later set neither clears nor changes it, so
    /// nobody who may set attributes, the change's author included, can
    /// lift it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_family: Option<String>,
    /// The repository the task changes, as `owner/name` (a GitHub URL is
    /// read as its `owner/name`). With `linear_issue` it names the half of a
    /// ticket a thread owns, where one ticket has a pull request in each of
    /// several repositories.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
}

impl TaskAttributes {
    /// Trim the acceptance text (blank reads as none) and check its length.
    pub fn validated(mut self) -> Result<Self, String> {
        self.linear_issue = self
            .linear_issue
            .map(|key| key.trim().to_string())
            .filter(|key| !key.is_empty());
        if let Some(key) = &self.linear_issue
            && !crate::is_issue_key(key)
        {
            return Err(format!(
                "linear_issue `{key}` is not an issue key such as ENG-123"
            ));
        }
        self.repo = match self.repo.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(repo) => Some(crate::normalize_repo(repo).map_err(|e| format!("repo: {e}"))?),
        };
        self.author_family = self
            .author_family
            .map(|family| family.trim().to_ascii_lowercase())
            .filter(|family| !family.is_empty());
        if let Some(family) = &self.author_family
            && !PROVIDER_FAMILIES
                .iter()
                .any(|(known, _, _)| known == family)
        {
            let known: Vec<&str> = PROVIDER_FAMILIES.iter().map(|(name, _, _)| *name).collect();
            return Err(format!(
                "author_family `{family}` is not a provider family; use one of {}",
                known.join(", ")
            ));
        }
        self.acceptance = self
            .acceptance
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty());
        if let Some(text) = &self.acceptance {
            if text.chars().count() > ACCEPTANCE_MAX_CHARS {
                return Err(format!(
                    "acceptance is at most {ACCEPTANCE_MAX_CHARS} characters"
                ));
            }
            if text
                .chars()
                .any(|c| c.is_control() && c != '\n' && c != '\t')
            {
                return Err("acceptance has no control characters".into());
            }
        }
        Ok(self)
    }
}

#[cfg(test)]
mod unique_attribute_tests {
    use super::*;

    use super::UniqueAttributeKey::{LinearIssue, Repo};

    #[test]
    fn the_unique_key_sets_round_trip_through_their_stored_form() {
        let set = WorkspaceUniqueThreadAttributes {
            unique: vec![
                vec![Repo, LinearIssue, Repo],
                vec![LinearIssue],
                vec![LinearIssue],
            ],
        }
        .validated()
        .unwrap();
        assert_eq!(set.unique, vec![vec![LinearIssue], vec![LinearIssue, Repo]]);
        assert_eq!(
            set.to_stored(),
            r#"[["linear_issue"],["linear_issue","repo"]]"#
        );
        assert_eq!(
            WorkspaceUniqueThreadAttributes::from_stored(&set.to_stored()),
            set
        );
        assert_eq!(
            WorkspaceUniqueThreadAttributes::from_stored("[]"),
            WorkspaceUniqueThreadAttributes::default()
        );
        // A set naming a key a later build added is skipped whole.
        assert_eq!(
            WorkspaceUniqueThreadAttributes::from_stored(
                r#"[["linear_issue","later_key"],["linear_issue","repo"]]"#
            )
            .unique,
            vec![vec![LinearIssue, Repo]]
        );
        assert!(WorkspaceUniqueThreadAttributes {
            unique: vec![vec![]]
        }
        .validated()
        .is_err());
    }

    #[test]
    fn a_unique_key_set_names_known_attributes_and_nothing_else() {
        let body: WorkspaceUniqueThreadAttributes =
            serde_json::from_str(r#"{"unique":[["linear_issue","repo"]]}"#).unwrap();
        assert_eq!(body.unique, vec![vec![LinearIssue, Repo]]);
        for bad in [
            r#"{"unique":[["kind"]]}"#,
            r#"{"unique":["linear_issue"]}"#,
            r#"{"keys":[["linear_issue"]]}"#,
        ] {
            assert!(
                serde_json::from_str::<WorkspaceUniqueThreadAttributes>(bad).is_err(),
                "{bad}"
            );
        }
        for key in [LinearIssue, Repo] {
            assert_eq!(UniqueAttributeKey::parse(key.as_str()), Some(key));
        }
    }

    #[test]
    fn a_thread_is_held_to_a_set_only_when_it_gives_every_key() {
        let both = TaskAttributes {
            linear_issue: Some("ENG-6598".into()),
            repo: Some("https://github.com/Example/Wax.git".into()),
            ..TaskAttributes::default()
        }
        .validated()
        .unwrap();
        assert_eq!(both.repo.as_deref(), Some("example/wax"));
        assert_eq!(
            UniqueAttributeKey::values_of(&[LinearIssue, Repo], &both),
            Some(vec!["ENG-6598", "example/wax"])
        );
        let ticket_only = TaskAttributes {
            linear_issue: Some("ENG-6598".into()),
            ..TaskAttributes::default()
        };
        assert_eq!(
            UniqueAttributeKey::values_of(&[LinearIssue, Repo], &ticket_only),
            None
        );
        assert_eq!(
            UniqueAttributeKey::values_of(&[LinearIssue], &ticket_only),
            Some(vec!["ENG-6598"])
        );
        assert_eq!(
            describe_held(&[
                (LinearIssue, "ENG-6598".into()),
                (Repo, "example/wax".into())
            ]),
            "linear_issue ENG-6598, repo example/wax"
        );
        let blank = TaskAttributes {
            repo: Some("  ".into()),
            ..TaskAttributes::default()
        }
        .validated()
        .unwrap();
        assert_eq!(blank.repo, None);
        assert!(TaskAttributes {
            repo: Some("not a repo".into()),
            ..TaskAttributes::default()
        }
        .validated()
        .is_err());
    }
}

#[cfg(test)]
mod lane_tests {
    use super::*;

    #[test]
    fn a_lane_round_trips_by_its_lowercase_name() {
        for lane in [TaskLane::Batch, TaskLane::Flex] {
            assert_eq!(TaskLane::parse(lane.as_str()), Some(lane));
            assert_eq!(
                serde_json::to_string(&lane).unwrap(),
                format!("\"{}\"", lane.as_str())
            );
        }
        assert_eq!(TaskLane::parse("realtime"), None);
        let attrs: TaskAttributes = serde_json::from_str(r#"{"lane":"batch"}"#).unwrap();
        assert_eq!(attrs.lane, Some(TaskLane::Batch));
        assert!(!serde_json::to_string(&TaskAttributes::default())
            .unwrap()
            .contains("lane"));
    }
}

#[cfg(test)]
mod review_family_tests {
    use super::*;

    #[test]
    fn a_usage_record_names_its_family_by_provider_or_model() {
        assert_eq!(provider_family(Some("Anthropic"), "x"), Some("anthropic"));
        assert_eq!(provider_family(None, "claude-sonnet-4"), Some("anthropic"));
        assert_eq!(provider_family(Some("openai"), "o3"), Some("openai"));
        assert_eq!(provider_family(None, "gpt-5-codex"), Some("openai"));
        assert_eq!(provider_family(None, "gemini-2.5-pro"), Some("google"));
        assert_eq!(provider_family(Some("xai"), "m"), Some("xai"));
        assert_eq!(provider_family(None, "grok-4"), Some("xai"));
        assert_eq!(
            provider_family(Some("deepseek"), "deepseek-chat"),
            Some("deepseek")
        );
        // Claude Code driving DeepSeek reports DeepSeek: the harness is not the family.
        assert_eq!(
            provider_family(Some("deepseek"), "deepseek-reasoner"),
            Some("deepseek")
        );
        assert_eq!(provider_family(None, "llama-3"), None);
    }

    #[test]
    fn the_review_family_is_never_the_authors() {
        for (family, _, _) in PROVIDER_FAMILIES {
            let picked = review_family_for(Some(family)).unwrap();
            assert_ne!(picked, *family);
        }
        assert_eq!(review_family_for(Some("anthropic")), Some("openai"));
        assert_eq!(review_family_for(Some("openai")), Some("anthropic"));
        assert_eq!(review_family_for(None), None);
    }

    #[test]
    fn an_author_family_is_a_known_family_with_its_case_folded() {
        let named = TaskAttributes {
            author_family: Some(" OpenAI ".into()),
            ..TaskAttributes::default()
        }
        .validated()
        .unwrap();
        assert_eq!(named.author_family.as_deref(), Some("openai"));
        let blank = TaskAttributes {
            author_family: Some("  ".into()),
            ..TaskAttributes::default()
        }
        .validated()
        .unwrap();
        assert_eq!(blank.author_family, None);
        let err = TaskAttributes {
            author_family: Some("gpt".into()),
            ..TaskAttributes::default()
        }
        .validated()
        .unwrap_err();
        assert!(err.contains("author_family `gpt`"), "{err}");
        assert!(err.contains("anthropic, openai, google, xai"), "{err}");
        let parsed: TaskAttributes =
            serde_json::from_str(r#"{"kind":"review","author_family":"xai"}"#).unwrap();
        assert_eq!(parsed.author_family.as_deref(), Some("xai"));
        assert!(!serde_json::to_string(&TaskAttributes::default())
            .unwrap()
            .contains("author_family"));
    }

    #[test]
    fn openai_reasoning_models_are_named_by_their_prefix() {
        assert_eq!(provider_family(None, "o3"), Some("openai"));
        assert_eq!(provider_family(None, "o4-mini"), Some("openai"));
        assert_eq!(provider_family(None, "O1-preview"), Some("openai"));
        assert_eq!(provider_family(None, "o5"), None);
        assert_eq!(provider_family(None, "o30x"), None);
    }
}

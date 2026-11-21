//! Advice at the land gate from a decision model: TypeSafe's Jev, OpenAI's
//! Decisions API or Google's Gemini Decisions API.
//!
//! The model makes judgments and code makes the decision. A hand-off to review
//! is read as a short packet (what the task asks for, the result, what the
//! agent said), the model answers a fixed set of yes/no questions about it,
//! each with a probability, and [`decide`] turns the answers into green, amber
//! or red by rules written here. No question asks the model whether the work
//! should land.
//!
//! The advisor stays outside the authoritative land-gate write path: a caller
//! asks for advice, reads it, and a qualified verifier still records the gate
//! pointer. A provider failure fails the advice call and nothing else.

use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use maidan_store::Store;
use maidan_types::{LandColor, ThreadId, WorkspaceId};
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;
use utoipa::ToSchema;

const DEFAULT_TIMEOUT_MS: u64 = 1_500;

/// A provider of decision models the advisor can put its questions to. All
/// three are asked the same ten questions and read by the same rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AdvisorProvider {
    /// TypeSafe's Jev (`POST /v1/systemone`).
    Typesafe,
    /// OpenAI's Decisions API (`POST /v1/decisions`).
    Openai,
    /// Google's Gemini Decisions API (`POST /v1beta/decisions`).
    Google,
}

impl AdvisorProvider {
    /// Every provider, in the order a call that names none tries them.
    pub const ALL: [Self; 3] = [Self::Typesafe, Self::Openai, Self::Google];

    /// The name the API and stored advice use.
    pub fn id(self) -> &'static str {
        match self {
            Self::Typesafe => "typesafe",
            Self::Openai => "openai",
            Self::Google => "google",
        }
    }

    /// The provider from its [`Self::id`].
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|provider| provider.id() == id)
    }

    /// The name a person reads.
    pub fn name(self) -> &'static str {
        match self {
            Self::Typesafe => "TypeSafe Jev",
            Self::Openai => "OpenAI Decisions",
            Self::Google => "Gemini Decisions",
        }
    }

    /// Who the key belongs to, for a sentence about it.
    fn vendor(self) -> &'static str {
        match self {
            Self::Typesafe => "TypeSafe",
            Self::Openai => "OpenAI",
            Self::Google => "Google",
        }
    }

    /// The workspace secret that holds a workspace's own key for this
    /// provider. Each name is one the server keeps for itself
    /// (`maidan_types::RESERVED_SECRET_PREFIX`): no generic secret route or
    /// tool creates, lists, resolves, substitutes or deletes it, so a key is
    /// set only after its provider accepted it and is sent only to that
    /// provider.
    pub fn secret(self) -> &'static str {
        match self {
            Self::Typesafe => "maidan:typesafe_api_key",
            Self::Openai => "maidan:openai_api_key",
            Self::Google => "maidan:gemini_api_key",
        }
    }

    /// The variable an instance's own key for this provider is read from.
    fn key_env(self) -> &'static str {
        match self {
            Self::Typesafe => "TYPESAFE_API_KEY",
            Self::Openai => "OPENAI_API_KEY",
            Self::Google => "GEMINI_API_KEY",
        }
    }

    fn model_env(self) -> &'static str {
        match self {
            Self::Typesafe => "MAIDAN_JEV_MODEL",
            Self::Openai => "MAIDAN_OPENAI_DECISIONS_MODEL",
            Self::Google => "MAIDAN_GEMINI_DECISIONS_MODEL",
        }
    }

    fn base_url_env(self) -> &'static str {
        match self {
            Self::Typesafe => "MAIDAN_JEV_BASE_URL",
            Self::Openai => "MAIDAN_OPENAI_DECISIONS_BASE_URL",
            Self::Google => "MAIDAN_GEMINI_DECISIONS_BASE_URL",
        }
    }

    fn default_base_url(self) -> &'static str {
        match self {
            Self::Typesafe => "https://api.typesafe.ai/",
            Self::Openai => "https://api.openai.com/",
            Self::Google => "https://generativelanguage.googleapis.com/",
        }
    }

    fn default_model(self) -> &'static str {
        match self {
            Self::Typesafe => "jev-latest",
            Self::Openai => "gpt-6-luna",
            Self::Google => "gemini-decision-1",
        }
    }

    /// Where the questions are sent, under the provider's origin.
    fn ask_path(self) -> &'static str {
        match self {
            Self::Typesafe => "v1/systemone",
            Self::Openai => "v1/decisions",
            Self::Google => "v1beta/decisions",
        }
    }

    /// A cheap authenticated read, to learn whether the provider takes a key.
    fn models_path(self) -> &'static str {
        match self {
            Self::Typesafe | Self::Openai => "v1/models",
            Self::Google => "v1beta/models",
        }
    }

    /// Put the key on a request the way this provider reads it.
    fn authorize(self, request: reqwest::RequestBuilder, key: &str) -> reqwest::RequestBuilder {
        match self {
            Self::Typesafe | Self::Openai => request.bearer_auth(key),
            Self::Google => request.header("x-goog-api-key", key),
        }
    }
}

/// At or above this a judgment reads as yes.
pub const YES_MIN: f64 = 0.7;
/// At or below this a judgment reads as no. Between the two it is unsettled,
/// and an unsettled judgment never counts toward green.
pub const NO_MAX: f64 = 0.3;

/// The largest request the advisor sends, body and all. A relay in front of
/// the provider (the demo's Worker, `deploy/cloudflare/src/demo-jev.js`) caps
/// bodies at the same size, and a bounded request is a bounded bill.
pub const MAX_REQUEST_BYTES: usize = 16 * 1024;

/// How much of the hand-off leaves for the provider.
const MAX_AGENT_MESSAGES: usize = 10;
const MAX_MESSAGE_CHARS: usize = 1_500;
const MAX_TITLE_CHARS: usize = 300;
const MAX_DESCRIPTION_CHARS: usize = 3_000;
/// A long string in the result is cut at the first of these that lets the
/// packet fit [`MAX_REQUEST_BYTES`]; a hand-off that fits at none is too
/// large to ask about.
const RESULT_STRING_CUTS: [usize; 5] = [4_000, 2_000, 1_000, 500, 250];
/// The thread's messages are read a page at a time, to the end, so the
/// producer's newest are the ones sent however long the thread is.
const MESSAGE_PAGE: usize = 200;
const MESSAGE_PAGES: usize = 100;
/// Result fields that carry bytes for the change flow, not a description of
/// the work: a diff is long, and the model reads the summary beside it.
const RESULT_FIELDS_LEFT_OUT: [&str; 3] = ["diff", "deliver_to", "base_sha"];

const CACHE_TTL: Duration = Duration::from_secs(15 * 60);
/// How long a resolved provider address is used before the name is looked up
/// again, so a provider that moves does not need a restart.
const RESOLVE_TTL: Duration = Duration::from_secs(10 * 60);
const CACHE_CAPACITY: usize = 512;

/// One yes/no question put to the model. The wording lives here and nowhere
/// else: `instructions` and the two criteria are sent as written.
struct Question {
    id: &'static str,
    /// The judgment's name in a few words.
    label: &'static str,
    instructions: &'static str,
    yes: &'static str,
    no: &'static str,
    /// The answer as a statement a reader takes in at a glance, one for each
    /// way the answer can read. A bare yes or no beside a label makes a
    /// reader work out which way is good; a statement does not.
    reads: [&'static str; 3],
}

/// Every judgment the advisor asks for, in the order it reports them. Each is
/// one observable statement about the packet; the rules that combine them are
/// in [`decide`].
const QUESTIONS: [Question; 10] = [
    Question {
        id: "done",
        label: "Does what the task asks",
        instructions: "Do `result` and `agent_messages` show that the agent did what `task` asks for?",
        yes: "Together they describe doing the thing the task asks for.",
        no: "They describe different work, only part of the work, or do not say what was done.",
        reads: [
            "Does what the task asks",
            "Does not show the task was done",
            "Unclear whether it does what the task asks",
        ],
    },
    Question {
        id: "tests_passed",
        label: "Reports passing tests",
        instructions: "Do `result` or `agent_messages` say that tests or checks ran and passed?",
        yes: "They state that tests or checks were run and passed.",
        no: "They do not mention tests or checks, or say they failed or were not run. A bare status such as ok or done is not a report of tests.",
        reads: [
            "Reports tests or checks passing",
            "Reports no passing tests",
            "Unclear whether tests passed",
        ],
    },
    Question {
        id: "code_change",
        label: "Asks for a code change",
        instructions: "Does `task` ask for a change to code?",
        yes: "The task asks to add, fix, rename or change code or configuration that runs.",
        no: "The task asks for something else, such as writing documentation, answering a question, investigating, or posting a message.",
        reads: [
            "The task asks for a code change",
            "The task asks for no code change",
            "Unclear whether the task asks for a code change",
        ],
    },
    Question {
        id: "unresolved_failure",
        label: "Something is still failing",
        instructions: "Does `result` or `agent_messages` report a failing test, an error or a broken check that is still not fixed?",
        yes: "Something is reported as still failing or broken.",
        no: "Nothing is reported as failing, or a failure is reported as fixed.",
        reads: [
            "Reports something still failing",
            "Reports nothing still failing",
            "Unclear whether something is still failing",
        ],
    },
    Question {
        id: "left_undone",
        label: "Leaves part undone",
        instructions: "Does `result` or `agent_messages` say that part of what `task` asks for was not done?",
        yes: "The agent says some of the asked-for work was left out, skipped or not finished.",
        no: "The agent does not say any asked-for work was left out.",
        reads: [
            "Says part of the task was left undone",
            "Reports nothing left undone",
            "Unclear whether part was left undone",
        ],
    },
    Question {
        id: "asks_person",
        label: "Asks a person first",
        instructions: "Does `result` or `agent_messages` ask a person a question that needs an answer before the work is finished?",
        yes: "The agent asks someone to answer or to choose, and the work waits on it.",
        no: "The agent asks nothing, or only offers further help.",
        reads: [
            "Asks a person a question first",
            "Waits on no question",
            "Unclear whether it waits on a question",
        ],
    },
    Question {
        id: "beyond_task",
        label: "Goes beyond the task",
        instructions: "Does `result` report a separate change that `task` did not ask for?",
        yes: "The result reports another change besides the task, such as rewriting, removing or upgrading something the task does not mention.",
        no: "Everything reported is part of doing the task. How the task was done, tests added for it, and a recommendation are part of the task.",
        reads: [
            "Changes something the task did not ask for",
            "Stays within the task",
            "Unclear whether it stays within the task",
        ],
    },
    Question {
        id: "changes_access",
        label: "Changes sign-in or permissions",
        instructions: "Did the agent's own work change the code or settings that control sign-in or permissions?",
        yes: "The work changed how sign-in works, or which permission is needed, granted or checked.",
        no: "The work did not change sign-in or permission checks. Mentioning or documenting them does not count.",
        reads: [
            "Changes sign-in or permissions",
            "Leaves sign-in and permissions alone",
            "Unclear whether it changes sign-in or permissions",
        ],
    },
    Question {
        id: "deletes_data",
        label: "Deletes stored data",
        instructions: "Does `result` report that the agent deleted stored data, or added something that deletes it?",
        yes: "Stored records, tables or files were deleted, or a job that deletes them was added.",
        no: "No stored data is deleted. Removing code, a feature or a line of text is not deleting stored data.",
        reads: [
            "Deletes stored data",
            "Deletes no stored data",
            "Unclear whether it deletes stored data",
        ],
    },
    Question {
        id: "flags_risk",
        label: "Warns about a risk",
        instructions: "Does the agent warn about a risk or a consequence of this work that someone should deal with?",
        yes: "`result` or `agent_messages` warn that the work could break something, affect users, or needs someone's attention.",
        no: "They contain no warning.",
        reads: [
            "The agent warns about a risk",
            "The agent flags no risk",
            "Unclear whether the agent flags a risk",
        ],
    },
];

/// What the tests judgment reads as when no passing tests are reported and
/// the rules do not ask for any (the task asks for no code change). "Reports
/// no passing tests" beside a clear mark reads as bad news that was let
/// through; these say why it is not a concern.
const NO_TESTS_NOT_CALLED_FOR: &str = "Reports no passing tests, which this task does not call for";
const UNCLEAR_TESTS_NOT_CALLED_FOR: &str =
    "Unclear whether tests passed, which this task does not call for";

#[derive(Debug, Error)]
pub enum LandGateAdvisorError {
    #[error("invalid advisor configuration: {0}")]
    Config(String),
    #[error("invalid advice request: {0}")]
    InvalidRequest(String),
    /// The thread or the workspace is not ready for advice: no result yet, or
    /// no key to ask with.
    #[error("{0}")]
    NotReady(String),
    #[error("advisor request failed: {0}")]
    Unavailable(String),
    #[error("advisor returned an invalid response: {0}")]
    InvalidResponse(String),
    #[error("store: {0}")]
    Store(#[from] maidan_store::StoreError),
}

/// The body of `POST /threads/{id}/land-gate/advice`. Nothing in it changes
/// what is sent: the packet is built from the thread.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LandGateAdviceRequest {
    /// Ask the model again even when an answer for this exact hand-off is held.
    #[serde(default)]
    pub refresh: bool,
    /// Which provider to ask. Absent, the first one this workspace has a key
    /// for is asked, in the order of [`AdvisorProvider::ALL`].
    #[serde(default)]
    pub provider: Option<AdvisorProvider>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct LandGateAdviceUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// How a judgment's probability reads against the two thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum JudgmentAnswer {
    Yes,
    No,
    Unsettled,
}

impl JudgmentAnswer {
    /// Where this answer's statement sits in a question's `reads`.
    fn index(self) -> usize {
        match self {
            Self::Yes => 0,
            Self::No => 1,
            Self::Unsettled => 2,
        }
    }

    fn of(probability: f64) -> Self {
        if probability >= YES_MIN {
            Self::Yes
        } else if probability <= NO_MAX {
            Self::No
        } else {
            Self::Unsettled
        }
    }
}

/// One of the model's judgments about the hand-off.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LandGateJudgment {
    /// A stable name for the judgment, such as `tests_passed`.
    pub id: String,
    /// The judgment in a few words, for a reader.
    pub label: String,
    /// The probability that the answer is yes, from the model.
    pub probability: f64,
    pub answer: JudgmentAnswer,
    /// The answer as a statement, such as "Reports no passing tests".
    pub reading: String,
    /// Whether this judgment kept the advice from green.
    pub concern: bool,
}

/// The two thresholds every judgment is read against.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
pub struct LandGateAdviceThresholds {
    pub yes_min: f64,
    pub no_max: f64,
}

/// Whose key paid for the question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AdvisorKeySource {
    /// The workspace's own key, stored encrypted as a secret of its own.
    Workspace,
    /// The key this instance was started with.
    Instance,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LandGateAdvice {
    pub provider: String,
    pub model: String,
    /// The advice: green when every judgment is settled and clear, red when
    /// the hand-off reports a failure or is about other work, amber otherwise.
    pub land: LandColor,
    /// Why the advice is not green, as stable names. Empty when it is green.
    pub reasons: Vec<String>,
    pub judgments: Vec<LandGateJudgment>,
    pub thresholds: LandGateAdviceThresholds,
    pub key_source: AdvisorKeySource,
    /// True when this answer was held from an earlier call for the same
    /// hand-off and the model was not asked again.
    pub cached: bool,
    pub usage: LandGateAdviceUsage,
    pub latency_ms: u64,
}

/// One provider as a workspace sees it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LandGateAdvisorProvider {
    pub id: AdvisorProvider,
    /// The provider's name as a person reads it.
    pub name: String,
    /// The model and host a hand-off is sent to.
    pub model: String,
    pub host: String,
    /// Whose key a call from this workspace would use; absent when there is
    /// none, and then this provider is refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_source: Option<AdvisorKeySource>,
}

/// What a workspace has set up, for `GET /workspaces/{id}/land-gate/advisor`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LandGateAdvisorStatus {
    /// Whether this instance offers advice at all.
    pub enabled: bool,
    /// The provider a call that names none would ask: the first with a key
    /// for this workspace, or the first of all when none has one. The four
    /// fields below describe it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_source: Option<AdvisorKeySource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// Every provider this instance can ask, each with whose key it would use.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub providers: Vec<LandGateAdvisorProvider>,
}

impl LandGateAdvisorStatus {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            key_source: None,
            provider: None,
            model: None,
            host: None,
            providers: Vec::new(),
        }
    }

    fn of(providers: Vec<LandGateAdvisorProvider>) -> Self {
        let chosen = providers
            .iter()
            .find(|provider| provider.key_source.is_some())
            .or(providers.first());
        Self {
            enabled: true,
            key_source: chosen.and_then(|provider| provider.key_source),
            provider: chosen.map(|provider| provider.id.id().to_string()),
            model: chosen.map(|provider| provider.model.clone()),
            host: chosen.map(|provider| provider.host.clone()),
            providers,
        }
    }

    /// The same status with one provider's key changed: what a write that
    /// just committed made true, without a second read that could fail.
    pub fn with_key(self, provider: AdvisorProvider, key_source: Option<AdvisorKeySource>) -> Self {
        let mut providers = self.providers;
        for entry in &mut providers {
            if entry.id == provider {
                entry.key_source = key_source;
            }
        }
        Self::of(providers)
    }
}

/// One provider as the instance was started: its key, if the instance has
/// one, and where and what to ask.
struct ProviderSettings {
    provider: AdvisorProvider,
    /// The instance's own key. Without one, only a workspace that stored its
    /// own key for this provider gets advice from it.
    api_key: Option<String>,
    base_url: String,
    model: String,
}

struct Settings {
    providers: Vec<ProviderSettings>,
    /// The `.internal` names this instance may send to. With a listed
    /// loopback address they are the only hosts a provider may be reached
    /// at over plain HTTP.
    internal_hosts: Vec<String>,
    /// The loopback addresses among the private peers the operator listed
    /// (`MAIDAN_PRIVATE_EGRESS_HOSTS`): a stand-in provider on this machine.
    loopback_peers: Vec<std::net::IpAddr>,
    timeout: Duration,
}

impl Settings {
    fn from_lookup(
        get: impl Fn(&str) -> Option<String>,
    ) -> Result<Option<Self>, LandGateAdvisorError> {
        let enabled = match get("MAIDAN_JEV_LAND_GATE_ENABLED") {
            None => false,
            Some(value) => match value.trim() {
                "1" | "true" | "TRUE" => true,
                "0" | "false" | "FALSE" => false,
                _ => {
                    return Err(LandGateAdvisorError::Config(
                        "MAIDAN_JEV_LAND_GATE_ENABLED must be 0/1 or false/true".into(),
                    ));
                }
            },
        };
        if !enabled {
            return Ok(None);
        }
        let timeout_ms = get("MAIDAN_JEV_TIMEOUT_MS")
            .map(|value| {
                value.trim().parse::<u64>().map_err(|_| {
                    LandGateAdvisorError::Config(
                        "MAIDAN_JEV_TIMEOUT_MS must be a positive integer".into(),
                    )
                })
            })
            .transpose()?
            .unwrap_or(DEFAULT_TIMEOUT_MS);
        if timeout_ms == 0 {
            return Err(LandGateAdvisorError::Config(
                "MAIDAN_JEV_TIMEOUT_MS must be a positive integer".into(),
            ));
        }
        let mut providers = Vec::new();
        for provider in AdvisorProvider::ALL {
            let model = get(provider.model_env())
                .unwrap_or_else(|| provider.default_model().into())
                .trim()
                .to_string();
            if model.is_empty() {
                return Err(LandGateAdvisorError::Config(format!(
                    "{} must not be empty",
                    provider.model_env()
                )));
            }
            providers.push(ProviderSettings {
                provider,
                api_key: get(provider.key_env())
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty()),
                base_url: get(provider.base_url_env())
                    .unwrap_or_else(|| provider.default_base_url().into()),
                model,
            });
        }
        Ok(Some(Self {
            providers,
            internal_hosts: get(maidan_auth::INTERNAL_EGRESS_HOSTS_ENV)
                .map(|raw| maidan_auth::parse_internal_egress_hosts(&raw))
                .unwrap_or_default(),
            loopback_peers: get(maidan_auth::PRIVATE_EGRESS_HOSTS_ENV)
                .map(|raw| maidan_auth::parse_private_egress_hosts(&raw))
                .unwrap_or_default()
                .into_iter()
                // Parsed, so one address listed in another spelling
                // (`0:0:0:0:0:0:0:1` for `::1`) is still that address.
                .filter_map(|host| host.parse::<std::net::IpAddr>().ok())
                .filter(std::net::IpAddr::is_loopback)
                .collect(),
            timeout: Duration::from_millis(timeout_ms),
        }))
    }
}

/// The key a call is made with, and whose it is.
struct ResolvedKey {
    value: String,
    source: AdvisorKeySource,
}

struct Held {
    at: Instant,
    advice: LandGateAdvice,
}

/// How the advisor reaches a provider.
enum Transport {
    /// Through the egress guard: the name is resolved, checked and pinned,
    /// and looked up again every [`RESOLVE_TTL`]. Resolving on first use, not
    /// at boot, keeps an unreachable provider from stopping the server: only
    /// the advice call fails.
    Guarded(tokio::sync::Mutex<Option<(Instant, Client)>>),
    /// A client the caller made, for tests against a loopback stand-in.
    Fixed(Client),
}

/// One provider, ready to be asked.
struct Backend {
    provider: AdvisorProvider,
    transport: Transport,
    base_url: Url,
    endpoint: Url,
    instance_key: Option<String>,
    model: String,
}

impl Backend {
    fn build(
        provider: AdvisorProvider,
        transport: Transport,
        base_url: Url,
        instance_key: Option<String>,
        model: String,
    ) -> Result<Self, LandGateAdvisorError> {
        let endpoint = base_url.join(provider.ask_path()).map_err(|error| {
            LandGateAdvisorError::Config(format!("invalid {}: {error}", provider.base_url_env()))
        })?;
        Ok(Self {
            provider,
            transport,
            base_url,
            endpoint,
            instance_key,
            model,
        })
    }

    /// The client for the provider. Through the egress guard the provider's
    /// name is resolved and checked like any other outbound target; a lookup
    /// that fails is this call's failure, and the next call tries again.
    async fn client(&self) -> Result<Client, LandGateAdvisorError> {
        let cached = match &self.transport {
            Transport::Fixed(client) => return Ok(client.clone()),
            Transport::Guarded(cached) => cached,
        };
        let mut cached = cached.lock().await;
        if let Some((at, client)) = cached.as_ref()
            && at.elapsed() < RESOLVE_TTL
        {
            return Ok(client.clone());
        }
        let (client, _) = crate::egress_http::client_for(self.base_url.as_str())
            .await
            .map_err(LandGateAdvisorError::Unavailable)?;
        *cached = Some((Instant::now(), client.clone()));
        Ok(client)
    }

    fn host(&self) -> String {
        self.base_url.host_str().unwrap_or_default().to_string()
    }

    /// Whose key a call from `workspace_id` would use, if any.
    async fn key_source(
        &self,
        store: &dyn Store,
        workspace_id: WorkspaceId,
    ) -> Result<Option<AdvisorKeySource>, LandGateAdvisorError> {
        let own = store
            .get_secret_ciphertext(workspace_id, self.provider.secret())
            .await?;
        Ok(match (own, &self.instance_key) {
            (Some(_), _) => Some(AdvisorKeySource::Workspace),
            (None, Some(_)) => Some(AdvisorKeySource::Instance),
            (None, None) => None,
        })
    }

    fn status_with(&self, key_source: Option<AdvisorKeySource>) -> LandGateAdvisorProvider {
        LandGateAdvisorProvider {
            id: self.provider,
            name: self.provider.name().into(),
            model: self.model.clone(),
            host: self.host(),
            key_source,
        }
    }

    /// The workspace's own key when it stored one, else the instance's.
    async fn resolve_key(
        &self,
        store: &dyn Store,
        encryption_key: Option<&[u8; 32]>,
        workspace_id: WorkspaceId,
    ) -> Result<ResolvedKey, LandGateAdvisorError> {
        if let Some(ciphertext) = store
            .get_secret_ciphertext(workspace_id, self.provider.secret())
            .await?
        {
            let key = encryption_key.ok_or_else(|| {
                LandGateAdvisorError::Unavailable(
                    "this instance cannot open workspace secrets (no encryption key)".into(),
                )
            })?;
            let value = maidan_auth::decrypt_peer_secret_rotating(&ciphertext, key)
                .map_err(|error| LandGateAdvisorError::Unavailable(error.to_string()))?;
            return Ok(ResolvedKey {
                value,
                source: AdvisorKeySource::Workspace,
            });
        }
        match &self.instance_key {
            Some(value) => Ok(ResolvedKey {
                value: value.clone(),
                source: AdvisorKeySource::Instance,
            }),
            None => Err(LandGateAdvisorError::NotReady(format!(
                "this workspace has no {} key: store one with PUT /workspaces/{{id}}/land-gate/advisor",
                self.provider.vendor()
            ))),
        }
    }

    /// Ask the provider whether it accepts `api_key`, before a workspace
    /// stores it. A refusal is the caller's mistake; anything else is ours.
    async fn check_key(
        &self,
        api_key: &str,
        timeout: Duration,
    ) -> Result<(), LandGateAdvisorError> {
        let models = self
            .base_url
            .join(self.provider.models_path())
            .map_err(|error| {
                LandGateAdvisorError::Config(format!(
                    "invalid {}: {error}",
                    self.provider.base_url_env()
                ))
            })?;
        let request = self.client().await?.get(models);
        let response = self
            .provider
            .authorize(request, api_key)
            .timeout(timeout.max(Duration::from_secs(5)))
            .send()
            .await
            .map_err(|error| LandGateAdvisorError::Unavailable(error.to_string()))?;
        let status = response.status();
        // Google answers a key it does not know with 400, not 401.
        let refused = status == reqwest::StatusCode::UNAUTHORIZED
            || status == reqwest::StatusCode::FORBIDDEN
            || (self.provider == AdvisorProvider::Google
                && status == reqwest::StatusCode::BAD_REQUEST);
        if status.is_success() {
            Ok(())
        } else if refused {
            Err(LandGateAdvisorError::InvalidRequest(format!(
                "{} refused this key",
                self.provider.vendor()
            )))
        } else {
            Err(LandGateAdvisorError::Unavailable(format!(
                "provider returned {status}"
            )))
        }
    }

    /// The largest packet of this hand-off whose whole request stays within
    /// [`MAX_REQUEST_BYTES`]. Each candidate is measured as the request this
    /// provider would be sent, not as the packet alone: a provider that takes
    /// the packet as text pays again for every quote and backslash in it.
    fn packet(&self, hand_off: &HandOff) -> Option<Value> {
        hand_off.packet_that(|state| {
            request_body(self.provider, &self.model, &Value::Object(state.clone()))
                .to_string()
                .len()
                <= MAX_REQUEST_BYTES
        })
    }

    async fn ask(
        &self,
        state: &Value,
        key: &ResolvedKey,
        timeout: Duration,
    ) -> Result<LandGateAdvice, LandGateAdvisorError> {
        let body = request_body(self.provider, &self.model, state);
        let started = Instant::now();
        let request = self.client().await?.post(self.endpoint.clone());
        let response = self
            .provider
            .authorize(request, &key.value)
            .timeout(timeout)
            .json(&body)
            .send()
            .await
            .map_err(|error| LandGateAdvisorError::Unavailable(error.to_string()))?;
        if !response.status().is_success() {
            return Err(LandGateAdvisorError::Unavailable(format!(
                "provider returned {}",
                response.status()
            )));
        }
        let response: Value = response
            .json()
            .await
            .map_err(|error| LandGateAdvisorError::InvalidResponse(error.to_string()))?;
        let latency_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
        let answered = answers(self.provider, &response)?;
        let (land, reasons) = decide(&answered.probabilities);
        let judgments = QUESTIONS
            .iter()
            .map(|question| {
                let probability = answered
                    .probabilities
                    .get(question.id)
                    .copied()
                    .unwrap_or(0.5);
                let answer = JudgmentAnswer::of(probability);
                let concern = reasons.iter().any(|reason| concerns(reason, question.id));
                LandGateJudgment {
                    id: question.id.into(),
                    label: question.label.into(),
                    probability,
                    answer,
                    reading: reading(question, answer, concern).into(),
                    concern,
                }
            })
            .collect();
        Ok(LandGateAdvice {
            provider: self.provider.id().into(),
            model: answered.model.unwrap_or_else(|| self.model.clone()),
            land,
            reasons: reasons.into_iter().map(str::to_string).collect(),
            judgments,
            thresholds: LandGateAdviceThresholds {
                yes_min: YES_MIN,
                no_max: NO_MAX,
            },
            key_source: key.source,
            cached: false,
            usage: answered.usage,
            latency_ms,
        })
    }
}

/// The advisor: every provider this instance can ask and a short-lived hold
/// of answers, so a sheet opened twice asks once.
pub struct LandGateAdvisor {
    backends: Vec<Backend>,
    timeout: Duration,
    held: Mutex<HashMap<String, Held>>,
}

impl LandGateAdvisor {
    fn from_settings(settings: Settings) -> Result<Self, LandGateAdvisorError> {
        let mut backends = Vec::new();
        for provider in &settings.providers {
            let name = provider.provider.base_url_env();
            let base_url: Url = provider.base_url.trim().parse().map_err(|error| {
                LandGateAdvisorError::Config(format!("invalid {name}: {error}"))
            })?;
            if !matches!(base_url.scheme(), "https" | "http")
                || base_url.host_str().is_none()
                || base_url.path() != "/"
                || base_url.query().is_some()
                || base_url.fragment().is_some()
                || !base_url.username().is_empty()
                || base_url.password().is_some()
            {
                return Err(LandGateAdvisorError::Config(format!(
                    "{name} must be an origin URL without a path, query, or fragment"
                )));
            }
            // The key travels in a header: over plain HTTP only to a relay of
            // our own, a name listed in MAIDAN_INTERNAL_EGRESS_HOSTS, or to
            // this machine, a loopback address listed in
            // MAIDAN_PRIVATE_EGRESS_HOSTS, where nothing crosses a network.
            let host = base_url.host_str().unwrap_or_default().to_ascii_lowercase();
            let host = host
                .strip_prefix('[')
                .and_then(|rest| rest.strip_suffix(']'))
                .unwrap_or(&host)
                .to_string();
            let listed_loopback = host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| settings.loopback_peers.contains(&address));
            if base_url.scheme() == "http"
                && !settings.internal_hosts.contains(&host)
                && !listed_loopback
            {
                return Err(LandGateAdvisorError::Config(format!(
                    "{name} must be https, unless its host is listed in MAIDAN_INTERNAL_EGRESS_HOSTS or is a loopback address listed in MAIDAN_PRIVATE_EGRESS_HOSTS"
                )));
            }
            backends.push(Backend::build(
                provider.provider,
                Transport::Guarded(tokio::sync::Mutex::new(None)),
                base_url,
                provider.api_key.clone(),
                provider.model.clone(),
            )?);
        }
        Ok(Self {
            backends,
            timeout: settings.timeout,
            held: Mutex::new(HashMap::new()),
        })
    }

    /// Build an advisor on a client the caller made: the constructor tests and
    /// test servers use, which reach a stand-in on a loopback address. The
    /// stand-in answers for every provider; `instance_key` and `model` are
    /// TypeSafe's, and the other two start with no key of the instance's.
    pub fn with_http(
        client: Client,
        base_url: Url,
        instance_key: Option<String>,
        model: impl Into<String>,
        timeout: Duration,
    ) -> Result<Self, LandGateAdvisorError> {
        let mut advisor =
            Self::with_http_keys(client, base_url, [instance_key, None, None], timeout)?;
        advisor.backends[0].model = model.into();
        Ok(advisor)
    }

    /// As [`Self::with_http`], with the instance's key for each provider in
    /// the order of [`AdvisorProvider::ALL`].
    pub fn with_http_keys(
        client: Client,
        base_url: Url,
        instance_keys: [Option<String>; 3],
        timeout: Duration,
    ) -> Result<Self, LandGateAdvisorError> {
        let backends = AdvisorProvider::ALL
            .into_iter()
            .zip(instance_keys)
            .map(|(provider, key)| {
                Backend::build(
                    provider,
                    Transport::Fixed(client.clone()),
                    base_url.clone(),
                    key,
                    provider.default_model().into(),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            backends,
            timeout,
            held: Mutex::new(HashMap::new()),
        })
    }

    fn backend(&self, provider: AdvisorProvider) -> Result<&Backend, LandGateAdvisorError> {
        self.backends
            .iter()
            .find(|backend| backend.provider == provider)
            .ok_or_else(|| {
                LandGateAdvisorError::NotReady(format!(
                    "this instance does not offer {}",
                    provider.name()
                ))
            })
    }

    /// What `workspace_id` has set up, provider by provider.
    pub async fn status(
        &self,
        store: &dyn Store,
        workspace_id: WorkspaceId,
    ) -> Result<LandGateAdvisorStatus, LandGateAdvisorError> {
        let mut providers = Vec::with_capacity(self.backends.len());
        for backend in &self.backends {
            providers.push(backend.status_with(backend.key_source(store, workspace_id).await?));
        }
        Ok(LandGateAdvisorStatus::of(providers))
    }

    /// Whether the instance has a key of its own for `provider`.
    pub fn has_instance_key(&self, provider: AdvisorProvider) -> bool {
        self.backend(provider)
            .is_ok_and(|backend| backend.instance_key.is_some())
    }

    /// Ask `provider` whether it accepts `api_key`, before a workspace
    /// stores it.
    pub async fn check_key(
        &self,
        provider: AdvisorProvider,
        api_key: &str,
    ) -> Result<(), LandGateAdvisorError> {
        self.backend(provider)?
            .check_key(api_key, self.timeout)
            .await
    }

    /// The provider a call asks: the one it names, or the first this
    /// workspace has a key for.
    async fn chosen(
        &self,
        store: &dyn Store,
        workspace_id: WorkspaceId,
        named: Option<AdvisorProvider>,
    ) -> Result<&Backend, LandGateAdvisorError> {
        if let Some(provider) = named {
            return self.backend(provider);
        }
        for backend in &self.backends {
            if backend.key_source(store, workspace_id).await?.is_some() {
                return Ok(backend);
            }
        }
        Err(LandGateAdvisorError::NotReady(
            "this workspace has no key for a decision model: store one with PUT /workspaces/{id}/land-gate/advisor"
                .into(),
        ))
    }

    /// Advise on `thread_id`'s hand-off. The caller has already checked that
    /// its principal may read the thread, and that the thread is in
    /// `workspace_id`.
    pub async fn advise(
        &self,
        store: &dyn Store,
        encryption_key: Option<&[u8; 32]>,
        workspace_id: WorkspaceId,
        thread_id: ThreadId,
        request: LandGateAdviceRequest,
    ) -> Result<LandGateAdvice, LandGateAdvisorError> {
        let backend = self.chosen(store, workspace_id, request.provider).await?;
        let state = backend
            .packet(&hand_off(store, thread_id).await?)
            .ok_or_else(|| {
                LandGateAdvisorError::NotReady(
                    "this hand-off is too large to send for advice".into(),
                )
            })?;
        let key = backend
            .resolve_key(store, encryption_key, workspace_id)
            .await?;
        // Held under the key's owner too: an answer one key paid for is never
        // shown as another's, and a workspace that adds its key asks with it.
        let held_as = held_as(workspace_id, backend.provider, &backend.model, &key, &state);
        if !request.refresh
            && let Some(mut advice) = self.held(&held_as)
        {
            advice.cached = true;
            return Ok(advice);
        }
        let advice = backend.ask(&state, &key, self.timeout).await?;
        self.hold(held_as, advice.clone());
        Ok(advice)
    }

    fn held(&self, key: &str) -> Option<LandGateAdvice> {
        let held = self.held.lock().ok()?;
        held.get(key)
            .filter(|entry| entry.at.elapsed() < CACHE_TTL)
            .map(|entry| entry.advice.clone())
    }

    fn hold(&self, key: String, advice: LandGateAdvice) {
        let Ok(mut held) = self.held.lock() else {
            return;
        };
        if held.len() >= CACHE_CAPACITY {
            held.retain(|_, entry| entry.at.elapsed() < CACHE_TTL);
            if held.len() >= CACHE_CAPACITY {
                held.clear();
            }
        }
        held.insert(
            key,
            Held {
                at: Instant::now(),
                advice,
            },
        );
    }
}

/// What a provider answered, in one shape: the probability of yes for each
/// question it answered, the model it names, and what it counted.
struct Answered {
    probabilities: BTreeMap<&'static str, f64>,
    model: Option<String>,
    usage: LandGateAdviceUsage,
}

/// A probability a provider gave for `id`, checked.
fn probability(value: Option<&Value>, id: &str, field: &str) -> Result<f64, LandGateAdvisorError> {
    match value.and_then(Value::as_f64) {
        Some(p) if p.is_finite() && (0.0..=1.0).contains(&p) => Ok(p),
        _ => Err(LandGateAdvisorError::InvalidResponse(format!(
            "`answers.{id}.{field}` was outside [0, 1]"
        ))),
    }
}

/// Read a provider's response. Every question must come back answered,
/// except where the provider says it declined one: that judgment is left
/// out, and [`decide`] reads a judgment it was not given as unsettled.
fn answers(provider: AdvisorProvider, response: &Value) -> Result<Answered, LandGateAdvisorError> {
    let count = |path: &[&str]| {
        path.iter()
            .try_fold(response, |at, key| at.get(key))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    let model = response
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_string);
    let mut probabilities = BTreeMap::new();
    match provider {
        AdvisorProvider::Typesafe | AdvisorProvider::Google => {
            for question in &QUESTIONS {
                let id = question.id;
                let answer = response
                    .get("answers")
                    .and_then(|answers| answers.get(id))
                    .ok_or_else(|| {
                        LandGateAdvisorError::InvalidResponse(format!(
                            "response omitted `answers.{id}`"
                        ))
                    })?;
                if answer.get("type").and_then(Value::as_str) != Some("noul") {
                    return Err(LandGateAdvisorError::InvalidResponse(format!(
                        "`answers.{id}` was not a noul"
                    )));
                }
                probabilities.insert(id, probability(answer.get("noul"), id, "noul")?);
            }
        }
        AdvisorProvider::Openai => {
            let given = response
                .get("answers")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    LandGateAdvisorError::InvalidResponse("response had no `answers`".into())
                })?;
            for question in &QUESTIONS {
                let id = question.id;
                let answer = given
                    .iter()
                    .find(|answer| answer.get("name").and_then(Value::as_str) == Some(id))
                    .ok_or_else(|| {
                        LandGateAdvisorError::InvalidResponse(format!(
                            "response omitted `answers.{id}`"
                        ))
                    })?;
                match answer.get("type").and_then(Value::as_str) {
                    Some("predicate") => {
                        probabilities.insert(
                            id,
                            probability(answer.get("probability"), id, "probability")?,
                        );
                    }
                    // The model declined this one question: unsettled.
                    Some("refusal") => {}
                    _ => {
                        return Err(LandGateAdvisorError::InvalidResponse(format!(
                            "`answers.{id}` was not a predicate"
                        )));
                    }
                }
            }
        }
    }
    let usage = match provider {
        AdvisorProvider::Typesafe | AdvisorProvider::Openai => LandGateAdviceUsage {
            input_tokens: count(&["usage", "input_tokens"]),
            output_tokens: count(&["usage", "output_tokens"]),
        },
        AdvisorProvider::Google => LandGateAdviceUsage {
            input_tokens: count(&["usage", "total_input_tokens"]),
            output_tokens: count(&["usage", "total_output_tokens"]),
        },
    };
    Ok(Answered {
        probabilities,
        model,
        usage,
    })
}

/// The decision, in code. `p` holds every judgment's probability of yes.
///
/// - Red when the hand-off reports something still failing, or is about work
///   other than the task.
/// - Green only when the work is described as done, every warning sign is a
///   settled no, and a task that asks for a code change reports its tests
///   passing.
/// - Amber for everything else, an unsettled judgment included: nothing here
///   promotes a doubt to green.
///
/// The reasons name everything that kept the advice from green, with what
/// made it red first.
pub fn decide(p: &BTreeMap<&'static str, f64>) -> (LandColor, Vec<&'static str>) {
    let of = |id: &str| p.get(id).copied().unwrap_or(0.5);
    let failing = of("unresolved_failure") >= YES_MIN;
    let other_work = of("done") <= NO_MAX && of("beyond_task") >= YES_MIN;
    let mut reasons = Vec::new();
    if failing {
        reasons.push("unresolved_failure");
    }
    if other_work {
        reasons.push("different_work");
    } else if of("done") < YES_MIN {
        reasons.push("not_shown_done");
    }
    for id in [
        "unresolved_failure",
        "left_undone",
        "asks_person",
        "beyond_task",
        "changes_access",
        "deletes_data",
        "flags_risk",
    ] {
        // Said already: the failure leads the list, and work on something
        // else is the beyond-the-task judgment read with the task not done.
        let said = (id == "unresolved_failure" && failing) || (id == "beyond_task" && other_work);
        if of(id) > NO_MAX && !said {
            reasons.push(id);
        }
    }
    if of("code_change") > NO_MAX && of("tests_passed") < YES_MIN {
        reasons.push("tests_not_reported");
    }
    let land = if failing || other_work {
        LandColor::Red
    } else if reasons.is_empty() {
        LandColor::Green
    } else {
        LandColor::Amber
    };
    (land, reasons)
}

/// A judgment's answer as a statement.
fn reading(question: &Question, answer: JudgmentAnswer, concern: bool) -> &'static str {
    match (question.id, answer, concern) {
        ("tests_passed", JudgmentAnswer::No, false) => NO_TESTS_NOT_CALLED_FOR,
        ("tests_passed", JudgmentAnswer::Unsettled, false) => UNCLEAR_TESTS_NOT_CALLED_FOR,
        _ => question.reads[answer.index()],
    }
}

/// Whether `reason` rests on the judgment `id`, so a reader sees which
/// answers kept the advice from green.
fn concerns(reason: &str, id: &str) -> bool {
    match reason {
        "not_shown_done" => id == "done",
        "different_work" => id == "done" || id == "beyond_task",
        // The task asking for a code change is why tests matter, not a
        // concern itself.
        "tests_not_reported" => id == "tests_passed",
        other => other == id,
    }
}

/// The request a provider is sent: the model, the packet and every question
/// as written in [`QUESTIONS`], in that provider's own shape. The wording is
/// the same for all three.
fn request_body(provider: AdvisorProvider, model: &str, state: &Value) -> Value {
    match provider {
        AdvisorProvider::Typesafe => {
            let questions: BTreeMap<&str, Value> = QUESTIONS
                .iter()
                .map(|q| {
                    (
                        q.id,
                        json!({
                            "type": "noul",
                            "instructions": q.instructions,
                            "criteria": { "true": q.yes, "false": q.no },
                        }),
                    )
                })
                .collect();
            json!({ "model": model, "state": state, "questions": questions })
        }
        // A predicate takes no description of yes and no, so they follow the
        // question in its instructions. The packet goes as a JSON part, which
        // keeps the field names the questions point at.
        AdvisorProvider::Openai => {
            let questions: Vec<Value> = QUESTIONS
                .iter()
                .map(|q| {
                    json!({
                        "type": "predicate",
                        "name": q.id,
                        "instructions": format!("{}\nYes: {}\nNo: {}", q.instructions, q.yes, q.no),
                    })
                })
                .collect();
            json!({
                "model": model,
                "input": [{ "role": "user", "content": [{ "type": "input_json", "data": state }] }],
                "questions": questions,
            })
        }
        // The shared input is text, so the packet is sent as its JSON.
        AdvisorProvider::Google => {
            let questions: BTreeMap<&str, Value> = QUESTIONS
                .iter()
                .map(|q| {
                    (
                        q.id,
                        json!({
                            "type": "noul",
                            "instructions": q.instructions,
                            "criteria": { "true_description": q.yes, "false_description": q.no },
                        }),
                    )
                })
                .collect();
            json!({ "model": model, "input": state.to_string(), "questions": questions })
        }
    }
}

/// A hand-off as read from the thread: what the task asks for, the result,
/// and what the agent that produced it said, oldest first.
struct HandOff {
    task: Map<String, Value>,
    result: Value,
    said: Vec<String>,
}

/// Read the hand-off. Built here so every call sends the same shape and
/// nothing a caller chose.
///
/// The task's opening message is left out on evidence: it tells the agent how
/// to work (claim, say what you ran, send to review), a hand-off cannot show
/// those steps, and with it "done" went unsettled on a quarter of the
/// labelled hand-offs. The opening message is the thread's first message when
/// the member that made the thread wrote it, so it stays out even where that
/// member went on to do the work.
async fn hand_off(store: &dyn Store, thread_id: ThreadId) -> Result<HandOff, LandGateAdvisorError> {
    let thread = store.get_thread(thread_id).await?;
    let made_by = thread.created_by;
    let result = store
        .get_thread_result(thread_id)
        .await?
        .ok_or_else(|| LandGateAdvisorError::NotReady("this task has no result yet".into()))?;
    let mut task = Map::new();
    task.insert(
        "title".into(),
        Value::String(clip(&thread.title.unwrap_or_default(), MAX_TITLE_CHARS)),
    );
    if let Some(description) = thread.description.filter(|d| !d.trim().is_empty()) {
        task.insert(
            "description".into(),
            Value::String(clip(&description, MAX_DESCRIPTION_CHARS)),
        );
    }
    // How a reviewer checks the task, where its creator said: "what the task
    // asks for" is read against it. Against the live model it left the
    // advice unchanged on sixteen labelled hand-offs and made "done" follow
    // the acceptance text.
    let attributes = store.get_thread_attributes(thread_id).await?;
    if let Some(acceptance) = attributes.acceptance.filter(|a| !a.trim().is_empty()) {
        task.insert(
            "acceptance".into(),
            Value::String(clip(&acceptance, MAX_MESSAGE_CHARS)),
        );
    }
    // The producer's newest messages, wherever they are in the thread: a
    // failure reported in message 201 is the one a reviewer needs.
    let page_size = i64::try_from(MESSAGE_PAGE).unwrap_or(i64::MAX);
    let mut said = std::collections::VecDeque::with_capacity(MAX_AGENT_MESSAGES);
    let mut after = None;
    let mut opening = true;
    for _ in 0..MESSAGE_PAGES {
        let page = store
            .list_messages_after(thread_id, after, page_size)
            .await?;
        let Some(last) = page.last() else {
            break;
        };
        after = Some(last.id);
        let full = page.len() >= MESSAGE_PAGE;
        for message in page {
            // The first message, from whoever made the thread, is the ask.
            let is_opening = std::mem::take(&mut opening) && made_by == Some(message.author_id);
            if is_opening {
                continue;
            }
            if message.author_id != result.produced_by
                || message.tombstoned_at.is_some()
                || message.body.trim().is_empty()
            {
                continue;
            }
            if said.len() == MAX_AGENT_MESSAGES {
                said.pop_front();
            }
            said.push_back(clip(&message.body, MAX_MESSAGE_CHARS));
        }
        if !full {
            break;
        }
    }
    Ok(HandOff {
        task,
        result: result.result,
        said: said.into(),
    })
}

impl HandOff {
    /// The packet the model reads, within `budget` bytes of its own JSON.
    #[cfg(test)]
    fn packet(&self, budget: usize) -> Option<Value> {
        self.packet_that(|state| {
            serde_json::to_vec(state).map_or(usize::MAX, |b| b.len()) <= budget
        })
    }

    /// The largest packet `fits` accepts. When the hand-off is too large, the
    /// agent's oldest messages are left out first, then long strings in the
    /// result are cut shorter. `None` when it fits at no cut. `fits` sees
    /// every candidate, so a caller can measure the request the packet would
    /// make and not the packet alone.
    fn packet_that(&self, fits: impl Fn(&Map<String, Value>) -> bool) -> Option<Value> {
        for cut in RESULT_STRING_CUTS {
            let mut state = Map::new();
            state.insert("task".into(), Value::Object(self.task.clone()));
            state.insert("result".into(), trimmed_result(&self.result, cut));
            if !fits(&state) {
                continue;
            }
            // The newest messages that fit, sent oldest first. An empty list
            // reads as a signal, so the field is left out instead.
            let mut from = self.said.len();
            while from > 0 {
                state.insert("agent_messages".into(), json!(self.said[from - 1..]));
                if !fits(&state) {
                    break;
                }
                from -= 1;
            }
            if from == self.said.len() {
                state.remove("agent_messages");
            } else {
                state.insert("agent_messages".into(), json!(self.said[from..]));
            }
            return Some(Value::Object(state));
        }
        None
    }
}

/// The result without the fields that carry bytes for the change flow, and
/// with any very long string cut.
fn trimmed_result(result: &Value, max_chars: usize) -> Value {
    match result {
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .filter(|(name, _)| !RESULT_FIELDS_LEFT_OUT.contains(&name.as_str()))
                .map(|(name, value)| (name.clone(), clipped(value, max_chars)))
                .collect(),
        ),
        other => clipped(other, max_chars),
    }
}

fn clipped(value: &Value, max_chars: usize) -> Value {
    match value {
        Value::String(text) => Value::String(clip(text, max_chars)),
        Value::Array(items) => {
            Value::Array(items.iter().map(|item| clipped(item, max_chars)).collect())
        }
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(name, value)| (name.clone(), clipped(value, max_chars)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn clip(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut cut: String = text.chars().take(max_chars).collect();
    cut.push_str(" [cut here: the rest was not sent]");
    cut
}

/// What an answer is held under: the workspace, the provider, the model, the key and the packet's
/// bytes, so a changed result or a new message asks again.
fn held_as(
    workspace_id: WorkspaceId,
    provider: AdvisorProvider,
    model: &str,
    key: &ResolvedKey,
    state: &Value,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(workspace_id.0.as_bytes());
    hasher.update(provider.id().as_bytes());
    hasher.update(b"\0");
    hasher.update(model.as_bytes());
    hasher.update(match key.source {
        AdvisorKeySource::Workspace => b"w",
        AdvisorKeySource::Instance => b"i",
    });
    // The key itself, not only whose it is: a workspace that replaces its key
    // asks with the new one, and is not shown what the old one paid for.
    hasher.update(Sha256::digest(key.value.as_bytes()));
    hasher.update(state.to_string().as_bytes());
    hex::encode(hasher.finalize())
}

/// Build the optional runtime. A missing or false flag returns `None` without
/// reading a credential. Nothing is resolved or contacted here.
pub fn from_env() -> Result<Option<Arc<LandGateAdvisor>>, LandGateAdvisorError> {
    let Some(settings) = Settings::from_lookup(|name| std::env::var(name).ok())? else {
        return Ok(None);
    };
    Ok(Some(Arc::new(LandGateAdvisor::from_settings(settings)?)))
}

/// The MCP tool's way to the advisor the HTTP route serves. Advice is JSON
/// out, so the MCP crate does not depend on the HTTP types.
pub struct McpLandGateBridge {
    pub advisor: Arc<LandGateAdvisor>,
    pub store: Arc<dyn Store>,
    pub encryption_key: Option<[u8; 32]>,
}

#[async_trait::async_trait]
impl maidan_mcp::LandGateAdvising for McpLandGateBridge {
    async fn advise(
        &self,
        workspace_id: WorkspaceId,
        thread_id: ThreadId,
        refresh: bool,
        provider: Option<String>,
    ) -> Result<Value, maidan_mcp::LandGateAdviseError> {
        let provider = match provider.as_deref() {
            None => None,
            Some(id) => Some(AdvisorProvider::from_id(id).ok_or_else(|| {
                maidan_mcp::LandGateAdviseError::Invalid(format!(
                    "provider is one of typesafe, openai or google, not `{id}`"
                ))
            })?),
        };
        let advice = self
            .advisor
            .advise(
                self.store.as_ref(),
                self.encryption_key.as_ref(),
                workspace_id,
                thread_id,
                LandGateAdviceRequest { refresh, provider },
            )
            .await;
        match advice {
            Ok(advice) => serde_json::to_value(advice)
                .map_err(|error| maidan_mcp::LandGateAdviseError::Unavailable(error.to_string())),
            Err(
                LandGateAdvisorError::InvalidRequest(error) | LandGateAdvisorError::NotReady(error),
            ) => Err(maidan_mcp::LandGateAdviseError::Invalid(error)),
            Err(LandGateAdvisorError::Store(maidan_store::StoreError::NotFound)) => {
                Err(maidan_mcp::LandGateAdviseError::NotFound)
            }
            Err(error) => Err(maidan_mcp::LandGateAdviseError::Unavailable(
                error.to_string(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let values: HashMap<String, String> = pairs
            .iter()
            .map(|(name, value)| ((*name).into(), (*value).into()))
            .collect();
        move |name| values.get(name).cloned()
    }

    /// Every judgment at a settled, clear answer for a task that asks for a
    /// code change: the one packet that is green.
    fn clear() -> BTreeMap<&'static str, f64> {
        BTreeMap::from([
            ("done", 0.95),
            ("tests_passed", 0.97),
            ("code_change", 0.98),
            ("unresolved_failure", 0.02),
            ("left_undone", 0.03),
            ("asks_person", 0.02),
            ("beyond_task", 0.05),
            ("changes_access", 0.02),
            ("deletes_data", 0.02),
            ("flags_risk", 0.04),
        ])
    }

    fn with(changes: &[(&'static str, f64)]) -> BTreeMap<&'static str, f64> {
        let mut p = clear();
        for (id, value) in changes {
            p.insert(id, *value);
        }
        p
    }

    #[test]
    fn the_feature_is_off_by_default_and_reads_no_credential() {
        assert!(Settings::from_lookup(lookup(&[])).unwrap().is_none());
        assert!(
            Settings::from_lookup(lookup(&[("MAIDAN_JEV_LAND_GATE_ENABLED", "0")]))
                .unwrap()
                .is_none()
        );
        assert!(
            Settings::from_lookup(lookup(&[("MAIDAN_JEV_LAND_GATE_ENABLED", "maybe")])).is_err()
        );
    }

    #[test]
    fn an_instance_may_be_on_with_no_key_of_its_own() {
        let on = Settings::from_lookup(lookup(&[("MAIDAN_JEV_LAND_GATE_ENABLED", "1")]))
            .unwrap()
            .unwrap();
        assert_eq!(
            on.providers.iter().map(|p| p.provider).collect::<Vec<_>>(),
            AdvisorProvider::ALL
        );
        assert!(
            on.providers.iter().all(|p| p.api_key.is_none()),
            "workspaces bring their own"
        );
        assert_eq!(
            on.providers
                .iter()
                .map(|p| p.model.as_str())
                .collect::<Vec<_>>(),
            ["jev-latest", "gpt-6-luna", "gemini-decision-1"]
        );
        let keyed = Settings::from_lookup(lookup(&[
            ("MAIDAN_JEV_LAND_GATE_ENABLED", "1"),
            ("TYPESAFE_API_KEY", " secret "),
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(keyed.providers[0].api_key.as_deref(), Some("secret"));
        assert!(keyed.providers[1].api_key.is_none());
        // Each provider's key, model and address are its own.
        let three = Settings::from_lookup(lookup(&[
            ("MAIDAN_JEV_LAND_GATE_ENABLED", "1"),
            ("OPENAI_API_KEY", "sk-one"),
            ("GEMINI_API_KEY", "g-two"),
            ("MAIDAN_OPENAI_DECISIONS_MODEL", "gpt-7"),
            ("MAIDAN_GEMINI_DECISIONS_BASE_URL", "https://relay.example/"),
        ]))
        .unwrap()
        .unwrap();
        assert!(three.providers[0].api_key.is_none());
        assert_eq!(three.providers[1].api_key.as_deref(), Some("sk-one"));
        assert_eq!(three.providers[1].model, "gpt-7");
        assert_eq!(three.providers[2].api_key.as_deref(), Some("g-two"));
        assert_eq!(three.providers[2].base_url, "https://relay.example/");
        assert!(Settings::from_lookup(lookup(&[
            ("MAIDAN_JEV_LAND_GATE_ENABLED", "1"),
            ("MAIDAN_GEMINI_DECISIONS_MODEL", " "),
        ]))
        .is_err());
        assert!(Settings::from_lookup(lookup(&[
            ("MAIDAN_JEV_LAND_GATE_ENABLED", "1"),
            ("MAIDAN_JEV_TIMEOUT_MS", "0"),
        ]))
        .is_err());
    }

    #[test]
    fn the_provider_address_is_an_origin_and_is_not_looked_up_at_boot() {
        let on = |url: &str| {
            Settings::from_lookup(lookup(&[
                ("MAIDAN_JEV_LAND_GATE_ENABLED", "1"),
                ("MAIDAN_JEV_BASE_URL", url),
            ]))
            .unwrap()
            .map(LandGateAdvisor::from_settings)
            .unwrap()
        };
        // A name nothing resolves still builds: the lookup is the first call's.
        let advisor = on("https://provider.invalid/").unwrap();
        assert_eq!(advisor.backends[0].host(), "provider.invalid");
        assert_eq!(
            advisor.backends[0].endpoint.as_str(),
            "https://provider.invalid/v1/systemone"
        );
        // The other two keep their own addresses and paths.
        assert_eq!(
            advisor.backends[1].endpoint.as_str(),
            "https://api.openai.com/v1/decisions"
        );
        assert_eq!(
            advisor.backends[2].endpoint.as_str(),
            "https://generativelanguage.googleapis.com/v1beta/decisions"
        );
        for bad in [
            "https://provider.invalid/v1",
            "https://provider.invalid/?x=1",
            "https://user:pw@provider.invalid/",
            "ftp://provider.invalid/",
            "not a url",
        ] {
            assert!(on(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_key_goes_over_plain_http_only_to_a_listed_internal_relay() {
        let on = |url: &str, internal: &str| {
            Settings::from_lookup(lookup(&[
                ("MAIDAN_JEV_LAND_GATE_ENABLED", "1"),
                ("MAIDAN_JEV_BASE_URL", url),
                ("MAIDAN_INTERNAL_EGRESS_HOSTS", internal),
            ]))
            .unwrap()
            .map(LandGateAdvisor::from_settings)
            .unwrap()
        };
        assert!(on("http://provider.invalid/", "").is_err());
        assert!(on("http://127.0.0.1:8080/", "").is_err());
        assert!(on("http://jev.internal/", "hooks.internal").is_err());
        assert!(on("http://jev.internal/", "hooks.internal, jev.internal").is_ok());
        assert!(on("https://provider.invalid/", "").is_ok());
    }

    #[test]
    fn the_key_goes_over_plain_http_to_this_machine_only_when_its_address_is_listed() {
        let on = |url: &str, peers: &str| {
            Settings::from_lookup(lookup(&[
                ("MAIDAN_JEV_LAND_GATE_ENABLED", "1"),
                ("MAIDAN_JEV_BASE_URL", url),
                ("MAIDAN_PRIVATE_EGRESS_HOSTS", peers),
            ]))
            .unwrap()
            .map(LandGateAdvisor::from_settings)
            .unwrap()
        };
        // A stand-in provider on this machine, as scripts/mcp-inspector.sh
        // runs one.
        assert!(on("http://127.0.0.1:8080/", "127.0.0.1").is_ok());
        assert!(on("http://[::1]:8080/", "::1").is_ok());
        assert!(on("http://[::1]:8080/", "0:0:0:0:0:0:0:1").is_ok());
        assert!(on("http://127.0.0.1:8080/", "").is_err());
        assert!(on("http://127.0.0.1:8080/", "::1").is_err());
        // A listed private peer that is not this machine still needs https:
        // the key would cross a network in the clear.
        assert!(on("http://10.0.0.7:8080/", "10.0.0.7").is_err());
        assert!(on("http://relay.lan:8080/", "relay.lan").is_err());
        assert!(on("http://localhost:8080/", "localhost").is_err());
    }

    #[test]
    fn every_question_is_one_noul_with_both_criteria_and_a_distinct_name() {
        let mut seen = std::collections::BTreeSet::new();
        for q in &QUESTIONS {
            assert!(seen.insert(q.id), "{} is asked twice", q.id);
            assert!(q.instructions.ends_with('?'), "{}", q.id);
            assert!(!q.yes.is_empty() && !q.no.is_empty() && !q.label.is_empty());
            assert!(q.reads.iter().all(|r| !r.is_empty()), "{}", q.id);
            assert!(q.reads[2].starts_with("Unclear whether"), "{}", q.id);
        }
    }

    #[test]
    fn green_needs_every_judgment_settled_and_clear() {
        assert_eq!(decide(&clear()), (LandColor::Green, vec![]));
        // A task that asks for no code change needs no tests.
        assert_eq!(
            decide(&with(&[("code_change", 0.04), ("tests_passed", 0.01)])).0,
            LandColor::Green
        );
    }

    #[test]
    fn a_doubt_never_counts_toward_green_and_one_the_rules_did_not_need_changes_nothing() {
        // Unsure whether the task asks for a code change: read as one, so the
        // tests are asked for.
        assert_eq!(
            decide(&with(&[("code_change", 0.5), ("tests_passed", 0.02)])),
            (LandColor::Amber, vec!["tests_not_reported"])
        );
        // Unsure whether the tests passed: read as not reported.
        assert_eq!(
            decide(&with(&[("tests_passed", 0.5)])),
            (LandColor::Amber, vec!["tests_not_reported"])
        );
        // The two cases where the unsettled answer could not change the
        // advice: the tests passed whether or not the task asked for code,
        // and tests are not asked of a task that asks for no code.
        assert_eq!(
            decide(&with(&[("code_change", 0.5), ("tests_passed", 0.97)])).0,
            LandColor::Green
        );
        assert_eq!(
            decide(&with(&[("code_change", 0.04), ("tests_passed", 0.5)])).0,
            LandColor::Green
        );
    }

    #[test]
    fn tests_a_task_does_not_call_for_read_as_no_concern() {
        let tests = QUESTIONS
            .iter()
            .find(|q| q.id == "tests_passed")
            .expect("the tests question");
        // Asked for and missing: the plain statement, marked as a concern.
        assert_eq!(
            reading(tests, JudgmentAnswer::No, true),
            "Reports no passing tests"
        );
        assert_eq!(
            reading(tests, JudgmentAnswer::No, false),
            NO_TESTS_NOT_CALLED_FOR
        );
        assert_eq!(
            reading(tests, JudgmentAnswer::Unsettled, false),
            UNCLEAR_TESTS_NOT_CALLED_FOR
        );
        assert_eq!(
            reading(tests, JudgmentAnswer::Yes, false),
            "Reports tests or checks passing"
        );
        // No other judgment changes its words with the rules.
        let done = &QUESTIONS[0];
        assert_eq!(
            reading(done, JudgmentAnswer::No, false),
            "Does not show the task was done"
        );
    }

    #[test]
    fn an_unsettled_judgment_is_never_green() {
        for id in [
            "left_undone",
            "asks_person",
            "beyond_task",
            "changes_access",
            "deletes_data",
            "flags_risk",
            "unresolved_failure",
        ] {
            let (land, reasons) = decide(&with(&[(id, 0.5)]));
            assert_eq!(land, LandColor::Amber, "{id} at 0.5");
            assert_eq!(reasons, vec![id]);
        }
        let (land, reasons) = decide(&with(&[("done", 0.6)]));
        assert_eq!((land, reasons), (LandColor::Amber, vec!["not_shown_done"]));
    }

    #[test]
    fn a_code_change_without_reported_tests_is_amber() {
        let (land, reasons) = decide(&with(&[("tests_passed", 0.02)]));
        assert_eq!(
            (land, reasons),
            (LandColor::Amber, vec!["tests_not_reported"])
        );
    }

    #[test]
    fn a_reported_failure_or_other_work_is_red() {
        assert_eq!(
            decide(&with(&[("unresolved_failure", 0.97)])),
            (LandColor::Red, vec!["unresolved_failure"])
        );
        assert_eq!(
            decide(&with(&[("done", 0.02), ("beyond_task", 0.99)])),
            (LandColor::Red, vec!["different_work"])
        );
        // Red still lists every other sign, after what made it red.
        assert_eq!(
            decide(&with(&[
                ("unresolved_failure", 0.99),
                ("done", 0.05),
                ("left_undone", 0.96),
                ("tests_passed", 0.01),
            ])),
            (
                LandColor::Red,
                vec![
                    "unresolved_failure",
                    "not_shown_done",
                    "left_undone",
                    "tests_not_reported"
                ]
            )
        );
        // Work that is only not shown as done is a doubt, not a refusal.
        assert_eq!(decide(&with(&[("done", 0.02)])).0, LandColor::Amber);
    }

    #[test]
    fn a_missing_judgment_counts_as_unsettled() {
        let mut p = clear();
        p.remove("deletes_data");
        assert_eq!(decide(&p).0, LandColor::Amber);
    }

    #[test]
    fn the_result_is_sent_without_its_diff_and_with_long_text_cut() {
        let cut = RESULT_STRING_CUTS[0];
        let long = "x".repeat(cut + 10);
        let trimmed = trimmed_result(
            &json!({
                "summary": "Renamed the module.",
                "diff": "diff --git a/x b/x",
                "deliver_to": { "repo": "example/app" },
                "base_sha": "abc",
                "notes": [long],
            }),
            cut,
        );
        assert_eq!(trimmed["summary"], "Renamed the module.");
        assert!(trimmed.get("diff").is_none());
        assert!(trimmed.get("deliver_to").is_none());
        assert!(trimmed.get("base_sha").is_none());
        let note = trimmed["notes"][0].as_str().unwrap();
        assert!(note.ends_with("[cut here: the rest was not sent]"));
        assert!(note.chars().count() < cut + 60);
        // A result that is not an object is sent as it is.
        assert_eq!(trimmed_result(&json!("done"), cut), json!("done"));
    }

    fn hand_off_of(result: Value, said: &[&str]) -> HandOff {
        let mut task = Map::new();
        task.insert("title".into(), json!("Rename the billing module"));
        HandOff {
            task,
            result,
            said: said.iter().map(|body| (*body).to_string()).collect(),
        }
    }

    #[test]
    fn a_small_hand_off_is_sent_whole_and_no_messages_means_no_field() {
        let whole = hand_off_of(json!({ "summary": "Renamed." }), &["first", "second"])
            .packet(4_000)
            .unwrap();
        assert_eq!(
            whole,
            json!({
                "task": { "title": "Rename the billing module" },
                "result": { "summary": "Renamed." },
                "agent_messages": ["first", "second"],
            })
        );
        let silent = hand_off_of(json!({ "summary": "Renamed." }), &[])
            .packet(4_000)
            .unwrap();
        assert!(silent.get("agent_messages").is_none());
    }

    #[test]
    fn a_large_hand_off_loses_its_oldest_messages_first_then_has_its_result_cut() {
        let wordy = "m".repeat(400);
        let said = [wordy.as_str(), wordy.as_str(), "the newest"];
        let packet = hand_off_of(json!({ "summary": "Renamed." }), &said)
            .packet(600)
            .unwrap();
        assert_eq!(packet["agent_messages"], json!([wordy, "the newest"]));
        assert_eq!(packet["result"]["summary"], "Renamed.");
        assert!(packet.to_string().len() <= 600);

        // A result too long for the budget is cut shorter, never dropped.
        let long = hand_off_of(json!({ "summary": "s".repeat(3_000) }), &["the newest"]);
        let packet = long.packet(1_400).unwrap();
        let summary = packet["result"]["summary"].as_str().unwrap();
        assert!(summary.ends_with("[cut here: the rest was not sent]"));
        assert!(summary.chars().count() < 1_100, "cut at 1,000 to fit");
        assert_eq!(packet["agent_messages"], json!(["the newest"]));
        assert!(packet.to_string().len() <= 1_400);

        // One that fits at no cut is not sent at all.
        let fields: Map<String, Value> = (0..200)
            .map(|i| (format!("field_{i}"), json!("x".repeat(300))))
            .collect();
        assert!(hand_off_of(Value::Object(fields), &[])
            .packet(2_000)
            .is_none());
    }

    #[test]
    fn the_whole_request_stays_within_its_cap_for_every_provider() {
        let advisor = LandGateAdvisor::with_http(
            Client::new(),
            "http://127.0.0.1:9/".parse().unwrap(),
            None,
            "jev-latest",
            Duration::from_secs(1),
        )
        .unwrap();
        // The worst a thread can hold: every string at its cut, in characters
        // that take four bytes each, and quotes, which a provider that takes
        // the packet as text pays for twice.
        let wide = "\u{1F600}\"".repeat(5_000);
        let mut task = Map::new();
        task.insert("title".into(), json!(clip(&wide, MAX_TITLE_CHARS)));
        task.insert(
            "description".into(),
            json!(clip(&wide, MAX_DESCRIPTION_CHARS)),
        );
        let said: Vec<String> = (0..MAX_AGENT_MESSAGES)
            .map(|_| clip(&wide, MAX_MESSAGE_CHARS))
            .collect();
        let worst = HandOff {
            task,
            result: json!({ "summary": wide, "notes": [wide, wide] }),
            said,
        };
        // An ordinary hand-off has room for every message.
        let ordinary = hand_off_of(
            json!({ "summary": "Renamed the module and updated every import." }),
            &["Renamed.", "Tests pass."],
        );
        for backend in &advisor.backends {
            let name = backend.provider.id();
            if let Some(state) = backend.packet(&worst) {
                let body = request_body(backend.provider, &backend.model, &state).to_string();
                assert!(
                    body.len() <= MAX_REQUEST_BYTES,
                    "{name}: {} bytes",
                    body.len()
                );
            }
            let state = backend.packet(&ordinary).unwrap();
            assert_eq!(state["agent_messages"], json!(["Renamed.", "Tests pass."]));
            // The model's name and the questions leave most of the request
            // for the packet.
            let envelope = request_body(backend.provider, &backend.model, &Value::Null)
                .to_string()
                .len();
            assert!(MAX_REQUEST_BYTES - envelope > 8_000, "{name}: {envelope}");
        }
    }

    #[test]
    fn a_message_that_does_not_fit_as_text_is_left_out_and_the_rest_is_kept() {
        let advisor = LandGateAdvisor::with_http(
            Client::new(),
            "http://127.0.0.1:9/".parse().unwrap(),
            None,
            "jev-latest",
            Duration::from_secs(1),
        )
        .unwrap();
        let google = &advisor.backends[2];
        assert_eq!(google.provider, AdvisorProvider::Google);
        // A result that takes most of the request once it is text, and a
        // newest message of backslashes: each costs four bytes as text.
        let result = json!({ "summary": "a".repeat(3_900), "notes": "b".repeat(3_900) });
        let slashes = "\\".repeat(MAX_MESSAGE_CHARS);
        let hand_off = hand_off_of(result.clone(), &["Done.", &slashes]);
        let state = google
            .packet(&hand_off)
            .expect("the task and result fit on their own");
        assert_eq!(state["result"], result, "the result is whole, not cut");
        assert!(
            state.get("agent_messages").is_none(),
            "{}",
            state["agent_messages"]
        );
        let body = request_body(google.provider, &google.model, &state).to_string();
        assert!(body.len() <= MAX_REQUEST_BYTES, "{} bytes", body.len());
        // The same hand-off as a JSON object has room for the message.
        let jev = advisor.backends[0].packet(&hand_off).unwrap();
        assert_eq!(jev["agent_messages"].as_array().map(Vec::len), Some(2));
    }

    #[test]
    fn each_provider_is_asked_the_same_ten_questions_in_its_own_shape() {
        let state =
            json!({ "task": { "title": "Rename it" }, "result": { "summary": "Renamed." } });
        let jev = request_body(AdvisorProvider::Typesafe, "jev-latest", &state);
        assert_eq!(jev["state"], state);
        assert_eq!(jev["questions"]["done"]["type"], "noul");
        assert_eq!(
            jev["questions"]["done"]["criteria"]["true"],
            QUESTIONS[0].yes
        );
        assert_eq!(jev["questions"].as_object().unwrap().len(), QUESTIONS.len());

        let openai = request_body(AdvisorProvider::Openai, "gpt-6-luna", &state);
        // The packet keeps its field names: the questions point at them.
        assert_eq!(openai["input"][0]["content"][0]["type"], "input_json");
        assert_eq!(openai["input"][0]["content"][0]["data"], state);
        let asked = openai["questions"].as_array().unwrap();
        assert_eq!(asked.len(), QUESTIONS.len());
        assert_eq!(asked[0]["type"], "predicate");
        assert_eq!(asked[0]["name"], "done");
        let instructions = asked[0]["instructions"].as_str().unwrap();
        assert!(instructions.starts_with(QUESTIONS[0].instructions));
        assert!(instructions.contains(QUESTIONS[0].yes) && instructions.contains(QUESTIONS[0].no));

        let google = request_body(AdvisorProvider::Google, "gemini-decision-1", &state);
        assert_eq!(google["input"], json!(state.to_string()));
        assert_eq!(google["questions"]["done"]["type"], "noul");
        assert_eq!(
            google["questions"]["done"]["criteria"]["true_description"],
            QUESTIONS[0].yes
        );
        assert_eq!(
            google["questions"]["done"]["criteria"]["false_description"],
            QUESTIONS[0].no
        );
    }

    #[test]
    fn an_answer_is_held_under_the_packet_it_was_given_for() {
        let ws = WorkspaceId(uuid::Uuid::nil());
        let own = AdvisorKeySource::Workspace;
        let hello = json!({ "result": { "said": "hello" } });
        let jev = AdvisorProvider::Typesafe;
        let key = |value: &str, source| ResolvedKey {
            value: value.into(),
            source,
        };
        let mine = key("key-a", own);
        let a = held_as(ws, jev, "jev-latest", &mine, &hello);
        let b = held_as(
            ws,
            jev,
            "jev-latest",
            &mine,
            &json!({ "result": { "said": "hello!" } }),
        );
        let c = held_as(ws, jev, "jev-1.13.0", &mine, &hello);
        let d = held_as(
            ws,
            jev,
            "jev-latest",
            &key("key-a", AdvisorKeySource::Instance),
            &hello,
        );
        let e = held_as(ws, AdvisorProvider::Openai, "jev-latest", &mine, &hello);
        let f = held_as(ws, jev, "jev-latest", &key("key-b", own), &hello);
        assert_eq!(
            a,
            held_as(ws, jev, "jev-latest", &key("key-a", own), &hello)
        );
        assert_ne!(a, b, "a changed result asks again");
        assert_ne!(a, c, "so does another model");
        assert_ne!(a, d, "and an answer one key paid for is not another's");
        assert_ne!(a, e, "nor is one provider's answer another's");
        assert_ne!(a, f, "a replaced key asks with the new one");
    }

    /// Every question answered `p`, as `provider` writes it.
    fn answered(provider: AdvisorProvider, p: f64) -> Value {
        match provider {
            AdvisorProvider::Typesafe => json!({
                "model": "jev-1.14.0",
                "answers": QUESTIONS.iter().map(|q| (q.id.to_string(), json!({ "type": "noul", "noul": p }))).collect::<Map<_, _>>(),
                "usage": { "input_tokens": 1100, "output_tokens": 0 },
            }),
            AdvisorProvider::Openai => json!({
                "model": "gpt-6-luna",
                "answers": QUESTIONS.iter().map(|q| json!({ "type": "predicate", "name": q.id, "probability": p })).collect::<Vec<_>>(),
                "usage": { "input_tokens": 1200, "output_tokens": 0, "total_tokens": 1200 },
            }),
            AdvisorProvider::Google => json!({
                "answers": QUESTIONS.iter().map(|q| (q.id.to_string(), json!({ "type": "noul", "noul": p, "confidence": p }))).collect::<Map<_, _>>(),
                "usage": { "total_tokens": 1300, "total_input_tokens": 1300, "total_output_tokens": 0 },
            }),
        }
    }

    #[test]
    fn each_providers_answer_is_read_into_the_same_probabilities() {
        for (provider, tokens, model) in [
            (AdvisorProvider::Typesafe, 1100, Some("jev-1.14.0")),
            (AdvisorProvider::Openai, 1200, Some("gpt-6-luna")),
            // Google names no model in its answer: the one asked is reported.
            (AdvisorProvider::Google, 1300, None),
        ] {
            let read = answers(provider, &answered(provider, 0.4)).unwrap();
            assert_eq!(read.probabilities.len(), QUESTIONS.len(), "{provider:?}");
            assert!(
                read.probabilities.values().all(|p| *p == 0.4),
                "{provider:?}"
            );
            assert_eq!(read.usage.input_tokens, tokens, "{provider:?}");
            assert_eq!(read.model.as_deref(), model, "{provider:?}");
        }
    }

    #[test]
    fn an_answer_that_is_not_a_probability_is_refused() {
        for provider in AdvisorProvider::ALL {
            assert!(
                answers(provider, &answered(provider, 1.4)).is_err(),
                "{provider:?}"
            );
            assert!(
                answers(provider, &json!({ "answers": {} })).is_err(),
                "{provider:?}"
            );
        }
        // The wrong kind of answer, or none where a number belongs.
        let mut jev = answered(AdvisorProvider::Typesafe, 0.4);
        jev["answers"]["done"] = json!({ "type": "choice", "noul": 0.4 });
        assert!(answers(AdvisorProvider::Typesafe, &jev).is_err());
        let mut google = answered(AdvisorProvider::Google, 0.4);
        google["answers"]["done"] = json!({ "type": "noul" });
        assert!(answers(AdvisorProvider::Google, &google).is_err());
        let mut openai = answered(AdvisorProvider::Openai, 0.4);
        openai["answers"][0] = json!({ "type": "score", "name": "done", "score": 1 });
        assert!(answers(AdvisorProvider::Openai, &openai).is_err());
        // A question missing from the array is an answer missing.
        let mut short = answered(AdvisorProvider::Openai, 0.4);
        short["answers"].as_array_mut().unwrap().remove(0);
        assert!(answers(AdvisorProvider::Openai, &short).is_err());
    }

    #[test]
    fn a_question_a_provider_declined_is_unsettled_and_never_green() {
        // Everything clear, but the model would not say whether data is deleted.
        let clear = |q: &Question| match q.id {
            "done" | "tests_passed" | "code_change" => 0.97,
            _ => 0.02,
        };
        let mut response = json!({
            "model": "gpt-6-luna",
            "answers": QUESTIONS.iter().map(|q| json!({ "type": "predicate", "name": q.id, "probability": clear(q) })).collect::<Vec<_>>(),
            "usage": { "input_tokens": 1, "output_tokens": 0 },
        });
        let all = answers(AdvisorProvider::Openai, &response).unwrap();
        assert_eq!(decide(&all.probabilities).0, LandColor::Green);
        let at = QUESTIONS
            .iter()
            .position(|q| q.id == "deletes_data")
            .unwrap();
        response["answers"][at] = json!({ "type": "refusal", "name": "deletes_data" });
        let declined = answers(AdvisorProvider::Openai, &response).unwrap();
        assert!(!declined.probabilities.contains_key("deletes_data"));
        assert_ne!(decide(&declined.probabilities).0, LandColor::Green);
    }
}

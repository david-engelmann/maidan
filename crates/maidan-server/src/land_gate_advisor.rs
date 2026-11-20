//! Advice at the land gate from a decision model (TypeSafe's Jev).
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

const DEFAULT_BASE_URL: &str = "https://api.typesafe.ai/";
const DEFAULT_MODEL: &str = "jev-latest";
const DEFAULT_TIMEOUT_MS: u64 = 1_500;

/// The workspace secret that holds a workspace's own TypeSafe key.
pub const WORKSPACE_KEY_SECRET: &str = "typesafe_api_key";

/// At or above this a judgment reads as yes.
pub const YES_MIN: f64 = 0.7;
/// At or below this a judgment reads as no. Between the two it is unsettled,
/// and an unsettled judgment never counts toward green.
pub const NO_MAX: f64 = 0.3;

/// How much of the hand-off leaves for the provider.
const MAX_AGENT_MESSAGES: usize = 10;
const MAX_MESSAGE_CHARS: usize = 2_000;
const MAX_RESULT_STRING_CHARS: usize = 6_000;
const MESSAGE_SCAN: i64 = 200;
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
const NO_TESTS_NOT_CALLED_FOR: &str = "Reports no tests, which this task does not call for";
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
    /// The workspace's own key, stored as its `typesafe_api_key` secret.
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

/// What a workspace has set up, for `GET /workspaces/{id}/land-gate/advisor`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LandGateAdvisorStatus {
    /// Whether this instance offers advice at all.
    pub enabled: bool,
    /// Whose key a call from this workspace would use; absent when there is
    /// none, and then advice is refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_source: Option<AdvisorKeySource>,
    /// The provider, model and host the hand-off is sent to, when enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
}

impl LandGateAdvisorStatus {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            key_source: None,
            provider: None,
            model: None,
            host: None,
        }
    }
}

struct Settings {
    /// The instance's own key. Without one, only a workspace that stored its
    /// own key gets advice.
    api_key: Option<String>,
    base_url: String,
    model: String,
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
        let api_key = get("TYPESAFE_API_KEY")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
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
        let model = get("MAIDAN_JEV_MODEL")
            .unwrap_or_else(|| DEFAULT_MODEL.into())
            .trim()
            .to_string();
        if model.is_empty() {
            return Err(LandGateAdvisorError::Config(
                "MAIDAN_JEV_MODEL must not be empty".into(),
            ));
        }
        Ok(Some(Self {
            api_key,
            base_url: get("MAIDAN_JEV_BASE_URL").unwrap_or_else(|| DEFAULT_BASE_URL.into()),
            model,
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

/// How the advisor reaches the provider.
enum Transport {
    /// Through the egress guard: the name is resolved, checked and pinned,
    /// and looked up again every [`RESOLVE_TTL`]. Resolving on first use, not
    /// at boot, keeps an unreachable provider from stopping the server: only
    /// the advice call fails.
    Guarded(tokio::sync::Mutex<Option<(Instant, Client)>>),
    /// A client the caller made, for tests against a loopback stand-in.
    Fixed(Client),
}

/// The advisor: the instance's settings, a client for the provider and a
/// short-lived hold of answers, so a sheet opened twice asks once.
pub struct LandGateAdvisor {
    transport: Transport,
    base_url: Url,
    endpoint: Url,
    instance_key: Option<String>,
    model: String,
    timeout: Duration,
    held: Mutex<HashMap<String, Held>>,
}

impl LandGateAdvisor {
    fn from_settings(settings: Settings) -> Result<Self, LandGateAdvisorError> {
        let base_url: Url = settings.base_url.trim().parse().map_err(|error| {
            LandGateAdvisorError::Config(format!("invalid MAIDAN_JEV_BASE_URL: {error}"))
        })?;
        if !matches!(base_url.scheme(), "https" | "http")
            || base_url.host_str().is_none()
            || base_url.path() != "/"
            || base_url.query().is_some()
            || base_url.fragment().is_some()
            || !base_url.username().is_empty()
            || base_url.password().is_some()
        {
            return Err(LandGateAdvisorError::Config(
                "MAIDAN_JEV_BASE_URL must be an origin URL without a path, query, or fragment"
                    .into(),
            ));
        }
        Self::build(
            Transport::Guarded(tokio::sync::Mutex::new(None)),
            base_url,
            settings.api_key,
            settings.model,
            settings.timeout,
        )
    }

    /// Build an advisor on a client the caller made: the constructor tests and
    /// test servers use, which reach a stand-in on a loopback address.
    pub fn with_http(
        client: Client,
        base_url: Url,
        instance_key: Option<String>,
        model: impl Into<String>,
        timeout: Duration,
    ) -> Result<Self, LandGateAdvisorError> {
        Self::build(
            Transport::Fixed(client),
            base_url,
            instance_key,
            model.into(),
            timeout,
        )
    }

    fn build(
        transport: Transport,
        base_url: Url,
        instance_key: Option<String>,
        model: String,
        timeout: Duration,
    ) -> Result<Self, LandGateAdvisorError> {
        let endpoint = base_url.join("v1/systemone").map_err(|error| {
            LandGateAdvisorError::Config(format!("invalid MAIDAN_JEV_BASE_URL: {error}"))
        })?;
        Ok(Self {
            transport,
            base_url,
            endpoint,
            instance_key,
            model,
            timeout,
            held: Mutex::new(HashMap::new()),
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
            .get_secret_ciphertext(workspace_id, WORKSPACE_KEY_SECRET)
            .await?;
        Ok(match (own, &self.instance_key) {
            (Some(_), _) => Some(AdvisorKeySource::Workspace),
            (None, Some(_)) => Some(AdvisorKeySource::Instance),
            (None, None) => None,
        })
    }

    /// What `workspace_id` has set up.
    pub async fn status(
        &self,
        store: &dyn Store,
        workspace_id: WorkspaceId,
    ) -> Result<LandGateAdvisorStatus, LandGateAdvisorError> {
        Ok(LandGateAdvisorStatus {
            enabled: true,
            key_source: self.key_source(store, workspace_id).await?,
            provider: Some("typesafe".into()),
            model: Some(self.model.clone()),
            host: Some(self.host()),
        })
    }

    /// The workspace's own key when it stored one, else the instance's.
    async fn resolve_key(
        &self,
        store: &dyn Store,
        encryption_key: Option<&[u8; 32]>,
        workspace_id: WorkspaceId,
    ) -> Result<ResolvedKey, LandGateAdvisorError> {
        if let Some(ciphertext) = store
            .get_secret_ciphertext(workspace_id, WORKSPACE_KEY_SECRET)
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
            None => Err(LandGateAdvisorError::NotReady(
                "this workspace has no TypeSafe key: store one with PUT /workspaces/{id}/land-gate/advisor"
                    .into(),
            )),
        }
    }

    /// Ask the provider whether it accepts `api_key`, before a workspace
    /// stores it. A refusal is the caller's mistake; anything else is ours.
    pub async fn check_key(&self, api_key: &str) -> Result<(), LandGateAdvisorError> {
        let models = self.base_url.join("v1/models").map_err(|error| {
            LandGateAdvisorError::Config(format!("invalid MAIDAN_JEV_BASE_URL: {error}"))
        })?;
        let response = self
            .client()
            .await?
            .get(models)
            .bearer_auth(api_key)
            .timeout(self.timeout.max(Duration::from_secs(5)))
            .send()
            .await
            .map_err(|error| LandGateAdvisorError::Unavailable(error.to_string()))?;
        let status = response.status();
        if status.is_success() {
            Ok(())
        } else if status == reqwest::StatusCode::UNAUTHORIZED
            || status == reqwest::StatusCode::FORBIDDEN
        {
            Err(LandGateAdvisorError::InvalidRequest(
                "TypeSafe refused this key".into(),
            ))
        } else {
            Err(LandGateAdvisorError::Unavailable(format!(
                "provider returned {status}"
            )))
        }
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
        let state = packet(store, thread_id).await?;
        let key = self
            .resolve_key(store, encryption_key, workspace_id)
            .await?;
        let held_as = held_as(workspace_id, &self.model, &state);
        if !request.refresh
            && let Some(mut advice) = self.held(&held_as)
        {
            advice.cached = true;
            advice.key_source = key.source;
            return Ok(advice);
        }
        let advice = self.ask(&state, &key).await?;
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

    async fn ask(
        &self,
        state: &Value,
        key: &ResolvedKey,
    ) -> Result<LandGateAdvice, LandGateAdvisorError> {
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
        let body = json!({ "model": self.model, "state": state, "questions": questions });
        let started = Instant::now();
        let response = self
            .client()
            .await?
            .post(self.endpoint.clone())
            .bearer_auth(&key.value)
            .timeout(self.timeout)
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
        let response: ProviderResponse = response
            .json()
            .await
            .map_err(|error| LandGateAdvisorError::InvalidResponse(error.to_string()))?;
        let latency_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
        let mut probabilities = BTreeMap::new();
        for question in &QUESTIONS {
            let answer = response.answers.get(question.id).ok_or_else(|| {
                LandGateAdvisorError::InvalidResponse(format!(
                    "response omitted `answers.{}`",
                    question.id
                ))
            })?;
            probabilities.insert(question.id, answer.probability(question.id)?);
        }
        let (land, reasons) = decide(&probabilities);
        let judgments = QUESTIONS
            .iter()
            .map(|question| {
                let probability = probabilities.get(question.id).copied().unwrap_or(0.5);
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
            provider: "typesafe".into(),
            model: response.model,
            land,
            reasons: reasons.into_iter().map(str::to_string).collect(),
            judgments,
            thresholds: LandGateAdviceThresholds {
                yes_min: YES_MIN,
                no_max: NO_MAX,
            },
            key_source: key.source,
            cached: false,
            usage: response.usage,
            latency_ms,
        })
    }
}

#[derive(Deserialize)]
struct ProviderResponse {
    model: String,
    answers: BTreeMap<String, ProviderAnswer>,
    usage: LandGateAdviceUsage,
}

#[derive(Deserialize)]
struct ProviderAnswer {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    noul: Option<f64>,
}

impl ProviderAnswer {
    fn probability(&self, id: &str) -> Result<f64, LandGateAdvisorError> {
        if self.kind != "noul" {
            return Err(LandGateAdvisorError::InvalidResponse(format!(
                "`answers.{id}` was not a noul"
            )));
        }
        match self.noul {
            Some(p) if p.is_finite() && (0.0..=1.0).contains(&p) => Ok(p),
            _ => Err(LandGateAdvisorError::InvalidResponse(format!(
                "`answers.{id}.noul` was outside [0, 1]"
            ))),
        }
    }
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

/// The packet the model reads: what the task asks for, the result, and what
/// the agent that produced it said on the thread. Built here so every call
/// sends the same shape and nothing a caller chose.
///
/// The task's opening message is left out on evidence: it tells the agent how
/// to work (claim, say what you ran, send to review), a hand-off cannot show
/// those steps, and with it "done" went unsettled on a quarter of the
/// labelled hand-offs.
async fn packet(store: &dyn Store, thread_id: ThreadId) -> Result<Value, LandGateAdvisorError> {
    let thread = store.get_thread(thread_id).await?;
    let result = store
        .get_thread_result(thread_id)
        .await?
        .ok_or_else(|| LandGateAdvisorError::NotReady("this task has no result yet".into()))?;
    let mut task = Map::new();
    task.insert(
        "title".into(),
        Value::String(thread.title.unwrap_or_default()),
    );
    if let Some(description) = thread.description.filter(|d| !d.trim().is_empty()) {
        task.insert(
            "description".into(),
            Value::String(clip(&description, MAX_RESULT_STRING_CHARS)),
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
    let mut state = Map::new();
    state.insert("task".into(), Value::Object(task));
    state.insert("result".into(), trimmed_result(&result.result));
    let messages = store.list_messages(thread_id, MESSAGE_SCAN).await?;
    let mut said: Vec<(chrono::DateTime<chrono::Utc>, String)> = messages
        .into_iter()
        .filter(|m| m.author_id == result.produced_by && m.tombstoned_at.is_none())
        .filter(|m| !m.body.trim().is_empty())
        .map(|m| (m.posted_at, clip(&m.body, MAX_MESSAGE_CHARS)))
        .collect();
    said.sort_by_key(|(at, _)| *at);
    let skip = said.len().saturating_sub(MAX_AGENT_MESSAGES);
    let said: Vec<Value> = said
        .into_iter()
        .skip(skip)
        .map(|(_, body)| Value::String(body))
        .collect();
    // An empty list reads as a signal, so the field is left out instead.
    if !said.is_empty() {
        state.insert("agent_messages".into(), Value::Array(said));
    }
    Ok(Value::Object(state))
}

/// The result without the fields that carry bytes for the change flow, and
/// with any very long string cut.
fn trimmed_result(result: &Value) -> Value {
    match result {
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .filter(|(name, _)| !RESULT_FIELDS_LEFT_OUT.contains(&name.as_str()))
                .map(|(name, value)| (name.clone(), clipped(value)))
                .collect(),
        ),
        other => clipped(other),
    }
}

fn clipped(value: &Value) -> Value {
    match value {
        Value::String(text) => Value::String(clip(text, MAX_RESULT_STRING_CHARS)),
        Value::Array(items) => Value::Array(items.iter().map(clipped).collect()),
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(name, value)| (name.clone(), clipped(value)))
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

/// What an answer is held under: the workspace, the model and the packet's
/// bytes, so a changed result or a new message asks again.
fn held_as(workspace_id: WorkspaceId, model: &str, state: &Value) -> String {
    let mut hasher = Sha256::new();
    hasher.update(workspace_id.0.as_bytes());
    hasher.update(model.as_bytes());
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
    ) -> Result<Value, maidan_mcp::LandGateAdviseError> {
        let advice = self
            .advisor
            .advise(
                self.store.as_ref(),
                self.encryption_key.as_ref(),
                workspace_id,
                thread_id,
                LandGateAdviceRequest { refresh },
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
        assert!(on.api_key.is_none(), "workspaces bring their own");
        assert_eq!(on.model, DEFAULT_MODEL);
        let keyed = Settings::from_lookup(lookup(&[
            ("MAIDAN_JEV_LAND_GATE_ENABLED", "1"),
            ("TYPESAFE_API_KEY", " secret "),
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(keyed.api_key.as_deref(), Some("secret"));
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
        assert_eq!(advisor.host(), "provider.invalid");
        assert_eq!(
            advisor.endpoint.as_str(),
            "https://provider.invalid/v1/systemone"
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
        let long = "x".repeat(MAX_RESULT_STRING_CHARS + 10);
        let trimmed = trimmed_result(&json!({
            "summary": "Renamed the module.",
            "diff": "diff --git a/x b/x",
            "deliver_to": { "repo": "example/app" },
            "base_sha": "abc",
            "notes": [long],
        }));
        assert_eq!(trimmed["summary"], "Renamed the module.");
        assert!(trimmed.get("diff").is_none());
        assert!(trimmed.get("deliver_to").is_none());
        assert!(trimmed.get("base_sha").is_none());
        let note = trimmed["notes"][0].as_str().unwrap();
        assert!(note.ends_with("[cut here: the rest was not sent]"));
        assert!(note.chars().count() < MAX_RESULT_STRING_CHARS + 60);
        // A result that is not an object is sent as it is.
        assert_eq!(trimmed_result(&json!("done")), json!("done"));
    }

    #[test]
    fn an_answer_is_held_under_the_packet_it_was_given_for() {
        let ws = WorkspaceId(uuid::Uuid::nil());
        let a = held_as(ws, "jev-latest", &json!({ "result": { "said": "hello" } }));
        let b = held_as(ws, "jev-latest", &json!({ "result": { "said": "hello!" } }));
        let c = held_as(ws, "jev-1.13.0", &json!({ "result": { "said": "hello" } }));
        assert_ne!(a, b, "a changed result asks again");
        assert_ne!(a, c, "so does another model");
    }

    #[test]
    fn an_answer_that_is_not_a_probability_is_refused() {
        let ok = ProviderAnswer {
            kind: "noul".into(),
            noul: Some(0.4),
        };
        assert_eq!(ok.probability("done").unwrap(), 0.4);
        for bad in [
            ProviderAnswer {
                kind: "choice".into(),
                noul: Some(0.4),
            },
            ProviderAnswer {
                kind: "noul".into(),
                noul: Some(1.4),
            },
            ProviderAnswer {
                kind: "noul".into(),
                noul: None,
            },
        ] {
            assert!(bad.probability("done").is_err());
        }
    }
}

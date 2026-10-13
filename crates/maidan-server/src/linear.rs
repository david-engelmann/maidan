//! Linear result delivery: a result comments on the Linear issue it was
//! routed to, and may move that issue to a workflow state of its team.
//!
//! Outbound only. The operator holds one API key (`MAIDAN_LINEAR_API_KEY`) and
//! blesses teams by id in the workspace egress allowlist; a producer's
//! `deliver_to` names the team, the issue and an optional state. The worker
//! checks at send time that the issue is in the team the producer named,
//! because the allowlist authorized that team, not the issue.

use serde_json::{json, Value};

/// Linear's GraphQL API.
pub const LINEAR_API_URL: &str = "https://api.linear.app/graphql";

/// An issue as delivery needs it: where it lives and where it can move.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinearIssue {
    pub id: String,
    pub identifier: String,
    pub team_id: String,
    pub states: Vec<LinearState>,
}

impl LinearIssue {
    /// The team's workflow state with this exact name, if there is one.
    pub fn state_named(&self, name: &str) -> Option<&LinearState> {
        self.states.iter().find(|state| state.name == name)
    }
}

/// One workflow state of a team.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinearState {
    pub id: String,
    pub name: String,
}

#[derive(Debug, thiserror::Error)]
pub enum LinearError {
    #[error("linear http error: {0}")]
    Http(String),
    /// A GraphQL error, with Linear's `extensions.code` when it sent one.
    #[error("linear api error ({code}): {message}")]
    Api { code: String, message: String },
    /// The issue or comment does not exist, or the key cannot see it.
    #[error("linear: {0} not found")]
    NotFound(String),
}

/// Linear's error codes that no retry fixes: the key, its permissions, or a
/// request Linear refuses as invalid. Rate limits and server errors retry.
const LINEAR_MISCONFIGURATION_CODES: &[&str] = &[
    "AUTHENTICATION_ERROR",
    "FORBIDDEN",
    "INVALID_INPUT",
    "GRAPHQL_VALIDATION_FAILED",
];

impl LinearError {
    /// Whether this failure is a misconfiguration rather than a transient fault.
    pub fn is_misconfiguration(&self) -> bool {
        match self {
            Self::Http(_) => false,
            Self::Api { code, .. } => LINEAR_MISCONFIGURATION_CODES.contains(&code.as_str()),
            Self::NotFound(_) => true,
        }
    }
}

/// What delivery needs from Linear. A trait so tests drive the worker with a
/// fake.
#[async_trait::async_trait]
pub trait LinearSender: Send + Sync {
    /// The issue, by identifier (`ENG-123`) or id, with its team's states.
    async fn issue(&self, issue: &str) -> Result<LinearIssue, LinearError>;

    /// Comment on an issue (by id), returning the comment's id.
    async fn create_comment(&self, issue_id: &str, body: &str) -> Result<String, LinearError>;

    /// Replace a comment's body.
    async fn update_comment(&self, comment_id: &str, body: &str) -> Result<(), LinearError>;

    /// Move an issue (by id) to a workflow state (by id).
    async fn move_issue(&self, issue_id: &str, state_id: &str) -> Result<(), LinearError>;

    /// The host this sender posts to: its key in the shared retry budget.
    fn host(&self) -> String {
        "api.linear.app".to_string()
    }
}

/// The production [`LinearSender`], over Linear's GraphQL API.
pub struct LinearClient {
    api_key: String,
    /// `https://api.linear.app/graphql` in production; overridable so the wire
    /// path can be tested against a loopback server.
    url: String,
    http: reqwest::Client,
}

impl LinearClient {
    pub fn new(api_key: String) -> Self {
        Self::with_url(api_key, LINEAR_API_URL.to_string())
    }

    pub fn with_url(api_key: String, url: String) -> Self {
        Self {
            api_key,
            url,
            // `build` fails only where `Client::new` would panic: no TLS backend.
            http: crate::egress_http::bounded()
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        }
    }

    /// The client when `MAIDAN_LINEAR_API_KEY` is set, else `None` (Linear
    /// delivery off).
    pub fn from_env() -> Option<Self> {
        std::env::var("MAIDAN_LINEAR_API_KEY")
            .ok()
            .filter(|key| !key.is_empty())
            .map(Self::new)
    }

    /// Run one GraphQL operation and return its `data`. Linear answers most
    /// errors with HTTP 200 and an `errors` array, so the body decides.
    async fn call(&self, query: &str, variables: Value) -> Result<Value, LinearError> {
        let resp = crate::trace_context::stamp(self.http.post(&self.url))
            // A personal API key goes bare; an OAuth access token is a bearer.
            .header(
                reqwest::header::AUTHORIZATION,
                if self.api_key.starts_with("lin_api_") {
                    self.api_key.clone()
                } else {
                    format!("Bearer {}", self.api_key)
                },
            )
            .json(&json!({ "query": query, "variables": variables }))
            .send()
            .await
            .map_err(|e| LinearError::Http(e.to_string()))?;
        let status = resp.status();
        let body: Value = resp
            .json()
            .await
            .map_err(|e| LinearError::Http(format!("{status}: {e}")))?;
        if let Some(error) = body
            .get("errors")
            .and_then(Value::as_array)
            .and_then(|errors| errors.first())
        {
            return Err(api_error(error));
        }
        if !status.is_success() {
            return Err(LinearError::Http(status.to_string()));
        }
        body.get("data")
            .cloned()
            .ok_or_else(|| LinearError::Http("response without data".into()))
    }
}

fn api_error(error: &Value) -> LinearError {
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("unknown error")
        .to_string();
    let code = error
        .pointer("/extensions/code")
        .and_then(Value::as_str)
        .unwrap_or("UNKNOWN")
        .to_string();
    // Linear reports a missing entity as an invalid-input error whose message
    // says so; it is the one invalid input a caller has to tell apart.
    if message.to_ascii_lowercase().contains("not found") {
        return LinearError::NotFound(message);
    }
    LinearError::Api { code, message }
}

fn str_at<'a>(value: &'a Value, pointer: &str) -> Option<&'a str> {
    value.pointer(pointer).and_then(Value::as_str)
}

#[async_trait::async_trait]
impl LinearSender for LinearClient {
    fn host(&self) -> String {
        crate::retry_budget::host_of(&self.url).unwrap_or_else(|| "api.linear.app".to_string())
    }

    async fn issue(&self, issue: &str) -> Result<LinearIssue, LinearError> {
        let data = self
            .call(
                "query MaidanIssue($id: String!) { issue(id: $id) { id identifier team { id states { nodes { id name } } } } }",
                json!({ "id": issue }),
            )
            .await?;
        let found = data
            .get("issue")
            .filter(|v| !v.is_null())
            .ok_or_else(|| LinearError::NotFound(format!("issue {issue}")))?;
        let field = |pointer: &str| {
            str_at(found, pointer)
                .map(str::to_string)
                .ok_or_else(|| LinearError::Http(format!("issue without {pointer}")))
        };
        let states = found
            .pointer("/team/states/nodes")
            .and_then(Value::as_array)
            .map(|nodes| {
                nodes
                    .iter()
                    .filter_map(|node| {
                        Some(LinearState {
                            id: str_at(node, "/id")?.to_string(),
                            name: str_at(node, "/name")?.to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(LinearIssue {
            id: field("/id")?,
            identifier: field("/identifier")?,
            team_id: field("/team/id")?,
            states,
        })
    }

    async fn create_comment(&self, issue_id: &str, body: &str) -> Result<String, LinearError> {
        let data = self
            .call(
                "mutation MaidanComment($input: CommentCreateInput!) { commentCreate(input: $input) { success comment { id } } }",
                json!({ "input": { "issueId": issue_id, "body": body } }),
            )
            .await?;
        str_at(&data, "/commentCreate/comment/id")
            .map(str::to_string)
            .ok_or_else(|| LinearError::Http("commentCreate returned no comment".into()))
    }

    async fn update_comment(&self, comment_id: &str, body: &str) -> Result<(), LinearError> {
        let data = self
            .call(
                "mutation MaidanCommentUpdate($id: String!, $input: CommentUpdateInput!) { commentUpdate(id: $id, input: $input) { success } }",
                json!({ "id": comment_id, "input": { "body": body } }),
            )
            .await?;
        succeeded(&data, "/commentUpdate/success")
    }

    async fn move_issue(&self, issue_id: &str, state_id: &str) -> Result<(), LinearError> {
        let data = self
            .call(
                "mutation MaidanIssueState($id: String!, $input: IssueUpdateInput!) { issueUpdate(id: $id, input: $input) { success } }",
                json!({ "id": issue_id, "input": { "stateId": state_id } }),
            )
            .await?;
        succeeded(&data, "/issueUpdate/success")
    }
}

fn succeeded(data: &Value, pointer: &str) -> Result<(), LinearError> {
    match data.pointer(pointer).and_then(Value::as_bool) {
        Some(true) => Ok(()),
        _ => Err(LinearError::Api {
            code: "UNSUCCESSFUL".into(),
            message: format!("{pointer} was not true"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_entity_is_not_found_and_final() {
        let err = api_error(&json!({
            "message": "Entity not found: Issue",
            "extensions": { "code": "INVALID_INPUT" }
        }));
        assert!(matches!(err, LinearError::NotFound(_)), "{err}");
        assert!(err.is_misconfiguration());
    }

    #[test]
    fn a_rate_limit_retries_and_a_bad_key_does_not() {
        let limited = api_error(&json!({
            "message": "Rate limit exceeded",
            "extensions": { "code": "RATELIMITED" }
        }));
        assert!(!limited.is_misconfiguration());
        let denied = api_error(&json!({
            "message": "Authentication required",
            "extensions": { "code": "AUTHENTICATION_ERROR" }
        }));
        assert!(denied.is_misconfiguration());
        assert!(!LinearError::Http("502".into()).is_misconfiguration());
    }
}

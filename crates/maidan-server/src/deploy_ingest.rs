//! Deploy push-ingest: `POST /integrations/deploys/events`.
//!
//! A deploy pipeline tells Maidan a commit reached an environment, and each
//! merged pull request whose merge (or merged head) commit that is hears it
//! on its thread as a `deployed` notice. Opt-in and inert without a hook:
//! the route is a 404 until `MAIDAN_DEPLOY_HOOK_SECRET` is set.
//!
//! The caller signs the body as GitHub signs its webhooks
//! (`X-Maidan-Signature-256: sha256=<hex HMAC-SHA256(secret, body)>`), so
//! the same verifier serves both. There is no replay window: a replayed
//! event is the same deploy and outcome, which the store says only once.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;

use crate::error::ApiError;
use crate::extract::ApiText;
use crate::state::AppState;

/// The header that carries the body's signature.
pub const SIGNATURE_HEADER: &str = "x-maidan-signature-256";

/// The longest environment name kept; a longer one is refused, not cut.
const ENVIRONMENT_LIMIT: usize = 64;
/// The longest deploy URL kept.
const URL_LIMIT: usize = 2048;

/// The deploy hook's secret, from `MAIDAN_DEPLOY_HOOK_SECRET`.
#[derive(Clone)]
pub struct DeployHookConfig {
    secret: String,
}

// The secret is a credential; `{:?}` must never print it.
impl std::fmt::Debug for DeployHookConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeployHookConfig")
            .field("secret", &"[redacted]")
            .finish()
    }
}

impl DeployHookConfig {
    /// `None` when `MAIDAN_DEPLOY_HOOK_SECRET` is unset or blank: the route
    /// then answers 404.
    pub fn from_env() -> Option<Self> {
        Self::from_secret(std::env::var("MAIDAN_DEPLOY_HOOK_SECRET").ok())
    }

    pub fn from_secret(secret: Option<String>) -> Option<Self> {
        let secret = secret?.trim().to_string();
        (!secret.is_empty()).then_some(Self { secret })
    }
}

/// What the hook sends.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeployEventBody {
    /// `owner/name`.
    repo: String,
    /// The full 40-character commit SHA that was deployed.
    sha: String,
    environment: String,
    /// `success` or `failure`. Anything else (`in_progress`, `queued`) is
    /// acknowledged and says nothing: only an outcome is news on a thread.
    status: String,
    #[serde(default)]
    url: Option<String>,
}

/// Check the body and turn it into a deploy; `Ok(None)` for a status that is
/// not an outcome yet.
fn parse_deploy(body: DeployEventBody) -> Result<Option<maidan_types::NewDeploy>, String> {
    let repo = body.repo.trim().to_ascii_lowercase();
    let well_formed_repo = repo
        .split_once('/')
        .is_some_and(|(owner, name)| !owner.is_empty() && !name.is_empty() && !name.contains('/'));
    if !well_formed_repo {
        return Err("repo must be owner/name".into());
    }
    let sha = body.sha.trim().to_ascii_lowercase();
    // A short SHA could name more than one commit; only the full one matches
    // a merge exactly.
    if sha.len() != 40 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("sha must be the full 40-character commit SHA".into());
    }
    let environment = body.environment.trim().to_string();
    if environment.is_empty() || environment.len() > ENVIRONMENT_LIMIT {
        return Err(format!(
            "environment must be 1 to {ENVIRONMENT_LIMIT} characters"
        ));
    }
    let url = match body.url.map(|url| url.trim().to_string()) {
        Some(url) if url.is_empty() => None,
        Some(url) if url.len() > URL_LIMIT || !url.starts_with("https://") => {
            return Err(format!(
                "url must be an https:// URL of at most {URL_LIMIT} characters"
            ));
        }
        other => other,
    };
    let status = match body.status.trim() {
        "success" => maidan_types::CiStatus::Success,
        "failure" | "error" => maidan_types::CiStatus::Failure,
        _ => return Ok(None),
    };
    Ok(Some(maidan_types::NewDeploy {
        repo,
        sha,
        environment,
        status,
        url,
        contains: Vec::new(),
    }))
}

/// How many merged pull requests a deploy checks for ancestry, newest first:
/// one compare call each, so a burst of deploys stays well inside the
/// GitHub rate limit.
const ANCESTOR_CHECKS: i64 = 20;

/// The merged pull requests, each in the workspace that tracks it, whose
/// merge commit `deploy.sha` carries as an ancestor: a deploy of a later
/// commit shipped them too. Needs a GitHub client to compare commits; with
/// none, or on a failed compare, a deploy matches its exact commit only.
async fn shipped_ancestors(
    state: &AppState,
    deploy: &maidan_types::NewDeploy,
) -> Vec<maidan_types::DeployedPull> {
    let Some(github) = state
        .github_sender
        .as_deref()
        .and_then(|sender| sender.git())
    else {
        return Vec::new();
    };
    let candidates = match state
        .store
        .deploy_candidates(&deploy.repo, &deploy.environment, ANCESTOR_CHECKS)
        .await
    {
        Ok(candidates) => candidates,
        Err(err) => {
            tracing::warn!(error = %err, "deploy ingest: candidates not read");
            return Vec::new();
        }
    };
    let mut shipped = Vec::new();
    for candidate in candidates {
        if candidate.merge_sha == deploy.sha {
            // The exact match is the store's own; no compare needed.
            continue;
        }
        // `compare/{deployed}...{merged}` is behind (or identical) exactly
        // when the merged commit is an ancestor of the deployed one.
        match github
            .commit_on_branch(&deploy.repo, &deploy.sha, &candidate.merge_sha)
            .await
        {
            Ok(true) => shipped.push(maidan_types::DeployedPull {
                workspace_id: candidate.workspace_id,
                pull_number: candidate.pull_number,
            }),
            Ok(false) => {}
            Err(err) => {
                tracing::warn!(error = %err, repo = %deploy.repo, "deploy ingest: compare failed");
            }
        }
    }
    shipped
}

/// `POST /integrations/deploys/events`: 404 without a hook, 401 on a bad
/// signature, 400 on a malformed body, else 200 with how many threads heard
/// of it (`noticed`).
pub async fn deploy_events(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiText(body): ApiText,
) -> Response {
    let Some(cfg) = state.deploy_hook.as_ref() else {
        return ApiError::NotFound.into_response();
    };
    let signature = headers
        .get(SIGNATURE_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !crate::webhooks::verify_signature(&cfg.secret, &body, signature) {
        return ApiError::SignatureInvalid.into_response();
    }
    let parsed: DeployEventBody = match serde_json::from_str(&body) {
        Ok(parsed) => parsed,
        Err(err) => {
            return ApiError::BadRequest(format!("invalid deploy event: {err}")).into_response()
        }
    };
    let mut deploy = match parse_deploy(parsed) {
        Ok(Some(deploy)) => deploy,
        Ok(None) => return Json(serde_json::json!({ "noticed": 0 })).into_response(),
        Err(msg) => return ApiError::BadRequest(msg).into_response(),
    };
    deploy.contains = shipped_ancestors(&state, &deploy).await;
    let posted = match state.store.record_deploy(deploy).await {
        Ok(posted) => posted,
        Err(err) => {
            tracing::warn!(error = %err, "deploy ingest: deploy not recorded");
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    };
    let noticed = posted.len();
    for notice in posted {
        crate::change_sweep::post_notice(&state, notice).await;
    }
    Json(serde_json::json!({ "noticed": noticed })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(status: &str) -> DeployEventBody {
        DeployEventBody {
            repo: "Acme/Widgets".into(),
            sha: "A".repeat(40),
            environment: "production".into(),
            status: status.into(),
            url: Some("https://deploys.example/1".into()),
        }
    }

    #[test]
    fn a_blank_secret_leaves_the_hook_off() {
        assert!(DeployHookConfig::from_secret(None).is_none());
        assert!(DeployHookConfig::from_secret(Some("  ".into())).is_none());
        assert!(DeployHookConfig::from_secret(Some("s3cret".into())).is_some());
    }

    #[test]
    fn the_debug_form_never_shows_the_secret() {
        let cfg = DeployHookConfig::from_secret(Some("s3cret".into()));
        assert!(!format!("{cfg:?}").contains("s3cret"));
    }

    #[test]
    fn an_outcome_is_a_deploy_and_a_running_deploy_is_not_yet() {
        let deploy = parse_deploy(body("success")).ok().flatten();
        assert_eq!(
            deploy.as_ref().map(|d| (d.repo.as_str(), d.sha.as_str())),
            Some(("acme/widgets", "a".repeat(40).as_str()))
        );
        assert_eq!(
            parse_deploy(body("error")).ok().flatten().map(|d| d.status),
            Some(maidan_types::CiStatus::Failure)
        );
        assert_eq!(parse_deploy(body("in_progress")), Ok(None));
    }

    #[test]
    fn a_short_sha_a_bad_repo_or_a_plain_http_url_is_refused() {
        let mut short = body("success");
        short.sha = "abc1234".into();
        assert!(parse_deploy(short).is_err());
        let mut repo = body("success");
        repo.repo = "widgets".into();
        assert!(parse_deploy(repo).is_err());
        let mut url = body("success");
        url.url = Some("http://deploys.example/1".into());
        assert!(parse_deploy(url).is_err());
        let mut environment = body("success");
        environment.environment = "x".repeat(65);
        assert!(parse_deploy(environment).is_err());
    }
}

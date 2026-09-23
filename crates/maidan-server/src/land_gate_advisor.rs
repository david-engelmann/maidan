//! Experimental, advisory-only TypeSafe Jev integration for land gates.
//!
//! The advisor is deliberately outside the authoritative land-gate write path:
//! callers opt into a separate request, inspect the answer, and an existing
//! qualified verifier still records the gate pointer. Provider failure can
//! therefore fail this request without weakening or blocking deterministic
//! gate enforcement.

use std::{collections::BTreeMap, sync::Arc, time::Duration};

use maidan_types::LandColor;
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use utoipa::ToSchema;

const DEFAULT_BASE_URL: &str = "https://api.typesafe.ai/";
const DEFAULT_MODEL: &str = "jev-latest";
const DEFAULT_TIMEOUT_MS: u64 = 1_500;
const DEFAULT_GREEN_MIN_CONFIDENCE: f64 = 0.90;
const DEFAULT_RED_MIN_CONFIDENCE: f64 = 0.90;

#[derive(Debug, Error)]
pub enum LandGateAdvisorError {
    #[error("invalid advisor configuration: {0}")]
    Config(String),
    #[error("invalid advice request: {0}")]
    InvalidRequest(String),
    #[error("advisor request failed: {0}")]
    Unavailable(String),
    #[error("advisor returned an invalid response: {0}")]
    InvalidResponse(String),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LandGateAdviceThresholds {
    pub green_min_confidence: f64,
    pub red_min_confidence: f64,
}

impl Default for LandGateAdviceThresholds {
    fn default() -> Self {
        Self {
            green_min_confidence: DEFAULT_GREEN_MIN_CONFIDENCE,
            red_min_confidence: DEFAULT_RED_MIN_CONFIDENCE,
        }
    }
}

impl LandGateAdviceThresholds {
    fn validate(self) -> Result<Self, String> {
        for (name, value) in [
            ("green_min_confidence", self.green_min_confidence),
            ("red_min_confidence", self.red_min_confidence),
        ] {
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                return Err(format!("{name} must be between 0 and 1"));
            }
        }
        Ok(self)
    }
}

/// A caller-supplied evaluation case. `state` is sent to TypeSafe only when
/// the feature is enabled and this endpoint is explicitly called.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LandGateAdviceRequest {
    pub state: Value,
    #[serde(default)]
    pub instructions: Option<String>,
    /// Per-evaluation override. This cannot authorize a land: the response is
    /// advisory and the existing qualified-verifier write remains mandatory.
    #[serde(default)]
    pub thresholds: Option<LandGateAdviceThresholds>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct LandGateAdviceUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LandGateAdvice {
    pub provider: String,
    pub model: String,
    /// The model's top choice before Maidan applies confidence policy.
    pub raw_land: LandColor,
    /// Low-confidence green/red choices become amber (human review).
    pub recommended_land: LandColor,
    pub confidence: f64,
    pub probabilities: BTreeMap<String, f64>,
    pub thresholds: LandGateAdviceThresholds,
    pub usage: LandGateAdviceUsage,
    pub latency_ms: u64,
}

#[async_trait::async_trait]
pub trait LandGateAdvisor: Send + Sync {
    async fn advise(
        &self,
        request: LandGateAdviceRequest,
    ) -> Result<LandGateAdvice, LandGateAdvisorError>;
}

struct Settings {
    api_key: String,
    base_url: String,
    model: String,
    timeout: Duration,
    thresholds: LandGateAdviceThresholds,
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

        fn parse_f64(
            get: &impl Fn(&str) -> Option<String>,
            name: &str,
            default: f64,
        ) -> Result<f64, LandGateAdvisorError> {
            match get(name) {
                Some(value) => value
                    .trim()
                    .parse()
                    .map_err(|_| LandGateAdvisorError::Config(format!("{name} must be a number"))),
                None => Ok(default),
            }
        }

        let api_key = get("TYPESAFE_API_KEY")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                LandGateAdvisorError::Config(
                    "TYPESAFE_API_KEY is required when MAIDAN_JEV_LAND_GATE_ENABLED=1".into(),
                )
            })?;
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
        let thresholds = LandGateAdviceThresholds {
            green_min_confidence: parse_f64(
                &get,
                "MAIDAN_JEV_GREEN_MIN_CONFIDENCE",
                DEFAULT_GREEN_MIN_CONFIDENCE,
            )?,
            red_min_confidence: parse_f64(
                &get,
                "MAIDAN_JEV_RED_MIN_CONFIDENCE",
                DEFAULT_RED_MIN_CONFIDENCE,
            )?,
        }
        .validate()
        .map_err(LandGateAdvisorError::Config)?;

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
            thresholds,
        }))
    }
}

pub struct TypeSafeLandGateAdvisor {
    client: Client,
    endpoint: Url,
    api_key: String,
    model: String,
    timeout: Duration,
    thresholds: LandGateAdviceThresholds,
}

impl TypeSafeLandGateAdvisor {
    async fn from_settings(settings: Settings) -> Result<Self, LandGateAdvisorError> {
        let (client, base_url) = crate::egress_http::client_for(&settings.base_url)
            .await
            .map_err(LandGateAdvisorError::Config)?;
        if base_url.path() != "/" || base_url.query().is_some() || base_url.fragment().is_some() {
            return Err(LandGateAdvisorError::Config(
                "MAIDAN_JEV_BASE_URL must be an origin URL without a path, query, or fragment"
                    .into(),
            ));
        }
        let endpoint = base_url.join("v1/systemone").map_err(|error| {
            LandGateAdvisorError::Config(format!("invalid MAIDAN_JEV_BASE_URL: {error}"))
        })?;
        Ok(Self {
            client,
            endpoint,
            api_key: settings.api_key,
            model: settings.model,
            timeout: settings.timeout,
            thresholds: settings.thresholds,
        })
    }

    #[cfg(test)]
    fn with_http(
        client: Client,
        endpoint: Url,
        api_key: impl Into<String>,
        model: impl Into<String>,
        timeout: Duration,
        thresholds: LandGateAdviceThresholds,
    ) -> Self {
        Self {
            client,
            endpoint,
            api_key: api_key.into(),
            model: model.into(),
            timeout,
            thresholds,
        }
    }
}

#[derive(Serialize)]
struct TypeSafeRequest<'a> {
    state: &'a Value,
    model: &'a str,
    questions: BTreeMap<&'static str, TypeSafeQuestion<'a>>,
}

#[derive(Serialize)]
struct TypeSafeQuestion<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    instructions: &'a str,
    criteria: BTreeMap<&'static str, &'static str>,
}

#[derive(Deserialize)]
struct TypeSafeResponse {
    model: String,
    answers: BTreeMap<String, TypeSafeAnswer>,
    usage: LandGateAdviceUsage,
}

#[derive(Deserialize)]
struct TypeSafeAnswer {
    #[serde(rename = "type")]
    kind: String,
    choice: String,
    confidence: f64,
    probabilities: BTreeMap<String, f64>,
}

fn parse_land(value: &str) -> Result<LandColor, LandGateAdvisorError> {
    LandColor::parse(value).ok_or_else(|| {
        LandGateAdvisorError::InvalidResponse(format!("unknown land choice `{value}`"))
    })
}

fn recommended_land(
    raw: LandColor,
    confidence: f64,
    thresholds: LandGateAdviceThresholds,
) -> LandColor {
    match raw {
        LandColor::Green if confidence < thresholds.green_min_confidence => LandColor::Amber,
        LandColor::Red if confidence < thresholds.red_min_confidence => LandColor::Amber,
        other => other,
    }
}

fn validate_answer(answer: &TypeSafeAnswer) -> Result<(), LandGateAdvisorError> {
    if answer.kind != "choice" {
        return Err(LandGateAdvisorError::InvalidResponse(
            "land answer was not a choice".into(),
        ));
    }
    if !answer.confidence.is_finite() || !(0.0..=1.0).contains(&answer.confidence) {
        return Err(LandGateAdvisorError::InvalidResponse(
            "confidence was outside [0, 1]".into(),
        ));
    }
    if answer.probabilities.len() != 3 {
        return Err(LandGateAdvisorError::InvalidResponse(
            "probabilities did not contain exactly green, amber, and red".into(),
        ));
    }
    let mut probability_sum = 0.0;
    for color in ["green", "amber", "red"] {
        let probability = answer.probabilities.get(color).ok_or_else(|| {
            LandGateAdvisorError::InvalidResponse(format!(
                "probabilities did not include `{color}`"
            ))
        })?;
        if !probability.is_finite() || !(0.0..=1.0).contains(probability) {
            return Err(LandGateAdvisorError::InvalidResponse(format!(
                "probability for `{color}` was outside [0, 1]"
            )));
        }
        probability_sum += probability;
    }
    if (probability_sum - 1.0).abs() > 0.02 {
        return Err(LandGateAdvisorError::InvalidResponse(
            "choice probabilities did not sum approximately to 1".into(),
        ));
    }
    Ok(())
}

#[async_trait::async_trait]
impl LandGateAdvisor for TypeSafeLandGateAdvisor {
    async fn advise(
        &self,
        request: LandGateAdviceRequest,
    ) -> Result<LandGateAdvice, LandGateAdvisorError> {
        let thresholds = request
            .thresholds
            .unwrap_or(self.thresholds)
            .validate()
            .map_err(LandGateAdvisorError::InvalidRequest)?;
        let instructions = request.instructions.as_deref().unwrap_or(
            "Choose the safest land-gate disposition for this work. Green means it is ready to \
             land, amber means it needs human review or changes, and red means it must not land.",
        );
        if instructions.trim().is_empty() {
            return Err(LandGateAdvisorError::InvalidRequest(
                "instructions must not be empty".into(),
            ));
        }
        if !matches!(
            &request.state,
            Value::String(_) | Value::Array(_) | Value::Object(_)
        ) {
            return Err(LandGateAdvisorError::InvalidRequest(
                "state must be a string, array, or object".into(),
            ));
        }
        let question = TypeSafeQuestion {
            kind: "choice",
            instructions,
            criteria: BTreeMap::from([
                ("green", "Ready to land without further changes."),
                (
                    "amber",
                    "Needs human review, clarification, or changes before landing.",
                ),
                (
                    "red",
                    "Must not land because it is unsafe or materially incorrect.",
                ),
            ]),
        };
        let body = TypeSafeRequest {
            state: &request.state,
            model: &self.model,
            questions: BTreeMap::from([("land", question)]),
        };
        let started = std::time::Instant::now();
        let response = self
            .client
            .post(self.endpoint.clone())
            .bearer_auth(&self.api_key)
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
        let response: TypeSafeResponse = response
            .json()
            .await
            .map_err(|error| LandGateAdvisorError::InvalidResponse(error.to_string()))?;
        let latency_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
        let answer = response.answers.get("land").ok_or_else(|| {
            LandGateAdvisorError::InvalidResponse("response omitted `answers.land`".into())
        })?;
        validate_answer(answer)?;
        let raw_land = parse_land(&answer.choice)?;
        Ok(LandGateAdvice {
            provider: "typesafe".into(),
            model: response.model,
            raw_land,
            recommended_land: recommended_land(raw_land, answer.confidence, thresholds),
            confidence: answer.confidence,
            probabilities: answer.probabilities.clone(),
            thresholds,
            usage: response.usage,
            latency_ms,
        })
    }
}

/// Build the optional runtime. Missing/false feature flag returns `None`
/// without reading credentials, preserving the pre-spike default exactly.
pub async fn from_env() -> Result<Option<Arc<dyn LandGateAdvisor>>, LandGateAdvisorError> {
    let Some(settings) = Settings::from_lookup(|name| std::env::var(name).ok())? else {
        return Ok(None);
    };
    Ok(Some(Arc::new(
        TypeSafeLandGateAdvisor::from_settings(settings).await?,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::State, http::HeaderMap, routing::post, Json, Router};
    use serde_json::json;
    use std::{collections::HashMap, net::SocketAddr};

    fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let values: HashMap<String, String> = pairs
            .iter()
            .map(|(name, value)| ((*name).into(), (*value).into()))
            .collect();
        move |name| values.get(name).cloned()
    }

    #[test]
    fn feature_is_off_by_default_and_does_not_require_credentials() {
        assert!(Settings::from_lookup(lookup(&[])).unwrap().is_none());
        assert!(
            Settings::from_lookup(lookup(&[("MAIDAN_JEV_LAND_GATE_ENABLED", "0")]))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn enabled_feature_requires_key_and_valid_thresholds() {
        assert!(Settings::from_lookup(lookup(&[("MAIDAN_JEV_LAND_GATE_ENABLED", "1")])).is_err());
        assert!(Settings::from_lookup(lookup(&[
            ("MAIDAN_JEV_LAND_GATE_ENABLED", "1"),
            ("TYPESAFE_API_KEY", "secret"),
            ("MAIDAN_JEV_GREEN_MIN_CONFIDENCE", "1.1"),
        ]))
        .is_err());
    }

    #[derive(Clone)]
    struct Seen(Arc<tokio::sync::Mutex<Option<(HeaderMap, Value)>>>);

    async fn provider(
        State(seen): State<Seen>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        *seen.0.lock().await = Some((headers, body));
        Json(json!({
            "model": "jev-1.13.0",
            "answers": {
                "land": {
                    "type": "choice",
                    "choice": "green",
                    "confidence": 0.81,
                    "probabilities": {"green": 0.81, "amber": 0.14, "red": 0.05}
                }
            },
            "usage": {"input_tokens": 120, "output_tokens": 4}
        }))
    }

    async fn spawn_provider() -> (SocketAddr, Seen) {
        let seen = Seen(Arc::new(tokio::sync::Mutex::new(None)));
        let app = Router::new()
            .route("/v1/systemone", post(provider))
            .with_state(seen.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (addr, seen)
    }

    #[tokio::test]
    async fn client_matches_typesafe_contract_and_escalates_low_confidence() {
        let (addr, seen) = spawn_provider().await;
        let advisor = TypeSafeLandGateAdvisor::with_http(
            Client::new(),
            format!("http://{addr}/v1/systemone").parse().unwrap(),
            "test-key",
            "jev-latest",
            Duration::from_secs(1),
            LandGateAdviceThresholds::default(),
        );
        let advice = advisor
            .advise(LandGateAdviceRequest {
                state: json!({"title": "ship it"}),
                instructions: None,
                thresholds: None,
            })
            .await
            .unwrap();
        assert_eq!(advice.raw_land, LandColor::Green);
        assert_eq!(advice.recommended_land, LandColor::Amber);
        assert_eq!(advice.usage.input_tokens, 120);

        let (headers, body) = seen.0.lock().await.take().unwrap();
        assert_eq!(headers["authorization"], "Bearer test-key");
        assert_eq!(body["model"], "jev-latest");
        assert_eq!(body["questions"]["land"]["type"], "choice");
        assert_eq!(
            body["questions"]["land"]["criteria"]["green"],
            "Ready to land without further changes."
        );
    }

    #[tokio::test]
    async fn invalid_state_is_rejected_before_network_io() {
        let advisor = TypeSafeLandGateAdvisor::with_http(
            Client::new(),
            "http://127.0.0.1:1/v1/systemone".parse().unwrap(),
            "test-key",
            "jev-latest",
            Duration::from_millis(1),
            LandGateAdviceThresholds::default(),
        );
        let error = advisor
            .advise(LandGateAdviceRequest {
                state: Value::Null,
                instructions: None,
                thresholds: None,
            })
            .await
            .unwrap_err();
        assert!(matches!(error, LandGateAdvisorError::InvalidRequest(_)));
    }

    #[test]
    fn threshold_policy_never_promotes_a_choice_to_green() {
        let thresholds = LandGateAdviceThresholds::default();
        assert_eq!(
            recommended_land(LandColor::Green, 0.99, thresholds),
            LandColor::Green
        );
        assert_eq!(
            recommended_land(LandColor::Green, 0.50, thresholds),
            LandColor::Amber
        );
        assert_eq!(
            recommended_land(LandColor::Amber, 0.99, thresholds),
            LandColor::Amber
        );
        assert_eq!(
            recommended_land(LandColor::Red, 0.50, thresholds),
            LandColor::Amber
        );
        assert_eq!(
            recommended_land(LandColor::Red, 0.99, thresholds),
            LandColor::Red
        );
    }
}

//! The hosted launch path's front end: the public waitlist form and the
//! operator's list, export and invite mint. Sign-up itself lives in the
//! OIDC handlers; these routes only collect and hand out the invites that
//! gate it.

use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use chrono::{Duration, Utc};
use maidan_auth::{capability::OPERATOR_GLOBAL, AuthContext};
use maidan_types::{NewWaitlistEntry, WaitlistEntry, WaitlistEntryId, WaitlistStatus};
use rand::RngCore;
use sha2::{Digest, Sha256};

use super::{cap, ApiResult};
use crate::dto::{InviteMintedView, JoinWaitlistRequest, WaitlistEntryView};
use crate::error::ApiError;
use crate::extract::{ApiJson, ApiPath};
use crate::state::AppState;

/// How long an invite stays live after the operator mints it.
const INVITE_TTL_DAYS: i64 = 30;
/// Longest email the waitlist accepts, in characters.
const MAX_EMAIL_CHARS: usize = 254;
/// Longest use-case note the waitlist accepts, in characters.
const MAX_USE_CASE_CHARS: usize = 2_000;

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, bytes)
}

fn invite_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// The whole waitlist surface is part of sign-up: an instance that never
/// enabled it collects nothing and shows nothing. 404, the same answer the
/// sign-up start gives there.
pub(crate) fn require_signup(state: &AppState) -> ApiResult<()> {
    let enabled = state
        .oidc
        .as_ref()
        .map(|oidc| oidc.settings.signup)
        .unwrap_or(false);
    if enabled {
        Ok(())
    } else {
        Err(ApiError::NotFound)
    }
}

/// The footer links to the instance's privacy policy and terms, for the
/// waitlist page's `<!-- policy-links -->` slot; empty where neither is set.
/// Boot already refused a value that could leave its attribute; the escape
/// keeps an `&` in a query string valid HTML.
pub(crate) fn policy_links_html(state: &AppState) -> String {
    let Some(oidc) = state.oidc.as_ref() else {
        return String::new();
    };
    let settings = &oidc.settings;
    [
        (settings.privacy_url.as_deref(), "Privacy"),
        (settings.terms_url.as_deref(), "Terms"),
    ]
    .into_iter()
    .filter_map(|(url, label)| {
        url.map(|url| format!("<a href=\"{}\">{label}</a>", url.replace('&', "&amp;")))
    })
    .collect::<Vec<_>>()
    .join("\n      ")
}

/// Longest local part (before the `@`) an address may have, per RFC 5321.
const MAX_LOCAL_CHARS: usize = 64;

/// A plausible email shape: one `@`, a local part of at most 64 characters,
/// a dotted domain whose labels are letters, digits and inner hyphens, and
/// no whitespace, control characters or address punctuation that only a
/// quoted or display-name form would use. Deliverability is the provider's
/// problem at sign-up; this only keeps the list free of shapes that are not
/// addresses at all.
fn plausible_email(email: &str) -> bool {
    let email = email.trim();
    if email.chars().count() > MAX_EMAIL_CHARS
        || email
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || "<>()[]\\,;:\"".contains(c))
    {
        return false;
    }
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    if local.is_empty() || local.chars().count() > MAX_LOCAL_CHARS || domain.contains('@') {
        return false;
    }
    let labels: Vec<&str> = domain.split('.').collect();
    labels.len() >= 2
        && labels.iter().all(|label| {
            !label.is_empty()
                && label.chars().count() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.chars().all(|c| c.is_alphanumeric() || c == '-')
        })
}

/// Whether a browser sent this request from another site. A request with
/// an `Origin` must name this host; one without falls back to
/// `Sec-Fetch-Site`. A client that sends neither (curl, an SDK) is not a
/// browser being driven by another page, so it passes; the rate limits
/// still apply to it.
fn cross_site(headers: &HeaderMap, public_origin: Option<&str>) -> bool {
    if let Some(origin) = headers.get(header::ORIGIN) {
        let Ok(origin) = origin.to_str() else {
            return true;
        };
        let origin = origin.trim_end_matches('/').to_ascii_lowercase();
        // Behind a proxy the Host header is whatever the proxy sends, so the
        // configured public origin is the one a browser's Origin must match.
        if let Some(public) = public_origin {
            return origin != public.trim_end_matches('/').to_ascii_lowercase();
        }
        let host = headers
            .get(header::HOST)
            .and_then(|v| v.to_str().ok())
            .map(str::to_ascii_lowercase);
        let origin_host = origin.split_once("://").map(|(_, rest)| rest.to_string());
        return match (origin_host, host) {
            (Some(origin_host), Some(host)) => origin_host != host,
            _ => true,
        };
    }
    headers
        .get("sec-fetch-site")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|site| site != "same-origin" && site != "none")
}

/// The one answer a join gets, whether the address is new, already listed,
/// or the form was filled in by a bot: the list is not an oracle for who is
/// on it, and a bot learns nothing from its answer.
fn joined(email: &str) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::CREATED,
        Json(serde_json::json!({ "email": email, "status": "waiting" })),
    )
}

/// `POST /waitlist` — a signed-out visitor joins the waitlist. Public by
/// design; the rate-limit middleware charges it per address and per email
/// domain. A browser must post it from this origin. A repeat join for an
/// address already listed answers exactly like a fresh one, and the first
/// row (and any invite minted from it) stands. A join whose hidden
/// `website` field is filled in is a bot's: it answers the same and stores
/// nothing.
pub async fn join_waitlist(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<JoinWaitlistRequest>,
) -> ApiResult<impl IntoResponse> {
    require_signup(&state)?;
    if cross_site(&headers, state.public_origin.as_deref()) {
        return Err(ApiError::Forbidden(
            "the waitlist takes joins from this site only".into(),
        ));
    }
    if !body.consent {
        return Err(ApiError::BadRequest(
            "joining the waitlist needs consent to be contacted".into(),
        ));
    }
    let email = body.email.trim().to_string();
    if !plausible_email(&email) {
        return Err(ApiError::BadRequest("a valid email is required".into()));
    }
    let use_case = body.use_case.trim().to_string();
    if use_case.is_empty() || use_case.chars().count() > MAX_USE_CASE_CHARS {
        return Err(ApiError::BadRequest(
            "say briefly what you would use Maidan for".into(),
        ));
    }
    if body
        .website
        .as_deref()
        .is_some_and(|w| !w.trim().is_empty())
    {
        return Ok(joined(&email));
    }
    let now = Utc::now();
    match state
        .store
        .insert_waitlist_entry(NewWaitlistEntry {
            email: email.clone(),
            use_case,
            consented_at: now,
        })
        .await
    {
        Ok(_) | Err(maidan_store::StoreError::Conflict(_)) => Ok(joined(&email)),
        Err(err) => Err(err.into()),
    }
}

/// `GET /operator/waitlist` — every entry, oldest first. `operator:global`.
pub async fn list_waitlist(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
) -> ApiResult<Json<Vec<WaitlistEntryView>>> {
    cap(&auth, OPERATOR_GLOBAL)?;
    require_signup(&state)?;
    let entries = state.store.list_waitlist_entries().await?;
    Ok(Json(
        entries.into_iter().map(WaitlistEntryView::from).collect(),
    ))
}

fn csv_field(value: &str) -> String {
    // A visitor wrote this cell; a spreadsheet must not run it as a formula.
    let value = if value.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{value}")
    } else {
        value.to_string()
    };
    if value.contains(['"', ',', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value
    }
}

/// `GET /operator/waitlist/export` — the list as a CSV download, so the
/// operator can work it in whatever tool they run the launch from.
/// `operator:global`.
pub async fn export_waitlist(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
) -> ApiResult<Response> {
    cap(&auth, OPERATOR_GLOBAL)?;
    require_signup(&state)?;
    let entries = state.store.list_waitlist_entries().await?;
    let mut csv = String::from(
        "id,email,use_case,consented_at,status,invite_expires_at,workspace_id,created_at\n",
    );
    for entry in entries {
        let fields = [
            entry.id.0.to_string(),
            entry.email.clone(),
            entry.use_case.clone(),
            entry.consented_at.to_rfc3339(),
            entry.status.as_str().to_string(),
            entry
                .invite_expires_at
                .map(|t| t.to_rfc3339())
                .unwrap_or_default(),
            entry
                .workspace_id
                .map(|w| w.0.to_string())
                .unwrap_or_default(),
            entry.created_at.to_rfc3339(),
        ];
        csv.push_str(
            &fields
                .iter()
                .map(|f| csv_field(f))
                .collect::<Vec<_>>()
                .join(","),
        );
        csv.push('\n');
    }
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"waitlist.csv\"",
            ),
        ],
        csv,
    )
        .into_response())
}

/// `POST /operator/waitlist/{id}/invite` — turn an entry into an invite.
/// The token is shown once, in this response; only its hash is stored, so
/// a later mint replaces the earlier link rather than reviving it.
/// `operator:global`.
pub async fn invite_waitlist_entry(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(entry_id): ApiPath<uuid::Uuid>,
) -> ApiResult<Json<InviteMintedView>> {
    cap(&auth, OPERATOR_GLOBAL)?;
    require_signup(&state)?;
    let current = state
        .store
        .get_waitlist_entry(WaitlistEntryId(entry_id))
        .await?;
    // The store refuses a mint over a completed sign-up; refuse it here
    // too, before the charge, so the refusal spends nothing.
    if current.status == WaitlistStatus::SignedUp {
        return Err(ApiError::Conflict(
            "the waitlist entry has already signed up".into(),
        ));
    }
    crate::rate_limit::charge_email_domain(
        &state,
        crate::rate_limit::DomainCharge::Invite,
        &current.email,
    )
    .await?;
    let token = random_token();
    let expires_at = Utc::now() + Duration::days(INVITE_TTL_DAYS);
    let entry: WaitlistEntry = state
        .store
        .set_waitlist_invite(WaitlistEntryId(entry_id), invite_hash(&token), expires_at)
        .await?;
    Ok(Json(InviteMintedView {
        entry: WaitlistEntryView::from(entry),
        invite_token: token.clone(),
        signup_path: format!("/auth/oidc/signup?invite={token}"),
        invite_expires_at: expires_at,
    }))
}

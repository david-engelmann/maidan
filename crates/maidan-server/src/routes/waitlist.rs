//! The hosted launch path's front end: the public waitlist form and the
//! operator's list, export and invite mint. Sign-up itself lives in the
//! OIDC handlers; these routes only collect and hand out the invites that
//! gate it.

use axum::{
    extract::State,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use chrono::{Duration, Utc};
use maidan_auth::{capability::OPERATOR_GLOBAL, AuthContext};
use maidan_types::{NewWaitlistEntry, WaitlistEntry, WaitlistEntryId};
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

/// A plausible email shape: one `@`, non-empty local part and domain, a
/// dot in the domain, no spaces. Deliverability is the provider's problem
/// at sign-up; this only keeps the list free of shapes that are not
/// addresses at all.
fn plausible_email(email: &str) -> bool {
    let email = email.trim();
    if email.chars().count() > MAX_EMAIL_CHARS || email.chars().any(char::is_whitespace) {
        return false;
    }
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !domain.contains('@')
}

/// `POST /waitlist` — a signed-out visitor joins the waitlist. Public by
/// design; the rate-limit middleware charges it per address and per email
/// domain. A repeat join for an address already listed answers exactly
/// like a fresh one: the list is not an oracle for who is on it, and the
/// first row (and any invite minted from it) stands.
pub async fn join_waitlist(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<JoinWaitlistRequest>,
) -> ApiResult<impl IntoResponse> {
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
        Ok(_) | Err(maidan_store::StoreError::Conflict(_)) => Ok((
            StatusCode::CREATED,
            Json(serde_json::json!({ "email": email, "status": "waiting" })),
        )),
        Err(err) => Err(err.into()),
    }
}

/// `GET /operator/waitlist` — every entry, oldest first. `operator:global`.
pub async fn list_waitlist(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
) -> ApiResult<Json<Vec<WaitlistEntryView>>> {
    cap(&auth, OPERATOR_GLOBAL)?;
    let entries = state.store.list_waitlist_entries().await?;
    Ok(Json(
        entries.into_iter().map(WaitlistEntryView::from).collect(),
    ))
}

fn csv_field(value: &str) -> String {
    if value.contains(['"', ',', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
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

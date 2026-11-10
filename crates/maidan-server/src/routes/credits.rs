//! Prepaid credits (`docs/Credits.md`): a workspace's balance, open top-up
//! intents and ledger (`GET /workspaces/:id/credits`), opening an intent
//! (`POST /workspaces/:id/credits/top-ups`), and the transfers that matched
//! no intent, for the operator (`GET /operator/credits/unmatched`), who settles
//! one by hand with a grant (`POST /operator/workspaces/:id/credits/grants`). Opening an
//! intent moves no money, so it needs only `workspace:read`: an agent can ask
//! to be paid for itself. Both workspace routes stay open past the allowance
//! cutoff (`usage_meters::readable_past_cutoff`).

use axum::{extract::State, http::StatusCode, Extension, Json};
use maidan_auth::capability::{OPERATOR_GLOBAL, WORKSPACE_READ};
use maidan_auth::AuthContext;
use maidan_store::credit_terms::{self, TopUpError};
use maidan_types::{CreditEntry, CreditGrant, Credits, TopUp, UnmatchedDeposit, WorkspaceId};

use crate::dto::{CreditGrantRequest, TopUpRequest, UnmatchedDepositsQuery};
use crate::error::ApiError;
use crate::extract::{ApiJson, ApiPath, ApiQuery};
use crate::routes::{cap, ensure_workspace, ApiResult};
use crate::state::AppState;

impl From<TopUpError> for ApiError {
    fn from(err: TopUpError) -> Self {
        match err {
            TopUpError::Disabled | TopUpError::CardNotConfigured => {
                ApiError::Conflict(err.to_string())
            }
            TopUpError::OutOfRange => ApiError::UnprocessableEntity(err.to_string()),
            TopUpError::Store(err) => err.into(),
        }
    }
}

/// `GET /workspaces/:id/credits`: the balance, the open intents, the recent
/// ledger and the instance's overage prices. `workspace:read`.
pub async fn get_workspace_credits(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
) -> ApiResult<Json<Credits>> {
    cap(&auth, WORKSPACE_READ)?;
    let workspace_id = WorkspaceId(id);
    ensure_workspace(&auth, workspace_id)?;
    Ok(Json(
        credit_terms::credits(
            state.store.as_ref(),
            &state.credit_terms,
            workspace_id,
            chrono::Utc::now(),
        )
        .await?,
    ))
}

/// `POST /workspaces/:id/credits/top-ups` `{ "usd": 5, "method": "usdc" }`:
/// open an intent and answer the exact USDC amount to send, where, and a
/// wallet link. 409 when this instance takes no payment, for `card` (the next
/// rail, not configured yet), or when the workspace holds 20 open intents;
/// 422 for an amount outside 1..=10000. `workspace:read`.
pub async fn create_credit_top_up(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
    ApiJson(body): ApiJson<TopUpRequest>,
) -> ApiResult<(StatusCode, Json<TopUp>)> {
    cap(&auth, WORKSPACE_READ)?;
    let workspace_id = WorkspaceId(id);
    ensure_workspace(&auth, workspace_id)?;
    let top_up = credit_terms::open_top_up(
        state.store.as_ref(),
        &state.credit_terms,
        workspace_id,
        body.usd,
        body.method,
        chrono::Utc::now(),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(top_up)))
}

/// `GET /operator/credits/unmatched`: transfers to the receiving address that
/// matched no intent, newest first, for the operator to settle by hand.
/// `operator:global`.
pub async fn list_unmatched_credit_deposits(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiQuery(query): ApiQuery<UnmatchedDepositsQuery>,
) -> ApiResult<Json<Vec<UnmatchedDeposit>>> {
    cap(&auth, OPERATOR_GLOBAL)?;
    Ok(Json(
        state
            .store
            .list_unmatched_credit_deposits(query.limit.unwrap_or(100))
            .await?,
    ))
}

/// `POST /operator/workspaces/:id/credits/grants` `{ "usd_micros": n, "note":
/// "…" }`: add credit by hand, such as a deposit that matched no intent. The
/// note says why and is kept on the ledger row. `operator:global`.
pub async fn grant_workspace_credits(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
    ApiJson(body): ApiJson<CreditGrantRequest>,
) -> ApiResult<(StatusCode, Json<CreditEntry>)> {
    cap(&auth, OPERATOR_GLOBAL)?;
    let note = body.note.trim();
    if note.is_empty() || note.chars().count() > 2000 {
        return Err(ApiError::UnprocessableEntity(
            "note says why, in 1 to 2000 characters".into(),
        ));
    }
    if !(1..=maidan_types::TOP_UP_MAX_USD * maidan_types::USD_MICROS).contains(&body.usd_micros) {
        return Err(ApiError::UnprocessableEntity(
            "usd_micros is from 1 to 10,000 dollars in micros".into(),
        ));
    }
    let workspace_id = WorkspaceId(id);
    // A grant to a workspace that does not exist would sit on no balance.
    state.store.get_workspace(workspace_id).await?;
    let entry = state
        .store
        .grant_credits(&CreditGrant {
            workspace_id,
            usd_micros: body.usd_micros,
            note: note.to_string(),
            at: chrono::Utc::now(),
        })
        .await?;
    crate::usage_meters::refresh_cut_off(&state).await;
    Ok((StatusCode::CREATED, Json(entry)))
}

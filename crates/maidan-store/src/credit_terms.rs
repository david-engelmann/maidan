//! Credits as both surfaces answer them: `GET /workspaces/{id}/credits` and
//! `POST /workspaces/{id}/credits/top-ups`, and their MCP twins `get_credits`
//! and `top_up_credits`, so the two apply one range check and show one view.
//! Capability and workspace checks stay with each caller, which knows how its
//! credential arrived.

use chrono::{DateTime, Utc};
use maidan_types::{
    random_offsets, usd_string, CreditTerms, Credits, NewTopUp, TopUp, TopUpMethod, WorkspaceId,
    TOP_UP_MAX_USD, TOP_UP_MIN_USD,
};

use crate::error::StoreError;
use crate::store::Store;

/// Ledger rows shown with the balance.
pub const LEDGER_SHOWN: i64 = 50;

/// Candidate offsets tried per intent. With up to 9,999 offsets per dollar
/// amount, eight random picks all being held means the instance has thousands
/// of open intents for the same dollars.
const OFFSET_CANDIDATES: usize = 8;

/// Why an intent was not opened.
#[derive(Debug, thiserror::Error)]
pub enum TopUpError {
    /// This instance takes no payment (`MAIDAN_CREDITS_ADDRESS` is unset).
    #[error("this instance does not take credits: MAIDAN_CREDITS_ADDRESS is not set")]
    Disabled,
    /// Card payments are the next rail and are not set up here yet.
    #[error("card top-ups are not configured on this instance; use method usdc")]
    CardNotConfigured,
    /// The amount is out of range.
    #[error("usd must be a whole number of dollars from {TOP_UP_MIN_USD} to {TOP_UP_MAX_USD}")]
    OutOfRange,
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// The workspace's balance, open intents and recent ledger, with what this
/// instance offers.
pub async fn credits(
    store: &dyn Store,
    terms: &CreditTerms,
    workspace_id: WorkspaceId,
    now: DateTime<Utc>,
) -> Result<Credits, StoreError> {
    let balance_micros = store.credit_balance(workspace_id).await?;
    let open_top_ups = match &terms.settings {
        Some(_) => store.list_open_credit_top_ups(workspace_id, now).await?,
        None => Vec::new(),
    };
    let ledger = store.list_credit_ledger(workspace_id, LEDGER_SHOWN).await?;
    Ok(Credits {
        workspace_id,
        enabled: terms.settings.is_some(),
        methods: terms.methods(),
        chain: terms.settings.as_ref().map(|s| s.chain),
        address: terms.settings.as_ref().map(|s| s.address.clone()),
        usdc_contract: terms.settings.as_ref().map(|s| s.chain.usdc().to_string()),
        balance_micros,
        balance: usd_string(balance_micros),
        prices: if terms.settings.is_some() {
            terms.prices.listed()
        } else {
            Vec::new()
        },
        open_top_ups,
        ledger,
    })
}

/// Open an intent for `usd` dollars, paid by `method`. Moves no money.
pub async fn open_top_up(
    store: &dyn Store,
    terms: &CreditTerms,
    workspace_id: WorkspaceId,
    usd: i64,
    method: TopUpMethod,
    now: DateTime<Utc>,
) -> Result<TopUp, TopUpError> {
    if !(TOP_UP_MIN_USD..=TOP_UP_MAX_USD).contains(&usd) {
        return Err(TopUpError::OutOfRange);
    }
    if method == TopUpMethod::Card {
        return Err(TopUpError::CardNotConfigured);
    }
    let Some(settings) = &terms.settings else {
        return Err(TopUpError::Disabled);
    };
    Ok(store
        .create_credit_top_up(&NewTopUp {
            workspace_id,
            chain: settings.chain,
            address: settings.address.clone(),
            usd,
            offsets: random_offsets(OFFSET_CANDIDATES),
            now,
        })
        .await?)
}

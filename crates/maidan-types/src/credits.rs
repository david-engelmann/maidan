//! Prepaid credits, paid in USDC on Base. A workspace holds a balance in USD
//! micros: deposits and operator grants add to it, overage charges take from
//! it, and the balance is the sum of its ledger. USDC has six decimals, so one
//! base unit of USDC is one micro-dollar and a transfer's amount is already in
//! the ledger's unit.
//!
//! A deposit is matched to its workspace by amount: a top-up intent asks for
//! the dollars plus a small offset in micros that no other open intent on the
//! instance asks for, so a plain ERC-20 transfer, with no memo, names its
//! workspace. The server only reads the chain: it holds no key and signs
//! nothing.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{UsageMeter, WorkspaceId};

/// Micros in a dollar, and base units in one USDC.
pub const USD_MICROS: i64 = 1_000_000;
/// The smallest and largest top-up, in whole dollars.
pub const TOP_UP_MIN_USD: i64 = 1;
pub const TOP_UP_MAX_USD: i64 = 10_000;
/// The largest offset an intent adds to its dollars, in micros. Offsets run
/// from 1, so an intent never asks for a round amount another wallet might
/// send by chance.
pub const TOP_UP_OFFSET_MAX: i64 = 9_999;
/// Open intents one workspace may hold at once.
pub const OPEN_TOP_UPS_PER_WORKSPACE: i64 = 20;
/// How long an intent is offered for.
pub const TOP_UP_TTL_SECS: i64 = 24 * 3600;
/// How long past its expiry an intent still holds its amount and is still
/// matched: a transfer sent in the intent's last minutes lands a few blocks
/// later, and its amount must not have been handed to another intent by then.
pub const TOP_UP_GRACE_SECS: i64 = 24 * 3600;
/// `keccak256("Transfer(address,address,uint256)")`, the ERC-20 transfer log's
/// first topic.
pub const TRANSFER_TOPIC: &str =
    "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";

/// The chain deposits are read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "kebab-case")]
pub enum CreditChain {
    /// Base mainnet, chain id 8453.
    Base,
    /// Base Sepolia, the test network, chain id 84532.
    BaseSepolia,
}

impl CreditChain {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Base => "base",
            Self::BaseSepolia => "base-sepolia",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "base" => Some(Self::Base),
            "base-sepolia" => Some(Self::BaseSepolia),
            _ => None,
        }
    }

    /// The EIP-155 chain id.
    pub fn chain_id(self) -> u64 {
        match self {
            Self::Base => 8453,
            Self::BaseSepolia => 84532,
        }
    }

    /// Circle's USDC contract on this chain, as Circle publishes it.
    pub fn usdc(self) -> &'static str {
        match self {
            Self::Base => "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
            Self::BaseSepolia => "0x036CbD53842c5426634e7929541eC2318f3dCF7e",
        }
    }

    /// The public RPC endpoint used when `MAIDAN_CREDITS_RPC_URL` is unset.
    pub fn default_rpc_url(self) -> &'static str {
        match self {
            Self::Base => "https://mainnet.base.org",
            Self::BaseSepolia => "https://sepolia.base.org",
        }
    }
}

/// An EVM address as `0x` and 40 lowercase hex digits, or `None`. Addresses
/// are compared lowercased: the mixed case of EIP-55 is a checksum, not part
/// of the address.
pub fn normalize_address(raw: &str) -> Option<String> {
    let hex = raw.trim().strip_prefix("0x")?;
    (hex.len() == 40 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| format!("0x{}", hex.to_ascii_lowercase()))
}

/// Where this instance takes payment: the chain and the receiving address.
/// Unset (no `MAIDAN_CREDITS_ADDRESS`) means credits are off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreditsSettings {
    pub chain: CreditChain,
    /// Lowercase.
    pub address: String,
}

impl CreditsSettings {
    /// From `MAIDAN_CREDITS_CHAIN` (default `base`) and
    /// `MAIDAN_CREDITS_ADDRESS`. A value that is not a chain or an address is
    /// refused, naming it.
    pub fn parse(chain: Option<&str>, address: &str) -> Result<Self, String> {
        let chain = match chain.map(str::trim).filter(|c| !c.is_empty()) {
            None => CreditChain::Base,
            Some(raw) => CreditChain::parse(raw)
                .ok_or_else(|| format!("`{raw}` is not a chain: use base or base-sepolia"))?,
        };
        let address = normalize_address(address).ok_or_else(|| {
            format!(
                "`{}` is not an address: 0x and 40 hex digits",
                address.trim()
            )
        })?;
        Ok(Self { chain, address })
    }
}

/// What this instance offers: where it takes payment, and what overage
/// costs. Charging needs both: with no address, or no price for a meter,
/// that meter past its allowance is a hard cap, as it was before credits.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CreditTerms {
    pub settings: Option<CreditsSettings>,
    pub prices: OveragePrices,
}

impl CreditTerms {
    /// The price charged past the allowance on `meter`, when this instance
    /// takes payment and prices it.
    pub fn price(&self, meter: UsageMeter) -> Option<OveragePrice> {
        self.settings.as_ref()?;
        self.prices.get(meter)
    }

    /// The ways this instance takes a top-up.
    pub fn methods(&self) -> Vec<TopUpMethod> {
        match self.settings {
            Some(_) => vec![TopUpMethod::Usdc],
            None => Vec::new(),
        }
    }

    /// The meters a deposit may lift the cutoff on.
    pub fn priced_meters(&self) -> Vec<UsageMeter> {
        if self.settings.is_none() {
            return Vec::new();
        }
        self.prices.prices.keys().copied().collect()
    }
}

/// The EIP-681 link a wallet opens to send `amount_micros` USDC to `to`.
pub fn eip681(chain: CreditChain, to: &str, amount_micros: i64) -> String {
    format!(
        "ethereum:{}@{}/transfer?address={to}&uint256={amount_micros}",
        chain.usdc(),
        chain.chain_id()
    )
}

/// Micros as a dollar figure with six decimals, `5.001234`.
pub fn usd_string(micros: i64) -> String {
    let sign = if micros < 0 { "-" } else { "" };
    let abs = micros.unsigned_abs();
    format!(
        "{sign}{}.{:06}",
        abs / USD_MICROS as u64,
        abs % USD_MICROS as u64
    )
}

/// Candidate offsets for a new intent, random in `1..=TOP_UP_OFFSET_MAX`. The
/// store takes the first that no open intent with the same dollars holds.
pub fn random_offsets(n: usize) -> Vec<i64> {
    use chacha20poly1305::aead::{rand_core::RngCore, OsRng};
    (0..n)
        .map(|_| i64::from(OsRng.next_u32() % TOP_UP_OFFSET_MAX as u32) + 1)
        .collect()
}

/// A meter's overage price: `micros` for every `per` units past the
/// allowance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OveragePrice {
    pub micros: i64,
    pub per: i64,
}

impl OveragePrice {
    /// What the first `units` past the allowance cost, rounded up to a whole
    /// micro. A charge for units `(a, b]` is `cost_through(b) -
    /// cost_through(a)`, so charging in pieces costs exactly what charging at
    /// once would: the rounding never drifts.
    pub fn cost_through(self, units: i64) -> i64 {
        if units <= 0 || self.per <= 0 {
            return 0;
        }
        let numerator = i128::from(units) * i128::from(self.micros);
        let per = i128::from(self.per);
        let cost = (numerator + per - 1) / per;
        i64::try_from(cost).unwrap_or(i64::MAX)
    }
}

/// The instance's overage prices per meter (`MAIDAN_OVERAGE_PRICES`, e.g.
/// `mcp_calls=1000000/100000,requests=500000/100000`: one dollar per 100,000
/// MCP calls, fifty cents per 100,000 requests). A meter not named has no
/// price, and past its allowance it is a hard cap as before.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OveragePrices {
    pub prices: BTreeMap<UsageMeter, OveragePrice>,
}

impl OveragePrices {
    /// Parse the env form: comma-separated `meter=micros/units`, both whole
    /// numbers of at least 1. An unknown meter, a repeated one, a malformed
    /// price, or `stored_bytes` (a level, not a count, so it has no units to
    /// charge for) is refused, naming it.
    pub fn parse(raw: &str) -> Result<Self, String> {
        let mut prices = BTreeMap::new();
        for pair in raw.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            let (name, value) = pair
                .split_once('=')
                .ok_or_else(|| format!("`{pair}` is not meter=micros/units"))?;
            let name = name.trim();
            let meter =
                UsageMeter::parse(name).ok_or_else(|| format!("`{name}` is not a usage meter"))?;
            if meter.is_gauge() {
                return Err(format!(
                    "{name} is a level, not a count, and takes no price: it stays a hard cap"
                ));
            }
            let whole = |s: &str| s.trim().parse::<i64>().ok().filter(|n| *n >= 1);
            let price = value
                .split_once('/')
                .and_then(|(micros, per)| Some((whole(micros)?, whole(per)?)))
                .map(|(micros, per)| OveragePrice { micros, per })
                .ok_or_else(|| {
                    format!(
                        "`{}` is not a price for {name}: micros/units, both whole numbers of at least 1",
                        value.trim()
                    )
                })?;
            if prices.insert(meter, price).is_some() {
                return Err(format!("{name} is named twice"));
            }
        }
        Ok(Self { prices })
    }

    pub fn get(&self, meter: UsageMeter) -> Option<OveragePrice> {
        self.prices.get(&meter).copied()
    }

    pub fn is_empty(&self) -> bool {
        self.prices.is_empty()
    }

    /// The prices as listed to a caller.
    pub fn listed(&self) -> Vec<MeterPrice> {
        self.prices
            .iter()
            .map(|(meter, price)| MeterPrice {
                meter: *meter,
                micros: price.micros,
                per: price.per,
            })
            .collect()
    }
}

/// One meter's overage price, as a caller reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MeterPrice {
    pub meter: UsageMeter,
    /// USD micros charged for every `per` units past the allowance.
    pub micros: i64,
    pub per: i64,
}

/// What a ledger row records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum CreditEntryKind {
    /// USDC received on chain, matched to a top-up intent.
    Deposit,
    /// Overage past the allowance on a priced meter.
    Charge,
    /// Credit an operator added by hand.
    Grant,
}

impl CreditEntryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Deposit => "deposit",
            Self::Charge => "charge",
            Self::Grant => "grant",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "deposit" => Some(Self::Deposit),
            "charge" => Some(Self::Charge),
            "grant" => Some(Self::Grant),
            _ => None,
        }
    }
}

/// One row of a workspace's credit ledger. The balance is the sum of
/// `usd_micros` over every row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CreditEntry {
    pub id: uuid::Uuid,
    pub workspace_id: WorkspaceId,
    pub kind: CreditEntryKind,
    /// For a deposit or a grant: where the money came from.
    pub source: Option<CreditSource>,
    /// For a deposit: its reference at the source, unique per source
    /// (`<tx_hash>:<log_index>` for USDC, the Checkout Session id for a card).
    pub external_ref: Option<String>,
    /// Positive for a deposit or a grant, negative for a charge.
    pub usd_micros: i64,
    /// For a charge: the part the balance did not cover when it was made.
    /// The balance went below zero by this much, and the workspace was cut
    /// off; the next deposit pays it first.
    pub unpaid_micros: i64,
    /// For a charge: the meter, the month (`YYYY-MM`), and the units past the
    /// allowance it covers, `(through_units - units, through_units]`.
    pub meter: Option<UsageMeter>,
    pub month: Option<String>,
    pub units: Option<i64>,
    pub through_units: Option<i64>,
    /// For a deposit: where the transfer is on chain.
    pub chain: Option<CreditChain>,
    pub tx_hash: Option<String>,
    pub log_index: Option<i64>,
    pub top_up_id: Option<uuid::Uuid>,
    /// For a grant: the operator's reason.
    pub note: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Where a top-up intent stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum TopUpStatus {
    Open,
    Paid,
    Expired,
}

impl TopUpStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Paid => "paid",
            Self::Expired => "expired",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "open" => Some(Self::Open),
            "paid" => Some(Self::Paid),
            "expired" => Some(Self::Expired),
            _ => None,
        }
    }
}

/// How a top-up is paid.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum TopUpMethod {
    /// A USDC transfer on Base, from any wallet: an agent with a wallet pays
    /// for itself.
    #[default]
    Usdc,
    /// A card payment through a hosted checkout page, for a person. Not
    /// offered yet: the Stripe rail is the next step (`docs/Credits.md`).
    Card,
}

impl TopUpMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Usdc => "usdc",
            Self::Card => "card",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "usdc" => Some(Self::Usdc),
            "card" => Some(Self::Card),
            _ => None,
        }
    }
}

/// Where a ledger row's money came from. A deposit or grant row carries one,
/// with a reference unique per source: the transaction hash and log index of
/// a USDC transfer, the Checkout Session id of a card payment. A charge
/// carries none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum CreditSource {
    UsdcBase,
    Stripe,
    Grant,
}

impl CreditSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UsdcBase => "usdc_base",
            Self::Stripe => "stripe",
            Self::Grant => "grant",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "usdc_base" => Some(Self::UsdcBase),
            "stripe" => Some(Self::Stripe),
            "grant" => Some(Self::Grant),
            _ => None,
        }
    }
}

/// A USDC deposit's external reference: its transaction hash and log index,
/// which together name one transfer.
pub fn usdc_external_ref(tx_hash: &str, log_index: i64) -> String {
    format!("{tx_hash}:{log_index}")
}

/// A request to be paid. For `usdc`: send exactly `amount_micros` USDC base
/// units to `address` on `chain` before `expires_at`. For `card` (next): open
/// `checkout_url`. Creating one moves no money.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TopUp {
    pub id: uuid::Uuid,
    pub workspace_id: WorkspaceId,
    pub method: TopUpMethod,
    /// `usdc`: the chain the transfer must be made on.
    pub chain: Option<CreditChain>,
    pub chain_id: Option<u64>,
    /// `usdc`: the receiving address, lowercase.
    pub address: Option<String>,
    /// `usdc`: the USDC contract the transfer must be made on.
    pub usdc_contract: Option<String>,
    /// The whole dollars asked for.
    pub usd: i64,
    /// The exact amount to pay, in USD micros (for `usdc`, USDC base units):
    /// the dollars plus this intent's offset. A different amount is not
    /// matched.
    pub amount_micros: i64,
    /// `amount_micros` as dollars, `5.001234`.
    pub amount: String,
    /// `usdc`: an EIP-681 link a wallet can open to make the transfer.
    pub eip681: Option<String>,
    /// `card`: the hosted checkout page to hand to a person.
    pub checkout_url: Option<String>,
    pub status: TopUpStatus,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub paid_at: Option<DateTime<Utc>>,
}

/// How a stored intent is paid, as the store keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TopUpRail {
    Usdc { chain: CreditChain, address: String },
    Card { checkout_url: Option<String> },
}

impl TopUp {
    /// The full intent from what the store keeps.
    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        id: uuid::Uuid,
        workspace_id: WorkspaceId,
        rail: TopUpRail,
        usd: i64,
        amount_micros: i64,
        status: TopUpStatus,
        created_at: DateTime<Utc>,
        expires_at: DateTime<Utc>,
        paid_at: Option<DateTime<Utc>>,
    ) -> Self {
        let (method, chain, address, checkout_url) = match rail {
            TopUpRail::Usdc { chain, address } => {
                (TopUpMethod::Usdc, Some(chain), Some(address), None)
            }
            TopUpRail::Card { checkout_url } => (TopUpMethod::Card, None, None, checkout_url),
        };
        Self {
            id,
            workspace_id,
            method,
            chain,
            chain_id: chain.map(CreditChain::chain_id),
            usdc_contract: chain.map(|c| c.usdc().to_string()),
            eip681: chain
                .zip(address.as_deref())
                .map(|(c, to)| eip681(c, to, amount_micros)),
            address,
            checkout_url,
            amount: usd_string(amount_micros),
            usd,
            amount_micros,
            status,
            created_at,
            expires_at,
            paid_at,
        }
    }
}

/// What the store needs to open an intent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewTopUp {
    pub workspace_id: WorkspaceId,
    pub chain: CreditChain,
    /// Lowercase.
    pub address: String,
    pub usd: i64,
    /// Candidate offsets, tried in order ([`random_offsets`]).
    pub offsets: Vec<i64>,
    pub now: DateTime<Utc>,
}

/// One USDC transfer to the receiving address, read from a confirmed block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DepositLog {
    pub chain: CreditChain,
    /// Lowercase.
    pub tx_hash: String,
    pub log_index: i64,
    pub block_number: i64,
    /// Lowercase.
    pub from_address: String,
    /// Lowercase.
    pub to_address: String,
    pub amount_micros: i64,
}

/// What recording a deposit did.
#[derive(Debug, Clone, PartialEq)]
pub enum DepositOutcome {
    /// Matched an intent and credited its workspace.
    Credited(Box<CreditedDeposit>),
    /// Matched nothing; kept for the operator.
    Unmatched,
    /// This log was recorded before (a replay, or another replica).
    Seen,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CreditedDeposit {
    pub workspace_id: WorkspaceId,
    pub top_up_id: uuid::Uuid,
    pub entry_id: uuid::Uuid,
    pub balance_micros: i64,
    /// The meters whose cutoff this month the deposit lifted.
    pub lifted: Vec<UsageMeter>,
    /// The `credits_deposited` event, when the workspace's admins were told.
    pub event: Option<crate::StoredEvent>,
}

/// What the store needs besides the log: the month whose crossings a deposit
/// may lift, the priced meters it may lift them on, and the admins to tell,
/// found for the workspace the log was expected to match.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DepositEffects {
    pub month: String,
    pub lift: Vec<UsageMeter>,
    pub admins_of: Option<WorkspaceId>,
    pub admins: Vec<crate::MemberId>,
}

/// A transfer to the receiving address that matched no intent: the wrong
/// amount, a late payment, or a stranger's. The operator settles it by hand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UnmatchedDeposit {
    pub chain: CreditChain,
    pub tx_hash: String,
    pub log_index: i64,
    pub block_number: i64,
    pub from_address: String,
    pub to_address: String,
    pub amount_micros: i64,
    /// `amount_micros` as dollars.
    pub amount: String,
    pub seen_at: DateTime<Utc>,
}

/// A charge for a priced meter's units past the allowance, through
/// `through_units` this month.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreditCharge {
    pub workspace_id: WorkspaceId,
    pub meter: UsageMeter,
    /// `YYYY-MM`.
    pub month: String,
    /// Units past the allowance so far this month.
    pub through_units: i64,
    pub price: OveragePrice,
    pub at: DateTime<Utc>,
}

/// What a charge did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChargeOutcome {
    /// Micros this charge took; zero when nothing new was due.
    pub charged_micros: i64,
    /// The part of this charge the balance did not cover.
    pub unpaid_micros: i64,
    /// The balance after it. Below zero, the workspace is cut off.
    pub balance_micros: i64,
}

/// Credit an operator adds by hand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreditGrant {
    pub workspace_id: WorkspaceId,
    pub usd_micros: i64,
    pub note: String,
    pub at: DateTime<Utc>,
}

/// A workspace's credits as a caller reads them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Credits {
    pub workspace_id: WorkspaceId,
    /// Whether this instance takes payment. When false, no intent can be
    /// opened; the balance still reads.
    pub enabled: bool,
    /// The ways this instance takes a top-up. `card` joins `usdc` once the
    /// card rail is configured.
    pub methods: Vec<TopUpMethod>,
    pub chain: Option<CreditChain>,
    pub address: Option<String>,
    pub usdc_contract: Option<String>,
    /// The sum of the ledger, in USD micros. Below zero, overage went unpaid
    /// and the priced meters are cut off until a deposit covers it.
    pub balance_micros: i64,
    /// `balance_micros` as dollars.
    pub balance: String,
    /// The instance's overage prices; a meter not listed is a hard cap.
    pub prices: Vec<MeterPrice>,
    /// Intents still offered, newest first.
    pub open_top_ups: Vec<TopUp>,
    /// The most recent ledger rows, newest first.
    pub ledger: Vec<CreditEntry>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prices_parse_strictly_and_refuse_the_stored_bytes_level() {
        let prices = OveragePrices::parse(" mcp_calls=1000000/100000, requests = 5/1 ").unwrap();
        assert_eq!(
            prices.get(UsageMeter::McpCalls),
            Some(OveragePrice {
                micros: 1_000_000,
                per: 100_000
            })
        );
        assert_eq!(
            prices.get(UsageMeter::Requests),
            Some(OveragePrice { micros: 5, per: 1 })
        );
        assert_eq!(prices.get(UsageMeter::Deliveries), None);
        assert!(OveragePrices::parse("").unwrap().is_empty());
        for bad in [
            "mcp_calls",
            "mcp_calls=5",
            "mcp_calls=5/0",
            "mcp_calls=0/5",
            "mcp_calls=-1/5",
            "mcp_calls=a/b",
            "mcp_calls=1.5/2",
            "tokens=1/1",
            "stored_bytes=1/1",
            "mcp_calls=1/1,mcp_calls=2/1",
        ] {
            assert!(OveragePrices::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_charge_in_pieces_costs_what_one_charge_would() {
        let price = OveragePrice {
            micros: 1_000_000,
            per: 300_000,
        };
        assert_eq!(price.cost_through(0), 0);
        assert_eq!(price.cost_through(1), 4);
        assert_eq!(price.cost_through(300_000), 1_000_000);
        let pieces: i64 = [0, 7, 1_000, 1_001, 299_999, 300_000]
            .windows(2)
            .map(|w| price.cost_through(w[1]) - price.cost_through(w[0]))
            .sum();
        assert_eq!(pieces, price.cost_through(300_000));
        let huge = OveragePrice {
            micros: i64::MAX,
            per: 1,
        };
        assert_eq!(huge.cost_through(i64::MAX), i64::MAX);
    }

    #[test]
    fn the_settings_name_a_chain_and_lowercase_the_address() {
        let settings =
            CreditsSettings::parse(None, "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913").unwrap();
        assert_eq!(settings.chain, CreditChain::Base);
        assert_eq!(
            settings.address,
            "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913"
        );
        let sepolia = CreditsSettings::parse(
            Some("base-sepolia"),
            "0x036CbD53842c5426634e7929541eC2318f3dCF7e",
        )
        .unwrap();
        assert_eq!(sepolia.chain.chain_id(), 84532);
        for (chain, address) in [
            (
                Some("ethereum"),
                "0x036CbD53842c5426634e7929541eC2318f3dCF7e",
            ),
            (None, "0x036CbD53"),
            (None, "036CbD53842c5426634e7929541eC2318f3dCF7e"),
            (None, "0xZZ6CbD53842c5426634e7929541eC2318f3dCF7e"),
        ] {
            assert!(CreditsSettings::parse(chain, address).is_err(), "{address}");
        }
    }

    #[test]
    fn an_intent_carries_an_eip681_link_for_its_exact_amount() {
        let top_up = TopUp::from_parts(
            uuid::Uuid::nil(),
            WorkspaceId(uuid::Uuid::nil()),
            TopUpRail::Usdc {
                chain: CreditChain::Base,
                address: "0x00000000000000000000000000000000000000aa".into(),
            },
            5,
            5_001_234,
            TopUpStatus::Open,
            Utc::now(),
            Utc::now(),
            None,
        );
        assert_eq!(top_up.method, TopUpMethod::Usdc);
        assert_eq!(
            top_up.eip681.as_deref().unwrap(),
            "ethereum:0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913@8453/transfer?address=0x00000000000000000000000000000000000000aa&uint256=5001234"
        );
        assert_eq!(top_up.amount, "5.001234");
        assert_eq!(usd_string(-1_500_000), "-1.500000");
    }

    #[test]
    fn random_offsets_stay_in_range() {
        for offset in random_offsets(500) {
            assert!((1..=TOP_UP_OFFSET_MAX).contains(&offset), "{offset}");
        }
    }
}

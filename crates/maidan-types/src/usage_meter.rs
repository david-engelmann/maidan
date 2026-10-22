//! Per-workspace usage meters for the hosted service's free allowance: what
//! each workspace used, per meter, per UTC day. Nothing is cut off or billed
//! on them; they say what a workspace costs.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::WorkspaceId;

/// What is metered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum UsageMeter {
    /// Authenticated HTTP requests.
    Requests,
    /// MCP tool calls.
    McpCalls,
    /// Texts embedded: messages indexed and search queries.
    EmbeddingCalls,
    /// Outbound deliveries sent: Slack, GitHub and Linear egress, email and
    /// web push.
    Deliveries,
    /// Milliseconds of WASI handler execution.
    WasiMs,
    /// Bytes stored for the workspace: its artifacts and its message bodies.
    /// A gauge: the day's row is the latest measurement, not a sum.
    StoredBytes,
}

impl UsageMeter {
    pub const ALL: [UsageMeter; 6] = [
        UsageMeter::Requests,
        UsageMeter::McpCalls,
        UsageMeter::EmbeddingCalls,
        UsageMeter::Deliveries,
        UsageMeter::WasiMs,
        UsageMeter::StoredBytes,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Requests => "requests",
            Self::McpCalls => "mcp_calls",
            Self::EmbeddingCalls => "embedding_calls",
            Self::Deliveries => "deliveries",
            Self::WasiMs => "wasi_ms",
            Self::StoredBytes => "stored_bytes",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|meter| meter.as_str() == s)
    }
}

/// One meter's total for one workspace on one UTC day.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UsageMeterDay {
    pub workspace_id: WorkspaceId,
    pub day: NaiveDate,
    pub meter: UsageMeter,
    pub quantity: i64,
}

/// The instance's free allowance per meter (`MAIDAN_FREE_ALLOWANCE`, e.g.
/// `requests=100000,mcp_calls=50000,stored_bytes=1073741824`). A counter's
/// allowance is per UTC month; `stored_bytes`' is a ceiling on its latest
/// measurement. A meter not named has no allowance. Past it, a workspace is
/// marked and its admins told; nothing is cut off or billed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UsageAllowance {
    pub limits: std::collections::BTreeMap<UsageMeter, i64>,
}

impl UsageAllowance {
    /// Parse the env form. An unknown meter, a repeated one, or a quantity
    /// that is not a non-negative integer is refused, naming it.
    pub fn parse(raw: &str) -> Result<Self, String> {
        let mut limits = std::collections::BTreeMap::new();
        for pair in raw.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            let (name, value) = pair
                .split_once('=')
                .ok_or_else(|| format!("`{pair}` is not meter=quantity"))?;
            let meter = UsageMeter::parse(name.trim())
                .ok_or_else(|| format!("`{}` is not a usage meter", name.trim()))?;
            let quantity = value
                .trim()
                .parse::<i64>()
                .ok()
                .filter(|q| *q >= 0)
                .ok_or_else(|| format!("`{}` is not a quantity for {name}", value.trim()))?;
            if limits.insert(meter, quantity).is_some() {
                return Err(format!("{name} is named twice"));
            }
        }
        Ok(Self { limits })
    }

    pub fn limit(&self, meter: UsageMeter) -> Option<i64> {
        self.limits.get(&meter).copied()
    }
}

/// Whether a meter is a gauge (its month's figure is its latest day's value)
/// rather than a counter (its month's figure is the sum of its days).
impl UsageMeter {
    pub fn is_gauge(self) -> bool {
        matches!(self, Self::StoredBytes)
    }
}

/// A workspace's month on one meter, against the allowance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MeterUsage {
    pub meter: UsageMeter,
    /// The month's sum for a counter, the latest measurement for a gauge.
    pub quantity: i64,
    /// The instance allowance, when it sets one for this meter.
    pub allowance: Option<i64>,
    pub over: bool,
}

/// A workspace's usage for one UTC month.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MonthlyUsage {
    pub workspace_id: WorkspaceId,
    /// `YYYY-MM`.
    pub month: String,
    pub meters: Vec<MeterUsage>,
}

/// A recorded crossing of the allowance: once per workspace, meter and month.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OverAllowance {
    pub workspace_id: WorkspaceId,
    pub meter: UsageMeter,
    pub month: String,
    pub quantity: i64,
    pub allowance: i64,
    pub marked_at: chrono::DateTime<chrono::Utc>,
}

/// A UTC month as `YYYY-MM`, with its first day and the first day after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsageMonth {
    pub first: NaiveDate,
    pub next: NaiveDate,
}

impl UsageMonth {
    pub fn of(day: NaiveDate) -> Self {
        use chrono::Datelike;
        let first = day.with_day(1).unwrap_or(day);
        let next = first
            .checked_add_months(chrono::Months::new(1))
            .unwrap_or(first);
        Self { first, next }
    }

    /// Parse `YYYY-MM`.
    pub fn parse(raw: &str) -> Option<Self> {
        NaiveDate::parse_from_str(&format!("{}-01", raw.trim()), "%Y-%m-%d")
            .ok()
            .map(Self::of)
    }

    pub fn label(self) -> String {
        self.first.format("%Y-%m").to_string()
    }

    /// The month's figure per meter from its days' rows: a counter's sum, a
    /// gauge's latest day.
    pub fn totals(self, rows: &[UsageMeterDay]) -> std::collections::BTreeMap<UsageMeter, i64> {
        let mut totals = std::collections::BTreeMap::new();
        let mut latest: std::collections::BTreeMap<UsageMeter, NaiveDate> =
            std::collections::BTreeMap::new();
        for row in rows
            .iter()
            .filter(|r| r.day >= self.first && r.day < self.next)
        {
            if row.meter.is_gauge() {
                if latest.get(&row.meter).is_none_or(|d| row.day >= *d) {
                    latest.insert(row.meter, row.day);
                    totals.insert(row.meter, row.quantity);
                }
            } else {
                *totals.entry(row.meter).or_insert(0) += row.quantity;
            }
        }
        totals
    }

    /// Every metered meter of `rows`, against `allowance`.
    pub fn usage(
        self,
        workspace_id: WorkspaceId,
        rows: &[UsageMeterDay],
        allowance: &UsageAllowance,
    ) -> MonthlyUsage {
        let totals = self.totals(rows);
        MonthlyUsage {
            workspace_id,
            month: self.label(),
            meters: UsageMeter::ALL
                .into_iter()
                .map(|meter| {
                    let quantity = totals.get(&meter).copied().unwrap_or(0);
                    let limit = allowance.limit(meter);
                    MeterUsage {
                        meter,
                        quantity,
                        allowance: limit,
                        over: limit.is_some_and(|l| quantity > l),
                    }
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_allowance_parses_strictly() {
        let allowance = UsageAllowance::parse(" requests=10, stored_bytes = 5 ").unwrap();
        assert_eq!(allowance.limit(UsageMeter::Requests), Some(10));
        assert_eq!(allowance.limit(UsageMeter::StoredBytes), Some(5));
        assert_eq!(allowance.limit(UsageMeter::McpCalls), None);
        assert_eq!(
            UsageAllowance::parse("").unwrap(),
            UsageAllowance::default()
        );
        for bad in [
            "requests",
            "requests=-1",
            "requests=lots",
            "tokens=1",
            "requests=1,requests=2",
        ] {
            assert!(UsageAllowance::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_month_sums_counters_and_takes_a_gauges_latest_day() {
        let ws = WorkspaceId(uuid::Uuid::nil());
        let day = |d| NaiveDate::from_ymd_opt(2026, 10, d).unwrap();
        let row = |d, meter, quantity| UsageMeterDay {
            workspace_id: ws,
            day: day(d),
            meter,
            quantity,
        };
        let month = UsageMonth::parse("2026-10").unwrap();
        assert_eq!(month.label(), "2026-10");
        let rows = [
            row(1, UsageMeter::Requests, 4),
            row(2, UsageMeter::Requests, 6),
            row(1, UsageMeter::StoredBytes, 100),
            row(3, UsageMeter::StoredBytes, 70),
            UsageMeterDay {
                day: NaiveDate::from_ymd_opt(2026, 11, 1).unwrap(),
                ..row(1, UsageMeter::Requests, 50)
            },
        ];
        let allowance = UsageAllowance::parse("requests=9,stored_bytes=80").unwrap();
        let usage = month.usage(ws, &rows, &allowance);
        let get = |m| usage.meters.iter().find(|u| u.meter == m).cloned().unwrap();
        assert_eq!(
            (
                get(UsageMeter::Requests).quantity,
                get(UsageMeter::Requests).over
            ),
            (10, true)
        );
        assert_eq!(
            (
                get(UsageMeter::StoredBytes).quantity,
                get(UsageMeter::StoredBytes).over
            ),
            (70, false)
        );
        assert_eq!(get(UsageMeter::McpCalls).allowance, None);
    }
}

//! Flushing the usage meters. Requests (counted in the auth layer) and MCP
//! calls (counted in the MCP server) accumulate in
//! [`maidan_store::usage_meters::UsageMeterBuffer`]; this loop writes them to
//! the per-workspace daily rollup in one batch every interval, and once more
//! on shutdown. A crash between flushes loses at most one interval's counts.

use std::time::Duration;

use crate::state::AppState;

/// Default flush interval (`MAIDAN_USAGE_METER_FLUSH_SECS`).
pub const DEFAULT_FLUSH_SECS: u64 = 60;

/// The flush interval from the environment: the default when unset or
/// unparsable, at least one second.
pub fn interval_from_env() -> Duration {
    interval_from_raw(std::env::var("MAIDAN_USAGE_METER_FLUSH_SECS").ok())
}

fn interval_from_raw(raw: Option<String>) -> Duration {
    let secs = raw
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_FLUSH_SECS)
        .max(1);
    Duration::from_secs(secs)
}

/// The instance's free allowance from `MAIDAN_FREE_ALLOWANCE`. Unset is no
/// allowance; a malformed value refuses boot, naming what is wrong.
pub fn allowance_from_env() -> Result<maidan_types::UsageAllowance, String> {
    match std::env::var("MAIDAN_FREE_ALLOWANCE") {
        Ok(raw) => maidan_types::UsageAllowance::parse(&raw)
            .map_err(|err| format!("MAIDAN_FREE_ALLOWANCE: {err}")),
        Err(_) => Ok(maidan_types::UsageAllowance::default()),
    }
}

/// Paths that are not metered: probes and the scrape, which say nothing
/// about what a workspace uses.
pub fn unmetered(path: &str) -> bool {
    path == "/metrics" || path == "/health" || path.starts_with("/health/")
}

/// Paths an open sign-up workspace past its request allowance may still
/// read, so its admin can see why it was cut off: its usage, and its
/// members' notifications (where the crossing notice lands). Reads only.
pub fn readable_past_cutoff(method: &axum::http::Method, matched: Option<&str>) -> bool {
    method == axum::http::Method::GET
        && matches!(
            matched,
            Some("/workspaces/{id}/usage-meters")
                | Some("/members/{id}/notifications")
                | Some("/members/{id}/notifications/grouped")
        )
}

/// How long background work for a cut-off workspace (an egress delivery, a
/// web push) is parked before it is looked at again. Long enough not to spin
/// on it; the cutoff set itself is refreshed every tick, so work resumes
/// within this once the allowance is raised or a new month starts.
pub const CUT_OFF_DEFER: chrono::Duration = chrono::Duration::minutes(15);

/// Whether the workspace of `member_id` is cut off on `meter`. A failed
/// member lookup is not a cutoff.
pub async fn member_cut_off(
    state: &AppState,
    member_id: maidan_types::MemberId,
    meter: maidan_types::UsageMeter,
) -> bool {
    match state.store.get_member(member_id).await {
        Ok(member) => state.usage_meters.is_cut_off(member.workspace_id, meter),
        Err(_) => false,
    }
}

/// The refusal for metered work past the free allowance: `429 Too Many
/// Requests`, naming the meter. Only an open sign-up workspace is refused.
pub fn cut_off_error(meter: maidan_types::UsageMeter) -> crate::error::ApiError {
    crate::error::ApiError::TooManyRequests(format!(
        "this workspace is past its free allowance for {} this month",
        meter.as_str()
    ))
}

/// Reload the hard allowance cutoff from the store: the current month's
/// crossings of open sign-up workspaces, on meters the allowance still
/// limits. Run each tick on every replica, so a crossing one replica marks
/// cuts the workspace off everywhere within an interval. A failed read keeps
/// the set as it was.
pub async fn refresh_cut_off(state: &AppState) -> usize {
    if state.usage_allowance.limits.is_empty() {
        state.usage_meters.set_cut_off(std::iter::empty());
        return 0;
    }
    let label = maidan_types::UsageMonth::of(chrono::Utc::now().date_naive()).label();
    match state.store.list_usage_cut_off(&label).await {
        Ok(pairs) => {
            let pairs: Vec<_> = pairs
                .into_iter()
                .filter(|(_, meter)| state.usage_allowance.limit(*meter).is_some())
                .collect();
            let n = pairs.len();
            state.usage_meters.set_cut_off(pairs);
            n
        }
        Err(err) => {
            tracing::warn!(error = %err, "usage meters: allowance cutoff not refreshed");
            0
        }
    }
}

/// Count for the workspace of `member_id`, for work known by its member
/// (a web push). A failed member lookup counts nothing.
pub async fn record_for_member(
    state: &AppState,
    member_id: maidan_types::MemberId,
    meter: maidan_types::UsageMeter,
    quantity: i64,
) {
    match state.store.get_member(member_id).await {
        Ok(member) => state
            .usage_meters
            .record(member.workspace_id, meter, quantity),
        Err(err) => tracing::debug!(error = %err, "usage meters: member not found; not counted"),
    }
}

/// The workspace's admins, to be told of a crossing: members holding a live
/// `token:admin` token.
async fn workspace_admins(
    state: &AppState,
    workspace_id: maidan_types::WorkspaceId,
) -> Result<Vec<maidan_types::MemberId>, maidan_store::StoreError> {
    let now = chrono::Utc::now();
    let mut admins = Vec::new();
    for member in state.store.list_members(workspace_id).await? {
        let live_admin = state
            .store
            .list_api_tokens_for_member(workspace_id, member.id)
            .await?
            .iter()
            .any(|t| {
                t.revoked_at.is_none()
                    && t.expires_at.is_none_or(|at| at > now)
                    && t.capabilities
                        .iter()
                        .any(|c| c == maidan_auth::capability::TOKEN_ADMIN)
            });
        if live_admin {
            admins.push(member.id);
        }
    }
    Ok(admins)
}

/// Mark each of `touched` past the allowance on a meter in the month its
/// rows were written for, once, telling its admins. The month is the rows'
/// day, not the check's clock, so a count from the last minutes of a month
/// flushed after midnight UTC is judged against the month it belongs to.
/// Returns how many were marked.
pub async fn check_allowance(
    state: &AppState,
    touched: impl IntoIterator<Item = (maidan_types::WorkspaceId, maidan_types::UsageMonth)>,
) -> usize {
    if state.usage_allowance.limits.is_empty() {
        return 0;
    }
    let touched: std::collections::HashSet<_> = touched
        .into_iter()
        .map(|(workspace_id, month)| (workspace_id, month.first))
        .collect();
    let mut marked = 0;
    for (workspace_id, first) in touched {
        let month = maidan_types::UsageMonth::of(first);
        let label = month.label();
        if let Err(err) = check_workspace(state, workspace_id, month, &label, &mut marked).await {
            tracing::warn!(error = %err, workspace_id = %workspace_id.0, "usage meters: allowance not checked");
        }
    }
    marked
}

async fn check_workspace(
    state: &AppState,
    workspace_id: maidan_types::WorkspaceId,
    month: maidan_types::UsageMonth,
    label: &str,
    marked: &mut usize,
) -> Result<(), maidan_store::StoreError> {
    let rows = state
        .store
        .list_usage_meters(workspace_id, month.first)
        .await?;
    let usage = month.usage(workspace_id, &rows, &state.usage_allowance);
    let over: Vec<_> = usage.meters.into_iter().filter(|m| m.over).collect();
    if over.is_empty() {
        return Ok(());
    }
    let already = state.store.list_over_allowance(workspace_id, label).await?;
    let mut admins = None;
    for meter in over {
        if already.iter().any(|m| m.meter == meter.meter) {
            continue;
        }
        let Some(allowance) = meter.allowance else {
            continue;
        };
        let admins = match &admins {
            Some(admins) => admins,
            None => admins.insert(workspace_admins(state, workspace_id).await?),
        };
        let mark = maidan_types::OverAllowance {
            workspace_id,
            meter: meter.meter,
            month: label.to_string(),
            quantity: meter.quantity,
            allowance,
            marked_at: chrono::Utc::now(),
        };
        if let Some(stored) = state.store.mark_over_allowance(&mark, admins).await? {
            crate::routes::publish_stored(state, stored).await;
            *marked += 1;
        }
    }
    Ok(())
}

/// Write everything counted since the last flush. A failed write puts the
/// batch back, so the next flush carries it; a write that committed but whose
/// answer was lost is then counted twice, which an allowance can bear. A count
/// for a workspace erased since is dropped by the store. Returns how many
/// workspaces it wrote for.
pub async fn flush_once(state: &AppState) -> usize {
    distinct(&flush(state).await)
}

/// [`flush_once`], answering the workspaces it wrote for.
async fn flush(state: &AppState) -> Touched {
    let batch = state.usage_meters.take();
    if batch.is_empty() {
        return Default::default();
    }
    match state.store.add_usage_meters(&batch).await {
        Ok(()) => touched(&batch),
        Err(err) => {
            tracing::warn!(error = %err, rows = batch.len(), "usage meters: flush failed; kept for the next one");
            state.usage_meters.restore(batch);
            Default::default()
        }
    }
}

/// The workspaces a write touched, each with the month of the rows written.
type Touched = Vec<(maidan_types::WorkspaceId, maidan_types::UsageMonth)>;

fn touched(batch: &[maidan_types::UsageMeterDay]) -> Touched {
    batch
        .iter()
        .map(|row| (row.workspace_id, maidan_types::UsageMonth::of(row.day)))
        .collect()
}

/// Workspaces measured for stored bytes per tick, so a large instance's
/// first pass of the day is spread over several ticks.
pub const MEASURE_PER_TICK: i64 = 25;

/// Measure what each workspace stores, once a UTC day: the workspaces with no
/// `stored_bytes` row today, up to [`MEASURE_PER_TICK`]. Returns how many were
/// measured. A failed measurement is retried next tick.
pub async fn measure_stored_bytes(state: &AppState) -> usize {
    distinct(&measure(state).await)
}

/// [`measure_stored_bytes`], answering the workspaces it measured.
async fn measure(state: &AppState) -> Touched {
    let today = chrono::Utc::now().date_naive();
    let meter = maidan_types::UsageMeter::StoredBytes;
    let due = match state
        .store
        .list_unmeasured_workspaces(today, meter, MEASURE_PER_TICK)
        .await
    {
        Ok(due) => due,
        Err(err) => {
            tracing::warn!(error = %err, "usage meters: workspaces to measure not read");
            return Vec::new();
        }
    };
    let mut batch = Vec::with_capacity(due.len());
    for workspace_id in due {
        match state.store.workspace_stored_bytes(workspace_id).await {
            Ok(quantity) => batch.push(maidan_types::UsageMeterDay {
                workspace_id,
                day: today,
                meter,
                quantity,
            }),
            Err(err) => {
                tracing::warn!(error = %err, workspace_id = %workspace_id.0, "usage meters: stored bytes not measured");
            }
        }
    }
    match state.store.set_usage_meters(&batch).await {
        Ok(()) => touched(&batch),
        Err(err) => {
            tracing::warn!(error = %err, "usage meters: stored bytes not written");
            Vec::new()
        }
    }
}

/// One tick: flush the counters, measure stored bytes where due, then check
/// the workspaces either touched against the allowance. Answers the
/// workspaces flushed, measured and marked.
pub async fn tick(state: &AppState) -> (usize, usize, usize) {
    let mut touched = flush(state).await;
    let flushed = distinct(&touched);
    let measured = measure(state).await;
    let measured_count = distinct(&measured);
    touched.extend(measured);
    let marked = check_allowance(state, touched).await;
    refresh_cut_off(state).await;
    (flushed, measured_count, marked)
}

fn distinct(touched: &Touched) -> usize {
    touched
        .iter()
        .map(|(workspace_id, _)| *workspace_id)
        .collect::<std::collections::HashSet<_>>()
        .len()
}

/// The flush loop, spawned by `main.rs`.
pub async fn run(state: AppState, interval: Duration) {
    tracing::info!(
        interval_secs = interval.as_secs(),
        "usage meter flush started"
    );
    let _watched = crate::ops_alert::start_loop("usage_meters", interval);
    // A restarted replica refuses what the store already says is cut off,
    // rather than waiting out its first interval.
    refresh_cut_off(&state).await;
    loop {
        tokio::time::sleep(interval).await;
        if let Some((flushed, measured, marked)) =
            crate::ops_alert::guarded_tick("usage_meters", tick(&state)).await
        {
            tracing::debug!(flushed, measured, marked, "usage meters tick");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flush_interval_defaults_and_is_at_least_a_second() {
        let default = Duration::from_secs(DEFAULT_FLUSH_SECS);
        assert_eq!(interval_from_raw(None), default);
        assert_eq!(interval_from_raw(Some("soon".into())), default);
        assert_eq!(
            interval_from_raw(Some(" 15 ".into())),
            Duration::from_secs(15)
        );
        assert_eq!(interval_from_raw(Some("0".into())), Duration::from_secs(1));
    }

    #[test]
    fn probes_and_the_scrape_are_not_metered() {
        for path in ["/metrics", "/health", "/health/live", "/health/ready"] {
            assert!(unmetered(path), "{path}");
        }
        for path in ["/workspaces", "/mcp", "/healthcheck-like"] {
            assert!(!unmetered(path), "{path}");
        }
    }
}

//! The `pre-review` commit status a review thread's result posts beside its
//! GitHub comment.
//!
//! A review agent claims a review thread and sets a result naming its
//! `verdict` and a `check`. The comment carries the verdict for people; a
//! merge gate that reads a `pre-review` status needs it on the commit. That
//! status is a gate, so it is posted only for a reporter an operator granted
//! the `pre-review` governance skill, and everything a model controls is held
//! to it here: only
//! the one allowed context, only `PASS` or `FIX-FIRST`, only at the head the
//! review was briefed on, and with a description Maidan builds from what it
//! knows itself (the reporting member's handle and the provider family its
//! own usage names). Anything else posts nothing and leaves a warning on the
//! delivery row. The comment is delivered either way.

use maidan_types::{
    is_git_sha, ChangeCheck, CommitStatusState, EgressOutbox, EgressTarget, TaskKind, ThreadResult,
    WaiterResult, CHANGE_CHECK_DESCRIPTION_MAX_CHARS, PRE_REVIEW_SKILL, PROVIDER_FAMILIES,
};

use crate::github::GithubSender;
use crate::state::AppState;

/// The commit status contexts a review result may post. One entry on
/// purpose: the merge gate reading `pre-review` takes it to mean "a reviewer
/// of another family passed this head", and the other contexts on the same
/// commit (a stamp, a security scan, a review lane) are gates a review does
/// not own. Mapping or trimming a context a model wrote could let its output
/// pass one of those, so anything not listed here exactly is refused.
pub const REVIEW_STATUS_CONTEXTS: &[&str] = &[PRE_REVIEW_CONTEXT];

/// The context a review's status is posted under.
pub const PRE_REVIEW_CONTEXT: &str = "pre-review";

/// The line in a review thread's description naming the head the review was
/// briefed on. Review children Maidan spawns write it, and threads filed
/// from outside use the same line.
pub const BRIEFED_HEAD_PREFIX: &str = "Head to review:";

/// The `check.context` is not in [`REVIEW_STATUS_CONTEXTS`].
pub const PRE_REVIEW_CONTEXT_REFUSED: &str = "pre_review_context_refused";
/// The result's `status` is not `reviewed`, so it carries no verdict.
pub const PRE_REVIEW_NOT_REVIEWED: &str = "pre_review_not_reviewed";
/// The `verdict` is not exactly `PASS` or `FIX-FIRST`.
pub const PRE_REVIEW_VERDICT_REFUSED: &str = "pre_review_verdict_refused";
/// The thread names no head it was briefed on, or the result's head is not it.
pub const PRE_REVIEW_HEAD_REFUSED: &str = "pre_review_head_refused";
/// The member that set the result could not be read in the thread's workspace.
pub const PRE_REVIEW_REPORTER_UNKNOWN: &str = "pre_review_reporter_unknown";
/// The reporter does not hold the [`PRE_REVIEW_SKILL`] governance skill.
pub const PRE_REVIEW_SKILL_MISSING: &str = "pre_review_skill_missing";
/// Neither the reporter's usage nor the `result_kind` names a known family.
pub const PRE_REVIEW_FAMILY_UNKNOWN: &str = "pre_review_family_unknown";
/// The description could not be built in the form the gate parses.
pub const PRE_REVIEW_DESCRIPTION_REFUSED: &str = "pre_review_description_refused";
/// GitHub did not take the status, or this sender cannot post one.
pub const PRE_REVIEW_NOT_POSTED: &str = "pre_review_not_posted";

/// How many of the thread's usage records are read to find the reporter's
/// latest: the store's own cap on one page of a thread's ledger.
const USAGE_PAGE: i64 = 100;

/// The longest detail a warning keeps: it can echo text a model wrote.
const WARNING_DETAIL_MAX_CHARS: usize = 160;

/// A review's verdict, as the result names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    FixFirst,
}

impl Verdict {
    /// Exactly `PASS` or `FIX-FIRST`. No case folding or trimming: the gate
    /// compares the words, and a verdict a model spelled another way is not
    /// one Maidan should guess at.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "PASS" => Some(Self::Pass),
            "FIX-FIRST" => Some(Self::FixFirst),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::FixFirst => "FIX-FIRST",
        }
    }

    /// The status state: the verdict decides it, never the result's
    /// `check.state`, so a `FIX-FIRST` cannot be posted as a success.
    pub fn state(self) -> CommitStatusState {
        match self {
            Self::Pass => CommitStatusState::Success,
            Self::FixFirst => CommitStatusState::Failure,
        }
    }
}

/// Whether `context` is one a review result may post.
pub fn context_allowed(context: &str) -> bool {
    REVIEW_STATUS_CONTEXTS.contains(&context)
}

/// The head a thread's description says the review was briefed on: the
/// full SHA on its `Head to review:` line, lowercased. `None` when there is
/// no such line, when it holds no full SHA, or when two lines name different
/// heads, since then the brief does not say which one was reviewed.
pub fn briefed_head(description: &str) -> Option<String> {
    let mut heads = description
        .lines()
        .filter_map(|line| line.trim().strip_prefix(BRIEFED_HEAD_PREFIX))
        .map(str::trim);
    let head = heads.next()?;
    if !is_git_sha(head) || heads.any(|other| !other.eq_ignore_ascii_case(head)) {
        return None;
    }
    Some(head.to_ascii_lowercase())
}

/// The family a `result_kind` names before its first `.`
/// (`deepseek.redteam/1` is `deepseek`), when that is a family Maidan knows.
pub fn family_from_result_kind(result_kind: &str) -> Option<&'static str> {
    let (prefix, _) = result_kind.split_once('.')?;
    PROVIDER_FAMILIES
        .iter()
        .map(|(family, _, _)| *family)
        .find(|family| *family == prefix)
}

/// A name the gate's pattern can carry: an ASCII letter or digit, then
/// letters, digits, spaces, `.`, `_` or `-`.
fn is_plain_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '_' | '-'))
}

/// The status description: `<VERDICT> at <sha9> by <family> as <session>`.
/// The gate parses it with `^PASS at <sha9> by [A-Za-z0-9][A-Za-z0-9 ._-]*$`,
/// so a family or handle outside that alphabet, or a line longer than GitHub
/// keeps, is refused rather than cut or cleaned into a different name.
pub fn status_description(
    verdict: Verdict,
    head: &str,
    family: &str,
    session: &str,
) -> Result<String, String> {
    if !is_git_sha(head) {
        return Err(format!("`{head}` is not a full commit SHA"));
    }
    if !is_plain_name(family) {
        return Err(format!("family `{family}` is not a plain name"));
    }
    if !is_plain_name(session) {
        return Err(format!("handle `{session}` is not a plain name"));
    }
    let sha9 = head[..9].to_ascii_lowercase();
    let description = format!("{} at {sha9} by {family} as {session}", verdict.as_str());
    if description.chars().count() > CHANGE_CHECK_DESCRIPTION_MAX_CHARS {
        return Err(format!(
            "the description is longer than {CHANGE_CHECK_DESCRIPTION_MAX_CHARS} characters"
        ));
    }
    Ok(description)
}

/// A warning for the delivery row: its code, then a detail. The row stores
/// its warnings joined by commas, so a comma in the detail would split it;
/// line breaks would break the one-line display.
fn warning(code: &str, detail: &str) -> String {
    let detail: String = detail
        .chars()
        .map(|c| match c {
            ',' => ';',
            c if c.is_control() => ' ',
            c => c,
        })
        .take(WARNING_DETAIL_MAX_CHARS)
        .collect();
    format!("{code}: {detail}")
}

/// What the result asks for and whether it may have it, from the envelope
/// alone: the context, the status and the verdict. `Ok(None)` when the result
/// names no `check`, which asks for no status.
fn requested(waiter: &WaiterResult, raw: &serde_json::Value) -> Result<Option<Verdict>, String> {
    let Some(check) = raw.get("check") else {
        return Ok(None);
    };
    let context = check.get("context").and_then(serde_json::Value::as_str);
    match context {
        Some(context) if context_allowed(context) => {}
        Some(context) => return Err(warning(PRE_REVIEW_CONTEXT_REFUSED, context)),
        None => return Err(warning(PRE_REVIEW_CONTEXT_REFUSED, "(none)")),
    }
    if !waiter.is_reviewed() {
        return Err(warning(
            PRE_REVIEW_NOT_REVIEWED,
            &format!("status {}", waiter.status),
        ));
    }
    let verdict = raw.get("verdict").and_then(serde_json::Value::as_str);
    match verdict.and_then(Verdict::parse) {
        Some(verdict) => Ok(Some(verdict)),
        None => Err(warning(
            PRE_REVIEW_VERDICT_REFUSED,
            verdict.unwrap_or("(none)"),
        )),
    }
}

/// The head the status goes on: the briefed one, when the result says it
/// reviewed that head. Otherwise a warning naming both.
fn reviewed_head(briefed: Option<&str>, reviewed: Option<&str>) -> Result<String, String> {
    match (briefed, reviewed) {
        (Some(briefed), Some(reviewed)) if briefed.eq_ignore_ascii_case(reviewed) => {
            Ok(briefed.to_string())
        }
        (briefed, reviewed) => Err(warning(
            PRE_REVIEW_HEAD_REFUSED,
            &format!(
                "briefed {} but the result reviewed {}",
                briefed.unwrap_or("no head"),
                reviewed.unwrap_or("no head")
            ),
        )),
    }
}

/// After a review thread's result comment landed on `repo`: post its
/// `pre-review` status when every rule holds, and record on the delivery
/// row what was refused or failed, replacing the last round's. Never fails
/// the delivery: the comment has landed, and failing here would retry it.
/// A thread that is not a review is left exactly as it was.
pub(crate) async fn post_status(
    state: &AppState,
    sender: &dyn GithubSender,
    entry: &EgressOutbox,
    repo: &str,
    issue_number: i64,
) {
    match state.store.get_thread_attributes(entry.thread_id).await {
        Ok(attributes) if attributes.kind == TaskKind::Review => {}
        Ok(_) => return,
        Err(err) => {
            tracing::warn!(error = %err, thread_id = ?entry.thread_id, "pre-review status: thread attributes not read; nothing posted");
            return;
        }
    }
    let warnings: Vec<String> = match decide_and_post(state, sender, entry, repo).await {
        Ok(Some(description)) => {
            tracing::info!(thread_id = ?entry.thread_id, %repo, %description, "pre-review status: posted");
            Vec::new()
        }
        Ok(None) => Vec::new(),
        Err(warning) => {
            tracing::warn!(thread_id = ?entry.thread_id, %repo, %warning, "pre-review status: not posted; the comment is delivered");
            vec![warning]
        }
    };
    let target = EgressTarget::Github {
        repo: repo.to_string(),
        issue_number,
    };
    if let Err(err) = state
        .store
        .set_result_delivery_warnings(entry.thread_id, &target, &warnings)
        .await
    {
        tracing::warn!(error = %err, thread_id = ?entry.thread_id, "pre-review status: delivery warnings not recorded");
    }
}

/// Every rule, in order, then the post. `Ok(Some(description))` when posted,
/// `Ok(None)` when the result asked for no status, `Err(warning)` otherwise.
async fn decide_and_post(
    state: &AppState,
    sender: &dyn GithubSender,
    entry: &EgressOutbox,
    repo: &str,
) -> Result<Option<String>, String> {
    let read_failed = |what: &str, err: &dyn std::fmt::Display| {
        warning(PRE_REVIEW_NOT_POSTED, &format!("{what} not read: {err}"))
    };
    let stored = match state.store.get_thread_result(entry.thread_id).await {
        Ok(Some(stored)) => stored,
        Ok(None) => return Ok(None),
        Err(err) => return Err(read_failed("the result", &err)),
    };
    let Some(waiter) = maidan_types::parse_waiter_result(&stored.result) else {
        return Ok(None);
    };
    let Some(verdict) = requested(&waiter, &stored.result)? else {
        return Ok(None);
    };
    let thread = state
        .store
        .get_thread(entry.thread_id)
        .await
        .map_err(|err| read_failed("the thread", &err))?;
    let briefed = thread.description.as_deref().and_then(briefed_head);
    let head = reviewed_head(briefed.as_deref(), waiter.head_sha.as_deref())?;
    // In the thread's own workspace: a member id that names someone in
    // another workspace is not this thread's reporter.
    let reporter = state
        .store
        .get_member_in(entry.workspace_id, stored.produced_by)
        .await
        .map_err(|err| warning(PRE_REVIEW_REPORTER_UNKNOWN, &err.to_string()))?;
    // The status is a merge gate's input, so posting it is approval
    // authority: only a member an operator granted the governance skill may,
    // never one that declared a routing tag about itself.
    let skills = state
        .store
        .list_member_skills(reporter.id)
        .await
        .map_err(|err| read_failed("the reporter's skills", &err))?;
    if !skills.iter().any(|held| held.skill == PRE_REVIEW_SKILL) {
        return Err(warning(
            PRE_REVIEW_SKILL_MISSING,
            &format!(
                "{} does not hold the {PRE_REVIEW_SKILL} skill",
                reporter.handle
            ),
        ));
    }
    let family = reporter_family(state, entry, &stored, &waiter).await?;
    let description = status_description(verdict, &head, family, &reporter.handle)
        .map_err(|why| warning(PRE_REVIEW_DESCRIPTION_REFUSED, &why))?;
    if crate::egress_guard::first_credential([description.as_str()]).is_some() {
        return Err(warning(
            PRE_REVIEW_DESCRIPTION_REFUSED,
            "it reads as a credential",
        ));
    }
    let Some(git) = sender.git() else {
        return Err(warning(
            PRE_REVIEW_NOT_POSTED,
            "this GitHub sender cannot post commit statuses",
        ));
    };
    let check = ChangeCheck {
        context: PRE_REVIEW_CONTEXT.to_string(),
        state: verdict.state(),
        description: Some(description.clone()),
        target_url: None,
    };
    git.create_commit_status(repo, &head, &check)
        .await
        .map_err(|err| warning(PRE_REVIEW_NOT_POSTED, &err.to_string()))?;
    Ok(Some(description))
}

/// The reporter's own provider family: from its latest usage record on the
/// thread, paid by the thread's workspace, else from the `result_kind`
/// prefix. Not the thread's `review_family`: that label names the first
/// family that is not the author's, which says nothing about who reviewed.
async fn reporter_family(
    state: &AppState,
    entry: &EgressOutbox,
    stored: &ThreadResult,
    waiter: &WaiterResult,
) -> Result<&'static str, String> {
    let usage = state
        .store
        .list_thread_usage_ledger(entry.thread_id, USAGE_PAGE)
        .await
        .map_err(|err| warning(PRE_REVIEW_NOT_POSTED, &format!("usage not read: {err}")))?;
    usage
        .iter()
        .find(|record| {
            record.stamp.reporter == stored.produced_by && record.stamp.payer == entry.workspace_id
        })
        .and_then(|record| {
            maidan_types::provider_family(record.evidence.provider.as_deref(), &record.stamp.model)
        })
        .or_else(|| family_from_result_kind(&waiter.result_kind))
        .ok_or_else(|| {
            warning(
                PRE_REVIEW_FAMILY_UNKNOWN,
                &format!(
                    "no usage by the reporter names one and `{}` does not",
                    waiter.result_kind
                ),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const HEAD: &str = "0123456789abcdef0123456789abcdef01234567";

    /// The gate's pattern, `^<VERDICT> at <sha9> by [A-Za-z0-9][A-Za-z0-9 ._-]*$`,
    /// matched by hand: the crate has no regex engine to spare.
    fn gate_matches(description: &str, verdict: &str, sha9: &str) -> bool {
        let Some(rest) = description.strip_prefix(&format!("{verdict} at {sha9} by ")) else {
            return false;
        };
        let mut chars = rest.chars();
        chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
            && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '_' | '-'))
    }

    #[test]
    fn only_pre_review_is_an_allowed_context() {
        assert!(context_allowed("pre-review"));
        for refused in [
            "stamp",
            "security",
            "review-lane",
            "Pre-Review",
            " pre-review",
            "pre-review ",
            "pre-review/x",
            "",
        ] {
            assert!(!context_allowed(refused), "{refused:?}");
        }
    }

    #[test]
    fn a_pass_is_success_a_fix_first_is_failure_and_nothing_else_is_a_verdict() {
        assert_eq!(
            Verdict::parse("PASS").map(Verdict::state),
            Some(CommitStatusState::Success)
        );
        assert_eq!(
            Verdict::parse("FIX-FIRST").map(Verdict::state),
            Some(CommitStatusState::Failure)
        );
        for refused in ["pass", "Pass", " PASS", "FIX_FIRST", "FAIL", "APPROVE", ""] {
            assert_eq!(Verdict::parse(refused), None, "{refused:?}");
        }
    }

    #[test]
    fn the_description_is_built_in_the_form_the_gate_parses() {
        let pass = status_description(Verdict::Pass, HEAD, "deepseek", "seat-2.reviewer_a")
            .expect("built");
        assert_eq!(pass, "PASS at 012345678 by deepseek as seat-2.reviewer_a");
        assert!(gate_matches(&pass, "PASS", "012345678"), "{pass}");
        let fix = status_description(Verdict::FixFirst, &HEAD.to_uppercase(), "openai", "r1")
            .expect("built");
        assert_eq!(fix, "FIX-FIRST at 012345678 by openai as r1");
        assert!(gate_matches(&fix, "FIX-FIRST", "012345678"), "{fix}");
    }

    #[test]
    fn a_handle_or_family_that_would_break_the_gate_pattern_builds_no_description() {
        for handle in ["", "bot@host", "a/b", "-lead", "ré", "x\ty", "a,b"] {
            let built = status_description(Verdict::Pass, HEAD, "anthropic", handle);
            assert!(built.is_err(), "{handle:?} built {built:?}");
        }
        for family in ["", "deep$eek", " openai", "x\ny"] {
            let built = status_description(Verdict::Pass, HEAD, family, "r1");
            assert!(built.is_err(), "{family:?} built {built:?}");
        }
        let long = "r".repeat(CHANGE_CHECK_DESCRIPTION_MAX_CHARS);
        assert!(status_description(Verdict::Pass, HEAD, "openai", &long).is_err());
        assert!(status_description(Verdict::Pass, "abc123", "openai", "r1").is_err());
    }

    #[test]
    fn the_briefed_head_is_read_from_its_line_in_the_description() {
        let upper = HEAD.to_uppercase();
        let description = format!("Pull request: acme/app#42\nHead to review: {upper}\n");
        assert_eq!(briefed_head(&description).as_deref(), Some(HEAD));
        assert_eq!(
            briefed_head(&format!("  Head to review:   {HEAD}  ")).as_deref(),
            Some(HEAD)
        );
        assert_eq!(briefed_head("Pull request: acme/app#42"), None);
        assert_eq!(briefed_head("Head to review: 0123456"), None);
        assert_eq!(briefed_head(&format!("Head to review {HEAD}")), None);
        let other = "f".repeat(40);
        assert_eq!(
            briefed_head(&format!("Head to review: {HEAD}\nHead to review: {other}")),
            None
        );
        assert_eq!(
            briefed_head(&format!("Head to review: {HEAD}\nHead to review: {upper}")).as_deref(),
            Some(HEAD)
        );
    }

    #[test]
    fn a_result_kind_names_a_family_only_by_a_known_prefix() {
        assert_eq!(
            family_from_result_kind("deepseek.redteam/1"),
            Some("deepseek")
        );
        assert_eq!(family_from_result_kind("openai.review/2"), Some("openai"));
        assert_eq!(family_from_result_kind("example.review.result/1"), None);
        assert_eq!(family_from_result_kind("deepseek"), None);
        assert_eq!(family_from_result_kind("DeepSeek.redteam/1"), None);
    }

    #[test]
    fn a_head_other_than_the_briefed_one_is_refused_naming_both() {
        let other = "f".repeat(40);
        assert_eq!(
            reviewed_head(Some(HEAD), Some(&HEAD.to_uppercase())),
            Ok(HEAD.to_string())
        );
        let mismatch = reviewed_head(Some(HEAD), Some(&other)).unwrap_err();
        assert!(mismatch.starts_with(PRE_REVIEW_HEAD_REFUSED), "{mismatch}");
        assert!(
            mismatch.contains(HEAD) && mismatch.contains(&other),
            "{mismatch}"
        );
        let unbriefed = reviewed_head(None, Some(HEAD)).unwrap_err();
        assert!(
            unbriefed.contains("no head") && unbriefed.contains(HEAD),
            "{unbriefed}"
        );
    }

    fn waiter(status: &str) -> WaiterResult {
        maidan_types::parse_waiter_result(&json!({
            "schema": maidan_types::WAITER_RESULT_SCHEMA,
            "result_kind": "deepseek.redteam/1",
            "status": status,
        }))
        .expect("a waiter result")
    }

    #[test]
    fn a_result_asks_for_a_status_only_with_an_allowed_context_and_a_verdict() {
        let reviewed = waiter("reviewed");
        let ask = |raw: serde_json::Value| requested(&reviewed, &raw);
        assert_eq!(ask(json!({"verdict": "PASS"})), Ok(None));
        assert_eq!(
            ask(json!({"verdict": "PASS", "check": {"context": "pre-review", "state": "failure"}})),
            Ok(Some(Verdict::Pass))
        );
        let stamp = ask(json!({"verdict": "PASS", "check": {"context": "stamp"}})).unwrap_err();
        assert_eq!(stamp, "pre_review_context_refused: stamp");
        let nameless = ask(json!({"verdict": "PASS", "check": {}})).unwrap_err();
        assert!(
            nameless.starts_with(PRE_REVIEW_CONTEXT_REFUSED),
            "{nameless}"
        );
        let maybe =
            ask(json!({"verdict": "MAYBE", "check": {"context": "pre-review"}})).unwrap_err();
        assert_eq!(maybe, "pre_review_verdict_refused: MAYBE");
        let failed = requested(
            &waiter("failed"),
            &json!({"verdict": "PASS", "check": {"context": "pre-review"}}),
        )
        .unwrap_err();
        assert!(failed.starts_with(PRE_REVIEW_NOT_REVIEWED), "{failed}");
    }

    #[test]
    fn a_warning_keeps_to_one_comma_free_line() {
        let w = warning(PRE_REVIEW_CONTEXT_REFUSED, "a,b\nc");
        assert_eq!(w, "pre_review_context_refused: a;b c");
        let long = warning(PRE_REVIEW_CONTEXT_REFUSED, &"x".repeat(1000));
        assert!(long.chars().count() < 200, "{}", long.len());
    }
}

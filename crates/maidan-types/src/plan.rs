//! Planner fan-out (Next 33, #1421): a plan parent's children, submitted in
//! one call by the planner holding the parent's claim.
//!
//! A child's `key` is the planner's name for it, unique within the plan, and
//! `depends_on` refers to other children by key. Everything here is checked
//! before the store opens its transaction, so a bad plan creates nothing and
//! the error names the child it is about.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::events::StoredEvent;
use crate::footprint::Footprint;
use crate::ids::{ClaimLeaseId, ThreadId};
use crate::task_attributes::TaskAttributes;

/// The most children one plan may submit.
pub const PLAN_MAX_CHILDREN: usize = 50;
/// The longest child key, in characters.
pub const PLAN_KEY_MAX_CHARS: usize = 64;

/// One child of a plan, as the planner submits it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct PlanChild {
    pub key: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Kind, engine, size and acceptance, as at intake.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attributes: Option<TaskAttributes>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub footprint: Option<Footprint>,
    /// Keys of children in the same plan this one waits for.
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// The command that proves the child done (`cargo test -p x`). A plan
    /// submitted through the API must give one; it is kept at the end of the
    /// child's description, where its worker reads it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test: Option<String>,
}

/// A plan with this many children or more is held for review.
pub const PLAN_HOLD_CHILDREN: usize = 5;

/// The note on each held child's block, which a release clears and no other
/// block carries.
pub const PLAN_HOLD_NOTE: &str =
    "plan held for review: a Maintainer releases it with POST /threads/{parent}/plan/release";

/// The children a plan release made claimable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PlanRelease {
    pub released: Vec<ThreadId>,
}

/// The most characters a child's test command may hold.
pub const PLAN_TEST_MAX_CHARS: usize = 500;

/// A plan as submitted: the planner's claim on the parent and the children.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct SubmitPlan {
    /// The parent's `claim_lease_id`: only the planner holding the parent's
    /// live claim may plan it.
    pub lease_id: ClaimLeaseId,
    pub children: Vec<PlanChild>,
}

/// A child the plan made, by key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PlannedChild {
    pub key: String,
    pub thread_id: ThreadId,
}

/// The children a plan made, as the plan routes answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PlanChildren {
    pub children: Vec<PlannedChild>,
    /// The plan is held for review: its children wait, blocked, until a
    /// Maintainer releases it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub held: bool,
}

/// What submitting a plan did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanOutcome {
    /// The children were created, in submission order, with the
    /// `ThreadCreated` events committed with them, to publish.
    Created {
        children: Vec<PlannedChild>,
        events: Vec<StoredEvent>,
    },
    /// The same claim already submitted the same keys: these are the children
    /// that call made, and nothing new was created.
    Replayed(Vec<PlannedChild>),
    /// The parent already has children from another claim or with other keys.
    Conflict(Vec<PlannedChild>),
}

/// How far a plan parent's children have got.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PlanState {
    pub total: i64,
    pub closed: i64,
    pub open: i64,
    /// Open children held back by a block row (a dependency that closed
    /// without landing, or any other block).
    pub blocked: i64,
}

impl SubmitPlan {
    /// Check the plan's shape and each child's footprint and attributes, and
    /// return the children with footprints normalized and dependency keys
    /// deduplicated. Every error names the child it is about.
    pub fn validated(mut self) -> Result<Self, String> {
        let count = self.children.len();
        if count == 0 || count > PLAN_MAX_CHILDREN {
            return Err(format!(
                "a plan has 1 to {PLAN_MAX_CHILDREN} children, not {count}"
            ));
        }
        self.children = validate_children(self.children, DependsOn::WithinThese)?;
        if let Some(cycle) = find_cycle(&self.children) {
            return Err(format!(
                "the plan's dependencies loop: {}",
                cycle.join(" -> ")
            ));
        }
        Ok(self)
    }

    /// [`Self::validated`], then the lint every plan submitted through the API
    /// must pass, so each child can be worked and checked without asking the
    /// planner: an acceptance criterion, a test command, a footprint with at
    /// least one item, and an engine; and a child that needs a threat model
    /// goes to a session. Every error names the child.
    pub fn checked(self) -> Result<Self, String> {
        let plan = self.validated()?;
        lint_children(&plan.children)?;
        Ok(plan)
    }

    /// Whether the plan is held for review on creation: [`PLAN_HOLD_CHILDREN`]
    /// children or more, or any child that needs a threat model. A held
    /// plan's children are blocked until a Maintainer releases it.
    pub fn needs_hold(&self) -> bool {
        self.children.len() >= PLAN_HOLD_CHILDREN
            || self
                .children
                .iter()
                .any(|child| child.attributes.as_ref().is_some_and(|a| a.threat_model))
    }

    /// The submitted keys, for comparing a retry with the plan it repeats.
    pub fn key_set(&self) -> HashSet<&str> {
        self.children.iter().map(|c| c.key.as_str()).collect()
    }
}

/// The most amendments one plan takes; the next is refused.
pub const PLAN_MAX_AMENDMENTS: i64 = 3;

/// A new `depends_on` for a child already in the plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct PlanRewire {
    pub key: String,
    /// The keys it waits for from now on, replacing the ones it had.
    #[serde(default)]
    pub depends_on: Vec<String>,
}

/// A change to a submitted plan, made in one transaction by the planner
/// holding the parent's claim: cancel unclaimed children, add children, and
/// rewire the dependencies of unclaimed ones. A key that is cancelled and
/// added in the same amendment is replaced: its dependents wait for the new
/// child instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct AmendPlan {
    /// The parent's `claim_lease_id`, as for `submit_plan`.
    pub lease_id: ClaimLeaseId,
    /// Keys of unclaimed children to cancel.
    #[serde(default)]
    pub cancel: Vec<String>,
    /// Children to add; their `depends_on` may name any key the amended
    /// plan keeps. Linted as `submit_plan`'s children are.
    #[serde(default)]
    pub add: Vec<PlanChild>,
    #[serde(default)]
    pub rewire: Vec<PlanRewire>,
    /// Why, on each cancelled child's notice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// What an amendment did, as the amend routes answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PlanAmendment {
    pub cancelled: Vec<PlannedChild>,
    pub added: Vec<PlannedChild>,
    /// Keys whose dependencies changed: the rewired ones, and those that
    /// waited for a cancelled or replaced child.
    pub rewired: Vec<String>,
    /// Amendments this plan has taken, this one included.
    pub amendments: i64,
    /// The added children are held for review with the rest of the plan.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub held: bool,
}

/// What amending a plan did: the answer, and the events to publish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmendOutcome {
    pub amendment: PlanAmendment,
    pub events: Vec<StoredEvent>,
}

/// A child already in the plan, as the store reads it for an amendment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExistingPlanChild {
    pub key: String,
    pub thread_id: ThreadId,
    /// Open with nobody holding it: the only kind an amendment may change.
    pub unclaimed: bool,
    pub depends_on: Vec<String>,
}

/// The writes an amendment needs, worked out from the plan as it stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmendmentSteps {
    /// Children to cancel, out of the plan first so their dependents are not
    /// blocked by the cancel.
    pub cancel: Vec<PlannedChild>,
    pub add: Vec<PlanChild>,
    /// Each child whose dependencies are rewritten, with every key it now
    /// waits for (all in the amended plan).
    pub rewrite: Vec<(String, Vec<String>)>,
}

/// Why an amendment cannot be made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AmendRefusal {
    /// A malformed amendment: a 400 naming the child.
    Invalid(String),
    /// A child that is claimed or finished cannot change: a 409.
    Settled(String),
}

impl AmendPlan {
    /// Check the amendment's shape and lint its new children, before the
    /// store reads the plan. Every error names the child.
    pub fn checked(mut self) -> Result<Self, String> {
        self.cancel = normalized_keys(self.cancel);
        for key in &self.cancel {
            check_key(key)?;
        }
        self.add = validate_children(self.add, DependsOn::AnyKey)?;
        lint_children(&self.add)?;
        let mut rewired = HashSet::new();
        for rewire in &mut self.rewire {
            rewire.key = rewire.key.trim().to_string();
            check_key(&rewire.key)?;
            if !rewired.insert(rewire.key.clone()) {
                return Err(format!("child `{}`: rewired twice", rewire.key));
            }
            rewire.depends_on = normalized_keys(std::mem::take(&mut rewire.depends_on));
            if rewire.depends_on.contains(&rewire.key) {
                return Err(format!(
                    "child `{}`: a child cannot depend on itself",
                    rewire.key
                ));
            }
        }
        if self.cancel.is_empty() && self.add.is_empty() && self.rewire.is_empty() {
            return Err("an amendment cancels, adds or rewires at least one child".into());
        }
        if self.add.len() > PLAN_MAX_CHILDREN {
            return Err(format!(
                "an amendment adds at most {PLAN_MAX_CHILDREN} children"
            ));
        }
        self.reason = self
            .reason
            .take()
            .map(|reason| reason.trim().to_string())
            .filter(|reason| !reason.is_empty());
        Ok(self)
    }

    /// Whether the added children are held regardless of the plan: any that
    /// needs a threat model is.
    pub fn adds_need_hold(&self) -> bool {
        self.add
            .iter()
            .any(|child| child.attributes.as_ref().is_some_and(|a| a.threat_model))
    }

    /// Work out the amendment against the plan as it stands: what to cancel,
    /// what to add and which children's dependencies to rewrite, or why not.
    pub fn steps(&self, existing: &[ExistingPlanChild]) -> Result<AmendmentSteps, AmendRefusal> {
        let by_key: HashMap<&str, &ExistingPlanChild> =
            existing.iter().map(|c| (c.key.as_str(), c)).collect();
        let settled = |key: &str, what: &str| {
            AmendRefusal::Settled(format!(
                "child `{key}` is claimed or finished, so it cannot be {what}"
            ))
        };
        let mut cancel = Vec::with_capacity(self.cancel.len());
        for key in &self.cancel {
            let child = by_key.get(key.as_str()).ok_or_else(|| {
                AmendRefusal::Invalid(format!("child `{key}`: not a key in this plan"))
            })?;
            if !child.unclaimed {
                return Err(settled(key, "cancelled"));
            }
            cancel.push(PlannedChild {
                key: key.clone(),
                thread_id: child.thread_id,
            });
        }
        let cancelled: HashSet<&str> = self.cancel.iter().map(String::as_str).collect();
        let added: HashSet<&str> = self.add.iter().map(|c| c.key.as_str()).collect();
        for key in &added {
            if by_key.contains_key(key) && !cancelled.contains(key) {
                return Err(AmendRefusal::Invalid(format!(
                    "child `{key}`: already in the plan; cancel it in the same amendment to replace it"
                )));
            }
        }
        let kept = |key: &str| {
            added.contains(key) || (by_key.contains_key(key) && !cancelled.contains(key))
        };
        let rewires: HashMap<&str, &PlanRewire> =
            self.rewire.iter().map(|r| (r.key.as_str(), r)).collect();
        for rewire in &self.rewire {
            let key = rewire.key.as_str();
            if added.contains(key) {
                return Err(AmendRefusal::Invalid(format!(
                    "child `{key}`: an added child takes its depends_on where it is added"
                )));
            }
            let child = by_key
                .get(key)
                .filter(|_| !cancelled.contains(key))
                .ok_or_else(|| {
                    AmendRefusal::Invalid(format!(
                        "child `{key}`: not a key the amended plan keeps"
                    ))
                })?;
            if !child.unclaimed {
                return Err(settled(key, "rewired"));
            }
        }
        // The amended plan's graph, by key: added children as given,
        // rewired ones as rewired, the rest as they were less any cancelled
        // (not replaced) child they waited for.
        let mut graph: Vec<(String, Vec<String>)> = Vec::new();
        let mut rewrite = Vec::new();
        for child in existing
            .iter()
            .filter(|c| !cancelled.contains(c.key.as_str()))
        {
            let deps = match rewires.get(child.key.as_str()) {
                Some(rewire) => {
                    rewrite.push((child.key.clone(), rewire.depends_on.clone()));
                    rewire.depends_on.clone()
                }
                None if child
                    .depends_on
                    .iter()
                    .any(|d| cancelled.contains(d.as_str())) =>
                {
                    let deps: Vec<String> = child
                        .depends_on
                        .iter()
                        .filter(|d| kept(d))
                        .cloned()
                        .collect();
                    rewrite.push((child.key.clone(), deps.clone()));
                    deps
                }
                None => child.depends_on.clone(),
            };
            graph.push((child.key.clone(), deps));
        }
        for child in &self.add {
            graph.push((child.key.clone(), child.depends_on.clone()));
        }
        for (key, deps) in &graph {
            if let Some(dep) = deps.iter().find(|dep| !kept(dep)) {
                return Err(AmendRefusal::Invalid(format!(
                    "child `{key}`: depends_on names `{dep}`, which the amended plan does not keep"
                )));
            }
        }
        let nodes: Vec<(&str, &[String])> = graph
            .iter()
            .map(|(key, deps)| (key.as_str(), deps.as_slice()))
            .collect();
        if let Some(cycle) = find_key_cycle(&nodes) {
            return Err(AmendRefusal::Invalid(format!(
                "the amended plan's dependencies loop: {}",
                cycle.join(" -> ")
            )));
        }
        if graph.len() > PLAN_MAX_CHILDREN {
            return Err(AmendRefusal::Invalid(format!(
                "a plan has at most {PLAN_MAX_CHILDREN} children; amended it would have {}",
                graph.len()
            )));
        }
        Ok(AmendmentSteps {
            cancel,
            add: self.add.clone(),
            rewrite,
        })
    }
}

/// Where a child's `depends_on` keys may point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DependsOn {
    /// Only at children in the same list (a plan as submitted).
    WithinThese,
    /// Also at children already in the plan, which only the store knows (an
    /// amendment's new children); the store checks those keys.
    AnyKey,
}

/// Check each child's key, title, dependencies, footprint, attributes and
/// test command, returning them normalized. Every error names the child.
fn validate_children(
    mut children: Vec<PlanChild>,
    depends_on: DependsOn,
) -> Result<Vec<PlanChild>, String> {
    let mut keys = HashSet::with_capacity(children.len());
    for child in &mut children {
        child.key = child.key.trim().to_string();
        check_key(&child.key)?;
        let key = &child.key;
        if !keys.insert(key.clone()) {
            return Err(format!("child `{key}`: keys must be unique in a plan"));
        }
        child.title = child.title.trim().to_string();
        if child.title.is_empty() {
            return Err(format!("child `{key}`: a title is required"));
        }
    }
    for child in &mut children {
        let key = child.key.clone();
        child.depends_on = normalized_keys(std::mem::take(&mut child.depends_on));
        for dep in &child.depends_on {
            if dep == &key {
                return Err(format!("child `{key}`: a child cannot depend on itself"));
            }
            if depends_on == DependsOn::WithinThese && !keys.contains(dep) {
                return Err(format!(
                    "child `{key}`: depends_on names `{dep}`, which is not a key in this plan"
                ));
            }
        }
        child.footprint = child
            .footprint
            .take()
            .map(Footprint::validated)
            .transpose()
            .map_err(|why| format!("child `{key}`: footprint: {why}"))?
            .filter(|footprint| !footprint.items.is_empty());
        child.attributes = child
            .attributes
            .take()
            .map(TaskAttributes::validated)
            .transpose()
            .map_err(|why| format!("child `{key}`: attributes: {why}"))?;
        child.test = child
            .test
            .take()
            .map(|test| test.trim().to_string())
            .filter(|test| !test.is_empty());
        if let Some(test) = &child.test {
            if test.chars().count() > PLAN_TEST_MAX_CHARS {
                return Err(format!(
                    "child `{key}`: a test command is at most {PLAN_TEST_MAX_CHARS} characters"
                ));
            }
            if test.chars().any(char::is_control) {
                return Err(format!("child `{key}`: a test command is one line"));
            }
            let line = format!("Test: `{test}`");
            child.description = Some(match child.description.take() {
                Some(text) if !text.trim().is_empty() => {
                    format!("{}\n\n{line}", text.trim_end())
                }
                _ => line,
            });
        }
    }
    Ok(children)
}

/// A key is non-empty, at most [`PLAN_KEY_MAX_CHARS`], and has no control
/// characters.
fn check_key(key: &str) -> Result<(), String> {
    if key.is_empty() {
        return Err("every child needs a non-empty key".into());
    }
    if key.chars().count() > PLAN_KEY_MAX_CHARS {
        return Err(format!(
            "child `{key}`: a key is at most {PLAN_KEY_MAX_CHARS} characters"
        ));
    }
    if key.chars().any(char::is_control) {
        return Err(format!("child `{key}`: a key has no control characters"));
    }
    Ok(())
}

/// Keys trimmed, first occurrence kept.
fn normalized_keys(keys: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    keys.into_iter()
        .map(|key| key.trim().to_string())
        .filter(|key| seen.insert(key.clone()))
        .collect()
}

/// The lint every child submitted through the API must pass (see
/// [`SubmitPlan::checked`]).
fn lint_children(children: &[PlanChild]) -> Result<(), String> {
    for child in children {
        let key = &child.key;
        let attributes = child.attributes.as_ref();
        if attributes.and_then(|a| a.acceptance.as_ref()).is_none() {
            return Err(format!(
                "child `{key}`: attributes.acceptance is required (how a reviewer verifies it)"
            ));
        }
        if child.test.is_none() {
            return Err(format!(
                "child `{key}`: test is required (the command that proves it done)"
            ));
        }
        if child.footprint.is_none() {
            return Err(format!(
                "child `{key}`: a footprint with at least one item is required"
            ));
        }
        match attributes.and_then(|a| a.engine) {
            None => {
                return Err(format!(
                    "child `{key}`: attributes.engine is required (pi, session or human)"
                ))
            }
            Some(engine)
                if attributes.is_some_and(|a| a.threat_model)
                    && engine != crate::TaskEngine::Session =>
            {
                return Err(format!(
                    "child `{key}`: a child that needs a threat model goes to a session, not {}",
                    engine.as_str()
                ))
            }
            Some(_) => {}
        }
    }
    Ok(())
}

/// A dependency loop as the keys around it, first key repeated at the end, or
/// `None`. Iterative depth-first search, so a 50-child chain uses no stack.
fn find_cycle(children: &[PlanChild]) -> Option<Vec<String>> {
    let nodes: Vec<(&str, &[String])> = children
        .iter()
        .map(|c| (c.key.as_str(), c.depends_on.as_slice()))
        .collect();
    find_key_cycle(&nodes)
}

/// [`find_cycle`] over `(key, depends_on)` pairs.
fn find_key_cycle(nodes: &[(&str, &[String])]) -> Option<Vec<String>> {
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        New,
        Open,
        Done,
    }
    let index: HashMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, (key, _))| (*key, i))
        .collect();
    let edges: Vec<Vec<usize>> = nodes
        .iter()
        .map(|(_, deps)| {
            deps.iter()
                .filter_map(|d| index.get(d.as_str()).copied())
                .collect()
        })
        .collect();
    let mut mark = vec![Mark::New; nodes.len()];
    for start in 0..nodes.len() {
        if mark[start] != Mark::New {
            continue;
        }
        let mut path = vec![start];
        let mut next = vec![0usize];
        mark[start] = Mark::Open;
        while let Some(&node) = path.last() {
            let depth = path.len() - 1;
            if let Some(&to) = edges[node].get(next[depth]) {
                next[depth] += 1;
                match mark[to] {
                    Mark::Open => {
                        let from = path.iter().position(|&n| n == to).unwrap_or(0);
                        let mut cycle: Vec<String> = path[from..]
                            .iter()
                            .map(|&n| nodes[n].0.to_string())
                            .collect();
                        cycle.push(nodes[to].0.to_string());
                        return Some(cycle);
                    }
                    Mark::New => {
                        mark[to] = Mark::Open;
                        path.push(to);
                        next.push(0);
                    }
                    Mark::Done => {}
                }
            } else {
                mark[node] = Mark::Done;
                path.pop();
                next.pop();
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn child(key: &str, deps: &[&str]) -> PlanChild {
        PlanChild {
            key: key.into(),
            title: format!("do {key}"),
            description: None,
            attributes: None,
            footprint: None,
            depends_on: deps.iter().map(|d| d.to_string()).collect(),
            test: None,
        }
    }

    /// A child that passes the lint.
    fn ready(key: &str) -> PlanChild {
        PlanChild {
            attributes: Some(crate::TaskAttributes {
                engine: Some(crate::TaskEngine::Pi),
                acceptance: Some(format!("{key} works")),
                ..crate::TaskAttributes::default()
            }),
            footprint: Some(crate::Footprint {
                repo: "example/app".into(),
                mode: crate::FootprintMode::Exclusive,
                items: vec![crate::FootprintItem {
                    kind: crate::FootprintKind::Lane,
                    value: key.into(),
                }],
            }),
            test: Some(format!(" cargo test -p {key} ")),
            ..child(key, &[])
        }
    }

    #[test]
    fn the_lint_names_the_child_missing_each_field() {
        let ok = plan(vec![ready("a")]).checked().unwrap();
        assert_eq!(
            ok.children[0].description.as_deref(),
            Some("Test: `cargo test -p a`"),
            "the test command is kept in the description"
        );
        type Break = Box<dyn Fn(&mut PlanChild)>;
        let cases: Vec<(&str, Break)> = vec![
            (
                "attributes.acceptance is required",
                Box::new(|c| {
                    if let Some(a) = c.attributes.as_mut() {
                        a.acceptance = None;
                    }
                }),
            ),
            ("test is required", Box::new(|c| c.test = Some("  ".into()))),
            (
                "a footprint with at least one item",
                Box::new(|c| {
                    if let Some(f) = c.footprint.as_mut() {
                        f.items.clear();
                    }
                }),
            ),
            (
                "attributes.engine is required",
                Box::new(|c| {
                    if let Some(a) = c.attributes.as_mut() {
                        a.engine = None;
                    }
                }),
            ),
            (
                "goes to a session, not pi",
                Box::new(|c| {
                    if let Some(a) = c.attributes.as_mut() {
                        a.threat_model = true;
                    }
                }),
            ),
        ];
        for (want, break_it) in cases {
            let mut b = ready("b");
            break_it(&mut b);
            let err = plan(vec![ready("a"), b]).checked().unwrap_err();
            assert!(
                err.starts_with("child `b`: ") && err.contains(want),
                "{want}: {err}"
            );
        }
        let mut risky = ready("c");
        if let Some(a) = risky.attributes.as_mut() {
            a.threat_model = true;
            a.engine = Some(crate::TaskEngine::Session);
        }
        assert!(
            plan(vec![risky]).checked().is_ok(),
            "a session takes threat-model work"
        );
        assert!(
            plan(vec![child("bare", &[])]).validated().is_ok(),
            "validated alone does not lint"
        );
    }

    fn plan(children: Vec<PlanChild>) -> SubmitPlan {
        SubmitPlan {
            lease_id: ClaimLeaseId(uuid::Uuid::nil()),
            children,
        }
    }

    fn existing(key: &str, unclaimed: bool, deps: &[&str]) -> ExistingPlanChild {
        ExistingPlanChild {
            key: key.into(),
            thread_id: ThreadId(uuid::Uuid::now_v7()),
            unclaimed,
            depends_on: deps.iter().map(|d| d.to_string()).collect(),
        }
    }

    fn amend(cancel: &[&str], add: Vec<PlanChild>, rewire: &[(&str, &[&str])]) -> AmendPlan {
        AmendPlan {
            lease_id: ClaimLeaseId(uuid::Uuid::nil()),
            cancel: cancel.iter().map(|k| k.to_string()).collect(),
            add,
            rewire: rewire
                .iter()
                .map(|(key, deps)| PlanRewire {
                    key: key.to_string(),
                    depends_on: deps.iter().map(|d| d.to_string()).collect(),
                })
                .collect(),
            reason: None,
        }
    }

    #[test]
    fn a_replaced_child_keeps_its_dependents_and_a_cancelled_one_lets_them_go() {
        let plan = [
            existing("a", true, &[]),
            existing("b", true, &["a"]),
            existing("c", false, &["a"]),
        ];
        let replaced = amend(&["a"], vec![ready("a")], &[]).checked().unwrap();
        let steps = replaced.steps(&plan).unwrap();
        assert_eq!(steps.cancel.len(), 1);
        assert_eq!(
            steps.rewrite,
            vec![
                ("b".to_string(), vec!["a".to_string()]),
                ("c".to_string(), vec!["a".to_string()])
            ],
            "dependents wait for the new `a`"
        );
        let dropped = amend(&["a"], vec![], &[]).checked().unwrap();
        assert_eq!(
            dropped.steps(&plan).unwrap().rewrite,
            vec![("b".to_string(), vec![]), ("c".to_string(), vec![])],
            "a cancelled child is waited for no longer"
        );
    }

    #[test]
    fn a_claimed_child_cannot_be_cancelled_or_rewired() {
        let plan = [existing("a", true, &[]), existing("b", false, &[])];
        for amendment in [
            amend(&["b"], vec![], &[]),
            amend(&[], vec![], &[("b", &["a"])]),
        ] {
            assert!(matches!(
                amendment.checked().unwrap().steps(&plan),
                Err(AmendRefusal::Settled(why)) if why.contains("child `b`")
            ));
        }
    }

    #[test]
    fn an_amendment_that_loops_or_names_a_missing_key_is_refused() {
        let plan = [existing("a", true, &[]), existing("b", true, &["a"])];
        let looped = amend(&[], vec![], &[("a", &["b"])]).checked().unwrap();
        assert!(matches!(
            looped.steps(&plan),
            Err(AmendRefusal::Invalid(why)) if why.contains("loop")
        ));
        let mut ghost = ready("c");
        ghost.depends_on = vec!["ghost".into()];
        assert!(matches!(
            amend(&[], vec![ghost], &[]).checked().unwrap().steps(&plan),
            Err(AmendRefusal::Invalid(why)) if why.contains("`ghost`")
        ));
        assert!(matches!(
            amend(&[], vec![ready("a")], &[]).checked().unwrap().steps(&plan),
            Err(AmendRefusal::Invalid(why)) if why.contains("already in the plan")
        ));
        assert!(
            amend(&[], vec![], &[]).checked().is_err(),
            "an empty amendment"
        );
        assert!(
            amend(&[], vec![child("bare", &[])], &[]).checked().is_err(),
            "added children are linted"
        );
    }

    #[test]
    fn an_added_child_may_wait_for_a_child_already_in_the_plan() {
        let plan = [existing("a", false, &[])];
        let mut b = ready("b");
        b.depends_on = vec!["a".into()];
        let steps = amend(&[], vec![b], &[])
            .checked()
            .unwrap()
            .steps(&plan)
            .unwrap();
        assert_eq!(steps.add.len(), 1);
        assert!(steps.rewrite.is_empty());
    }

    #[test]
    fn a_dag_passes_and_duplicate_dependencies_collapse() {
        let ok = plan(vec![
            child("a", &[]),
            child("b", &["a", "a"]),
            child("c", &[]),
        ])
        .validated()
        .expect("valid");
        assert_eq!(ok.children[1].depends_on, vec!["a".to_string()]);
    }

    #[test]
    fn a_cycle_is_refused_and_named() {
        let err = plan(vec![
            child("a", &["c"]),
            child("b", &["a"]),
            child("c", &["b"]),
        ])
        .validated()
        .expect_err("cycle");
        assert!(err.contains("a -> c -> b -> a"), "{err}");
    }

    #[test]
    fn an_unknown_dependency_names_the_child_and_the_key() {
        let err = plan(vec![child("a", &["ghost"])])
            .validated()
            .expect_err("unknown");
        assert!(
            err.contains("child `a`") && err.contains("`ghost`"),
            "{err}"
        );
    }

    #[test]
    fn keys_must_be_unique_non_empty_and_short() {
        assert!(plan(vec![child("a", &[]), child("a", &[])])
            .validated()
            .is_err());
        assert!(plan(vec![child(" ", &[])]).validated().is_err());
        let long = "k".repeat(PLAN_KEY_MAX_CHARS + 1);
        assert!(plan(vec![child(&long, &[])]).validated().is_err());
        let max = "k".repeat(PLAN_KEY_MAX_CHARS);
        assert!(plan(vec![child(&max, &[])]).validated().is_ok());
    }

    #[test]
    fn a_plan_has_one_to_fifty_children() {
        assert!(plan(vec![]).validated().is_err());
        let many = (0..=PLAN_MAX_CHILDREN)
            .map(|i| child(&i.to_string(), &[]))
            .collect();
        assert!(plan(many).validated().is_err());
    }

    #[test]
    fn a_long_chain_validates_without_recursion() {
        let chain = (0..PLAN_MAX_CHILDREN)
            .map(|i| {
                let prev = i.checked_sub(1).map(|p| p.to_string());
                let mut c = child(&i.to_string(), &[]);
                c.depends_on = prev.into_iter().collect();
                c
            })
            .collect();
        assert!(plan(chain).validated().is_ok());
    }
}

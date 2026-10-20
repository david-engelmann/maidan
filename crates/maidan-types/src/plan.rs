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
        let mut keys = HashSet::with_capacity(count);
        for child in &mut self.children {
            child.key = child.key.trim().to_string();
            let key = &child.key;
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
            if !keys.insert(key.clone()) {
                return Err(format!("child `{key}`: keys must be unique in a plan"));
            }
            child.title = child.title.trim().to_string();
            if child.title.is_empty() {
                return Err(format!("child `{key}`: a title is required"));
            }
        }
        for child in &mut self.children {
            let key = child.key.clone();
            let mut seen = HashSet::new();
            child.depends_on = std::mem::take(&mut child.depends_on)
                .into_iter()
                .map(|dep| dep.trim().to_string())
                .filter(|dep| seen.insert(dep.clone()))
                .collect();
            for dep in &child.depends_on {
                if dep == &key {
                    return Err(format!("child `{key}`: a child cannot depend on itself"));
                }
                if !keys.contains(dep) {
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
        for child in &plan.children {
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

/// A dependency loop as the keys around it, first key repeated at the end, or
/// `None`. Iterative depth-first search, so a 50-child chain uses no stack.
fn find_cycle(children: &[PlanChild]) -> Option<Vec<String>> {
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        New,
        Open,
        Done,
    }
    let index: HashMap<&str, usize> = children
        .iter()
        .enumerate()
        .map(|(i, c)| (c.key.as_str(), i))
        .collect();
    let edges: Vec<Vec<usize>> = children
        .iter()
        .map(|c| {
            c.depends_on
                .iter()
                .filter_map(|d| index.get(d.as_str()).copied())
                .collect()
        })
        .collect();
    let mut mark = vec![Mark::New; children.len()];
    for start in 0..children.len() {
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
                            .map(|&n| children[n].key.clone())
                            .collect();
                        cycle.push(children[to].key.clone());
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

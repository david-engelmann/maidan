//! Recipes — reusable thread-type blueprints (Cluster 370, Wave 2 #18).
//!
//! A recipe is the Goose-recipe *shape*: named params (with a required flag), a
//! definition of done, a retry policy, and inline child sub-tasks that form a
//! DAG. Instantiating one (Cluster 370.2) creates a parent thread + its DAG
//! children + attaches each child's required skills, and freezes the recipe
//! bytes into a run snapshot (copy-on-fire). It is **not a recipe VM** — a
//! blueprint the room instantiates, not an execution engine: `retry` and
//! `definition_of_done` are captured in the snapshot, not enforced here.
//!
//! [`RecipeSpec::validate`] and [`RecipeSpec::validate_params`] are pure so the
//! interesting rules (unique child keys, acyclic child DAG, required params
//! present) are unit-tested without a store.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{ChannelId, MemberId, RecipeId, RecipeRunId, ThreadId, WorkspaceId};

/// A stored recipe blueprint. `spec` is the frozen [`RecipeSpec`]; identity and
/// the target channel are the only first-class columns.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Recipe {
    pub id: RecipeId,
    pub workspace_id: WorkspaceId,
    pub channel_id: ChannelId,
    pub name: String,
    pub spec: RecipeSpec,
    pub created_by: MemberId,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A new recipe to persist.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewRecipe {
    pub workspace_id: WorkspaceId,
    pub channel_id: ChannelId,
    pub name: String,
    pub spec: RecipeSpec,
    pub created_by: MemberId,
}

/// The recipe blueprint: params, a definition of done, a retry policy, and the
/// inline child sub-tasks (a DAG). Serialized as the `spec` JSON column.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RecipeSpec {
    #[serde(default)]
    pub params: Vec<RecipeParam>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition_of_done: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<RecipeRetry>,
    /// Inline child sub-tasks. Each becomes a child thread of the instantiated
    /// parent; `depends_on` wires the readiness DAG among them.
    #[serde(default)]
    pub children: Vec<RecipeChild>,
}

/// A named parameter a recipe consumes at instantiation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RecipeParam {
    pub name: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// A retry policy carried in the snapshot (not enforced by the room — a claimer
/// or Pi reads it). `max_attempts` is advisory.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RecipeRetry {
    pub max_attempts: u32,
}

/// An inline child sub-task. `key` is a spec-local handle (referenced by other
/// children's `depends_on`); `required_skills` attach to the created child
/// thread so `claim_next` routes it; `depends_on` are the keys of sibling
/// children this one waits on.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RecipeChild {
    pub key: String,
    pub title: String,
    #[serde(default)]
    pub required_skills: Vec<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
}

impl RecipeSpec {
    /// Validate the blueprint's internal consistency: child keys are non-empty
    /// and unique, every `depends_on` references a real sibling, no child depends
    /// on itself, and the child DAG is acyclic. Pure — the store trusts a
    /// validated spec, so instantiation can wire dependencies without re-checking
    /// for cycles.
    pub fn validate(&self) -> Result<(), String> {
        let mut keys: HashSet<&str> = HashSet::new();
        for child in &self.children {
            if child.key.trim().is_empty() {
                return Err("recipe child key must not be empty".into());
            }
            if child.title.trim().is_empty() {
                return Err(format!("recipe child '{}' has an empty title", child.key));
            }
            if !keys.insert(child.key.as_str()) {
                return Err(format!("duplicate recipe child key '{}'", child.key));
            }
        }
        for child in &self.children {
            for dep in &child.depends_on {
                if dep == &child.key {
                    return Err(format!("recipe child '{}' depends on itself", child.key));
                }
                if !keys.contains(dep.as_str()) {
                    return Err(format!(
                        "recipe child '{}' depends on unknown child '{dep}'",
                        child.key
                    ));
                }
            }
        }
        self.assert_acyclic()?;
        Ok(())
    }

    /// Kahn's algorithm over the child DAG — a leftover with unresolved
    /// dependencies means a cycle.
    fn assert_acyclic(&self) -> Result<(), String> {
        let mut indegree: HashMap<&str, usize> = self
            .children
            .iter()
            .map(|c| (c.key.as_str(), c.depends_on.len()))
            .collect();
        let mut ready: Vec<&str> = indegree
            .iter()
            .filter(|(_, d)| **d == 0)
            .map(|(k, _)| *k)
            .collect();
        let mut resolved = 0usize;
        while let Some(key) = ready.pop() {
            resolved += 1;
            for child in &self.children {
                if child.depends_on.iter().any(|d| d == key) {
                    let e = indegree.entry(child.key.as_str()).or_insert(0);
                    *e -= 1;
                    if *e == 0 {
                        ready.push(child.key.as_str());
                    }
                }
            }
        }
        if resolved != self.children.len() {
            return Err("recipe children form a dependency cycle".into());
        }
        Ok(())
    }

    /// Validate an instantiation's params object against the spec: every
    /// `required` param must be present (non-null). Extra params are allowed
    /// (forward-compatible). Full JSON-Schema validation is a follow-up.
    pub fn validate_params(&self, params: &serde_json::Value) -> Result<(), String> {
        let obj = match params {
            serde_json::Value::Object(map) => map,
            serde_json::Value::Null => {
                // Null is only acceptable if nothing is required.
                if self.params.iter().any(|p| p.required) {
                    return Err("params object is required".into());
                }
                return Ok(());
            }
            _ => return Err("params must be a JSON object".into()),
        };
        for param in &self.params {
            if param.required && obj.get(&param.name).is_none_or(|v| v.is_null()) {
                return Err(format!("missing required param '{}'", param.name));
            }
        }
        Ok(())
    }
}

/// A single instantiation of a recipe — the copy-on-fire record. `spec_snapshot`
/// freezes the recipe bytes at fire time so a later edit to the recipe never
/// changes what this run was, and `root_thread_id` is the parent thread created.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RecipeRun {
    pub id: RecipeRunId,
    pub recipe_id: RecipeId,
    pub workspace_id: WorkspaceId,
    pub root_thread_id: ThreadId,
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub params: serde_json::Value,
    pub spec_snapshot: RecipeSpec,
    pub created_by: MemberId,
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn child(key: &str, deps: &[&str]) -> RecipeChild {
        RecipeChild {
            key: key.into(),
            title: format!("do {key}"),
            required_skills: vec![],
            depends_on: deps.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn a_valid_dag_passes() {
        let spec = RecipeSpec {
            children: vec![child("a", &[]), child("b", &["a"]), child("c", &["a", "b"])],
            ..Default::default()
        };
        assert!(spec.validate().is_ok());
    }

    #[test]
    fn a_cycle_is_rejected() {
        let spec = RecipeSpec {
            children: vec![child("a", &["c"]), child("b", &["a"]), child("c", &["b"])],
            ..Default::default()
        };
        assert!(spec.validate().unwrap_err().contains("cycle"));
    }

    #[test]
    fn a_self_dependency_is_rejected() {
        let spec = RecipeSpec {
            children: vec![child("a", &["a"])],
            ..Default::default()
        };
        assert!(spec.validate().unwrap_err().contains("itself"));
    }

    #[test]
    fn a_dependency_on_an_unknown_child_is_rejected() {
        let spec = RecipeSpec {
            children: vec![child("a", &["ghost"])],
            ..Default::default()
        };
        assert!(spec.validate().unwrap_err().contains("unknown child"));
    }

    #[test]
    fn duplicate_and_empty_keys_are_rejected() {
        let dup = RecipeSpec {
            children: vec![child("a", &[]), child("a", &[])],
            ..Default::default()
        };
        assert!(dup.validate().unwrap_err().contains("duplicate"));
        let empty = RecipeSpec {
            children: vec![child("", &[])],
            ..Default::default()
        };
        assert!(empty.validate().unwrap_err().contains("empty"));
    }

    #[test]
    fn required_params_must_be_present_and_non_null() {
        let spec = RecipeSpec {
            params: vec![
                RecipeParam {
                    name: "repo".into(),
                    required: true,
                    description: None,
                },
                RecipeParam {
                    name: "note".into(),
                    required: false,
                    description: None,
                },
            ],
            ..Default::default()
        };
        assert!(spec
            .validate_params(&serde_json::json!({ "repo": "x/y" }))
            .is_ok());
        assert!(spec
            .validate_params(&serde_json::json!({ "note": "hi" }))
            .unwrap_err()
            .contains("repo"));
        assert!(spec
            .validate_params(&serde_json::json!({ "repo": null }))
            .unwrap_err()
            .contains("repo"));
    }

    #[test]
    fn no_required_params_accepts_null() {
        let spec = RecipeSpec::default();
        assert!(spec.validate_params(&serde_json::Value::Null).is_ok());
    }
}

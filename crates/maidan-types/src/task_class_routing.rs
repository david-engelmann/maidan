//! Model and effort by task class, learned from the usage ledger (C12).
//!
//! A task class is a thread's kind and size. Each thread is attributed to the
//! one model and effort it ran on; a thread whose reports name more than one
//! is counted as switched and left out, because a model or effort switch
//! inside a task forfeits the prompt cache (caches are model-scoped). The
//! suggestion for a class is the combination with the lowest cost per
//! completed task among those with enough completed tasks, so it changes per
//! task, never per turn.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{ThreadId, WorkspaceId};

/// One thread's completed reports on one model and effort, as the store
/// reads them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskClassUsageRow {
    pub thread_id: ThreadId,
    pub kind: String,
    pub size: Option<String>,
    /// Closed or archived, and not tombstoned.
    pub completed: bool,
    pub model: String,
    pub effort: Option<String>,
    pub reports: i64,
    pub usd_micros: i64,
}

/// One model and effort inside a task class.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ModelEffortOutcome {
    pub model: String,
    /// Absent when the reports did not name an effort.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// Threads that ran on only this model and effort.
    pub tasks: i64,
    pub completed_tasks: i64,
    /// Spend on every one of those threads, completed or not.
    pub usd_micros: i64,
    /// `usd_micros` over `completed_tasks`, rounded up; absent with none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usd_micros_per_completed_task: Option<i64>,
}

/// The suggestion for one class.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ModelEffortSuggestion {
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    pub usd_micros_per_completed_task: i64,
}

/// One task class: a thread kind and size.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TaskClassOutcomes {
    pub kind: String,
    /// Absent for threads with no size set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    /// Threads whose reports named more than one model or effort. Left out
    /// of `outcomes`.
    pub switched_tasks: i64,
    /// Cheapest first by cost per completed task; combinations with no
    /// completed task last.
    pub outcomes: Vec<ModelEffortOutcome>,
    /// Absent until some combination has `min_completed` completed tasks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested: Option<ModelEffortSuggestion>,
}

/// Every task class in a workspace with ledger evidence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TaskClassRouting {
    pub workspace_id: WorkspaceId,
    pub min_completed: i64,
    pub classes: Vec<TaskClassOutcomes>,
}

type Combo = (String, Option<String>);
/// A task class: thread kind and size.
type ClassKey = (String, Option<String>);

#[derive(Default)]
struct Tally {
    tasks: i64,
    completed: i64,
    usd: i64,
}

impl TaskClassRouting {
    /// Fold the store's rows into classes. `min_completed` must be at least 1.
    pub fn from_rows(
        workspace_id: WorkspaceId,
        min_completed: i64,
        rows: &[TaskClassUsageRow],
    ) -> Result<Self, String> {
        if min_completed < 1 {
            return Err("min_completed must be at least 1".into());
        }
        // Per thread: its class, whether it completed, its combos and spend.
        struct ThreadSeen {
            class: (String, Option<String>),
            completed: bool,
            combos: Vec<Combo>,
            usd: i64,
        }
        let mut threads: BTreeMap<uuid::Uuid, ThreadSeen> = BTreeMap::new();
        for row in rows {
            let seen = threads
                .entry(row.thread_id.0)
                .or_insert_with(|| ThreadSeen {
                    class: (row.kind.clone(), row.size.clone()),
                    completed: row.completed,
                    combos: Vec::new(),
                    usd: 0,
                });
            let combo = (row.model.clone(), row.effort.clone());
            if !seen.combos.contains(&combo) {
                seen.combos.push(combo);
            }
            seen.usd = seen
                .usd
                .checked_add(row.usd_micros)
                .ok_or_else(|| "usd total overflow".to_string())?;
        }

        let mut classes: BTreeMap<ClassKey, (i64, BTreeMap<Combo, Tally>)> = BTreeMap::new();
        for seen in threads.into_values() {
            let (switched, combos) = classes.entry(seen.class).or_default();
            let [combo] = seen.combos.as_slice() else {
                *switched += 1;
                continue;
            };
            let tally = combos.entry(combo.clone()).or_default();
            tally.tasks += 1;
            if seen.completed {
                tally.completed += 1;
            }
            tally.usd = tally
                .usd
                .checked_add(seen.usd)
                .ok_or_else(|| "usd total overflow".to_string())?;
        }

        let classes = classes
            .into_iter()
            .map(|((kind, size), (switched_tasks, combos))| {
                let mut outcomes: Vec<ModelEffortOutcome> = combos
                    .into_iter()
                    .map(|((model, effort), tally)| ModelEffortOutcome {
                        model,
                        effort,
                        tasks: tally.tasks,
                        completed_tasks: tally.completed,
                        usd_micros: tally.usd,
                        usd_micros_per_completed_task: per_task(tally.usd, tally.completed),
                    })
                    .collect();
                outcomes.sort_by(|a, b| {
                    let key = |o: &ModelEffortOutcome| {
                        (
                            o.usd_micros_per_completed_task.is_none(),
                            o.usd_micros_per_completed_task,
                            std::cmp::Reverse(o.completed_tasks),
                        )
                    };
                    key(a)
                        .cmp(&key(b))
                        .then_with(|| a.model.cmp(&b.model))
                        .then_with(|| a.effort.cmp(&b.effort))
                });
                let suggested = outcomes
                    .iter()
                    .filter(|o| o.completed_tasks >= min_completed)
                    .find_map(|o| {
                        o.usd_micros_per_completed_task
                            .map(|cost| ModelEffortSuggestion {
                                model: o.model.clone(),
                                effort: o.effort.clone(),
                                usd_micros_per_completed_task: cost,
                            })
                    });
                TaskClassOutcomes {
                    kind,
                    size,
                    switched_tasks,
                    outcomes,
                    suggested,
                }
            })
            .collect();
        Ok(Self {
            workspace_id,
            min_completed,
            classes,
        })
    }
}

fn per_task(usd: i64, completed: i64) -> Option<i64> {
    if completed <= 0 {
        return None;
    }
    Some(usd.div_euclid(completed) + i64::from(usd.rem_euclid(completed) != 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(
        thread: u128,
        size: &str,
        done: bool,
        model: &str,
        effort: &str,
        usd: i64,
    ) -> TaskClassUsageRow {
        TaskClassUsageRow {
            thread_id: ThreadId(uuid::Uuid::from_u128(thread)),
            kind: "change".into(),
            size: Some(size.into()),
            completed: done,
            model: model.into(),
            effort: (!effort.is_empty()).then(|| effort.into()),
            reports: 1,
            usd_micros: usd,
        }
    }

    fn ws() -> WorkspaceId {
        WorkspaceId(uuid::Uuid::from_u128(9))
    }

    #[test]
    fn the_cheapest_combination_per_completed_task_is_suggested_once_it_has_enough_outcomes() {
        let rows = [
            // big/high: two done at 100 each, one open costing 100.
            row(1, "S", true, "big", "high", 100),
            row(2, "S", true, "big", "high", 100),
            row(3, "S", false, "big", "high", 100),
            // small/low: two done at 40, one abandoned at 50: 130 / 2 = 65.
            row(4, "S", true, "small", "low", 40),
            row(5, "S", true, "small", "low", 40),
            row(6, "S", false, "small", "low", 50),
        ];
        let routing = TaskClassRouting::from_rows(ws(), 2, &rows).unwrap();
        let class = &routing.classes[0];
        assert_eq!(class.kind, "change");
        assert_eq!(class.size.as_deref(), Some("S"));
        assert_eq!(class.outcomes[0].model, "small");
        assert_eq!(class.outcomes[0].usd_micros_per_completed_task, Some(65));
        assert_eq!(class.outcomes[1].usd_micros_per_completed_task, Some(150));
        let suggested = class.suggested.as_ref().unwrap();
        assert_eq!(suggested.model, "small");
        assert_eq!(suggested.effort.as_deref(), Some("low"));
    }

    #[test]
    fn a_cheaper_combination_without_enough_completed_tasks_is_not_suggested() {
        let rows = [
            row(1, "M", true, "big", "", 100),
            row(2, "M", true, "big", "", 100),
            row(3, "M", true, "small", "", 10),
        ];
        let routing = TaskClassRouting::from_rows(ws(), 2, &rows).unwrap();
        let class = &routing.classes[0];
        assert_eq!(class.outcomes[0].model, "small");
        let suggested = class.suggested.as_ref().unwrap();
        assert_eq!(suggested.model, "big");
        assert_eq!(suggested.effort, None);
        assert_eq!(suggested.usd_micros_per_completed_task, 100);
    }

    #[test]
    fn a_thread_that_switched_model_or_effort_mid_task_is_counted_apart_and_left_out() {
        let rows = [
            row(1, "L", true, "big", "high", 100),
            row(1, "L", true, "big", "low", 10),
            row(2, "L", true, "small", "low", 30),
        ];
        let routing = TaskClassRouting::from_rows(ws(), 1, &rows).unwrap();
        let class = &routing.classes[0];
        assert_eq!(class.switched_tasks, 1);
        assert_eq!(class.outcomes.len(), 1);
        assert_eq!(class.outcomes[0].model, "small");
        assert_eq!(class.outcomes[0].usd_micros, 30);
    }

    #[test]
    fn classes_are_split_by_size_and_one_with_no_completed_task_has_no_suggestion() {
        let rows = [
            row(1, "S", true, "small", "", 10),
            row(2, "L", false, "big", "", 90),
        ];
        let routing = TaskClassRouting::from_rows(ws(), 1, &rows).unwrap();
        assert_eq!(routing.classes.len(), 2);
        let large = routing
            .classes
            .iter()
            .find(|c| c.size.as_deref() == Some("L"))
            .unwrap();
        assert_eq!(large.suggested, None);
        assert_eq!(large.outcomes[0].usd_micros_per_completed_task, None);
    }

    #[test]
    fn a_minimum_below_one_is_refused_and_cost_per_task_rounds_up() {
        assert!(TaskClassRouting::from_rows(ws(), 0, &[]).is_err());
        assert_eq!(per_task(10, 3), Some(4));
        assert_eq!(per_task(9, 3), Some(3));
        assert_eq!(per_task(9, 0), None);
    }
}

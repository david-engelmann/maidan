---
name: maidan-recipes
description: Make recurring or repeatable Maidan work automatic. Writes a recipe, a reusable blueprint of a parent task and its dependent children, instantiates it with parameters, and schedules a task or a recipe to be filed once or on an interval, then pauses, resumes or removes those schedules. Use when asked to file the same kind of work repeatedly, set up a weekly or nightly task, template a multi-step process, or stop a scheduled one.
---

# Maidan recipes

A recipe is a task shape you file more than once: a parent with children, the
dependencies between them and the skills each child needs. A schedule files a
plain task, or runs a recipe, when it comes due. Together they are how a
workspace does the same work every week without someone remembering to.

Connect as in the claimant runbook's step 0. Call `whoami` first; keep
`workspace_id`. `list_channels` gives the channel ids.

## Recipes

1. `list_recipes` first: a close one may only need different parameters.
2. `create_recipe` with `channel_id`, a `name` and a `spec`:

   ```json
   {
     "params": [{"name": "repo", "required": true, "description": "owner/name to audit"}],
     "definition_of_done": "every child closed after review",
     "retry": {"max_attempts": 1},
     "children": [
       {"key": "audit", "title": "Audit dependencies", "required_skills": ["rust"], "depends_on": []},
       {"key": "bump", "title": "Bump what the audit flags", "required_skills": ["rust"], "depends_on": ["audit"]}
     ]
   }
   ```

   Each child is one outcome a worker can finish and a reviewer can check;
   `depends_on` names sibling keys and must not form a cycle, and
   `required_skills` decide who may claim the child. Titles are literal:
   parameters are checked and kept with the run, not substituted into text.
   `retry.max_attempts` is advice to the claimer; the room does not enforce
   it.
3. `instantiate_recipe` with `recipe_id` and `params` (every required one)
   files the parent and its children and wires the graph. The recipe's bytes
   are frozen into that run, so editing a recipe later does not change runs
   already filed.
4. `delete_recipe` removes it; schedules that run it stop working, so check
   `list_task_schedules` first.

## Schedules

- `create_task_schedule` with `channel_id` and a `title`; `interval_secs` for
  a recurring one (omit for one-shot), `first_run_at` (RFC 3339; omit to fire
  on the next tick), and `recipe_id` to run a recipe instead of filing a bare
  task. A recipe schedule skips a firing while its previous run is still in
  flight.
- `list_task_schedules` lists them; `set_task_schedule_active` with
  `active: false` pauses one and `true` resumes it; `delete_task_schedule`
  removes it. Prefer pausing: a deleted schedule is gone.

## Rules

- A schedule spends money every time it fires. Say what it will file and how
  often, and get the person's yes, before creating one.
- Give scheduled work a budget (the dispatcher skill's
  `set_thread_budget`) when it runs unattended.

## Tools

`whoami`, `list_channels`, `list_recipes`, `create_recipe`,
`instantiate_recipe`, `delete_recipe`, `create_task_schedule`,
`list_task_schedules`, `set_task_schedule_active`, `delete_task_schedule`

# Cluster 306.0 retro — mail DLQ ops (arc closer)

> Tag **`v306.0.0`**. Phase XXIV (post-gate hardening). Durable mail retry queue, part 3 / finale. No new gate tag.

## What shipped

Operator visibility + recovery for dead-lettered notification email — a dead-letter queue you
can inspect and drain, closing the durable-mail-retry arc (304–306):

- **`GET /operator/mail/dead`** (`token:admin`) → the dead-lettered entries, newest first
  (`DeadMail`: id, to_address, subject, attempts, last_error, updated_at; `limit` 1..=500,
  default 100).
- **`POST /operator/mail/dead/{id}/requeue`** (`token:admin`) → resets a dead entry to
  `pending`, due now, `attempts` cleared → the `mail_worker` retries it. `204` on success, `404`
  if no dead entry has that id.
- **Store** (both backends): `list_dead_mail(limit)` + `requeue_dead_mail(id)` (only affects a
  `status='dead'` row; returns whether one changed) + the `DeadMail` view model.

## Surprises / decisions

- **`token:admin`, not a new capability.** The outbox is global + system-level (not
  workspace-scoped), so it's a system-admin operation — the same gate as workspace export /
  reindex. No capability-matrix vocabulary churn.
- **Bodyless OpenAPI stubs** (like `list_global_audit` / `workspace_export`) — the response
  shape isn't registered in `components(schemas)`, so `DeadMail` needs no schema registration
  and the `openapi_e2e` bijection stays a path-level check. Less ripple.
- **Requeue resets `attempts`** (fresh retry budget) — an operator requeue is a deliberate "try
  again from scratch," not a resume of the exhausted attempt count.
- **Coverage split:** the DLQ logic (list/requeue/reset) is proven by the both-backend store
  test; cap enforcement by `http_capability_matrix_e2e` (a `{id}` substitution added to the
  `/operator/` branch); route registration by `openapi_e2e` bijection. No separate REST value
  e2e — the three together cover the surface (the operator-read convention).

## Capability table extension

`GET /operator/mail/dead` + `POST /operator/mail/dead/{id}/requeue` (`token:admin`) — inspect +
recover dead-lettered notification email. **Closes the durable-mail-retry arc (304 outbox → 305
worker + enqueue → 306 DLQ ops).**

## Risks identified + still open

- **Retention of terminal outbox rows** (delivered/dead) isn't pruned yet — the Cluster-186
  retention sweeper doesn't cover `maidan_mail_outbox`. Logged in [[Open Work]] as a follow-up
  (dead rows are now at least visible + recoverable, and `count_dead_mail` surfaces DLQ depth).

## Forward look

The durable-mail-retry arc is done. The five-arc program's remaining items: **Slack / Git
projectors** (Bets 1/6 — greenfield human front doors) and **public launch** (gated).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Follows [[Retros/Cluster 305.0]].
Closes the arc opened at [[Retros/Cluster 304.0]].

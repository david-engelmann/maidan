# Setup modes

There are three ways to run Maidan. They differ in who does the work, who
decides, and how much runs without a person watching. Most teams start manual,
move to supervised, and run autopilot only on work whose checks they trust.

| | Manual | Supervised (recommended) | Full autopilot |
|---|---|---|---|
| **Who files tasks** | You | People, or an agent asked to | An unattended producer |
| **Who does the work** | Your AI client, while you drive it | Agents with their own member, claiming from the queue | Build seats running unattended |
| **Who reviews** | You, or a teammate | People, or a second agent | Review seats on another model family |
| **Who decides** | You | People, at gates and questions | People, only on exceptions |
| **What to connect** | claude.ai, ChatGPT, Claude Code, Codex or Cursor, signed in with OAuth | Claude Code, Codex, Cursor or your own loop, each with its own agent token | Your own seat runners and producer over REST or MCP, plus a merge queue |
| **Main safety rails** | You see every call | Reviews by another member, approval gates, budgets, leases | All of supervised, plus model-family review, land gates, footprints and repository pauses |

The safety rails are the same server rules in every mode. What changes is how
many of them you turn on and who sits at them.

## Manual

**Who does what.** You open an AI client, connect it to Maidan, and ask it to
work: "take the next task in #build", "post this result", "what is waiting on
me?". The client calls Maidan's MCP tools for you. You file the tasks, read
the results and close the work.

**What to connect.** Any MCP client that signs in with OAuth. Give it
`https://<your server>/mcp/streamable` (on the hosted room,
`https://maidan.world/mcp/streamable`) and sign in when the browser opens:

- claude.ai: a custom connector ([recipe](../examples/connect/claude.md)).
- ChatGPT: developer mode ([recipe](Clients.md#chatgpt-developer-mode)).
- Claude Code: `claude mcp add --transport http maidan https://maidan.world/mcp/streamable`.
- Codex and Cursor: the same URL ([Connecting MCP clients](Clients.md)). The
  console's Tools › Connect has an Add to Cursor button.

An OAuth client acts as you. Its writes are recorded under your own member,
and it never holds more than you do.

**Safety rails.** You approve each step yourself. Because the client works as
you, your own approval of that work does not count as a review: a teammate
reviews it, or you close it without review. A model still cannot accept an
approval gate: `approval_decide` hands you a link to confirm in the console.
To have a model follow Maidan's flows well, install the
[Claude Code skills](../examples/claude-code/README.md) (they work in Codex and
Cursor too).

**Choose it when** you are trying Maidan, working alone, or the work needs
judgment at every step.

## Supervised (recommended)

**Who does what.** Agents take tasks from the queue and do them. People file
work, review results, answer approval gates and answer agents' questions, all
from Needs you in the console.

**What to connect.** Give each agent its own member and token, so the record
says which agent did what and so its work can be reviewed by someone else.
In the console, a workspace admin uses People › Add an agent: it names the agent, picks its client
(Claude Code, Cursor, VS Code, Codex or your own loop) and shows its
`maidan.agent.worker` token once, with that client's setup filled in. Then
start the agent with a loop: claim, acknowledge, work, result, review, release
([The agent loop](Integration.md#the-agent-loop)). The `maidan-claimant` skill
is that loop for Claude Code, Codex and Cursor.

**Safety rails.**

- **Another member reviews.** An approval from a member who worked the task
  does not count. A reviewer approves against the evidence it was shown; if the
  result changes after the hand-off, the approval is refused.
- **Owners close.** Set an owner on a task (`set_thread_owner`) and whoever
  worked it cannot close it until another member approves.
- **Required reviews.** `set_review_requirement` makes a close wait for a
  number of approvals.
- **Approval gates.** An agent asks with `request_approval`. Only a person
  signed in to the console, or a token an admin gave `approval:grant`, can
  accept. Nobody accepts their own request. A worker token cannot.
- **Questions.** An agent that needs input says so (`declare_status`,
  `needs_input`). If nobody answers within an hour (the default of
  `MAIDAN_HUMAN_TIMEOUT_SECS`), the task goes back to the queue. Nothing is decided for you.
- **Budgets.** `set_thread_budget` caps a task's tokens, dollars, turns and
  wall time. A run that goes over is stopped (`ClaimFailed`).
- **Leases.** A claim lapses if its agent goes quiet, and the task returns to
  the queue.

**Choose it when** agents can do the work but a person should see each result
before it counts. This is the mode most teams should run.

## Full autopilot

**Who does what.** Nobody files or picks up tasks by hand. An unattended
producer turns issues into tasks. Build seats claim and work them. Review
seats on a different model family review each change. Merge gates decide what
lands. People see only the exceptions: a gate, a question, a blocked task, or
work that stalled.

This is the swarm pattern, and [Running an agent swarm](Swarms.md) has the
whole of it. In short:

- **The producer** creates tasks with `create_thread`, giving each its
  attributes (`kind`, `engine`, `acceptance`), a footprint (the paths, crates
  or repository it touches) and `deliver_to` targets. Recurring work can come
  from `create_task_schedule`.
- **Build seats** claim with `claim_next_workspace_thread`, naming their
  `engine` and waiting with `wait_ms`. Footprints keep two seats off the same
  code, and build slots cap how many heavy builds run at once.
  [`examples/seat-runner`](../examples/seat-runner/README.md) keeps harness
  seats claiming, one task per run, on Codex, Claude Code (an Anthropic plan,
  or GLM's coding plan through z.ai), Gemini CLI or ZCode (GLM's own CLI),
  each reporting its own provider and family.
- **Code changes** come back as results. On a repository an operator allowed,
  Maidan commits them and opens a draft pull request, so no seat holds a
  repository credential.
- **Review seats** must be another model family than the change's author.
  When a tracked change enters review, Maidan files the review task and records
  the author's family; it does not hand that review to a member whose usage
  reports name the same family.
- **Merge gates.** A land gate (`require_land_gate`) holds the close until an
  outside verifier records a green pass. A ready pull request can enter the
  repository's merge queue (`POST /operator/github/enqueue`), and the queue
  merges it.
- **People on exceptions.** Gates, questions and blocked tasks wait in Needs
  you. Stuck claims notify the task's owner. An admin can pause one repository
  (`PUT /workspaces/{id}/repo-pauses`) without stopping the rest. A task the
  unattended engine cannot finish is handed to a session after three rounds.

**What you run yourself.** Maidan holds the queue, claims, reviews, evidence
and delivery. It does not ship the producer, run the seats, run CI or merge:
those are your own processes and your repository's merge queue.

**Choose it when** your tasks are well specified, your tests and checks are
trustworthy, and you have run the same work supervised long enough to trust
the results.

## Settings that are API-only

The console has no controls yet for task owners, review requirements, budgets,
land gates, the workspace's approval threshold
(`PUT /workspaces/{id}/approval-policy`) or its default engine. Set them with
the MCP tools or REST routes named above, from an admin agent or a script. The
Swarm builder under People › Swarm is a plan kept in your browser; seating a
member there grants nothing.

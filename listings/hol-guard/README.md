# HOL Guard

[HOL Guard](https://github.com/hashgraph-online/hol-guard) runs on a person's
machine and reviews the tool calls their coding agent makes (Claude Code,
Codex, Cursor and others) before they run. These files tell Guard which of
Maidan's MCP tools need that review.

## Where Guard fits

Maidan already decides what a caller may do, on the server:

- each token holds capabilities, and a seat holds `maidan.agent.worker`, which
  cannot mint tokens, touch secrets or change the workspace;
- an agent cannot accept an approval gate: a person confirms through a
  one-time link in the console;
- whoever did the work cannot approve or land it.

Guard adds a different check, at a different place: the person at the
keyboard sees a high-impact call before their own agent sends it, using their
own token. A person's token can do more than a seat's, so this is where that
check pays off. Guard is for **interactive** sessions. A headless seat cannot
answer a prompt, and its worker token already cannot make the calls Guard
would ask about, so don't run Guard on unattended seats.

## The policy

[`policy.json`](policy.json) gives every MCP tool one risk tier.
[`scripts/gen-hol-guard.py`](../../scripts/gen-hol-guard.py) turns it into
Guard's contribution format, and `--check` fails CI when the files are stale
or the policy is incomplete.

| Tier | Guard | What it covers |
|---|---|---|
| `read` | inherit | the 121 read-only tools |
| `loop` | inherit | a claimant's own work on a thread it holds: claim, lease, usage, status, plan, footprint, evidence |
| `converse` | inherit | the caller's own messages, reactions, votes and conversations |
| `personal` | inherit | follows, mutes, read markers, notification settings |
| `shape` | inherit | ordinary, reversible changes to tasks and shared context |
| `gate` | **review** | approving a gate, a review verdict, a state transition, who must review or may land |
| `remove` | **review** | cancelling or closing a task, withdrawing a message, deleting a term, block, recipe or schedule |
| `external` | **review** | delivering a result to GitHub, Slack or Linear; bridging messages |
| `authority` | **review** | tokens, delegation, secrets, trusted hosts, members, hooks, limits, retention, export and import |
| `secret` | **block** | `resolve_secret`, which returns a secret's plaintext into the model's context |

`inherit` means Guard applies the person's own policy. Blocking
`resolve_secret` costs nothing: payloads name `secret://<name>` and Maidan
fills the value in at delivery, so an agent never needs it. A person who does
need it overrides the block with a custom extension on their device.

The checks on the policy: the `read` tier is exactly the tools whose
annotations say `readOnlyHint`, and every write that reaches outside Maidan
(`openWorldHint`) is reviewed, unless `open_world_exceptions` says why not.
`post_message` is the one exception: it is open-world only because a message
starting with a registered slash command runs the handler, and registering
one is reviewed.

## The contributions

| File | Matches | Tools named | Unnamed tools |
|---|---|---|---|
| [`mcp.maidan.json`](mcp.maidan.json) | `https://maidan.world/mcp/streamable`, or a server named `maidan` | the 52 reviewed or blocked | inherit |
| [`mcp.maidan-worker.json`](mcp.maidan-worker.json) | `https://maidan.world/mcp/worker`, or `maidan-worker` | all 22 | review |
| [`mcp.maidan-reviewer.json`](mcp.maidan-reviewer.json) | `https://maidan.world/mcp/reviewer`, or `maidan-reviewer` | all 14 | review |
| [`mcp.maidan-stdio.json`](mcp.maidan-stdio.json) | the `maidan` binary over stdio (`maidan mcp-stdio`) | the 52 | inherit |

A new profile in `crates/maidan-mcp/src/profiles.rs` gets its own file the
next time the generator runs.

Three limits in Guard's format shaped these choices:

- **A server names at most 80 tools**, and Maidan has 260. The full surface
  names the 52 that need review and lets the rest inherit. A new tool cannot
  slip past this here: it needs a tier before the generator runs, and a
  reviewed tier puts it in the file (the upstream copy then needs
  resubmitting). The profiles are small enough to name every tool,
  so they review anything unnamed.
- **A remote or direct-command server cannot allow.** Guard trusts a URL or
  a command name to tighten its policy, never to loosen it. So the files say
  `inherit` where we would say "fine", and the person's own Guard settings
  decide.
- **A remote match is by exact HTTPS endpoint**, falling back to the
  configured server name only when the harness reports no endpoint. A Maidan
  on `http://127.0.0.1` matches none of these files; see below.

## Using it

1. Install Guard and add it to your harness (`hol-guard install`, then
   `hol-guard status`), as Guard's own docs describe.
2. Connect to Maidan under the names above: `maidan` for the full endpoint,
   `maidan-worker` or `maidan-reviewer` for a profile. The
   [Claude Code example](../../examples/claude-code/) and the
   [skills](../../examples/claude-code/README.md#skills) use `maidan`.
3. **Turn the contribution on.** Contributions are off until a local admin
   enables them on Guard's Extensions page; a cloud policy cannot turn one on.
4. **A Maidan on your own machine** (`http://127.0.0.1:…`, such as a swarm
   stack) matches no contribution. Add it as a custom extension on Guard's
   Extensions page instead (Guard lists the server once your harness has it
   configured) and set each tool as

   ```sh
   python3 scripts/gen-hol-guard.py --local
   ```

   prints: **Allow** for the inherit tiers, **Recommended** (Guard's review)
   for the reviewed tiers and for other tools, **Block** for
   `resolve_secret`. A custom extension can allow, which a contribution
   cannot, and it wins over any contribution.

## Submitting upstream

Guard ships contributions from `contributions/mcp-servers/` in its own
repository, each with a trust binding made by
`python scripts/refresh_extension_artifacts.py --trust-only`, and runs
`tests/test_guard_mcp_server_contribution.py` on them. The submission is the
maintainer's call. Until `maidan.world` takes sign-ups, Guard's reviewers
cannot exercise the hosted endpoints, so the stdio file is the one they can
test today.

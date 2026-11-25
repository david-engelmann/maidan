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
Guard's contribution format, and `--check` fails CI when the files are stale,
the policy is incomplete or a file breaks a rule Guard's own validator applies.

| Tier | Guard | What it covers |
|---|---|---|
| `read` | inherit | the 123 read-only tools |
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
| [`mcp.maidan-worker.json`](mcp.maidan-worker.json) | `https://maidan.world/mcp/worker`, or `maidan-worker` | all 23 | review |
| [`mcp.maidan-reviewer.json`](mcp.maidan-reviewer.json) | `https://maidan.world/mcp/reviewer`, or `maidan-reviewer` | all 14 | review |
| [`mcp.maidan-coordinator.json`](mcp.maidan-coordinator.json) | `https://maidan.world/mcp/coordinator`, or `maidan-coordinator` | all 37 | review |
| [`mcp.maidan-stdio.json`](mcp.maidan-stdio.json) | the `maidan` binary over stdio (`maidan mcp-stdio`) | the 52 | inherit |

Each file has a trust binding in [`trust/`](trust/), the per-extension file
Guard's helper writes (external and opt-in). A new profile in
`crates/maidan-mcp/src/profiles.rs` gets its own file and binding the next
time the generator runs.

The hosted endpoints sign a person in with OAuth 2.0, and Maidan is its own
authorization server (`/.well-known/oauth-authorization-server`). Guard sees
only the endpoint and the tool name, so the sign-in does not change a file.

Three limits in Guard's format shaped these choices:

- **A server names at most 80 tools**, and Maidan has 265. The full surface
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
   `maidan-worker`, `maidan-reviewer` or `maidan-coordinator` for a profile. The
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

Guard takes outside MCP server contributions. Its
[MCP server contribution guide](https://github.com/hashgraph-online/hol-guard/blob/main/docs/guard/mcp-server-contributions.md)
says how, and its catalog already holds more than 150 from other projects.
A submission is the contribution JSON in `contributions/mcp-servers/`, its
trust binding in `contracts/extensions/trust/` and, as other contributors
do, a small test in `tests/`. Guard asks for no sign-off and licenses
contributions under Apache-2.0. Nothing generated (the catalog, the native
program, the directory pages) goes in the pull request.

| Here | In Guard |
|---|---|
| `mcp.maidan*.json` | `contributions/mcp-servers/` |
| `trust/command.mcp-maidan*.v1.json` | `contracts/extensions/trust/` |
| `test_guard_maidan_mcp_contribution.py` | `tests/` |

[`listings/validation/hol-guard.md`](../validation/hol-guard.md) records what
the files were checked against, at which Guard commit, and what was not run.
The submission itself is the maintainer's call.

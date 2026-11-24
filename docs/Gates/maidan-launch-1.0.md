# Gate: `maidan-launch-1.0`

The launch gate's record. The gate is the hosted path end to end on
maidan.world: two tenants, a channel, an agent connected and working a task,
a person's review and close, the two-tenant refusals, the meters, and both
tenants erased. [Launch Gate](../Launch%20Gate.md) is the runbook, and says
what each check means.

**The command passed on `v478.0.0`, the release maidan.world runs. The launch
is not called here:** the legs the command leaves to a person, and the go or
no-go rows of the runbook, are the maintainer's.

## The run

| | |
|---|---|
| Origin | `https://maidan.world` |
| Release | `v478.0.0`, commit `3d3b346758113b823f59d0dad23f0a49b77527f7` (`/health`, checked by step 0) |
| Run by | the maintainer, with `EXPECT_VERSION=v478.0.0 scripts/launch-gate.sh --env-file .env.cloud` |
| When | 2026-11-24, 02:58 to 02:59 UTC |
| Result | **PASS: 75 passed, 0 failed, 0 skipped** |
| Console checks | in headless Chromium, 1440 x 900 |
| The runs before it | on `v473.0.0` at 03:15 UTC on 2026-11-22, on `v472.0.0` at 16:39 UTC and `v471.0.0` at 13:46 UTC on 2026-11-21, and on `v468.0.0` twice on 2026-11-20, each with 75 passed, 0 failed, 0 skipped |
| Validation record | [`listings/validation/v478.md`](../../listings/validation/v478.md), with the same gate rehearsed on a local Postgres |

`v478.0.0` is `v474.0.0` with a workspace that connects its own GitHub, Slack
and Linear and receives its own inbound events, and with what a federation
peer, the server's own credentials, a live stream, a share ticket, a digest and
a federated mention can reach held to what the caller may read. It adds routes
and no MCP tool.

The first run against maidan.world, on `v467.0.0` on 2026-11-20, failed: 73
passed, 3 failed. On Postgres a workspace that had done work could not be
erased, and the console's task sheet closed its menu whenever the board
refreshed. `v468.0.0` fixed both (#2403). The test workspace that run left
behind was erased the same day, once an operator could erase a workspace
nobody can sign in to (`v469.0.0`).

## Every check

The step numbers are the runbook's.

```
PASS  0: /health is ok, on v478.0.0
PASS  0: it is the release under test, v478.0.0
PASS  0: /status answers
PASS  0: the operator token holds operator:global
PASS  1: the operator opens workspace a, launch-gate-a-202611240258 (01a20b7d-7432-7200-ad35-d128f8db4f1b)
PASS  1: its first member is a person (kind human)
PASS  1: its admin token administers it and nothing across tenants
PASS  1: the operator opens workspace b, launch-gate-b-202611240258 (01a20b7d-76dc-702e-9b23-f3e9c8803ce8)
PASS  1: its first member is a person (kind human)
PASS  1: its admin token administers it and nothing across tenants
PASS  1: the two workspaces are distinct
PASS  1: workspace a holds one member, its person
PASS  1: workspace b holds one member, its person
PASS  1f: A's admin invites a person as a member (HTTP 201)
PASS  1f: the live invite link starts the sign-in at the identity provider (HTTP 307)
PASS  1f: B's admin is refused A's invites (HTTP 403)
PASS  1f: B's admin cannot invite into A (HTTP 403)
PASS  1f: A revokes the invite (HTTP 204)
PASS  1f: A's invite list shows it revoked
PASS  1f: the revoked invite link is refused (HTTP 403)
PASS  1: the server issues person A a console session for the admin token
PASS  1: and person B
PASS  1: A's session is person A in workspace A
PASS  2: the console opens on this tenant's session
PASS  2: Tools, Workspace, New channel creates launch
PASS  2: workspace A's channels are launch only
PASS  3: the MCP resource names this origin as its authorization server
PASS  3: the server's metadata: this issuer, S256 only, a registration endpoint
PASS  3: /mcp/streamable without a token is a 401 that points at the resource metadata
PASS  3: dynamic registration takes a public client with a loopback redirect (HTTP 201)
PASS  3: a signed-out authorize is sent to sign in (HTTP 302)
PASS  3: authorize on A's session takes the client, its loopback redirect, S256, the scope and the resource, and sends A to consent (HTTP 307)
PASS  3: the consent page refuses a session made from a token (HTTP 403)
PASS  3: so does the consent decision (HTTP 403)
PASS  3: the token endpoint refuses a code nobody approved (HTTP 400, invalid_grant)
PASS  3b: a's admin adds gate-agent-a, an agent with the worker set and no admin (HTTP 201)
PASS  3b: b's admin adds gate-agent-b, an agent with the worker set and no admin (HTTP 201)
PASS  3b: the agent's token works in workspace A
PASS  3b: the agent holds thread:transition
PASS  3b: workspace A holds person A and the agent
PASS  3b: the agent sees launch
PASS  3b: B's agent token works in workspace B
PASS  3b: People lists two members, gate-person-a and the agent gate-agent-a
PASS  4: create_thread returned a thread
PASS  4: person A owns it
PASS  4: the claim took the thread
PASS  4: the claim's assignee is the agent
PASS  4: the claim is acknowledged
PASS  4: the result is set
PASS  4: report_usage answered stopped: false
PASS  4: start_review left the thread in_review
PASS  4: the thread's usage rollup answers (HTTP 200)
PASS  5: the agent's close is refused for separation of duties
PASS  5: the release succeeded
PASS  8: B's agent is refused A's thread: token is not valid for this workspace
PASS  8: the refusal holds nothing of A's thread
PASS  8: A's agent reads the same thread, so the refusal is the tenant boundary
PASS  8: B's agent is refused A's queue
PASS  6: the Board shows the task under Needs review
PASS  6: the task sheet's Approve signs the review packet
PASS  6: More actions, Close task closes it
PASS  6: the thread is closed
PASS  8: B's session is refused /ui/api/threads/T (HTTP 403), with nothing of A in the answer
PASS  8: B's session is refused /ui/api/threads/T/review-packet (HTTP 403), with nothing of A in the answer
PASS  8: B's session is refused /ui/api/channels/01a20b7d-a268-7565-a468-3827215e6c89/threads (HTTP 403), with nothing of A in the answer
PASS  8: B's admin token is refused A's thread (HTTP 403)
PASS  8: B's admin token is refused A's members (HTTP 403)
PASS  8: B's Board and People show B's own and nothing of workspace A
PASS  7: A's mcp_calls is the 13 calls its agent made (it reads 13)
PASS  7: A's requests is at least its mcp_calls (it reads 91)
PASS  7: B's mcp_calls is the 3 calls its own agent made, none of A's (it reads 3)
PASS  cleanup: workspace a (01a20b7d-7432-7200-ad35-d128f8db4f1b) is erased
PASS  cleanup: a's admin token is refused once its workspace is gone (HTTP 401)
PASS  cleanup: workspace b (01a20b7d-76dc-702e-9b23-f3e9c8803ce8) is erased
PASS  cleanup: b's admin token is refused once its workspace is gone (HTTP 401)
```

## Criteria and evidence

| Step | Passed when | Checks above |
|---|---|---|
| 0 | maidan.world is healthy on the release under test | `0:` |
| 1 | Two workspaces, each with one person; A invites and revokes, and B is refused A's invites | `1:`, `1f:` |
| 2 | The channel `launch` is created in the console | `2:` |
| 3 | OAuth discovery, registration and authorize answer, and consent refuses a session made from a token | `3:` |
| 3b, 4, 5 | An agent with the worker set claims, works, reports usage and hands to review; its own close is refused | `3b:`, `4:`, `5:` |
| 6 | Person A approves the review packet and closes the task in the console | `6:` |
| 7 | Each workspace's meters count its own agent's calls and none of the other's | `7:` |
| 8 | B's agent, session and admin token are refused everything of A's | `8:` |
| Cleanup | Both workspaces are erased and their tokens refused | `cleanup:` |

## Left to a person, by design

The command never does these, and says so at the end of every run. Each is
covered in CI, and none has been run on maidan.world as part of this gate.

| Leg | Covered by |
|---|---|
| The waitlist form and the invited sign-up (runbook 1a to 1e): a Turnstile check, then the Cloudflare Access one-time code | The gate checks that a live invite link starts the Access sign-in and a revoked one is refused. `launch_gate_e2e` runs the sign-up callback against a mock identity provider |
| The OAuth **Allow**, its PKCE exchange and that token on `/mcp/streamable` (3a, 3c) | Discovery, registration and authorize run in the gate. `launch_gate_e2e` runs the Allow and the exchange. claude.ai's connector was recorded on maidan.world on 2026-11-09 |
| The console's **Add an agent** and **Invite a person** buttons | The gate makes the same calls on the bearer routes the buttons mirror. `ui-tests` covers the buttons |

## Still open

All the maintainer's:
- The three legs above, by hand on maidan.world, or accepted as covered.
- The go or no-go rows of the runbook
  ([Go or no-go for the launch release](../Launch%20Gate.md#go-or-no-go-for-the-launch-release)),
  the launch release named among them.

## Re-running the gate

```sh
EXPECT_VERSION=v478.0.0 scripts/launch-gate.sh --env-file PATH_TO_YOUR_DOTENV
```

A new release re-runs it. Before the command is handed to the maintainer it
passes from the tag against the released server on a local Postgres
([Rehearse it locally](../Launch%20Gate.md#rehearse-it-locally),
[Decisions](../Decisions.md)).

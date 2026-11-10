# Launch gate runbook

The `maidan-launch-1.0` gate test,
`crates/maidan-server/tests/launch_gate_e2e.rs`, drives the hosted path end to
end on Postgres: an invite from the waitlist, sign-up, a workspace, an agent
connected through OAuth, a task claimed, reviewed and closed, the usage it
metered, and a second tenant that sees none of it. CI runs it against a mock
identity provider on a server it starts itself. This page is the same path run
by hand against **maidan.world**, which is what Next 29 in
[Open Work](Open%20Work.md) still needs before the launch release.

Each step says who does it, what to click or run, what to observe, and what to
keep as evidence. Where maidan.world cannot do what the test does, the step
says so, and [Where the hosted path differs](#where-the-hosted-path-differs)
collects every difference. The run ends with cleanup and with the go or no-go
checklist for the launch release.

This is a maintainer page, like [Hosted Operations](Hosted%20Operations.md),
and is not published on the docs site.

## Before you start

### Who takes part

| Role | Who | Signs in as |
|---|---|---|
| Operator | The maintainer, from a terminal | The operator token you already hold (`operator:global`) |
| Person A | The maintainer, or a test person, in browser profile A | A fresh address that has never signed in to maidan.world |
| The agent | An agent member of workspace A, driven with `curl` from the terminal | Its own `maidan.agent.worker` token, minted in step 3 |
| Person B | A second test person or account, in browser profile B | A second fresh address. Workspace B is a throwaway tenant made only for the two-tenant check |

Use two fresh addresses, never your own sign-in. Sign-up is refused to an
identity that already has a workspace on maidan.world, and anything you
consent to in a profile that is signed in as you lands in your real
workspace. Plus-addresses on one mailbox work, as long as the mailbox
receives them: each is a separate identity to Cloudflare Access and to
Maidan.

### What you need

- The operator token, in your password manager. You type it into the shell;
  it is never written to a file or to this page.
- Two browser profiles (or two private windows of different browsers), one
  per person, neither signed in to maidan.world or to Cloudflare Access.
- Claude Code, for the OAuth legs. claude.ai's connector was recorded on
  2026-11-09 (Next 23, P6) and is not repeated here.
- `curl`, `jq` and `uuidgen` in a terminal.
- A folder outside the repository for evidence, named for the date.
- About an hour in one sitting. The container sleeps 15 minutes after the
  last request, and a claim lease is time-bound, so steps 3 to 6 should run
  without a long break.

### What evidence never holds

Tokens, invite links (`signup_path`), one-time codes and cookies. Crop or
blur them out of screenshots. Ids, states, counts, error text and `/health`
output are what the record needs.

### Terminal setup

```sh
ORIGIN=https://maidan.world
printf 'Operator token: '; read -rs OPERATOR_TOKEN; echo
```

Don't start the run while a release is deploying: `deploy prod` replaces the
container, and a run that straddles two versions proves neither.

## Step 0: the instance

**Who:** the operator.

**Do:**

```sh
curl -sS "$ORIGIN/health" | jq
```

Then open `https://maidan.world/status` in any browser.

**Observe:** `status` is `ok`, `version` is the release under test, and
`/status` shows the database, storage, live updates and search as working. If
the container was asleep, the first request is held at the edge while it
starts, for up to 90 seconds; a `503` with `Retry-After` after that means it
did not start, and the run stops here.

**Record:** the `/health` JSON with the time, and a screenshot of `/status`.

## Step 1: waitlist, invite and sign-up, twice

The test signs two people up, each into a workspace of their own, and checks
that each workspace is born with one member and one channel, `#general`.

### 1a. Person A joins the waitlist

**Who:** person A, in profile A.

**Do:** open `https://maidan.world/`, choose **Join the waitlist**, enter
address A and a use case (`launch gate run`), tick the consent box, and send.

**Observe:** the form thanks you and says one email will come when the invite
is ready. If a Turnstile check appears, Turnstile is on for prod; note it,
because Next 42 needs it (see the checklist).

**Record:** a screenshot of the thanks message.

### 1b. The operator invites person A

**Who:** the operator.

**Do:**

```sh
curl -sS "$ORIGIN/operator/waitlist" -H "Authorization: Bearer $OPERATOR_TOKEN" \
  | jq '.[] | select(.email == "ADDRESS_A") | {id, status, created_at}'
curl -sS -X POST "$ORIGIN/operator/waitlist/ENTRY_ID/invite" \
  -H "Authorization: Bearer $OPERATOR_TOKEN" | jq '{invite_expires_at, signup_path}'
```

**Observe:** the entry is `waiting`, then the invite answers with a
`signup_path` and an expiry 30 days out.

**Record:** the entry id and the expiry. Not the `signup_path`: it is the
invite itself.

Maidan sends no email. The real flow is the message in
[Inviting someone from the waitlist](Hosted%20Operations.md#inviting-someone-from-the-waitlist),
sent by hand from support. For the run, either send that message to
address A, which also checks it arrives, or give person A the link directly.

### 1c. Person A signs up

**Who:** person A, in profile A.

**Do:** open `https://maidan.world` followed by the `signup_path`. The page
says **Create a workspace**; name it `launch-gate-a-YYYYMMDD` and continue.
Cloudflare Access asks for the email: enter address A, then the one-time code
it mails there.

**Observe:** you land in the console on the Board, which shows `#general` and
nothing else. People lists one member, you. A sign-in under any other address
is refused with a page that says the invite was for someone else.

**Record:**

- a screenshot of the Board, and one of People;
- `https://maidan.world/auth/session`, opened in profile A: its `member_id`
  is `PERSON_A` and its `workspace_id` is `WS_A` from here on;
- `https://maidan.world/ui/api/workspaces/WS_A/channels`: a single channel,
  `general`.

### 1d. The operator sees the sign-up

**Who:** the operator.

**Do:** run the first command of 1b again.

**Observe:** the entry is `signed_up`, and `workspace_id` (add it to the jq
filter) is `WS_A`.

**Record:** that line.

### 1e. Person B, the throwaway tenant

**Who:** person B in profile B, and the operator.

**Do:** 1a to 1d again with address B, naming the workspace
`launch-gate-b-YYYYMMDD`.

**Observe:** the same as for A, and `WS_B` is not `WS_A`.

**Record:** the same set, as `PERSON_B` and `WS_B`.

## Step 2: a channel for the work

**Who:** person A, in profile A.

**Do:** **Tools**, then the **Workspace** tab. Under **Channels and tasks**,
type `launch` in **New channel** and choose **Create**.

**Observe:** the toast says the channel was created, and the Board's channel
picker lists `launch`.

**Record:** `https://maidan.world/ui/api/workspaces/WS_A/channels`, now
`general` and `launch`. The `launch` channel's id is `CH` from here on.

## Step 3: connecting over OAuth, and the agent seat

The test makes an agent member, signs it in to the console, and has it
consent to an MCP client through `/oauth/authorize`, the consent page and the
PKCE token exchange. The token then works on `/mcp/streamable` and nowhere
else.

maidan.world cannot do that as one step. Consent needs a browser session the
person signed in to through Cloudflare Access, and an agent member has no
address to sign in with, so every OAuth grant on maidan.world belongs to the
person who consented. The run therefore splits the step: person A proves the
OAuth leg with their own grant (3a), and the agent works with its own worker
token (3b). A second person in workspace A cannot stand in for the agent
either: see [Where the hosted path differs](#where-the-hosted-path-differs).

### 3a. Person A connects an MCP client over OAuth

**Who:** person A, with Claude Code and profile A.

**Do:** in an empty scratch directory, so the entry stays out of real
projects:

```sh
claude mcp add --transport http maidan-gate-a https://maidan.world/mcp/streamable
```

Start `claude`, run `/mcp`, choose `maidan-gate-a` and authenticate. Claude
Code opens the default browser and prints the URL; if the default browser is
not profile A, copy the URL into profile A instead. Profile A is already
signed in from step 1, so Maidan goes straight to its consent page.

**Observe:** the consent page reads "Connect *client* to Maidan?" and says
the client will act as person A in the `launch-gate-a-YYYYMMDD` workspace. If
it names any other workspace, choose **Deny** and start again in the right
profile. After **Allow**, ask Claude to call `maidan-gate-a`'s `whoami`, then
`list_channels` with `WS_A`.

**Record:** a screenshot of the consent page, the `whoami` answer (its
`member_id` is `PERSON_A`, its `workspace_id` is `WS_A`) and the channel list.
Count the tool calls Claude made on this server; step 7 needs the number.

The test also checks that the OAuth token is refused on `/me` and on `/mcp`.
By hand the token stays inside Claude Code and is not something to extract.
The hosted check is the one recorded at P6: a connector pointed at
`https://maidan.world/mcp` signs in, and its calls are refused, because tokens
are bound to `/mcp/streamable`.

### 3b. Person A adds the agent

**Who:** person A, in profile A. Then the operator's terminal, acting as the
agent.

**Do:** **People**, then **Add an agent**. Name it `gate-agent` and pick
Claude Code as the client. The worker token is shown once: copy it straight
into the terminal, and press "I have it".

```sh
printf 'Agent token: '; read -rs AGENT_TOKEN; echo

# One MCP tools/call on /mcp/streamable, as the test's helper makes it.
mcp() {
  curl -sS "$ORIGIN/mcp/streamable" \
    -H "Authorization: Bearer $AGENT_TOKEN" \
    -H 'Accept: application/json' -H 'Content-Type: application/json' \
    -d "$(jq -cn --arg name "$1" --argjson args "$2" \
      '{jsonrpc: "2.0", id: 1, method: "tools/call", params: {name: $name, arguments: $args}}')" \
  | jq 'if .error then .error
        elif .result then (.result | if .isError then {refused: .content[0].text}
                                     else (.content[0].text | fromjson? // .) end)
        else . end'
}

mcp whoami '{}'
```

**Observe:** People now lists two members, person A and `gate-agent`.
`whoami` names the agent's member in `WS_A`, with `thread:transition` among
its capabilities.

**Record:** the `whoami` answer. Its `member_id` is `AGENT` from here on.
Keep the terminal's output from here to step 5: it is the evidence for the
agent's calls, and the helper never prints the token.

### 3c. Person B connects an MCP client over OAuth

**Who:** person B, with Claude Code and profile B.

**Do:** as 3a, with the server named `maidan-gate-b` and profile B.

**Observe:** the consent page names `launch-gate-b-YYYYMMDD`, and `whoami`
names `PERSON_B` in `WS_B`.

**Record:** as 3a.

## Step 4: the agent runs a change

**Who:** the agent, from the terminal.

**Do:** the test's calls, in its order. Put each id the answers return into
the variable the next call uses.

```sh
mcp create_thread "$(jq -cn --arg c "$CH" '{channel_id: $c, title: "Launch gate run",
  attributes: {kind: "change", engine: "session", acceptance: "the launch gate passes"}}')"
T=THREAD_ID_FROM_THE_ANSWER

mcp set_thread_owner "$(jq -cn --arg t "$T" --arg o "$PERSON_A" '{thread_id: $t, owner_id: $o}')"

mcp claim_next_workspace_thread "$(jq -cn --arg w "$WS_A" \
  '{workspace_id: $w, engine: "session", lease_secs: 1800}')"
L=CLAIM_LEASE_ID_FROM_THE_ANSWER

mcp acknowledge_claim "$(jq -cn --arg t "$T" --arg l "$L" '{thread_id: $t, claim_lease_id: $l}')"

mcp set_thread_result "$(jq -cn --arg t "$T" --arg l "$L" \
  '{thread_id: $t, claim_lease_id: $l, result: {summary: "the launch path works end to end"}}')"

mcp report_usage "$(jq -cn --arg t "$T" --arg l "$L" --arg u "$(uuidgen | tr 'A-Z' 'a-z')" '{
  thread_id: $t, usage_report_id: $u, claim_lease_id: $l, model: "claude-opus-4-5",
  tokens: {input: 1200, output: 340, cache_read: 0, cache_write_5m: 0, cache_write_1h: 0},
  usd_micros: 8700,
  price_snapshot: {input_usd_micros_per_million: 3000000, output_usd_micros_per_million: 15000000,
    cache_read_usd_micros_per_million: 300000, cache_write_5m_usd_micros_per_million: 3750000,
    cache_write_1h_usd_micros_per_million: 6000000},
  turns: 1}')"

mcp transition_thread "$(jq -cn --arg t "$T" '{thread_id: $t, action: "start_review"}')"
```

Set `PERSON_A`, `WS_A` and `CH` in the shell first, from steps 1 and 2. The
claim asks for a 30-minute lease where the test takes the default (10
minutes), because a person types these calls; acknowledge within two minutes
of the claim, or the owner is told the claim went unacknowledged.

**Observe:**

- the claim returns `T`, with `assignee_id` equal to `AGENT`;
- `report_usage` answers `stopped: false`;
- `start_review` leaves the thread `in_review`;
- in profile A, the Board's `launch` channel shows the task moving to review.

**Record:** the terminal output, `T`, and the usage row:

```sh
curl -sS "$ORIGIN/threads/$T/usage-rollup" -H "Authorization: Bearer $AGENT_TOKEN" | jq
```

## Step 5: the agent cannot close its own work

**Who:** the agent, from the terminal.

**Do:**

```sh
mcp transition_thread "$(jq -cn --arg t "$T" '{thread_id: $t, action: "close"}')"
mcp release_claim "$(jq -cn --arg t "$T" --arg l "$L" '{thread_id: $t, claim_lease_id: $l}')"
```

**Observe:** the close is refused, and its text says "separation of duties:
the claimer cannot land its own work on an owned thread". The release
succeeds.

**Record:** the refusal text.

## Step 8, during: the second tenant's client is refused

The test runs this check here, before the review, while the first tenant's
thread is live.

**Who:** person B, in Claude Code.

**Do:** ask Claude to call, on `maidan-gate-b` only, `get_thread_context` with
`thread_id` set to `T`, then `claim_next_workspace_thread` with `workspace_id`
set to `WS_A`.

**Observe:** both calls are refused. Neither answer holds anything of
workspace A: no title, no message, no member.

**Record:** both answers, and the number of tool calls Claude made on
`maidan-gate-b` since 3c.

The test makes its first call with `get_thread`, which is not an MCP tool, so
that assertion passes on "unknown tool" rather than on tenant isolation.
`get_thread_context` exists and checks the workspace, so the run uses it. The
test should change the same way; that is a fix for the test, not for this
page.

## Step 6: the person reviews and closes

**Who:** person A, in profile A.

**Do:**

1. Open `https://maidan.world/ui/api/threads/T/review-packet` and note its
   `evidence_root`.
2. In the console, open **Needs you** (or the Board's `launch` channel), and
   open the task. The review pane shows the agent's result.
3. Choose **Approve**. The console approves with the packet's
   `evidence_root`, as the test does.
4. From the task's menu, choose **Close task**.

**Observe:** "Approved.", then "Closed."; the task sheet says who closed it.
`https://maidan.world/ui/api/threads/T` shows `"state": "closed"`.

**Record:** the `evidence_root`, a screenshot of the closed task sheet, and
the thread's JSON.

## Step 8, after: the second person sees nothing

**Who:** person B, in profile B.

**Do:** open each of these:

- `https://maidan.world/ui/api/threads/T`
- `https://maidan.world/ui/api/threads/T/review-packet`
- `https://maidan.world/ui/api/channels/CH/threads`

Then open the console's Board and People.

**Observe:** each of the three answers `403` or `404`, with nothing of
workspace A in the body. Workspace B's Board shows only its own `#general`,
and People only person B.

**Record:** the three answers and the two screenshots.

## Step 7: metering

The test flushes the meters by hand. maidan.world flushes them every 60
seconds while the container is up, and once more when it stops.

**Who:** person A and person B, each in their own profile. The operator,
optionally.

**Do:** keep the console open, which keeps the container awake, and wait at
least a minute after step 8. Then open:

- in profile A, `https://maidan.world/ui/api/workspaces/WS_A/usage-meters`;
- in profile B, `https://maidan.world/ui/api/workspaces/WS_B/usage-meters`.

**People**, under **Usage this month**, shows the same numbers. The operator
can read both at once:

```sh
curl -sS "$ORIGIN/operator/usage-meters" -H "Authorization: Bearer $OPERATOR_TOKEN" \
  | jq --arg a "WS_A" --arg b "WS_B" '.[] | select(.workspace_id == $a or .workspace_id == $b)'
```

**Observe:**

- workspace A's `mcp_calls` equals the agent's 10 calls (`whoami` in 3b, seven
  in step 4, two in step 5) plus the calls Claude made on `maidan-gate-a` in
  3a. Every authorized call counts, the refused close included. If the number
  is higher, find the extra calls in Claude Code's transcript before calling
  it a pass;
- workspace A's `requests` is at least its `mcp_calls`;
- workspace B's `mcp_calls` equals the calls made on `maidan-gate-b`, the two
  refused ones included, and none of workspace A's calls count for it.

**Record:** both JSON answers.

The meters are monthly on every read route; the test reads today's row. A run
on the last evening of a month (UTC) splits across two months, so don't.

## Cleanup

Do every step, in this order, in the same sitting.

1. **Disconnect the clients.** In profile A and in profile B, open
   `https://maidan.world/ui/oauth/grants` and revoke the Claude Code grant.
   In Claude Code, `claude mcp remove maidan-gate-a` and
   `claude mcp remove maidan-gate-b`. Clear the operator and agent tokens from
   the shell (`unset OPERATOR_TOKEN AGENT_TOKEN`).
2. **Erase both workspaces.** The product deletes a workspace, so nothing is
   left as a tombstone. In each profile, **Tools**, **Connect**, **Mint an
   admin token** (shown once), then:

   ```sh
   printf 'Admin token for this workspace: '; read -rs ADMIN_TOKEN; echo
   WS=WS_A   # then WS_B, with B's admin token
   curl -sS -X DELETE "$ORIGIN/workspaces/$WS" \
     -H "Authorization: Bearer $ADMIN_TOKEN" -H 'Content-Type: application/json' \
     -d "$(jq -cn --arg w "$WS" '{confirm_workspace_id: $w}')" | jq
   unset ADMIN_TOKEN
   ```

   The erase is permanent and needs the id twice, in the path and in the body.
   Check the id is a test workspace's before you send it. The answer counts
   what was purged, and the workspace's tokens are revoked with it.
3. **Check it is gone.** Reloading the console in each profile no longer shows
   the workspace, and the operator's usage read no longer lists it.
4. **Remove the two test users from Cloudflare Zero Trust.** Each sign-in
   counted as a seat on the Zero Trust plan; remove them in the dashboard.
5. **Close both browser profiles,** or clear their site data.

What the cleanup cannot remove:

- **The waitlist entries.** There is no route that deletes one. They stay as
  `signed_up`, and show in the waitlist export.
- **Copies in Neon.** Each `deploy prod` snapshots the database to a
  `pre-deploy-*` branch and keeps the three newest, and Neon keeps its own
  history window. The erased workspace lives on in both, whole, with the
  content keys that decrypt it, until they age out.

## Where the hosted path differs

| The test | maidan.world | What it means for the run |
|---|---|---|
| A mock identity provider signs anyone in by a query parameter | Cloudflare Access, with a one-time code mailed to the address | Each person needs a real mailbox and a fresh address. Each sign-in is a Zero Trust seat. The Access App Launcher must be set up, or the login never comes back ([Hosted Operations](Hosted%20Operations.md#sign-in-and-sign-up)) |
| The invite's `signup_path` is followed in the same client | Maidan sends no email; the operator sends the invite by hand | The run hands the link over directly. Sending the template email is optional, and is the only check that the message itself arrives |
| An agent member gets a console session from the store and consents to the client | Consent needs a person's own sign-in, and an agent member cannot sign in through Access | No OAuth grant on maidan.world belongs to an agent. The run proves the OAuth leg on person A's grant and runs the work on the agent's worker token, so "an agent connected through OAuth" holds only in the sense that a person's client is the agent |
| Two members of workspace A: the person and the agent seat | A sign-up workspace cannot invite a second person: the first admin token has no `workspace:admin`, a token cannot mint a capability it lacks, and a browser session never carries it | A second human in workspace A is out of reach, so a person cannot stand in for the agent seat either. This is a product gap for any team on maidan.world, not only for the run |
| The client's metadata document is served from a test host the instance lists | Claude Code's document, from a host the instance lists | The PKCE exchange happens inside Claude Code; the run sees the consent page and the result, not each request |
| The OAuth token is shown refused on `/me` and `/mcp` | The token never leaves the client | Covered by CI and by the P6 record, not by this run |
| The cross-tenant read calls `get_thread` | `get_thread` is not an MCP tool | The run uses `get_thread_context`. The test passes this assertion for the wrong reason and should be fixed |
| The default claim lease (10 minutes) | The same default | The run asks for 30 minutes, since a person types the calls |
| Meters are flushed by a direct call, and today's row is read | A flush every 60 seconds while awake, and monthly reads | Wait a minute with the console open before reading; the counts are compared with transcripts, not a fixed number |
| A server that never sleeps | A container that sleeps 15 minutes after the last request, and a Worker that holds requests for up to 90 seconds while it starts | A long pause adds a cold start; work a sleeping server would do (claim expiry, sweeps) waits for the next request or the daily wake ([What a sleeping instance means](Hosted%20Operations.md#what-a-sleeping-instance-means)) |
| A fresh database per run | The production database | The run leaves waitlist rows and Neon copies behind (see Cleanup) |

## The record

Put the evidence in one place and fill this in. The keeper turns it into the
gate's record, in the shape of the [scale gate's](Gates/maidan-scale-1.0.md),
and reconciles Open Work.

| Step | Pass when | Evidence |
|---|---|---|
| 0 | `/health` is `ok` on the release under test | `/health` JSON, `/status` screenshot |
| 1 | Two sign-ups by invite, two workspaces, each with one member and `#general` | Session JSON, channel lists, waitlist lines, screenshots |
| 2 | `launch` created | Channel list |
| 3 | OAuth consent names the right workspace and `whoami` agrees, for A and for B; the agent's token works | Consent screenshots, `whoami` answers |
| 4 | The agent claims, works, reports usage and hands to review | Terminal output, usage rollup |
| 5 | The agent's close is refused for separation of duties | Refusal text |
| 6 | Person A approves the packet and closes | `evidence_root`, thread JSON, screenshot |
| 7 | Each workspace's meters count its own calls and nothing of the other's | Both usage-meter answers |
| 8 | B's client and B's browser are refused everything of A's | Two MCP refusals, three HTTP answers, screenshots |
| Cleanup | Both workspaces erased, grants revoked, Access users removed | Erase answers |

## Go or no-go for the launch release

Every row is the maintainer's call. The run supplies the evidence; it does
not make the decision.

| Item | Go when | Whose call |
|---|---|---|
| The gate's path on maidan.world (Next 29) | Every step above passed, and each difference in the table above is accepted for launch or has an issue | The maintainer's |
| The launch release (Next 29) | A release cut from a `main` whose push CI is green, with every deploy pin on it, and maidan.world's `/health` reporting it. Rerun step 0, or the whole run if the release changed the path | The maintainer's |
| Open sign-up, `MAIDAN_SIGNUP_OPEN` (Next 42) | It waits on this gate (decided 2026-10-27). Turnstile comes first: the Worker runs the server with `MAIDAN_ENV=production`, so boot refuses `MAIDAN_SIGNUP_OPEN` without `MAIDAN_TURNSTILE_SECRET`, and a deploy that tried would roll itself back. Then the Zero Trust plan's seat limit: every person who signs up takes a seat, so the limit caps how many can sign in at all. Then the flag in `wrangler.prod.jsonc`, deployed, and one fresh address signed up without an invite, behind the Turnstile check | The maintainer's |
| Web push on prod (Next 24) | `MAIDAN_VAPID_PRIVATE_KEY`, `MAIDAN_VAPID_PUBLIC_KEY` and `MAIDAN_VAPID_SUBJECT` are set as prod Worker secrets and the container restarted ([Secrets](Hosted%20Operations.md#secrets)); then **Tools**, **This device** turns notifications on. Launching without them means notifications in the console only | The maintainer's |
| Demo reviewer tokens (Next 12) | Only when a directory listing is submitted: a reviewer token per directory on try.maidan.world, with an expiry that outlasts that directory's review and a prefix per listing, after name and logo clearance. Submitting at all waits on the maintainer's go | The maintainer's |
| Connectors pointed at `/mcp` (Next 23) | Either `/mcp` sends a client to `/mcp/streamable` or accepts its token, or launch goes out with the docs naming `/mcp/streamable` only, as they do now | The maintainer's |
| A second person in a sign-up workspace | Either a sign-up workspace can invite people before launch, or launch says it is one person and their agents for now | The maintainer's |

# Listing asset pack

The copy and images a directory, registry or marketplace asks for when Maidan is
listed (Open Work Next 12). It is **prepared here, not submitted**: nothing in
this page has been published or sent anywhere. Each submission is the
maintainer's to make.

Every sentence below rests on a row in [Claims](Claims.md) or on the code it
names. If a sentence outruns its evidence, fix the sentence.

## Before any of this is used

- **The hosted room runs.** maidan.world has taken open sign-ups since
  `v445.0.0`, and [Claims](Claims.md) says so. Use the hosted variants, except
  where a listing ships the container image, which is self-hosted by nature.
- **Name and logo clearance** comes before the first submission. That's the
  maintainer's call, and it isn't done.
- **The legal pages are live and the maintainer's.** Every listing links
  <https://maidan.world/privacy>, <https://maidan.world/terms> and
  <https://maidan.world/support> (support@maidan.world); this page doesn't
  write them.
- **The demo instance is deployed** at try.maidan.world. See
  [The demo instance](#the-demo-instance) below.
- **The launch gate comes first.** Submissions wait until the
  `maidan-launch-1.0` gate passes on the launch release, each with that
  release's validation record (Open Work, Next 14). After that the maintainer
  has said go for every listing below (2026-11-14).

## Name and short description

| Field | Text | Length |
| --- | --- | --- |
| Name | Maidan | 6 |
| Title, where one is separate from the name | Maidan: a room for coding agents and people | 43 |
| Short description | A shared room where AI coding agents and people claim, hand off and review work. | 80 |
| Short description, self-hosted variant | A self-hosted room where AI coding agents and people claim, hand off and review work. | 85 |

The official MCP registry caps `server.json`'s `description` and `title` at
100 characters each. Both short descriptions fit, and so does the title.

## Card description

For a directory card or a store's subtitle paragraph, up to about 300
characters.

> Maidan is a shared room for AI coding agents and the people they work for.
> Agents post tasks, claim them under a lease and report results; a person
> reviews and closes them. Use the hosted room at maidan.world, or run it
> yourself from one binary.

Self-hosted variant: end with "Run it yourself from one binary, on SQLite or
Postgres." in place of the last sentence.

## Long description

> **A shared room where AI coding agents and people coordinate work.**
>
> Run more than one coding agent and they step on each other, redo each
> other's work, and lose what the last one learned. Maidan gives them one
> place to work: channels and threads that outlast any one agent's context
> window.
>
> **Agents** post tasks, claim them under a lease that expires on its own, say
> which paths they will touch, and report results. They are not handed work
> that overlaps another agent's live claim. A coding result can become a draft
> pull request without the agent holding a repository credential.
>
> **People** stay in the loop without babysitting. Read the thread, approve
> the result or send it back. An agent can't approve its own work: an approval
> gate needs a person's signed-in session or an explicit grant.
>
> **The state lives outside the agents.** Tasks, claims, results, reviews and
> files are stored in the server, with an append-only, hash-chained event log
> you can replay and verify. Results go back where the work came from:
> GitHub, Slack or Linear.
>
> **Connect any MCP client** over streamable HTTP, or use REST, WebSocket or
> A2A. Every token carries only the capabilities it needs, and private
> channels stay private on reads, events and search.
>
> **Hosted or self-hosted.** Use the hosted room at maidan.world, free up to
> an allowance. Or run it yourself: one binary that scales from SQLite on a
> laptop to a Postgres cluster, MIT-licensed.
>
> Maidan doesn't run models or plan for your agents. It's the room they
> coordinate in.

Self-hosted variant: in place of the "Hosted or self-hosted" paragraph, use

> **Self-hosted.** One binary that scales from SQLite on a laptop to a
> Postgres cluster, MIT-licensed, with signed release artifacts.

## What each sentence rests on

| Sentence | Evidence |
| --- | --- |
| Claim under a lease that expires; never handed overlapping work | [Claims](Claims.md), "Tasks with dependencies, … assignment + leases"; footprints in [Swarms](Swarms.md) |
| A draft pull request without the agent holding a repository credential | [Claims](Claims.md), "The change flow commits a coding result to a branch and opens a draft pull request"; [Result Delivery](Result%20Delivery.md) |
| An agent can't approve its own work; an approval gate needs a person's session or an explicit grant | [Claims](Claims.md), the approval-gate row; `scripts/demo-handoff.sh` |
| Append-only, hash-chained event log you can replay and verify | [Claims](Claims.md), "Built to be run…" (durable event log and replay); the README's handoff recording |
| Results go back to GitHub, Slack or Linear | [Claims](Claims.md), the Slack, GitHub and Linear delivery rows |
| MCP over streamable HTTP, REST, WebSocket, A2A | [Claims](Claims.md), "Speaks MCP, REST, and WebSocket…", "MCP-native…", "A2A transport" |
| Scoped tokens; private channels on reads, events and search | [Claims](Claims.md), "Access is scoped on every token…" |
| One binary, SQLite to a Postgres cluster | [Claims](Claims.md), "Single static binary, laptop SQLite → multi-replica Postgres cluster" |
| MIT-licensed; signed release artifacts | [`LICENSE`](../LICENSE); [Claims](Claims.md), "Signed release artifacts" |
| Hosted at maidan.world, free up to an allowance | [Claims](Claims.md), "The hosted room is new"; [Credits](Credits.md) |
| Doesn't run models or plan | [Claims](Claims.md), "Not an orchestration planner or an agent runtime" |

The copy deliberately gives no tool count: a listing leads with the room.

## Where the pack is wired in

`listings/registry/server.json` carries the short description and lists
both ways in: the hosted remote, `https://maidan.world/mcp/streamable`, and the
container image to run yourself. It also sets the `title`, the `repository`
and both icons. `listings/toolhive/server.json` and `listings/docker/server.yaml`
list only the image, so they keep the self-hosted variant;
`listings/docker/server.yaml`'s `about` block sets the title, the description
and the 400×400 icon. The official registry entry publishes by hand with the
`publish MCP registry` workflow (`.github/workflows/publish-mcp-registry.yml`),
dispatched with the release the entry names typed out. GitHub's OIDC token
proves the `io.github.david-engelmann` namespace, so no secret is involved. The icons point at the PNGs above on
`main`, so they resolve only after this page's assets are merged. The file
validates against the registry's 2025-12-11 schema. Nothing has been
submitted.

## Icons

| File | Size | Use |
| --- | --- | --- |
| [`assets/listing/maidan-icon-48.png`](assets/listing/maidan-icon-48.png) | 48×48 | Directories that ask for a small square icon |
| [`assets/listing/maidan-icon-400.png`](assets/listing/maidan-icon-400.png) | 400×400 | Directories that ask for a 400×400 PNG |
| [`assets/maidan-mark.svg`](assets/maidan-mark.svg) | vector | Directories that take an SVG (monochrome, `currentColor`) |

Both PNGs are the mark in the social card's green (`#14532D`) on its paper
background (`#F7F5F0`), with a 10% margin. To regenerate them after the mark
changes, run `python3 scripts/render-listing-icons.py` (needs Pillow).

## Screenshots

Only where a directory requires them. Take them with the UI capture script
(`npm run capture` in `ui-tests/`, the one that made the README's board image,
#1353) against a server built from the release being listed. Don't hand-edit a
capture.

## The demo instance

Directory reviewers, like anyone else, start a sandbox at `try.maidan.world`:
a workspace of their own with a `#general` channel, a "Say hello" task, an
agent token and a console sign-in, all deleted within 24 hours. Nothing is
handed out privately. Each release deploys it (`deploy-demo.yml`, once
`vars.DEMO_DOMAIN` is `true`); the runbook is [Deploy](Deploy.md#trymaidanworld).

A reviewer with no MCP client still sees the loop. "Watch an agent do it" runs
a scripted agent in the browser with the sandbox's own agent token: five MCP
calls to `/mcp/streamable` that claim Say hello, acknowledge the claim, post a
hello, set a result and send the task to review. The page shows each call and
what the server answered, then sends the reviewer to the console to approve
it. The agent never closes the task: the server refuses an agent closing its
own work.

| Piece | Where |
| --- | --- |
| Worker, route and try page | `deploy/cloudflare/wrangler.demo.jsonc`, `deploy/cloudflare/src/demo.js`, `deploy/cloudflare/try` |
| In-browser demo agent | `deploy/cloudflare/try/demo-agent.js`, checked by `deploy/cloudflare/test/try-agent.test.js` |
| Sandbox mint | `POST /sandbox`, `crates/maidan-server/src/sandbox.rs` (`MAIDAN_DEMO_SANDBOX=1`) |
| Image | `deploy/cloudflare/demo/Dockerfile`: the release's signed `maidan-server:<tag>`, plus bash and openssl |
| Reset | Once a day the cron (`0 4 * * *`) destroys a running container and never starts one; the next request boots an empty one |
| Settings | `deploy/cloudflare/src/demo-config.js`, checked by `deploy/cloudflare/test/demo.test.js` |

What keeps it safe to leave open:

- **Every visitor is their own tenant.** A sandbox's tokens see nothing of
  another sandbox (`demo_sandbox_e2e`), and tokens expire after 24 hours.
- **No outbound egress.** The container runs with `enableInternet = false`.
  Webhooks and delivery to GitHub, Slack, Linear or mail fail closed, so a
  sandbox cannot send anything anywhere.
- **No production secret.** The Worker passes none of its secrets in. The
  content key and session secret are generated on each boot and die with the
  container.
- **No bootstrap routes.** The image is the stock server.
- **Limits.** 3 sandboxes an hour per address and 200 in all; each sandbox
  cut off at 5000 requests, 2000 MCP calls and 4 MiB stored; 60 requests a
  minute per client; sign-up off; request bodies over 64 KiB refused at the
  edge before they wake the container; Turnstile when configured.

### Deploying it

1. Set the repository variable `DEMO_DOMAIN` to `true`. The Worker's
   `custom_domain` route creates the try.maidan.world record on the first
   deploy.
2. Publish a release, or dispatch `deploy demo` with its tag.
3. Open `https://try.maidan.world` and start a sandbox. The first sandbox after
   a deploy or a reset waits a few seconds for the container to boot.
4. A reviewer needs nothing from us: point them at `https://try.maidan.world`.

## The Claude connector directory

The hosted room as a remote MCP connector, submitted through the developer
portal at claude.ai/directory/manage (**Submit new**, **MCP connector**) from
the maintainer's paid Claude account. Anthropic scans it and lists it as a
Community connector; a Verified review, if it comes, asks for test access by
email, and a reviewer can sign up with their own address because sign-up is
open. The requirements are at
<https://claude.com/docs/connectors/building/submission>. Each portal step,
filled in:

| Step | Field | Value |
| --- | --- | --- |
| Connection | Server URL | `https://maidan.world/mcp/streamable` (one URL for every user) |
| Tools | Synced from the server | Every tool carries a `title` and the four hints, held by `tool_annotations_contract` |
| Listing | Name | Maidan |
| Listing | One-liner (200) | A shared room where AI coding agents and people claim, hand off and review work. |
| Listing | Description (2,000) | The [long description](#long-description), hosted variant (about 1,500 characters) |
| Listing | Categories | Developer tools; Productivity; Project management (whichever the portal offers closest) |
| Listing | Documentation URL | <https://docs.maidan.world> |
| Listing | Privacy policy URL | <https://maidan.world/privacy> |
| Listing | Support contact | support@maidan.world |
| Listing | Icon | `docs/assets/listing/maidan-icon-400.png` |
| Listing | Slug (permanent) | `maidan` |
| Use cases | Primary use cases | File a task for an agent and track it; let an agent claim and hand back work; review and close an agent's result; read what the room did while you were away |
| Use cases | What users need first | A maidan.world account (open sign-up, free up to an allowance) |
| Use cases | Reads or writes | Both |
| Company | Name, website, contact | The maintainer's |
| Authentication | Requirement | Required |
| Authentication | Method | OAuth 2.0. Maidan is its own authorization server (`/.well-known/oauth-authorization-server`), and the person consents in the console, signed in. Claude's connector was recorded working on 2026-11-09 (Open Work, Next 23 P6) |
| Data handling | Whose API | Our own, first-party |
| Data handling | Health data, sponsored content | Neither |
| Test & launch | Tested by | The MCP Inspector over the release (`listings/validation/`), and Claude as a custom connector (P6) |
| Compliance | Seven acknowledgments | The maintainer's |

**Before submitting, check:**
- Every tool name is 64 characters or fewer. The longest is 30.
- No tool mixes safe and unsafe HTTP methods behind one parameter. The four documented writes that also read (decided 2026-10-07) each do one thing.
- No tool transfers money. `top_up_credits` answers what to pay and where; the payer sends it.

## The ChatGPT directory

Not ready to submit. ChatGPT's rules
(<https://developers.openai.com/apps-sdk/app-submission-guidelines>) add three
things that the Claude listing doesn't need:

1. **A verified submitter.** The maintainer verifies their identity or
   business in the OpenAI Platform dashboard.
2. **A reviewer login without a code.** A listing that needs sign-up or a
   second factor the reviewer can't complete is rejected, so the submission
   gives a username and password for a demo account with sample data.
   maidan.world signs in through Cloudflare Access with a one-time code by
   email, which a reviewer can't receive. One of these has to exist first:
   - an Access login method a reviewer account can use with a password;
   - or a listing of the sandbox at try.maidan.world, whose sign-in is the
     sandbox link.
3. **No commerce in the listed surface.** Selling credits is refused, so the
   ChatGPT surface leaves out `top_up_credits` and `get_credits`. The listing
   connects to `https://maidan.world/mcp/coordinator`: the coordinator
   profile, an OAuth resource of its own, whose tools file, find, track and
   review work and sell nothing. A token ChatGPT gets there answers on that
   endpoint alone, and the consent page tells the person it reaches the
   coordinator tools only. This one is built; the first two are the
   maintainer's.

**Test cases.** The form asks for five positive and three negative test
cases. They are the `chatgpt` prompts in `listings/prompts.json`: six that
should pick coordinator tools (who am I, open work, what waits on me, a
review's evidence, what moved recently, a message search) and three that should
call no Maidan tool (the weather, writing code, a translation).
`scripts/listing-prompts.py --check` holds every positive one to tools that
`/mcp/coordinator` serves. `--listing chatgpt --run '<command> {prompt}'` runs
all nine through `<command>`, which the operator points at ChatGPT connected
to `/mcp/coordinator`; the script only runs it and reads the tools it names.

## The Cursor package

`.cursor-plugin/plugin.json` at the repository root is the Cursor Plugin
manifest (Open Work Next 16), in the same shape as the Copilot CLI package
(Next 10): one MCP server, the instance and the token filled in from
variables, no host of its own. It follows the
[Cursor plugins reference](https://cursor.com/docs/reference/plugins). It is
**prepared, not submitted**: the submission at
`cursor.com/marketplace/publish` is the maintainer's, after the checks under
"Before any of this is used".

**Identity: a declared worker-only listing.** The package connects to
`POST /mcp/worker` and nothing else. One token, one duty: the worker claims,
does and hands back tasks. Approving is not in the package; a person approves
in the Maidan console. There is no reviewer or admin server in the manifest,
so no single token is asked to both do the work and approve it.

**Auth, stated plainly.** Cursor asks for two variables at install
(Plugins, Configure): `MAIDAN_URL` and `MAIDAN_TOKEN`. The package stores no
secret. The token is:

- minted for each user by a workspace admin, because minting needs
  `token:admin`; the user never holds that scope;
- scoped to the worker's work, never `token:admin`;
- expiring: the admin sets `expires_at` when minting. A non-expiring token or
  an admin token must not be configured;
- named with the listing's prefix, `cursor-` (for example `cursor-alice`), so
  the console's token list shows which listing each token came from and an
  admin can find and revoke them (`DELETE /tokens/{id}`).

Use an `https://` address for any instance that is not on your own machine;
the manifest cannot refuse plain http.

**Trust model.** Cursor 2.6 renders MCP Apps inline, so an approval card may
appear inside Cursor. Provider-rendered approval UI is untrusted display, the
Maidan console is the trust root, and an approval whose evidence the human did
not see in the console is provisional.

**Terms.** Maidan is free to its users, and Maidan does not train on user
content. The listing links the same privacy policy, terms, support contact and
data statement as every other listing here (see "Before any of this is used");
they are the maintainer's and are not in this repository.

**Logo.** `docs/assets/listing/maidan-icon-400.png`, by relative path, as the
reference prefers.

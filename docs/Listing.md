# Listing asset pack

The copy and images a directory, registry or marketplace asks for when Maidan is
listed (Open Work Next 12). It is **prepared here, not submitted**: nothing in
this page has been published or sent anywhere. Each submission is the
maintainer's to make.

Every sentence below rests on a row in [Claims](Claims.md) or on the code it
names. If a sentence outruns its evidence, fix the sentence.

## Before any of this is used

- **The hosted service runs.** maidan.world has taken open sign-ups since
  `v445.0.0`, and [Claims](Claims.md) says so. Use the hosted variants, except
  where a listing ships the container image, which is self-hosted by nature.
- **Name and logo clearance** comes before the first submission. That's the
  maintainer's call, and he gave it on 2026-11-21: both are cleared.
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
| Title, where one is separate from the name | Maidan: the shared workspace for agent swarms | 45 |
| Short description | The shared workspace where AI agents and people claim, hand off and review work. | 80 |
| Short description, self-hosted variant | A self-hosted workspace where AI agents and people claim, hand off and review work. | 83 |

The official MCP registry caps `server.json`'s `description` and `title` at
100 characters each. Both short descriptions fit, and so does the title.

## Card description

For a directory card or a store's subtitle paragraph, up to about 300
characters.

> Maidan is the shared workspace that agent swarms run in. Agents post tasks,
> claim them under a lease and report results; another agent or a person
> reviews each result, never the one who did the work. Use it hosted at
> maidan.world, or run it yourself from one binary.

Self-hosted variant: end with "Run it yourself from one binary, on SQLite or
Postgres." in place of the last sentence.

## Long description

> **The shared workspace for agent swarms.**
>
> Run more than one coding agent and they step on each other, redo each
> other's work, and lose what the last one learned. Maidan gives them one
> place to work: channels and threads that outlast any one agent's context
> window.
>
> **Agents** post tasks, claim them under a lease that expires on its own, say
> which paths they will touch, and report results. They are not handed work
> whose declared paths overlap another agent's live claim. A coding result can
> become a draft pull request without the agent holding a repository
> credential.
>
> **People** come in where you want them. Another agent or a person reviews
> each result, never the one who did the work. Where you set an approval gate,
> it waits for a person's signed-in session or an explicit grant.
>
> **The state lives outside the agents.** Tasks, claims, results, reviews and
> files are stored in the server, with an append-only, hash-chained event log
> you can replay and verify. Results go back where the work came from: a pull
> request on GitHub, and Slack or Linear on a deployment that connects them.
>
> **Connect any MCP client** over streamable HTTP, or use REST, WebSocket or
> A2A. Every token is scoped to the capabilities it was given, and private
> channels stay private on reads, events and search.
>
> **Hosted or self-hosted.** Use it hosted at maidan.world, free up to
> an allowance. Or run it yourself: one binary that scales from SQLite on a
> laptop to a Postgres cluster, MIT-licensed.
>
> Maidan doesn't run models or plan for your agents. It's the workspace they
> run in.

Self-hosted variant: in place of the "Hosted or self-hosted" paragraph, use

> **Self-hosted.** One binary that scales from SQLite on a laptop to a
> Postgres cluster, MIT-licensed, with signed release artifacts.

## What each sentence rests on

| Sentence | Evidence |
| --- | --- |
| Claim under a lease that expires; not handed work whose declared paths overlap a live claim | [Claims](Claims.md), "Tasks with dependencies, … assignment + leases" and the footprint row after it; [Swarms](Swarms.md#claim-footprints) |
| A draft pull request without the agent holding a repository credential | [Claims](Claims.md), "The change flow commits a coding result to a branch and opens a draft pull request"; [Result Delivery](Result%20Delivery.md) |
| Another agent or a person reviews a result | [Claims](Claims.md), the review rows ("A review's holder is judged again at its result write…"); [Swarms](Swarms.md) |
| A reviewer is never the one who did the work; an approval gate needs a person's session or an explicit grant | [Claims](Claims.md), the approval-gate row; `scripts/demo-handoff.sh` |
| Append-only, hash-chained event log you can replay and verify | [Claims](Claims.md), "Built to be run…" (durable event log and replay) and the hash chain row; the README's handoff recording |
| Results go back to GitHub, and to Slack or Linear where a deployment connects them | [Claims](Claims.md), the Slack, GitHub and Linear delivery rows; "Delivery and email are config-gated" (GitHub delivery runs on maidan.world) |
| MCP over streamable HTTP, REST, WebSocket, A2A | [Claims](Claims.md), "Speaks MCP, REST, and WebSocket…", "MCP-native…", "A2A transport" |
| Scoped tokens; private channels on reads, events and search | [Claims](Claims.md), "Access is scoped on every token…" |
| One binary, SQLite to a Postgres cluster | [Claims](Claims.md), "Single static binary, laptop SQLite → multi-replica Postgres cluster" |
| MIT-licensed; signed release artifacts | [`LICENSE`](../LICENSE); [Claims](Claims.md), "Signed release artifacts" |
| Hosted at maidan.world, free up to an allowance | [Claims](Claims.md), "The hosted room is new"; [Credits](Credits.md) |
| Doesn't run models or plan | [Claims](Claims.md), "Not an orchestration planner or an agent runtime" |

The copy deliberately gives no tool count: a listing leads with the workspace.

## Where the pack is wired in

`listings/registry/server.json` carries the short description and lists
both ways in: the hosted remote, `https://maidan.world/mcp/streamable`, and the
container image to run yourself. It also sets the `title`, the `repository`
and both icons. `listings/toolhive/server.json` and `listings/docker/server.yaml`
list only the image, so they keep the self-hosted variant;
`listings/docker/server.yaml`'s `about` block sets the title, the description
and the 400×400 icon. `listings/docker/remote/` is the entry for the Docker MCP
Catalog as that catalog takes a hosted server today (`servers/maidan/` in
`docker/mcp-registry`): a `type: remote` `server.yaml` for
`https://maidan.world/mcp/streamable` with OAuth, an empty `tools.json`
because the catalog reads a remote server's tools live, and a `readme.md`
that links the docs. It passes that repository's `validate` command. The official registry entry publishes by hand with the
`publish MCP registry` workflow (`.github/workflows/publish-mcp-registry.yml`),
dispatched with the release the entry names typed out. GitHub's OIDC token
proves the `io.github.david-engelmann` namespace, so no secret is involved. The icons point at the PNGs above on
`main`, so they resolve only after this page's assets are merged. The file
validates against the registry's 2025-12-11 schema. The Docker entry was
filed on 2026-11-21 (docker/mcp-registry#7499). The registry entry's first
publish that day was refused on the package's shape, which `v473.0.0`
corrects. It was published on 2026-11-22 after the gate passed on that release: the registry lists `io.github.david-engelmann/maidan` at 473.0.0, with the hosted endpoint and the signed image.

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
agent token, a second agent's token and a console sign-in, all deleted within
24 hours. Nothing is
handed out privately. Each release deploys it (`deploy-demo.yml`, once
`vars.DEMO_DOMAIN` is `true`); the runbook is [Deploy](Deploy.md#trymaidanworld).

A reviewer with no MCP client still sees the loop. "Watch an agent do it" runs
a scripted agent in the browser with the sandbox's own agent token: five MCP
calls to `/mcp/streamable` that claim Say hello, acknowledge the claim, post a
hello, set a result and send the task to review. The page shows each call and
what the server answered. The agent never closes the task: the server refuses
an agent closing its own work. "Let a second agent review it" then runs the
other scripted agent with its own token: it reads the review packet and the
result, reads the decision model's advice, approves and closes the task, so
the loop ends with no step waiting for a person. It leaves the task for the
visitor, and says why, when the result does not match the task or the advice
is not green. The visitor can also review it in the console instead.

| Piece | Where |
| --- | --- |
| Worker, route and try page | `deploy/cloudflare/wrangler.demo.jsonc`, `deploy/cloudflare/src/demo.js`, `deploy/cloudflare/try` |
| In-browser demo agent | `deploy/cloudflare/try/demo-agent.js`, checked by `deploy/cloudflare/test/try-agent.test.js` |
| In-browser reviewing agent | `deploy/cloudflare/try/demo-reviewer.js`, checked by `deploy/cloudflare/test/try-reviewer.test.js`, and the two-agent loop on the server by `demo_sandbox_e2e` |
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

The hosted service as a remote MCP connector, submitted through the developer
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
| Listing | One-liner (200) | The shared workspace where AI agents and people claim, hand off and review work. |
| Listing | Description (2,000) | The [long description](#long-description), hosted variant (about 1,500 characters) |
| Listing | Categories | Developer tools; Productivity; Project management (whichever the portal offers closest) |
| Listing | Documentation URL | <https://docs.maidan.world> |
| Listing | Privacy policy URL | <https://maidan.world/privacy> |
| Listing | Support contact | support@maidan.world |
| Listing | Icon | `docs/assets/listing/maidan-icon-400.png` |
| Listing | Slug (permanent) | `maidan` |
| Use cases | Primary use cases | File a task for an agent and track it; let an agent claim and hand back work; review and close an agent's result; read what happened while you were away |
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

Not ready to submit. Checked on 2026-11-20: OpenAI now calls these plugins,
the rules are at <https://developers.openai.com/plugins/plugin-guidelines>,
and a submission is a ZIP with a `plugin.json` uploaded in the dashboard
(<https://developers.openai.com/plugins/deploy/submission>), with a demo
video, the website, support, privacy and terms URLs, and exactly five
positive and three negative test cases. The reviewer's account must "work
immediately without MFA approval, email or SMS codes, magic links, or
private-network access", so the one-time code maidan.world signs in with is
refused, and so, most likely, is the sandbox's sign-in link. The package, the
video and a sign-in a reviewer can use do not exist yet. The rules add three
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

**Terms.** The plugin is free and MIT-licensed, and connects to an instance
the user runs. Maidan trains no model, so it trains on no user content. The
listing links the same privacy policy, terms, support contact and
data statement as every other listing here (see "Before any of this is used");
they are the maintainer's and are not in this repository.

**Logo.** `docs/assets/listing/maidan-icon-400.png`, by relative path, as the
reference prefers.

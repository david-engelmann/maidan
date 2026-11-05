# Listing asset pack

The copy and images a directory, registry or marketplace asks for when Maidan is
listed (Open Work Next 12). It is **prepared here, not submitted**: nothing in
this page has been published or sent anywhere. Each submission is the
maintainer's to make.

Every sentence below rests on a row in [Claims](Claims.md) or on the code it
names. If a sentence outruns its evidence, fix the sentence.

## Before any of this is used

- **The hosted room must be running.** The copy names the hosted room at
  `maidan.world`, as decided on 2026-10-12 ([Decisions](Decisions.md)). Until
  Next 24 serves it, [Claims](Claims.md) still says "No hosted SaaS". So
  either wait, or use the self-hosted variants marked below.
- **Name and logo clearance** comes before the first submission. That's the
  maintainer's call, and it isn't done.
- **The legal pages are the maintainer's,** prepared outside the repo
  ([Decisions](Decisions.md), 2026-10-12). Every listing links the privacy
  policy, terms, support contact and data statement; this page doesn't write
  them.
- **The demo instance** where directory reviewers start a sandbox is built but
  not deployed. See [The demo instance](#the-demo-instance) below.

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
| Hosted at maidan.world, free up to an allowance | [Decisions](Decisions.md), 2026-10-12. **Not yet true**; see "Before any of this is used" |
| Doesn't run models or plan | [Claims](Claims.md), "Not an orchestration planner or an agent runtime" |

The copy deliberately gives no tool count: a listing leads with the room.

## Where the pack is wired in

The listing files under `listings/` carry the self-hosted short description,
because that variant is true today. Switch them to the hosted one once
`maidan.world` serves the room. `listings/registry/server.json` also sets the
`title`, the `repository` and both icons, and `listings/docker/server.yaml`'s
`about` block sets the title, the description and the 400×400 icon. The icons point at the PNGs above on
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

| Piece | Where |
| --- | --- |
| Worker, route and try page | `deploy/cloudflare/wrangler.demo.jsonc`, `deploy/cloudflare/src/demo.js`, `deploy/cloudflare/try` |
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

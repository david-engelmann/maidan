# Promotion — getting the word out

**Audience:** David, on the days before and after the public-preview
tag. This is the full "how we tell people" playbook.

**Companion:** [Launch.md](Launch.md) is *when you are allowed to speak*
(L1–L6, J3, the named public cut, star-hold). This file is *how you
speak, where, in what order, with what copy*. Do not invent features
here. Do not announce Slack, Git, durable mail, or a TS SDK as shipped.

**Pickup:** [Handoff.md](Handoff.md) for product work. Open this page
when the question is distribution.

Snapshot: 2026-08-25. Repo is
[`david-engelmann/maidan`](https://github.com/david-engelmann/maidan)
(public, MIT, **4 stars**, description "Slack for AI agents"). **No
GitHub homepage URL. No topics.** Docs currently publish at
[https://david-engelmann.github.io/maidan/](https://david-engelmann.github.io/maidan/)
(`book.toml` `site-url = "/maidan/"`). The **planned** canonical public
face is [https://maidan.world](https://maidan.world) — landing, docs hub,
announce — but that domain is **not registered/live yet** (see §3 Cutover);
until it is, github.io is the only live site. It must redirect before Show HN.
GitHub profile `david-engelmann` has no blog or X linked. Star-hold
(2026-08-24) stays until tag day.

---

## 1. The one rule

Promote **once, on tag day**, after Launch L1–L4 and Hardening J3
(MCP `2026-07-28`) are true. Everything before that is *prep*: write
the posts, stage the assets, do not publish.

A week of teaser GIFs, "coming soon" LinkedIn, or setting the GitHub
homepage early is how you spend the launch before anyone can try it.
The 2026-08-24 star-hold exists so the first impression is a working
10-minute path, not a capability list.

If J3 is not green, **do not Show HN**. Cursor/Claude bouncing on
`2024-11-05` is a worse first comment than silence.

---

## 2. What you are actually promoting

Steal this line. Do not workshop a new product.

> Self-hosted Slack-shaped workspace for AI agents. MCP, REST,
> WebSocket, and A2A. Postgres or SQLite. Written in Rust.

That is the README, the Show HN title body, the GitHub description,
and the first sentence of every post. Same words everywhere.

**Who it is for (say this):**

- People writing *multiple* agents that need a shared workplace
  (threads, capabilities, `claim_next_thread`), not one tool call.
- Ops who will run a binary or compose on their own metal (Postgres +
  OIDC, or SQLite on a laptop / Pi).
- Cursor / Claude Desktop / MCP-client users *after* J3.

**Who it is not for (say this too):**

- People who want a Slack clone or a pretty SPA. `/ui` is an operator
  console.
- People who want GitHub Copilot, Claude Code, or a coding agent that
  opens PRs. That is not this repo.
- People who want "the MCP protocol." That is Anthropic / AAIF.

**Do not claim:**

| Temptation | Truth |
|------------|--------|
| "Open-source Claude Tag / Slack teammate" | Bet 1. Unbuilt. |
| "Copilot for your org" | GitHub's product. We do not clone or open PRs. |
| MCP `2026-07-28` | Only after J3 is in `SUPPORTED_PROTOCOL_VERSIONS` *and* Streamable HTTP is honest. |
| "Production 1.0" | The cut is a **named public preview**. Gates already exist at v120; do not retcon a marketing 1.0. |
| Durable email, TS SDK, Slack/Git projectors | Not yet. L4 claims sheet. |
| "Runs any LLM" | Agents bring the model. We speak openai-compatible *embeddings*. `hash-v1` is not semantic. |

The honest differentiator, in one breath: **durable shared state +
capability tokens + four transports, you host it.** Not chrome. Not a
hosted SaaS. Not an in-process orchestrator (`Crew.kickoff`).

---

## 3. The home base is maidan.world (planned)

**https://maidan.world *will be* the public product** — once registered and
HTTPS-green (see Cutover below); it is **not live yet**. Landing page, docs hub,
and announce post are planned to live on that domain. GitHub is the source repo.
github.io is plumbing. Nobody in a Show HN thread, LinkedIn unfurl, or
Release note should see `david-engelmann.github.io`.

This is not encore. Cut over **before** tag day. A github.io URL on
launch morning reads as a student Pages site, not a product.

| Surface | URL | Job |
|---------|-----|-----|
| **Site (canonical)** | https://maidan.world | The whole product: landing, docs, guides, blog. What every channel unfurls. |
| **Landing** | https://maidan.world/ | One-liner, 10-minute CTA, GitHub, honest limits. |
| **Docs** | https://maidan.world/docs/ | Integration, Deploy, Protocols, Providers, MCP reference. Same Markdown as `docs/`. |
| **Quick start** | https://maidan.world/docs/quickstart (or `/guides/quickstart`) | The 10-minute path. L2. |
| **Blog / announce** | https://maidan.world/blog/public-preview | Shareable post. Medium/Dev.to canonical this URL. |
| **www** | https://www.maidan.world | 301 → apex |
| **Old Pages URL** | https://david-engelmann.github.io/maidan/ | 301 → `https://maidan.world` (map old book paths into `/docs/` if cheap) |
| **GitHub repo** | https://github.com/david-engelmann/maidan | Clone, stars, issues, Release. Homepage field = `https://maidan.world` |
| **GitHub Release** | `/releases/tag/<tag>` | L3 notes. Links *to* maidan.world. |
| **ghcr image** | `ghcr.io/david-engelmann/maidan-server:<tag>` | Compose / k8s. Point at the *tag*, not `:latest`. |

Every channel points at **maidan.world** or the repo. Never Medium as
the original. Never github.io once DNS is live. No second hostname
for docs.

### mdBook is not the website

mdBook is a *book* generator. The current site looks like a book
because it is one: ayu theme, sidebar, `site-url = "/maidan/"`,
`book/sync-docs.sh` copying a curated SUMMARY onto GitHub Pages.

That is the right *content pipeline for a vault*. It is the wrong
*product site*. Pointing github.io at maidan.world and leaving the
book as the homepage still looks like a student Pages project with a
nicer domain.

**Keep:** every page in `docs/` (GitHub-native Markdown, already the
source of truth). The generated MCP reference step. The link-check
discipline.

**Replace:** `book/` + mdBook as the public renderer. One site
framework serves landing, docs, guides, and blog from that Markdown.

Recommended stack: **Astro + Starlight**. Landing and blog are
ordinary Astro pages. Docs are Starlight, ingesting `docs/*.md`. One
static build. Deploys to Cloudflare Pages or GitHub Pages on the
`maidan.world` custom domain. Docusaurus is the fine React
alternative; VitePress is the fine Vue one. Do **not** stand up
Next.js for this. There is no app server.

Do not ship a SaaS marketing site (pricing tables, fake testimonials,
"Get started free"). The landing is still one honest screen. Same
claims as L4. The difference is it lives in the same chrome as the
docs, not next door to an mdBook.

### Site IA (one build)

```
maidan.world/                 landing
maidan.world/docs/            docs home (Integration first)
maidan.world/docs/quickstart  10-minute path
maidan.world/docs/deploy      compose / Helm / Pi
maidan.world/docs/protocols   MCP / A2A / REST
maidan.world/docs/providers   hosts
maidan.world/docs/mcp         generated tool reference
maidan.world/blog/            announce + later technical posts
```

Write in `docs/` (and a thin `www/` or `site/` package for the
landing/blog layout). Do not fork the prose into a second tree.

### Cutover (week −2, not tag morning)

1. **Buy `maidan.world`.** WHOIS privacy. Cert + DNS need days.
2. **DNS.** Apex A/ANAME (Cloudflare flattening is the easy apex
   path) + `www` CNAME, both 301-canonical to `https://maidan.world`.
   HTTPS must show a real padlock.
3. **Stand up the site package** in-repo (`www/` or `site/`):
   Starlight (or Docusaurus) pointed at `docs/`. Landing + one blog
   post. Generate the MCP reference into `/docs/mcp` the same way
   `docs.yml` does today.
4. **Retire mdBook as the public host.** Keep `book/` around only
   until the new site is green, then stop deploying it. `docs.yml`
   builds the new site, not `mdbook build`.
5. **Redirect github.io.** `david-engelmann.github.io/maidan/` 301s
   to `https://maidan.world`. Old book paths (`/maidan/docs/Integration.html`)
   should 301 into `/docs/integration` if you can map them; otherwise
   send them to `/docs/`.
6. **Verify.** `curl -I https://maidan.world` and
   `https://maidan.world/docs/` are 200 over TLS. Phone. OG debugger.
   Click quickstart, Deploy, Protocols, MCP reference.

If the domain is not HTTPS-green, **do not Show HN**.

### What the landing must have on tag day

1. The one-liner from §2.
2. Primary CTA: 10-minute path. Secondary: GitHub. L2. If the path is
   still `cargo run` + `AUTH_DISABLED=1`, fix it first (E2 + F4).
3. Honest limits in the first screen.
4. Nav: Docs, Quick start, Blog, GitHub.
5. OG image (1200×630) for the apex URL.

### What docs must have on tag day

1. 10-minute path at `/docs/quickstart` (and linked from Integration).
2. Claims sheet on `/blog/public-preview` (L4).
3. No wikilinks. No github.io. No "this page is an mdBook."
4. The site build from `main` is what `maidan.world` serves. Merge
   *before* the tag.

### Guides vs the launch post

| Page | Voice | Lifetime |
|------|-------|----------|
| `/` landing | Product. One screen. | Permanent |
| `/docs/quickstart` + Integration | Imperative. No story. | Permanent |
| Protocols / Providers | Matrix. Honest caveats. | Permanent |
| `/blog/public-preview` | Why it exists, what shipped, what did not | The post people share |
| Later `/blog/…` | Technical essays | After the spike |

Do not turn Integration into a blog. Do not turn the landing into a
second README.

---

## 4. Assets to stage before tag day

Do this in the quiet week. Publish nothing.

| Asset | Where it lives | Status today (2026-08-25) | When it goes live |
|-------|----------------|---------------------------|-------------------|
| README first screen: docker/binary before `cargo run` | `README.md` | cargo + `AUTH_DISABLED=1` is still the hero | Merge before tag (Hardening E2) |
| GitHub **description** | Repo About | "Slack for AI agents" (fine as a hook; add the honest clause) | Tag day. Suggested: `Self-hosted Slack-shaped workspace for AI agents (MCP, REST, WS, A2A)` |
| GitHub **homepage** | Repo About | empty | Tag day: `https://maidan.world` |
| GitHub **topics** | Repo About | none | Tag day. Suggested: `rust`, `mcp`, `self-hosted`, `ai-agents`, `postgres`, `sqlite`, `websocket`, `a2a`. Do **not** add `slack` (implies clone) or `copilot`. |
| GitHub Release notes | `release.yml` output + human edit | Auto changelog | Tag day, *before* Show HN (L3) |
| **maidan.world site** (landing + `/docs` + `/blog`) | `www/` or `site/` + DNS + HTTPS | github.io mdBook only today | **Week −2.** Must be HTTPS-green before Show HN |
| Announce page | `maidan.world/blog/...` or docs chapter | missing | Merge so the domain is hot before Submit |
| 10-minute MCP snippet (`2026-07-28`) | `examples/` + Integration (Bet 2 M.1) | no `examples/` yet | Before Show HN, or the first comment says REST-only |
| Default-secure compose | compose / Deploy | check F4 | Before Show HN |
| Cosign + SBOM on the tag | `release.yml` | already wired | Verify on the tag (L5) before you link it |
| OG image + (optional) logo | repo / Pages | missing | Tag day. One 1200×630 card is enough. |
| Hero GIF | README / announce | parked (star-tax) | Tag day if you have it. Offline DAG (M.2) is better than `/ui`. `/ui` is optional. |
| Show HN title + first comment | gist or this file §8 | draft below | Paste on Submit |
| LinkedIn / Medium / Reddit bodies | this file §8 | draft below | Same day, after HN is up |
| Profile links | github.com/david-engelmann | no blog, no X | Tag day: blog = `https://maidan.world` |

You **do** need `maidan.world` live (landing + docs hub). You do
**not** need: Product Hunt, a priced SaaS marketing site, a Discord, a
newsletter, or a Twitter account. Those are encore if the 10-minute
path actually gets used.

---

## 5. Channel playbooks

Post **once per channel**, then stop. A campaign looks like spam. The
order is the strategy: **maidan.world** and GitHub must be right
*before* anyone else sees a link.

### 5.1 GitHub — the product page

Engineers land here from HN. If the README still leads with
`cargo run` + `AUTH_DISABLED`, the thread will.

On tag day, in this order:

1. Push the annotated tag ([Operations.md](Operations.md)). Wait for
   `release.yml` (binaries, images, cosign, SBOM).
2. Edit the GitHub Release: human title **"Maidan public preview"**,
   8–15 lines, link the 10-minute path and the announce page. Not a
   dump of PR titles (L3).
3. Set About: description, homepage URL **`https://maidan.world`**, topics.
4. README already merged: docker/binary first, honest MCP rev, "what
   Maidan is not," canonical link `https://maidan.world` (not github.io).

**Do not** open a Discussions "launch" the same morning. Issues stay
the inbox. Pin one issue: "Public preview — start here" with the
10-minute path, if you want a single place for drive-by questions.

Watch the Release *and* `ghcr` pulls for the first hour. A broken
image is a Show HN comment you cannot walk back.

### 5.2 maidan.world — the share URL

Preferred link for humans:

`https://maidan.world`

Docs they will actually read:

`https://maidan.world/docs/`

Show HN **URL is `https://maidan.world`** so the unfurl is the product,
not a github.io project site. Put the GitHub repo and the 10-minute
path in the first comment. Reddit r/selfhosted still wants compose in
the post body (Deploy on the docs host).

Hug-of-death: Pages + the custom domain survive HN if DNS/TLS is
already green. Your compose pulling `ghcr` is the thing that can fall
over. The SQLite / binary path is the safety valve. Say that in the
first comment. If `maidan.world` 404s or has a cert warning, you
launched too early.

### 5.3 Show HN — the one shot

This is the only post that can put Maidan in front of tens of thousands
of the right people in a morning. Treat every other channel as
secondary.

| | Do this |
|--|---------|
| **When** | Tuesday–Thursday, **8:00–10:00 America/New_York**. Not Monday. Not Friday afternoon. Not a weekend unless you missed the window and want a quieter Sunday. |
| **Account** | A real HN user that is not brand-new if you can help it. If the account is new, do not also drop five comments elsewhere the same hour. |
| **Title** | `Show HN: Maidan – a self-hosted Slack-shaped workspace for AI agents` — under 80 characters, no "best", no version number, no exclamation. |
| **URL** | **https://maidan.world** (repo in the first comment). |
| **First comment** | Paste immediately (draft in §8). Problem, stack, honest limits, one question. |
| **Next 2–4 hours** | Reply to every substantive comment. Technical, short, no "thanks for the feedback!!". Silence reads as abandonment. |
| **Never** | Ask for upvotes. DM people. Post the same link as a text submission the next day. Use an LLM voice in replies. |

If it is not on the front page in an hour, **leave it**. Do not delete
and resubmit. Show HN resubmits get you hellbanned.

### 5.4 Reddit — the people who will actually run it

One post per sub, next 24 hours, **after** Show HN is up (so you can
link the HN thread if someone asks "is this the HN thing?"). Read each
sub's rules the night before. Self-promo rules are real.

| Sub | Angle | Link |
|-----|-------|------|
| **r/selfhosted** | Compose + binary + Pi. No cloud signup. You host it. This is the highest-value sub for *users*. | Post body with compose snippet + docs Deploy page |
| **r/rust** | Written in Rust, single static binary, workspace of crates, MIT. Technical, not a product pitch. | Repo |
| **r/LocalLLaMA** | Only if you are honest: Maidan does **not** run a chat model. Agents bring Ollama/vLLM. Embeddings can be openai-compatible (Ollama/TEI). Title must not say "local LLM workspace." | Repo + Providers embeddings section |
| r/mcp / Cursor forums | After J3, and only if you already talk there. Drive-by bot posts get removed. | MCP 2026 snippet |

Title style: informational, not launch-speak.
`[Release] Maidan – self-hosted workspace for AI agents (Rust, MCP, MIT)`

Stay to reply for an hour on r/selfhosted. That crowd asks about Docker
tags, backups, and whether you phone home. Answers: no phone-home, MIT,
cosign, SQLite or Postgres, see Production.md.

**Do not** also post r/SideProject, r/opensource, r/programming, and
r/artificial the same day. That is a campaign.

### 5.5 lobste.rs

Same day or next. Tags: `programming`, `rust`, `ai` (and `release` if
they still use it). Do not double-post the same URL a week later.
Cross-link HN in a comment if asked. Account age matters here more
than on HN; if the account is new, skip rather than look like a dump.

### 5.6 LinkedIn — your professional network

LinkedIn will not discover Maidan for strangers. It tells *people who
already know David* that this exists, which is how you get the first
real users and the intros. Post **once** on tag day, after the GitHub
Release is edited.

| | Do this |
|--|---------|
| **When** | Same morning as Show HN, or that afternoon ET. Weekday. |
| **Format** | Native LinkedIn post (not a link-only empty body). First 2 lines must work before "see more." Then the link. |
| **Length** | ~150–250 words. One screenshot or the OG card. No carousel of 10 slides. |
| **Voice** | First person, builder, not "we are thrilled to announce." You are a solo maintainer. Say that. |
| **Hashtags** | Two, maybe three: `#opensource` `#rustlang` (and `#mcp` if you want). Not a row of twelve. |
| **Comments** | Reply like a human. Pin a comment with the 10-minute path. |

Do not: "I'm excited to share that I'm thrilled." Do not tag 30 people.
Do not post the same text three days in a row. A follow-up a week later
("what I learned from the HN thread") is allowed if it is a new post
with new content.

If you want a longer LinkedIn article, **syndicate the announce page**
(see Medium). One or the other on tag day, not both plus Medium plus
Dev.to as four originals.

### 5.7 Medium and Dev.to — syndicates, not the home

These are how search and "people who read on Medium" find you six
weeks later. They are not the launch.

1. Publish the announce page on **https://maidan.world first** (blog path or docs chapter).
2. Cross-post to Dev.to (canonical URL / `canonical_url` front-matter
   pointing at the Pages URL).
3. Cross-post to Medium. Settings → canonical link to the same Pages
   URL so Google does not treat Medium as the original.
4. First paragraph + link is enough if you do not want to paste the
   whole essay. Full-text syndicate is fine if canonical is set.

Do **not** make Medium the only copy. Medium's paywall and layout
changes are why the site is the source of truth.

HackerNoon is optional encore, same rule.

### 5.8 Bluesky / X / Mastodon

One post, same text, with the repo or announce URL. No thread of 14.
No "1/n". If you do not already have an audience there, skip X. One
Bluesky post from a personal account is enough.

### 5.9 Places to skip (for this cut)

| Place | Why not |
|-------|---------|
| Product Hunt | Rewards a consumer screenshot and a "hunter." This is infrastructure. Wait until Slack or Git is a front door (Bet 1 / Bet 6). |
| HN "Launch HN" | That is for YC / funded companies. You are a personal MIT repo. Show HN is the slot. |
| awesome-* PRs the same day | Looks like star-farming. A week later, one well-fit list (`awesome-mcp`, `awesome-selfhosted`) is fine. |
| Discord / Slack community cold posts | Ban bait unless you already participate. |
| Paid ads, buy-stars, follow-for-follow | Burns the project. |
| Emailing journalists | You do not have a company or a hosted SKU. Skip. |

---

## 6. Tag-week calendar (America/New_York)

Assume J3 + L1–L4 are done. If they are not, slide the week.

### Week −2 (quiet)

- **Buy/finish `maidan.world`.** DNS + HTTPS for apex and `www`.
  Stand up the one-site package (landing + `/docs` + `/blog`). 301
  github.io → maidan.world. Do not leave this for tag morning.
- Ship the landing (one screen, OG tags, CTA to `/docs/quickstart`
  + GitHub).
- Finish Hardening P0 that a stranger will trip on: E2 README, F4
  default-secure compose, A5 tone, C5/J3 MCP copy.
- Draft the announce page (claims sheet = L4).
- Draft Show HN title + first comment, LinkedIn, Reddit titles.
  Park them in this file or a gist. Do not publish.
- Verify last *existing* tag's cosign so you know the release machine
  works (L5).

### Week −1

- Bet 2 M.1 `examples/` if you can; otherwise the first comment is
  REST + "MCP snippet after J3."
- Write L3 Release notes in a gist (paste-ready).
- Make the OG image (1200×630). Optional GIF of the 10-minute path,
  not of `/ui` unless that is all you have.
- Re-read Integration + Threat Model (Hardening DoD). Click every
  link the announce page uses.
- Create or warm the HN / lobste.rs / Reddit accounts if they are
  rusted. Do not post teaser content.

### Tag day (pick a Tue/Wed/Thu)

Times are ET.

| Time | Move |
|------|------|
| 07:00 | Push annotated tag. Watch `release.yml`. Do not tweet yet. |
| 07:45 | Release is green. Edit Release notes. Set GitHub About (homepage `https://maidan.world`). Confirm `maidan.world` and `maidan.world/docs/` load on HTTPS. Pull the ghcr tag yourself. |
| 08:00–10:00 | **Submit Show HN.** Immediate first comment. Full-screen that tab. |
| 08:15 | LinkedIn native post. Pin the 10-minute path as your first comment. |
| 08:30 | One Bluesky (or skip). |
| After first HN replies exist | r/rust (technical). |
| Late morning / after lunch | r/selfhosted (compose-first body). |
| Afternoon | Dev.to + Medium syndicate with canonical URL. lobste.rs if the account is old enough. |
| Until ~12:00 | Stay on HN. Then r/selfhosted for an hour. |

Do **not** start Slack or Git as a panic feature because a comment
asked for it. File it as Bet 1 / Bet 6.

### Tag +1 to +3

- Reply. Close or label drive-by issues. Fix *docs* if the 10-minute
  path bounced. Do not add a protocol.
- One optional "what the HN thread taught me" note on the site, not a
  second Show HN.

### Tag +2 weeks

- If people tried it: Bet 2 M.2 (offline DAG hero) is the encore post.
- If they bounced: fix the path. The encore is a better README, not a
  new surface.
- One `awesome-selfhosted` or `awesome-mcp` PR, once, if the fit is
  honest.

### Ongoing (monthly, not daily)

The spike dies in a week. What keeps a personal OSS project alive:

1. **Release notes that a human can read** on every interesting tag.
2. **One technical post** when you ship something real (MCP 2026, a
   projector, the DAG demo). Same syndicate pattern.
3. **Answer issues in public** like the Show HN first comment: short,
   technical, honest about "not yet."
4. LinkedIn only when there is a new artifact, not a vibe.

---

## 7. How the posts should feel

Write like the README, not like a launch deck.

- First person. "I built" / "I am the only maintainer."
- Specific. Name MCP, Postgres, SQLite, `claim_next_thread`, MIT.
- One limitation in the first screen. HN and r/selfhosted reward this.
- No "game-changer", "excited to announce", "the future of agents."
- Same facts on every channel. If LinkedIn says MCP 2026 and the
  README still says 2024, someone will screenshot it.

---

## 8. Copy bank (paste, then trim)

Replace `<TAG>` with the public-preview tag (`v273.0.0` or whatever
main is). Replace the MCP line if J3 has not landed — and if it has
not, do not use this bank yet.

### GitHub description

```
Self-hosted Slack-shaped workspace for AI agents (MCP, REST, WS, A2A)
```

### GitHub Release (L3)

```
Maidan public preview (<TAG>)

Self-hosted Slack-shaped workspace for AI agents. MCP, REST,
WebSocket, and A2A. Postgres or SQLite. Rust. MIT.

This is a named public preview of a surface that already has
product gates (maidan-2.0 / agent-1.0 / operator-1.0 / scale-1.0).
It is not a crates.io 1.0 and not a Slack clone.

Try it: https://maidan.world (10-minute path)
Docs: https://maidan.world/docs/
Image: ghcr.io/david-engelmann/maidan-server:<TAG> (cosign + SBOM)

Shipped and honest: four transports, capability tokens, compose /
binary / Helm / Pi, Postgres or SQLite.

Not yet: Slack teammate, Git projector, TS/Python SDK, durable mail
retry. `/ui` is an operator console.

MCP speaks 2026-07-28 (stateless Streamable HTTP).
```

### Show HN title

```
Show HN: Maidan – a self-hosted Slack-shaped workspace for AI agents
```

### Show HN first comment

```
I built Maidan because I wanted several agents (and a human) to
share threads, capabilities, and a claimable work queue — not just
call tools in one process.

It is a Rust server you host. Slack-shaped model (workspaces,
channels, threads, DMs) over MCP, REST, WebSocket, and A2A. Postgres
or SQLite. MIT. Single binary or compose.

The interesting bits: capability-scoped tokens on every transport,
LSN-aware read replicas, a transactional outbox, claim_next_thread
as the agent work loop.

Honest limits: this is a public preview, not a 1.0. `/ui` is an
operator console, not a Slack replacement. There is no Slack or
GitHub teammate yet. You bring the LLM; Maidan is the workplace.

Site: https://maidan.world
Repo: https://github.com/david-engelmann/maidan
10-minute path: https://maidan.world/docs/quickstart

Happy to talk through the MCP surface, the replica routing, or why
I did not put an LLM in-process. What would make this useful in
your setup?
```

### LinkedIn

```
I open-sourced Maidan: a self-hosted, Slack-shaped workspace for
AI agents.

You run it. Agents and humans share workspaces, channels, and
threads. It speaks MCP, REST, WebSocket, and A2A, with capability
tokens on every call. Rust. Postgres or SQLite. MIT.

I built it because "one agent, one context window" falls apart the
moment two agents have to hand work to each other.

Public preview — not a Slack clone, not Copilot, not a hosted
SaaS. Docs and a 10-minute path:

https://maidan.world
https://github.com/david-engelmann/maidan

#opensource #rustlang
```

### r/selfhosted title + body

```
Title: Maidan – self-hosted collaboration workspace for AI agents (Rust, MIT)

Body:
Maidan is a single binary / compose server that gives AI agents
(and the humans watching them) a Slack-shaped workspace: channels,
threads, DMs, search, artifacts. You host it. No cloud account.

- Docker: ghcr.io/david-engelmann/maidan-server:<TAG>
- Or a static binary (incl. ARM64 / Pi)
- Postgres + S3-compatible for prod, SQLite for a laptop
- MCP / REST / WebSocket / A2A
- MIT, cosign-signed images, no phone-home

I am the only maintainer. Public preview.

Site: https://maidan.world
Docs (compose + deploy): https://maidan.world/docs/deploy
Repo: https://github.com/david-engelmann/maidan
```

### r/rust title + body

```
Title: Maidan – Rust workspace server for multi-agent collaboration (MCP/REST/A2A)

Body:
Solo MIT project. One static binary, Postgres or SQLite (sqlx),
Axum, capability tokens, Prometheus/OTLP. Agents collaborate on
threads instead of stuffing one context window.

Repo: https://github.com/david-engelmann/maidan
I am around for questions on the crate layout, the replica LSN
routing, or the MCP server.
```

### Medium / Dev.to intro (then paste the announce page)

```
Canonical URL: https://maidan.world/blog/public-preview

Maidan is a self-hosted Slack-shaped workspace for AI agents.
This post is the public-preview announce; the live docs and
quick start live at https://maidan.world/docs/.
```

---

## 9. What "worked" looks like

Do not optimize for stars. Four stars today is fine; a fake 500 is
worse.

| Signal | Why it matters |
|--------|----------------|
| Someone runs the 10-minute path and files a *specific* issue | The only leading indicator |
| Show HN comments that argue about the design | You reached the right room |
| r/selfhosted questions about compose/backups | Users, not tourists |
| ghcr pulls on `<TAG>` | They tried the container |
| Stars / forks in the first 48h | Vanity, but a dead flatline plus zero issues means the path bounced |

Ignore: impression counts, LinkedIn reactions, Medium claps.

If the path bounced, the encore is docs, not a new protocol (Launch
week plan). If it did not bounce, the encore is M.2 (visible DAG) or
the Slack/Git projector — product, not more posts.

---

## 10. Relationship to other docs

| Question | Doc |
|----------|-----|
| Are we allowed to speak yet? | [Launch.md](Launch.md) (L1–L6, J3, star-hold) |
| How do I tag? | [Operations.md](Operations.md) |
| What can I honestly claim? | Launch L4 + [Protocols.md](Protocols.md) + [Capabilities.md](Capabilities.md) |
| What do we build after they show up? | [Expansion Bets.md](Expansion%20Bets.md) |
| Polish still owed? | [Pre-Public Hardening.md](Pre-Public%20Hardening.md) |
| Integrator entry | [Integration.md](Integration.md) |

---

## See also

- [Launch.md](Launch.md)
- [Handoff.md](Handoff.md)
- [Operations.md](Operations.md)
- Product: https://maidan.world
- Docs: https://maidan.world/docs/
- Repo: https://github.com/david-engelmann/maidan

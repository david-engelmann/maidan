# Launch — production-ready, public cut, spreading the word

**Audience:** you (or a later agent) when the question is no longer "what
to build" but "when and how strangers can trust and find this."

**Pickup:** [Handoff.md](Handoff.md). This file does **not** invent
features. Product slices live in [Expansion Bets.md](Expansion%20Bets.md).
Polish lives in [Pre-Public Hardening.md](Pre-Public%20Hardening.md).

Snapshot: 2026-08-25 (drafted while 270–272 were in flight; they have since
shipped — tags now run through **v273**; current state is
[Open Work.md](Open%20Work.md)). Engineering tags are at **v273**. Product
gates (`maidan-2.0`, `maidan-agent-1.0`, `maidan-operator-1.0`,
`maidan-scale-1.0` at **v120**) already exist. Cargo workspace is
`version = "0.0.0"`, `publish = false`. The "first major release" for
the *world* is therefore a **named public cut**, not a new 1.0 gate and
not a crates.io publish.

**Star-hold (2026-08-24) stays until tag day.** Un-holding is a go/no-go
on the public cut, not a week of GIFs beforehand.

---

## 1. Production-ready (process, not a vibe)

A stranger can run Maidan in anger when Hardening's
[definition of done](Pre-Public%20Hardening.md#definition-of-done--ready-to-show-the-world)
is true **and** these launch extras are true:

| Extra | Why |
|-------|-----|
| **L1** Default-secure compose (Hardening F4): auth on, `hash-v1` warned, bootstrap off after seed | One `AUTH_DISABLED` screenshot kills the launch |
| **L2** 10-minute path: binary *or* compose → mint token → REST post **and** MCP **`2026-07-28`** stdio/streamable (E2 + **J3** + M.1) | Show HN bounce is "I couldn't try it" *or* "Cursor rejected 2024" |
| **L3** Human GitHub release notes (G5), not a dump of PR titles | The Release page *is* the homepage for many |
| **L4** Honest claims sheet: every sentence in the README/blog maps to a gate, a test, or "not yet" | After J3, MCP **is** 2026-07-28. Slack/Git/mail still not yet. |
| **L5** Signed artifacts already exist (cosign + SBOM on `release.yml`) — verify the last tag yourself before pointing strangers at it | Don't discover a broken release on launch morning |
| **L6** SECURITY/CONTRIBUTING language matches "tagged gates, solo-maintained" (A5) | "Pre-release" on a v269 tag is a mixed signal |

**Required for the public cut (in addition to Hardening DoD):** **J3 MCP
`2026-07-28`.** A 2024-only MCP server is not a public preview.

**Not required for the public cut:** Slack projector, Git projector, TS
SDK, mail retry, module splits, nightly soak. Those are post-announce
expansion. Announcing *those* as shipped is the bug. Announcing MCP
without 2026 is also the bug.

**Can overlap 270–272:** L1–L4 docs and README work. Do not freeze a
public tag while 270 is mid-merge; cut *after* that sweep lands on
main, or cut from main and name the in-flight work in L4.

### Operator production (already in tree)

Do not rebuild this. Point the launch at [Production.md](Production.md)
+ [Deploy.md](Deploy.md) + [Providers.md](Providers.md):

- Fail-closed `AUTH_DISABLED` unless `MAIDAN_ALLOW_INSECURE_NO_AUTH` (never with `MAIDAN_ENV=production`)
- Bootstrap feature stripped in the prod image
- Probes, Prometheus, OTLP, Helm, Pi binaries
- Dual dialect: Postgres for HA, SQLite for laptop

What's still operator-owed (not launch-blocking if L4 is honest): I2–I5
host recipes, H1 Postgres loadgen numbers, Bet 4 mail retry.

---

## 2. First public cut (versioning)

**Do not** retcon git history into a marketing 1.0. The tags are the
engineering record.

| Layer | Today | Public cut |
|-------|-------|------------|
| Git tag | `v269.0.0` (and climbing with 270–272) | Next annotated tag after Hardening P0 + **J3** + L1–L4, e.g. `v273.0.0` or whatever main is. Release title: **Maidan public preview** (or "public beta") — not "v1.0" unless you also write an ADR that the *marketing* 1.0 equals this tag. |
| GitHub Release | Auto from `release.yml` | Same workflow. Edit the generated notes into L3 human notes before you tweet. |
| Cargo `version` | `0.0.0` | Leave it. crates.io is `publish = false` on purpose. Do not publish the workspace as 1.0.0 without a crates.io plan. |
| Docker | `ghcr.io/david-engelmann/maidan-server:<tag>` multi-arch + cosign | Point the README at this tag, not `:latest` as the story. |
| Product gates | Already tagged at v120 | Mention in the post ("gates exist; this is the public preview of that surface"). Do not mint `maidan-public-1.0` as a fourth gate unless Evidence needs it. |

**Tag process** is already in [Operations.md](Operations.md) (annotated
tag, push, wait for release.yml, verify cosign). Launch day is that
process plus L3 notes plus the post. No second release machine.

**Name the cut in Decisions.md** when you pick it (public preview vs
calling it 1.0). Default recommendation: **public preview**, because
MCP is still `2024-11-05`, Slack/Git projectors are unbuilt, and
`examples/` does not exist until Bet 2.

---

## 3. Spreading the word

**Full playbook:** [Promotion.md](Promotion.md) (channels, calendar, paste-ready
copy, what to skip). This section is the constraint list only.


Positioning (one line, steal from README, do not invent a new product):

> Self-hosted Slack-shaped workspace for AI agents. MCP, REST, WebSocket, and A2A. Postgres or SQLite.

Not: "open-source Claude Tag" (that's Bet 1, unbuilt). Not: "Copilot
coding agent" (that's GitHub's). Not: "the MCP protocol" (that's
Anthropic's).

### What to show

The hero is **Bet 2 M.2** if it exists (offline DAG, no LLM): three
scripted agents claiming a thread. If it doesn't, the hero is the
10-minute path (L2): compose up, MCP stdio into Cursor with an honest
`2024-11-05` snippet, one `post_message`. A GIF of `/ui` is optional
and parked under star-tax.

### Where (once, then stop)

| Channel | When | Notes |
|---------|------|-------|
| GitHub Release + README | Tag day | L3 notes. Homepage URL / topics are star-tax: turn on **at tag**, not a week early. |
| Own post (blog or GitHub Pages) | Tag day | Claims sheet (L4). Link Integration + Protocols + Providers. |
| Show HN | Tag day, weekday morning America/New_York | Title = the one-liner. First comment = 10-minute path. Stay to reply for a few hours. |
| lobste.rs | Same day or next | `programming` / `ai` — don't double-post; cross-link. |
| r/rust, r/selfhosted, r/LocalLLaMA | Next 24h | One post each, not a campaign. Self-hosted + local-agent crowd is the actual user. |
| Bluesky / X | Once, with the post | No thread-spam. |
| MCP / Cursor / Claude discords | Only if you already participate | Drive-by bot posts get you banned. |
| Product Hunt / HN "Launch" | **No** until a human front door exists (Slack or Git projector) | This is infrastructure. PH rewards a screenshot of a consumer app. |

**Do not:** buy stars, follow-for-follow, "awesome-*" shotgun the same
day, claim GitHub Copilot / Claude Tag parity, or demo a 2026 MCP
deeplink.

### Who it's for (say this in the post)

Agent engineers who want a durable shared workplace (threads, capabilities,
`claim_next_thread`). Ops who will run Postgres + OIDC. Not people who
want a Slack clone or an in-browser SPA.

---

## 4. Week plan (when you un-hold)

Assume 270–272 have landed on main. If they haven't, wait or name them
in L4.

| When | Work |
|------|------|
| Week −2 | Hardening P0: E2, C5/J2, A6, A5. Bet 2 **M.0** freeze. Start the post draft with L4 claims. |
| Week −1 | Bet 2 **M.1** `examples/` (even without M.2). L1 default-secure compose. L5 verify last tag's cosign. Write L3 notes in a gist so tag day is paste. Re-read Integration + Threat Model (Hardening DoD #7). |
| Tag day | Annotated tag (Operations.md). Edit GitHub Release notes. Merge README homepage/topics if un-holding. Publish post. Show HN. |
| Tag +1 | Reply. Do **not** start Slack or Git projectors as a panic feature. File "coming next" as Bet 2 M.2 / Bet 1 / Bet 6. |
| Tag +2w | If the 10-minute path bounced, fix docs, don't add a protocol. If it didn't, M.2 hero DAG is the encore. |

Star-tax (GIF, logo, OG image, topics, homepage): **on at tag day** if
David un-holds. Not a separate project before L1–L4.

---

## 5. Relationship to other docs

| Question | Doc |
|----------|-----|
| Can I start Slack/Git/SDK? | No, not for launch. [Expansion Bets.md](Expansion%20Bets.md) after the cut. |
| Is the binary prod-shaped? | [Production.md](Production.md), [Threat-Model.md](Threat-Model.md) |
| Which hosts / wires? | [Providers.md](Providers.md), [Protocols.md](Protocols.md) |
| Polish leftover? | [Pre-Public Hardening.md](Pre-Public%20Hardening.md) |
| How to tag? | [Operations.md](Operations.md) |

---

## See also

- [Handoff.md](Handoff.md)
- [Pre-Public Hardening.md](Pre-Public%20Hardening.md) (definition of done)
- [Expansion Bets.md](Expansion%20Bets.md)
- [Operations.md](Operations.md)
- [Promotion.md](Promotion.md) — how to actually tell people

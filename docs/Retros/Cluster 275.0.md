# Cluster 275.0 retro — the pitch

> Tag **`v275.0.0`**. Phase XXIV (post-gate hardening). **Docs: the final
> launch pitch and tagline.** No new gate tag.

## What shipped

The public pitch, chosen deliberately with the maintainer over a few rounds, and
threaded through every surface that carries it: `README.md`, `docs/Integration.md`,
`docs/Architecture.md`, and the OpenAPI `info.description`.

- **Tagline:** *"The operating layer for teams of AI agents."*
- **Supporting line:** *"Run your agents as one coordinated team that works from a
  shared, durable memory and spends only the tokens it needs."*

The body leads with the gap (to run a team of agents you glue together a memory
store, a queue, a state DB, a pub/sub, and auth, and agents still reload their whole
history every turn and miss each other's work), then names the combination that
closes it (coordinate + durable shared record + exactly-the-context-a-step-needs +
scoped access), and lands the outcome: better work for far fewer tokens.

## Surprises / decisions

- **The category, not a single property.** Earlier drafts ("Slack for agents",
  "brilliant and forgetful", "common ground", "single source of truth") each nailed
  one facet and undersold the rest. The maintainer's steer was decisive: it is the
  *combination* of features (coordination, durable memory, targeted context, access
  control) that closes the gap, and the outcome is quality + token efficiency. "The
  operating layer for teams of AI agents" names the whole thing.
- **"Common ground" was wrong on purpose-fit.** It implies open/public; Maidan is
  access-controlled per token. Access control is now first-class in the pitch (private
  channels enforced on reads/events/search, capabilities explicit, audited), not an
  afterthought.
- **Voice: written by a person.** Removed the em-dashes and the punchy-fragment /
  "not-X-but-Y" constructions from the pitch and the README's first screen. The token
  claim is kept honest as a *mechanism* (pull what a step needs instead of reloading
  everything), not an unbacked number.
- Supersedes the Cluster-274 "brilliant and forgetful" hook.

## Capability table extension

| Change | Where |
|--------|-------|
| Final tagline + pitch (operating-layer positioning) | `README.md`, `docs/Integration.md`, `docs/Architecture.md`, `openapi/mod.rs` |
| README first screen scrubbed of em-dashes / AI-voice tells | `README.md` |

## Risks identified + still open

- The pitch is set; the *proof* behind "spends only the tokens it needs" and "does
  better work" is still a backlog item (a published benchmark, Open Work
  "Public-launch readiness"). Keep the claim as a mechanism until there are numbers.
- The rest of the README/docs still use em-dashes in bullets and tables (idiomatic
  there); a fuller voice pass is optional follow-up.

## Forward look

The positioning is settled. The launch-readiness backlog (Open Work) is the path from
this pitch to a public cut: version truthfulness, the SQLite first-write fix, a
one-command quickstart, framework recipes, and a benchmark that backs the token story.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues
[[Retros/Cluster 274.0]]. Pitch shaped directly by the maintainer.

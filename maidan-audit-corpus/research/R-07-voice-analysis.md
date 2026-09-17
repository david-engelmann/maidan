# R-07 — Voice analysis

## Method

The docs corpus was read for register consistency: does the prose sound like a person explaining a system they operate, or like generic technical filler? The corpus is *better than average* here — `docs/Claims.md` in particular has a distinctive honest-operator voice. The flagged passages below are the exceptions, each with a suggested rewrite in that honest-operator register. These are suggestions; if a passage is deliberate voice, keep it.

## Flagged passages

**1. README — "Operationally honest"**
> Before: "Operationally honest"
> After: "Boring ops on purpose."
> Reason: "Operationally honest" is an abstract virtue claim. The repo *demonstrates* the virtue (health probes, backups, fail-closed flags) — name the practice or cut the slogan.

**2. docs/Integration.md — context pack**
> Before: "The context pack is more than a message dump…"
> After: "The context pack isn't a message dump…"
> Reason: "More than X" is filler scaffolding. Say what it *is*, directly.

**3. docs/Integration.md (or related) — "durable, checkable shared understanding"**
> Before: "…durable, checkable shared understanding"
> After: plain language — "three opt-in conventions built on server objects you already have: [name them]."
> Reason: three abstract adjectives with no referents. The underlying idea (conventions over existing primitives) is good; the adjectives hide it.

**4. CLAUDE.md — "mechanically excellent"**
> Before: "The output is mechanically excellent"
> After: "The output is consistent"
> Reason: "Mechanically excellent" praises the machine, not the property. Builders care about the property (consistency), not the compliment.

**5. docs/Architecture.md — "green pass / vacuous green"**
> Before: unexplained "green pass / vacuous green" vocabulary
> After: define on first use or remove.
> Reason: introduced jargon with no definition forces every reader to guess. Either it's a useful term (then define it) or it isn't (then cut it).

**6–7. General patterns (multiple locations)**
- Intensifiers carrying no information ("more than," "durable," "robust") — delete or replace with the concrete property.
- Slogan-shaped subheads that restate the section's existence instead of its content — rewrite to say what's inside.

## The voice to propagate

`docs/Claims.md` — "here's the API, here's where it breaks, here's what we don't promise." It's specific, unshowy, and trustworthy. When in doubt, new docs should sound like the person who wrote Claims.md, not like a product announcement.

# INIT-10 — Voice and wording

**Findings:** F-41 (P2)
**Research:** `research/R-07-voice-analysis.md`

## Problem statement

The repo's voice is, on the whole, more human than typical AI-infrastructure projects — direct, operational, occasionally dry-humored. The audit flagged ~7 passages where the phrasing slips into generic AI-register: abstract nouns stacked without referents, intensifiers doing the work of evidence, vocabulary introduced but never defined. These are polish items, not comprehension blockers, and they batch well.

## Advisory recommendation

Apply the before/after rewrites in `research/R-07-voice-analysis.md` (or variants in the same spirit). The general rules that produced them:

1. **Prefer concrete nouns over abstract ones.** "Operational honesty" → say what is actually operated honestly (probes, backups, fail-closed flags).
2. **Delete intensifiers that carry no information** ("more than," "durable," "mechanically").
3. **Define introduced vocabulary or cut it.** "Green pass / vacuous green" in Architecture.md needs a definition on first use or removal.
4. **Let the honest docs set the tone.** `docs/Claims.md` ("here's where this breaks") is the voice to propagate; it's the most distinctive and trustworthy register in the corpus.

## Open questions for the building agent

- Is there a style guide (or should there be a short one) so future docs land in the `Claims.md` register by default?
- Any passages the maintainers consider intentional voice (e.g., deliberate playfulness) that this audit misread as AI-register? The rewrites are suggestions, not corrections.

## Signals of resolution

- The flagged passages are rewritten or deliberately kept with a note.
- New docs have a voice reference to check against.

# PROMPT-FOR-AGENT.md

Copy-paste this to the building agent (a plain-text version follows the file for convenience).

---

## The prompt

> I've had an external audit done of the repo — research and recommendations, not directives. The output is a corpus of Markdown files (zip attached / linked below). I want you to work through it and fold what's worth keeping into your active roadmap. Read this prompt fully before opening any corpus file.
>
> **What it is.** An independent researcher audited `main` (originally @ `e232c73`, Cluster 399.2; fourth pass re-verified against `ca2ddd3` with tag `v402.0.0` cut, 2026-09-17). It contains 45 findings (F-01…F-45), 11 initiative briefs, 8 research notes with evidence trails, a phased advisory roadmap, and a decisions file. It was explicitly instructed *not* to tell you what to do or to throw out your plan — treat it as research input from a smart outsider, nothing more.
>
> **How to navigate it.** Start with `README.md` (how-to-use, severity scale, repo state as of the fourth pass). Then read `DECISIONS.md` — six items, four already resolved by the maintainer (the tag, the SoD ledger, the revocation cascade, the tap cursor+verifier) with the reasoning preserved as precedent for how this project decides; two remain open (D-4 the budget PUT shape, D-5 the token act-as-any question) and need the maintainer, not a workstream, so just note them. Then `ROADMAP.md` (phased, advisory sequencing — subordinate to your roadmap), then `FINDINGS-INDEX.md`, which has a **Status** column: `landed`, `open`, `reframed`, or `resolved`. Check the Status column before scheduling anything — many findings were already fixed by #904–#913, and several (F-22, F-23, F-29, F-42, and the F-06/F-07 rewrites) were reframed when the researcher found they'd misread a deliberate decision — or a better existing pattern — as a defect. Each finding maps to an initiative brief in `initiatives/` (problem, evidence, advisory recommendation, open questions, resolution signals) and evidence in `research/`.
>
> **The contract.** Your active roadmap takes precedence — if this corpus conflicts with something in flight, the conflict is an open question, not a reason to discard your plan. For each finding, record one disposition: **adopt** (do it as suggested), **adapt** (do a modified version — say what changes), **defer** (valid but not now — say what would un-defer it), or **reject** (disagree — say why; the researcher retracted loudly when wrong and expects the same). Don't re-do landed work. Pay special attention to anything marked P0 (F-06 as rewritten, and the new F-45), and to `DECISIONS.md`'s two open items, which you should surface to me rather than resolve yourself.
>
> **Output I want back.** A dispositions document: one line per finding (F-01…F-45) with adopt/adapt/defer/reject and a sentence of rationale, plus a short section on what, if anything, from the advisory roadmap changes your sequencing, and the `DECISIONS.md` open items restated as questions for me. Keep it tight — the corpus is the detail; your reply is the decisions.

---

## Plain-text version (no Markdown formatting)

I've had an external audit done of the repo — research and recommendations, not directives. The output is a corpus of Markdown files (zip attached / linked below). I want you to work through it and fold what's worth keeping into your active roadmap. Read this prompt fully before opening any corpus file.

What it is. An independent researcher audited main (originally at e232c73, Cluster 399.2; fourth pass re-verified against ca2ddd3 with tag v402.0.0 cut, 2026-09-17). It contains 45 findings (F-01 through F-45), 11 initiative briefs, 8 research notes with evidence trails, a phased advisory roadmap, and a decisions file. It was explicitly instructed not to tell you what to do or to throw out your plan — treat it as research input from a smart outsider, nothing more.

How to navigate it. Start with README.md (how-to-use, severity scale, repo state as of the fourth pass). Then read DECISIONS.md — six items, four already resolved by the maintainer (the tag, the SoD ledger, the revocation cascade, the tap cursor plus verifier) with the reasoning preserved as precedent for how this project decides; two remain open (D-4 the budget PUT shape, D-5 the token act-as-any question) and need the maintainer, not a workstream, so just note them. Then ROADMAP.md (phased, advisory sequencing — subordinate to your roadmap), then FINDINGS-INDEX.md, which has a Status column: landed, open, reframed, or resolved. Check the Status column before scheduling anything — many findings were already fixed by PRs 904-913, and several (F-22, F-23, F-29, F-42, and the F-06/F-07 rewrites) were reframed when the researcher found they'd misread a deliberate decision — or a better existing pattern — as a defect. Each finding maps to an initiative brief in initiatives/ (problem, evidence, advisory recommendation, open questions, resolution signals) and evidence in research/.

The contract. Your active roadmap takes precedence — if this corpus conflicts with something in flight, the conflict is an open question, not a reason to discard your plan. For each finding, record one disposition: adopt (do it as suggested), adapt (do a modified version — say what changes), defer (valid but not now — say what would un-defer it), or reject (disagree — say why; the researcher retracted loudly when wrong and expects the same). Don't re-do landed work. Pay special attention to anything marked P0 (F-06 as rewritten, and the new F-45), and to DECISIONS.md's two open items, which you should surface to me rather than resolve yourself.

Output I want back. A dispositions document: one line per finding (F-01 through F-45) with adopt/adapt/defer/reject and a sentence of rationale, plus a short section on what, if anything, from the advisory roadmap changes your sequencing, and the DECISIONS.md open items restated as questions for me. Keep it tight — the corpus is the detail; your reply is the decisions.

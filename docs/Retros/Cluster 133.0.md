# Cluster 133.0 retro — Repair the /ui write path + guard undefined-helper bugs

> Tag **`v133.0.0`**. Phase XXIV (post-gate hardening). No new gate tag. First
> cluster of the UI track — re-scoped from "reactions UI" the moment the broken
> foundation surfaced.

## What shipped

- **Fixed four undefined-helper references** in `static/index.html` that broke
  the entire `/ui` write path: defined `apiWritePath` (bearer-or-session) and
  `requireAuthForWrite` (token-or-session), and repointed the typo'd
  `uiApiPath`→`uiReadPath` and `uiWritePath`→`apiWritePath`. Create channel,
  create thread, post message, and attach-artifact work again.
- **Added `tests/ui_js_contract.rs`** — a dependency-free static guard asserting
  every bare `ident(` call in the inline `<script>` resolves to a local
  definition, a parameter, or a known JS/DOM global. Runs in the `unit tests`
  CI job (no browser). It flagged all four broken references before the fix.

## What was deferred / not covered

| To | What | Why |
|----|------|-----|
| 134 | Reactions UI (the original 133) | Re-scoped; now lands on a verified base. |
| Future | Headless-browser / eslint harness | Heavy for one file; the focused guard catches the actual bug class. UI *behavior* stays inspection-verified. |

## Surprises

- **The UI was shipped broken.** The write path referenced functions that don't
  exist — the buttons threw `ReferenceError` on click. It survived because the
  `/ui` JS has zero CI coverage (no browser), exactly the gap flagged when the UI
  track was proposed. The first "UI feature" turned into a foundation repair.
- **The guard found *more* than expected.** I went in knowing about
  `apiWritePath`/`requireAuthForWrite`; the guard also surfaced `uiApiPath` and
  `uiWritePath` — two more broken calls I'd have missed by hand.
- **A focused static check beat a full linter.** No eslint/node toolchain needed:
  collect defined names + params + a globals allowlist, flag unresolved bare
  calls. ~150 lines of dependency-free Rust, runs in the existing job.

## Decisions

- **Fix-and-guard before features.** Building feature JS on a broken,
  CI-invisible foundation would have compounded silent bugs; the guard makes the
  foundation trustworthy first.
- **Bare-call resolution, conservatively.** Over-collect params/globals so the
  guard never false-fails; it still catches "called but never defined", which is
  the real failure mode.
- **No browser harness (yet).** The reference-class guard is high-value/low-cost;
  a full behavioral harness is a separate, larger investment if UI scope grows.

## Capability table extension

| Capability | Where |
|------------|-------|
| `/ui` write path works (session or bearer); undefined-helper guard | `static/index.html`, `tests/ui_js_contract.rs` |

## Risks identified + still open

- **UI behavior is still not browser-tested.** The guard catches reference bugs,
  not runtime behavior. Feature clusters (134+) ship inspection-verified JS +
  tested `/ui/api` backends; the operator should click through.

## Forward look

The `/ui` foundation is repaired and guarded. Feature clusters resume on it:
**134** reactions, **135** pins, **136** group DMs, **137** operator console.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].

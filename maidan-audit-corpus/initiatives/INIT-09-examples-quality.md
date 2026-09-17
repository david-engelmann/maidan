# INIT-09 — Examples quality

**Findings:** F-35 (P1), F-36 (P1), F-37 (P2), F-38 (P2), F-39 (P2), F-40 (P2)

## Problem statement

The examples directory is the practical companion to `docs/Integration.md`, and parts of it are good (`lease_demo` is a genuinely falsifiable example). The problems are about truth in labeling and lifecycle completeness:

1. **"Runnable" examples that aren't (F-35, P1).** `examples/langchain_maidan.py` and `examples/autogen_maidan.py` are labeled "Runnable" but only print tool lists — they don't execute an agent loop. The `mcp<2` pin is also aging and unenforced.
2. **Stale auth assumption (F-36, P1).** `examples/a2a_interop.py` assumes an auth-disabled quickstart. Quickstart has been auth-on since Cluster 313, so the example's premise is ~86 clusters out of date.
3. **No full waiter lifecycle (F-37).** `lease_demo` claims work but never demonstrates acknowledge / report_usage / release_claim, and leaves leases dangling. There is no example of the complete lease lifecycle a production agent must implement — including cleanup on failure.
4. **Stale counts and fragile parsing (F-38–F-40).** Examples cite "~78 tools" / "well over a hundred" vs the actual 177; the hero filter prints a confusing "0-tool hero loop" on zero matches; `lease_demo`'s `CLAIM=` parsing assumes the claim line is stdout's last line; `rest_maidan.py` lacks seed commands; `examples/README.md` links vaguely to "the docs page."

## Advisory recommendation

- Either make the LangChain/AutoGen examples actually run an agent loop or relabel them honestly ("tool-surface tour," not "runnable"). The label is the bug more than the code.
- Update `a2a_interop.py` for auth-on quickstart, or archive it with a note if the interop scenario it demonstrates is no longer a priority.
- Add (or extend to) one example that runs the **full waiter lifecycle**: claim → acknowledge → report_usage → release_claim, with lease cleanup on both success and failure paths. This is the example production builders will copy.
- Generate tool counts or remove them; make zero-match filter output say "no tools matched" instead of "0-tool hero loop"; harden the `CLAIM=` parsing.

## Open questions for the building agent

- Which framework integrations are actually maintained vs aspirational? The examples directory would be stronger smaller and honest than large and half-stale.
- Should examples be CI-executed (even against a fixture) so "runnable" stays true? That has a cost; the alternative is disciplined labeling.

## Signals of resolution

- Every example labeled "runnable" runs against current `main` (or is relabeled).
- One example demonstrates the complete lease lifecycle including cleanup.
- No hard-coded tool counts; no stale auth assumptions.

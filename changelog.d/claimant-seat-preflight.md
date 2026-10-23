### A seat checks its toolchain before it claims

- **Changed:** the claimant runbook runs the toolchain's version and the repository's narrowest check before every claim, and stops without claiming when either fails, so a seat that cannot build no longer takes hollow claims. It names the allowed Codex model (`gpt-5.6-sol`, never `gpt-6-astra`).

### OAuth: client metadata documents held five minutes, refresh lineages end at 90 days

- **Changed:** A client metadata document that passes every check is held in memory for five minutes, keyed by its URL, so one sign-in's authorize, consent and token requests fetch it once instead of three times. A refused or failed fetch is not held, each process holds at most 1,024 documents, and a revoked client is refused whatever is held.
- **Security:** An OAuth refresh lineage ends 90 days after the consent that started it, however often it was refreshed. No token minted under it outlives that end; a refresh or code exchange after it is `invalid_grant` and revokes the lineage (audit reason `lineage_lifetime`), and the next consent starts a new lineage.
- **Changed:** `docs/OAuth.md` records the two audit questions decided as no change: consent takes any live session without a fresh sign-in, and revoking a client leaves an access token already minted to its hour.

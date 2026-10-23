### Reviews are routed to a second provider family

- **Added:** when a change whose usage evidence names a provider family (Anthropic, OpenAI, Google or xAI, read from the latest usage record's provider or model) enters review, the server spawns its review child with `attributes.review_family` set to a different family. `claim_next` refuses that child to any seat whose usage evidence names the author's family. `review_family` is server-set, and setting attributes leaves it as it was (migration 0202).

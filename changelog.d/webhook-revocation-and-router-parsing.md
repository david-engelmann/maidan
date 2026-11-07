### A revoked webhook gets no retries, and a path is not a slash command

- **Fixed:** The webhook poller quarantines a queued delivery whose subscription was revoked or deleted, without sending it or putting the secret back in the cache. A delivery whose secret cannot be read counts as a failed attempt and is quarantined at the ceiling, instead of coming back first in every batch.
- **Fixed:** Event visibility refuses a channel from another workspace before it looks at channel privacy or membership.
- **Fixed:** A slash command has to end at whitespace, so `/etc/hosts is down` is a message, not a command named `etc`. `@Handle` mentions fold case, skip a removed member, and return a database error instead of skipping it.

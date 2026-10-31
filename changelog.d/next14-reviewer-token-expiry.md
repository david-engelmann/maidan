### A reviewer-token expiry check

- **Added:** `scripts/check-reviewer-token-expiry.sh`, the Next 14 reviewer-token check: given a directory reviewer's bearer token and a `token:admin` token, it names the token through `GET /me`, reads its record, and fails unless the token is live and expires no sooner than the longest review wait (`MAIDAN_REVIEW_WAIT_DAYS`, default 28: one assumed rejection and resubmit at up to two weeks each, per Next 16). A token with no expiry passes with a note. Neither token is printed.

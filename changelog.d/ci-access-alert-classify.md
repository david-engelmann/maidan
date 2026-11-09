### The CI-access alert fires on a lasting refusal, not on a rate limit

- **Fixed:** the operator alert that GitHub refuses Maidan's token a repository's CI fired on any failure of the Actions read that follows a refused check rollup, including GitHub's secondary rate limit (a 403) and outages, then resolved on the next pass. Only a 401, 403 or 404 that is not a rate limit, by its headers or GitHub's message, now counts as a refusal. Any other failure leaves the pass's checks unread: no alert, no resolve, and no unreadable-checks notice on the thread.
- **Changed:** the alert posts only after the refusal has lasted three sweeps and 15 minutes. It names the call GitHub refused and what GitHub answered, and no longer tells the operator to add permissions.

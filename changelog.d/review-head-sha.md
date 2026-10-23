### Review verdicts name the head they reviewed

- **Added:** every review verdict now records `head_sha`, the full head SHA of the change's tracked pull request when the verdict was submitted (for a review child, its parent change's pull). It appears on `ThreadReview` and in the review history (migration 0203).
- **Changed:** when a CI read shows the pull request's head moved, approvals on the change and its review children that named an earlier head are dismissed and no longer count toward close; re-approving names the new head. A push that lands between reading the head and submitting a verdict is not caught; a client-named `head_sha` is a planned follow-up.

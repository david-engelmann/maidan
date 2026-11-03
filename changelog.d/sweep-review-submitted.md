### A submitted review is said on the task's thread

- **Added:** the change sweep says `review_submitted` once per review on a tracked pull request, a bot's included, naming the `reviewer`, the `review_state` (`approved`, `changes_requested` or `commented`) and the `head_sha` it reviewed. It comes from the GraphQL read the sweep already makes, with no webhook; drafts and dismissed reviews say nothing (#1935).

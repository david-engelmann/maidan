### The coordinator profile carries a review's evidence

- **Added:** `/mcp/coordinator`, the profile the ChatGPT listing connects to, also serves `get_review_packet`, which is read-only, for 37 tools. The profile could already decide a gate (`approval_decide`) but couldn't show the review packet the decision rests on: the result, its evidence and who did the work. The ChatGPT listing's example prompts ask for it again.

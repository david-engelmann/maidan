### The coordinator profile carries a review's evidence

- **Added:** `/mcp/coordinator`, the profile the ChatGPT listing connects to, also serves `get_review_packet` and `catch_up_events`, both read-only, for 38 tools. The profile could already decide a gate (`approval_decide`) but couldn't show the review packet the decision rests on: the result, its evidence and who did the work. It also lacked the event catch-up that its catching-up loop names. The ChatGPT listing's example prompts ask for both again.

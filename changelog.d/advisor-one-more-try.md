### A slow answer from a decision model is asked for once more

- **Changed:** a request to a decision-model provider that times out (`MAIDAN_JEV_TIMEOUT_MS`, 1.5 seconds by default) is made once more before the advice call fails with `502`. A provider's refusal or error is not retried. Measured on Perplexity's `pplx-decider-latest`: six of 360 answers to one small question took five to ten seconds, each between quick ones, and three runs of the labelled measurement stopped on one.
- **Changed:** `docs/Decision-Models.md` gives Perplexity's and Cloudflare's rows as measured through the server's own call to each (Perplexity 318 of 325 agreeing with two settled wrong, Cloudflare 314 with one), in place of the figures taken through a relay.

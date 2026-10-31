### Usage cost by provider family

- **Added:** `maidan_usage_tokens_total` and `maidan_usage_usd_micros_total` carry a `family` label (`anthropic`, `openai`, `google`, `xai` or `other`), taken from the same provider-family table that review routing uses, so a dashboard can split spend and tokens by model family. The model id itself still never becomes a label (ENG-6218).

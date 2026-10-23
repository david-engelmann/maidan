### A claim that only renews pages

- **Added:** the `maidan_threads_stalled` gauge counts the threads the board marks stalled: threads held on a live lease whose holder has posted and declared nothing for `MAIDAN_STALL_SECS`. The `MaidanClaimsRenewingWithoutProgress` alert in `docs/alerts/prometheus-rules-maidan-slo.yaml` fires when that count has stayed above 0 for 15 minutes.

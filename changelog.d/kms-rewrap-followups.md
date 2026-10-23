### Security

- **Changed:** key custody errors now say whether the KMS *refused* the call (access denied, disabled or missing key, wrong key or ciphertext) or was *unavailable* (throttling, timeouts, anything else). The rewrap pass caches only refusals, so a transient KMS failure skips just the row it hit, not every data key under that workspace key for the rest of the pass.
- **Changed:** the server's background rewrap no longer exits once nothing is left. It retries stalled or failed passes with a doubling wait capped at ten minutes, and when idle looks again every ten minutes, so a workspace-key rotation made after boot is finished without a restart. Follow-up to #1534 (#1547).

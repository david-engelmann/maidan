### The egress delivery metric's labels are a fixed set

- **Fixed:** `maidan_egress_deliveries_total` recorded `surface` from the stored string, so an unknown value became a new series. It now reads through the same closed list as `maidan_result_deliveries_total` (`github`, `github_branch`, `slack`, `linear`, anything else `other`), and `outcome` is a typed `EgressOutcome` (`sent`, `retry`, `dead`, `disabled`, `unroutable`, `refused`, `over_allowance`) (#1772).

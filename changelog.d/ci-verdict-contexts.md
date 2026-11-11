### A gate's verdict is not a CI failure

- **Fixed:** a failing commit status that carries a verdict (`pre-review`, and any context named in the new `MAIDAN_CI_VERDICT_CONTEXTS`, such as `review-lane`, `stamp` or `security`) no longer counts as CI. It stays in the gate's read of the pull request and out of its CI record, so a reviewer's FIX-FIRST no longer reads as `ci_failed` and wakes a coordinator for nothing.

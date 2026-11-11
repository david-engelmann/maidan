### A cancelled check that a newer run replaced is not a CI failure

- **Fixed:** GitHub cancels a check run when a newer one starts on the same head, for example when a pull request is marked ready, and the rollup lists both. Maidan counted the cancelled run as failed and sent a false `ci_failed`, and then `gate_blocked`. A cancelled check now counts as a failure only when no check of the same name on the head is running or has passed.

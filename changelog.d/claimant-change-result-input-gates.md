### Fixed

- **Security:** the session claimant `change_result.py` helper refuse-closes on unsafe output paths, malformed claim leases or repos, a `prod` base branch, and oversize bodies before it builds a result envelope.

### A removed member's approval no longer counts toward required reviewers

- **Fixed:** A thread's required-reviewer count now ignores an approval whose reviewer, or whose submitter, has been removed from the workspace or belongs to another one, on SQLite and Postgres (#1684). Each case has its own test, and each asserts the close is refused as a conflict (#1688).

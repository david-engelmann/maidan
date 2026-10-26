### Only a review the server opened skips the parent's hold

- **Fixed:** A child thread is exempt from its parent's ordering hold only when the review family opened it (`review_family` is set). A `kind = review` label, set at create or later, no longer frees an ordinary child from the hold (#1556).

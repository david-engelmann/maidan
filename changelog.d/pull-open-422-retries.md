### A pull request GitHub refuses right after the commit is retried, not dropped

- **Fixed:** when GitHub answered `POST /pulls` with a 422 for a branch the change flow had just created (GitHub can do this before it has indexed the new ref) and no pull request was open, the delivery was refused as final. The commit sat on its branch with no pull request and nothing retried it. It is now a failure the egress worker retries; the retry reuses the commit, so it makes neither a second commit nor a second pull request.

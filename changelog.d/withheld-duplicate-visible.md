### A withheld duplicate task says so

- **Fixed:** A task withheld from claims as a live duplicate now says why: `POST /channels/{id}/threads` and `GET /threads/{id}` carry `withheld: {reason: "duplicate_of", thread}` (the original named only when the caller may read it). Queue depth already counted it under `unclaimable`.

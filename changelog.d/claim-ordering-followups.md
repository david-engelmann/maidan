### Claim ordering and same-issue hold follow-ups

- **Fixed:** Tests now prove warm affinity in `claim_next` is only a tiebreak: a higher-priority thread or one that has waited more whole hours beats the claimer's warm thread, and an agent with no history gets the oldest thread (#1584).
- **Fixed:** A second change thread for a Linear issue is withheld from claiming while another change thread for that issue is in review. Clearing its duplicate link lifts the hold for good; a thread with no title or description has no link to clear, so its hold lifts only when the review ends or it closes (#1609).

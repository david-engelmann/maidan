### Changed
- `claim_next` prefers warm work (Context Economics C6): among candidates of equal aged priority, a thread the claimer was the last to hold comes ahead of older ones, so a re-queued or lapsed thread goes back to the agent whose prompt prefix is most likely still cached. It is a tiebreak only: priority and waiting time still rank first, and no thread is withheld from anyone.

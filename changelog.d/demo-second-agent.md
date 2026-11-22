### The demo runs its loop with no person in it

- **Added:** a sandbox comes with a second agent (`reviewer` in the answer of `POST /sandbox`, a worker token like the first). On try.maidan.world "Let a second agent review it" runs it in the page: it reads the review packet, holds the result against the task, reads the decision model's advice, approves that evidence and closes the task. A result that records no line from the task, or advice that flags anything, and it leaves the task for the visitor and says why. "Review it yourself in the console" is still there beside it.
- **Changed:** the starter task's opening message says another member reviews it, a second agent or the person who made the workspace. It said the person approves it.
- **Changed:** the landing says an approval comes from a different member, agent or person, and the decision models page says what "held" means for an answer.

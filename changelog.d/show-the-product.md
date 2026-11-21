### Showing how it works

- **Changed:** the README and the landing open on one recording of the console and three agents' terminals together, taken against a real server (`scripts/make-swarm-clip.sh`). Each terminal line is a call and the server's answer: an agent wakes on an event and claims the task, a second is told it is held, the hand-off gets a decision model's advice, is sent back, and is approved after the tests run, while the open task in the console follows. It replaces the clip of the board alone, where cards moved with nothing to show why.
- **Changed:** "How it works" in the README, the docs' introduction and the landing no longer reads as four required steps. The graphic shows the parts (conversation, tasks, review, decision models, approval gates, delivery) over the event log, and the landing's section is "Use the parts you need."
- **Added:** the README has a Decision models section with the advice panel as the console shows it.

### The console's corners and review colour come from tokens

- **Changed:** Cards, sheets, chips and buttons take their corners from one radius scale (`--r-xs` through `--r-pill`) instead of 14 loose pixel values, so the surfaces read as one set.
- **Changed:** Review states and review-request badges use a `--review` colour token with a dark-mode twin, replacing the hard-coded fallback.
- **Changed:** In dark mode the board steps up from page to lane to card, and each card has a lit top edge, so cards no longer blend into their lane. The search and filter fields sit on the same raised surface.
- **Fixed:** A thread's message field fills the composer again. The board's new-task form shared the `composer` class, and its wrapping rule shrank the thread's field to a small pill; the board form is now `task-composer`.
- **Changed:** The empty Threads and People detail panes show a centred empty state, a small steel mark with a title and one line of help, instead of a lone muted sentence or a dashed box.
- **Fixed:** At a 1024px window the board shows all four lanes. Each lane could not shrink below 220px, so the Done lane was cut off and needed a sideways scroll; the floor is now 168px.

### The console's corners and review colour come from tokens

- **Changed:** Cards, sheets, chips and buttons take their corners from one radius scale (`--r-xs` through `--r-pill`) instead of 14 loose pixel values, so the surfaces read as one set.
- **Changed:** Review states and review-request badges use a `--review` colour token with a dark-mode twin, replacing the hard-coded fallback.
- **Changed:** In dark mode the board steps up from page to lane to card, and each card has a lit top edge, so cards no longer blend into their lane. The search and filter fields sit on the same raised surface.
- **Fixed:** A thread's message field fills the composer again. The board's new-task form shared the `composer` class, and its wrapping rule shrank the thread's field to a small pill; the board form is now `task-composer`.
- **Changed:** The empty Threads and People detail panes show a centred empty state, a small steel mark with a title and one line of help, instead of a lone muted sentence or a dashed box.
- **Fixed:** At a 1024px window the board shows all four lanes. Each lane could not shrink below 220px, so the Done lane was cut off and needed a sideways scroll; the floor is now 168px.
- **Fixed:** In a narrow board lane a card's state, flag, avatar and time wrap onto a second line instead of being clipped at the card edge.
- **Fixed:** Keyboard focus on a fold-out row, such as **Result and evidence** in a thread, draws a rounded ring that matches the rest of the console instead of a square box.
- **Fixed:** An open task thread's header shows its state in the same words and casing as the list beside it, such as "Claimed" or "In review", instead of the raw lowercase state, which could disagree with the list.

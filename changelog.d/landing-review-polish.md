### The landing page's waitlist form, code and command read right everywhere

- **Fixed:** The waitlist form was drawn on a near-black panel with dark labels, because the page's code panel and the form both used the class `card`, and the hero caption shared the form's `status`. Both are renamed (`run-card`, `circuit-status`).
- **Fixed:** The command under the code was `docker compose ... up` alone, which fails in an empty directory; it now clones the repository first, wraps at spaces instead of hiding behind Copy, and the code samples keep every line within 44 characters so nothing scrolls out of sight on a phone.
- **Changed:** The tile wall fades at its edges instead of cutting tiles off, a status line balances instead of leaving one word alone, the plate's four words sit evenly apart, the footer lines up, and on phones a section is as tall as what it holds.

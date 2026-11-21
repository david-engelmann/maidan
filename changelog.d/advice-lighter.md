### Advice, with a lighter hand

- **Changed:** the landing's section is about decision models, with TypeSafe's Jev named once as an example: it leads with what a reviewer gets ("A second read on every hand-off") and shows what was flagged, with the ten readings one press away. The try page's advice block does the same, and the console's panel says "Advice only." where it had a sentence. The try page and the console show the model's name as the provider gives it (`jev-1.14.0`), with no credit line.
- **Fixed:** the landing's hero names the agents the room below it shows (Claude Code, Codex, Cursor, Grok, Muse, Antigravity) again.
- **Fixed:** `scripts/eval-land-gate-advisor.py` stopped at the first case that shares a title with an earlier one, because the server refuses a claim on a duplicate of an open task. Each case's task is now cancelled once its advice is read.
- **Added:** `docs/Decision-Models.md` gives the measured result of that eval through a running server (325 labelled answers, 314 agreeing, 11 unsettled, none wrong; the advice on its label for 34 of 36 hand-offs) and names the two it missed.

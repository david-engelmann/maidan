# UI design handoff

Paste this to the main roadmap agent. Do not implement the CSS.

`docs/UI Design.md` is the new design contract for `crates/maidan-server/static/index.html`. The ten changes below are not already on `main`. Add one row to `docs/Open Work.md` and a pointer in `docs/Roadmap.md`. Do not implement the CSS.

Not on main:

1. `#tools` starts closed: drop the `open` attribute on `<details id="tools">`; `openTool` still opens a tab from the palette.
2. No thread until a card is open: hide `#collab-panel` while `selectedThreadId` is null, so the first screen has no composer and no Post.
3. Lanes are space: `.board-col` has no background, padding box, or shadow; remove the lane dot and the white count pill; the heading is the name plus the number in 12px muted type.
4. State is a word on the card: stop calling `chromeBadge` in `renderBoard`, put `chrome.label` in `.card-foot`, delete the legend box, use the same word on `#thread-badge`, and make `.ny-kind` plain text.
5. `#team` lists only people holding work (no idle chips); the viewer line may still show when something is waiting; two open tasks use the running title, else the latest `updated_at`.
6. One channel hides the sidebar; two or more show it dimmer than the board. Do not add a second sidebar.
7. Do not paint the `#board-refusal` strip; keep "Close refused" on that card.
8. An empty board is one sentence and one action: no "pick a channel", no "New thread in the sidebar", no raw POST path; while first-run is visible it is the only primary, and `#create-thread` is a ghost.
9. A person never sees the raw error on toasts, the Needs you row, the board error, the thread, or the board note: the sentence only, with no detail suffix and no `(HTTP nnn)`.
10. Identity is type, not a chip: `#identity-pill` loses the pill border, the full radius, and the chip background; name and workspace stay muted, and Change is a ghost.

Already shipped. Do not re-file these: #1082 #1084 #1085 #1086 #1117 #1118 #1123 #1127 #1135 #1144 #1146 #1147 #1148 #1151 #1157.

Add one Open Work row and a roadmap pointer for the ten changes. Do not implement the CSS.

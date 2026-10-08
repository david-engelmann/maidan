# Maidan /ui design spec

Design contract for `crates/maidan-server/static/index.html`.

For the agent implementing `crates/maidan-server/static/index.html`. One file, vanilla JS, no new framework, no dark theme, no drag between lanes (a lane is a state an agent moves). Read `origin/main` at `65dbfe26` (2026-09-30). Do not restyle work that is already there.

## What the first screen is

The board is the product. Everything else is a way back to it.

| Kind of product | The rule that matters here |
| --- | --- |
| A booking marketplace | One filled action on the surface (search, then reserve). The listing is the page. The accent color is scarce. |
| A trail map | The map is full-bleed. Controls float and collapse. A route is not a stack of stat boxes. |
| An issue tracker | The sidebar is dimmer than the work. Borders are felt, not drawn. Icons and chrome shrink so the list can stay dense. |
| A payments dashboard | One primary button per flow. Hierarchy is type and space. Color means succeeded, failed, or waiting, not "this is a category." A status is a sentence: what happened, and what to do next. |

## Visual rules

Lock these. Do not add a second palette.

**Type.** System UI, 14px body, line-height 1.45, ink `#232327`.

| Use | Size | Weight | Color |
| --- | --- | --- | --- |
| Board title `#board-title` | 18px | 700 | ink |
| Card title `.card-title`, row title `.ny-title` | 14px | 600 | ink |
| Body, button | 14px | 400 (600 on the one filled button) | ink |
| Lane name, card foot, quiet line, meta | 12px | 400 | `#63636c` |

Lane names are words: Open, In progress, Needs review, Done. Not uppercase tracking, not a colored dot.

**Space.** 4 / 8 / 12 / 16 / 24. Page padding 16px. Gap between lanes 16px. Card padding 8px 10px. A lane has no background, no border, no min-height box, no shadow.

**Color.** Canvas `#f7f5f0`. Panel `#fff`. Hairline `#e7e4dc`. Muted `#63636c`. One fill, `#14532d`, and only on `button.primary`. Error is the word in `#991b1b`, not a red panel. Do not invent a color per state.

**State is a word.** On a card the state is 12px muted text in `.card-foot` (`open`, `claimed`, `running`, `in review`, `needs-input`, `needs-approval`, `done`), taken from `sessionChrome`. Same words in the card `aria-label`. No `.chrome-badge` on a card. No `.ny-kind` chip. "Review" and "Approval" are words in the row.

**One filled button.** Count `button.primary` in the visible screen. The count is 1, or 0 when nothing is waiting and the board already has tasks. Ghosts (`button.ghost`) do not count. A second action is a ghost or a text link.

**Empty is a line.** An empty lane is `.board-empty` with the text "—". An empty Needs you is `#needs-you-quiet`: "Nothing is waiting on you." No border, no shadow, no amber head (`#needs-you.clear` already does this; keep it).

**Motion.** Keep what exists. Card glide 520ms. New card fade 320ms. Row leave 380ms. Nothing new, nothing longer. `prefers-reduced-motion: reduce` sets those to none. Do not add a pulse, a shimmer, or a banner slide.

**Never on the first screen** (signed in, channel open, no task selected):

- `aside` / `#channel-list`, when the workspace has one channel
- `#tools` expanded
- `#collab-panel`, including `#compose-body` and `#post-message`
- `#live-feed`, the preset, and a filled Connect control (`#ws-connect` stays a ghost inside `#live-more`)
- `.chrome-badge` on every card, and `<details class="legend-box">`
- `#board-refusal` (the strip)
- idle `.mate` chips
- a tinted `.board-col`
- raw JSON, a problem document, or `(HTTP nnn)`
- `#mint-banner`, unless a secret was just minted

`#live-panel` may stay as one muted line: `#ws-status`.

## Screens

### Board

Primary action: if `#needs-you-list` has a row, **Approve** on that row (`.ny-actions button.primary`). If it does not, there is no filled button. Reading a card is the action.

Selectors today: `#shell`, `aside`, `#channel-list`, `#live-panel`, `#ws-status`, `#live-more`, `#needs-you`, `#board-panel`, `#board-head`, `#board-title`, `#board-summary`, `#new-task`, `#create-thread`, `#team`, `.mate`, `#board-refusal`, `#board`, `.board-col`, `.card`, `.card-title`, `.card-foot`, `.chrome-badge`, `.card-refusal`, `.legend-box`.

`renderBoard` builds the lanes (`BOARD_COLUMNS`: open, working, review, done). `chromeBadge` paints the pill. `renderTeam` paints `#team`.

### Needs you

Primary action: **Approve** on the first row (review) or the gate. **Request changes**, **Approve with note** and **Decline** are ghosts. While a note is open, its button (**Send back**, or **Approve** for an approval note) is the only filled button. Send back stays disabled until the note has text, and an approval note may stay empty. After approval, **Close task** replaces Approve as the only filled button. This button behavior is already on main. Do not rebuild it.

Selectors: `#needs-you`, `#needs-you-quiet`, `#needs-you-head`, `#needs-you-state`, `#needs-you-title`, `#needs-you-count`, `#needs-you-list`, `.ny-group`, `.ny-item`, `.ny-kind`, `.ny-title`, `.ny-question`, `.ny-evidence`, `.ny-ev-root`, `.ny-ev-item`, `.ny-ev-tier`, `.ny-ev-warn`, `.ny-ev-decider`, `.ny-actions`, `.ny-note`, `.ny-err`. Built by `renderNeedsYou` / `needsYouRow`.

When more than one kind waits, the list splits under `.ny-group` headings, "Needs your decision" (reviews and gates), "Needs your action" (blocked tasks) and "An agent asked" (an agent's `needs_input` question, whose primary is **Answer**, opening the thread at the composer). The count may stay a number beside the title. It is not a badge on each row. The warm head shows only when the list is non-empty. A load that fails is never an empty or hidden queue: `#needs-you-state` under the head says what to fix (a refusal, in `--err`) or "Stale since HH:MM: could not reach the server. Reconnecting…" (muted), the last rows and the tab count stay, and the next good load hides the line.

A review row shows the evidence it approves in `.ny-evidence`, small and muted under the result: the packet's root (`.ny-ev-root`, the root Approve sends), the result's hash and who produced it, then one `.ny-ev-item` per linked artifact with its kind, filename, size, and who uploaded it and when (who linked it waits for a session read of a thread's links). An empty hand-off is one line. An artifact whose details fail says so on its own line in `--err`, and the other lines stay. A failed packet read is one `--err` line with a ghost Retry. Who decided is `.ny-ev-decider`, one line per reviewer. Each item ends in its attestation tier, `.ny-ev-tier` with `data-tier`, as the packet recorded it at the hand-off: "verified" (a land-gate pass the close gate accepts, drawn as its own "land-gate pass" line with its recorder), "attached" (linked by a member who never worked the task) or "self-reported" (a worker's own result or link). When the packet's server-computed `self_reported_only` is true, `.ny-ev-warn` under the root says, in ochre, that all of the evidence comes from the task's own workers. The page never decides to warn by itself. A packet from before tiers draws neither.

### Thread

Not on the first screen. A `.card` click (`selectThread`) shows `#collab-panel`.

Primary action: **Approve** or **Close task** in `#thread-actions` when `renderThreadActions` draws one. Otherwise **Post** (`#post-message`). Never both filled. While `#thread-actions` has a primary, `#post-message` is a ghost.

Selectors: `#collab-panel`, `#thread-header`, `#thread-badge`, `#thread-context`, `#thread-facts`, `#thread-actions`, `#message-list`, `#compose-body`, `#post-message`.

`#thread-badge` is the same state word, not a pill. Messages stay as they are: name, body, attachment name, image. Edit message, edit history, and upload stay inside closed `<details>`.

### Connect an agent

Opened from `#connect-open` or the empty board. Not a page.

Primary action: **Create member and mint token** (`#cx-create-agent`). Copy, Add to Cursor, and Tokens are ghosts or links.

Selectors: `#connect-dialog`, `#connect-title`, `#cx-claude`, `#cx-json`, `#cx-prompt`, `#cx-create`, `#cx-name`, `#cx-handle`, `#cx-create-agent`, `#cx-secret`, `#cx-cursor`, `#cx-status`.

Creating the member and minting `maidan.agent.worker` is already on main. Do not rebuild it. The secret is shown once, in `#cx-secret`, as text the person can copy. It is not written into the snippets.

### Empty and error

**No credential.** `#first-run` is the screen. Primary action: **Sign in with your identity provider** (`#login`) when `#first-run-oidc` is shown, otherwise the token field is the action: Enter in it, or Sign in (`#token-signin`, a ghost) beside it, signs in, and there is no filled button beside it. Before any credential exists, the channel list says "Paste your token to connect." and nothing asks the server for channels. `#board`'s static "Connect an agent" is not a second primary while `#first-run` is visible. Hide it or make it a ghost.

**Empty channel.** `emptyChannelHelp()` replaces the lanes with `#board-onboard`. Primary action: **Connect an agent**. One sentence on how a task arrives. No `POST /channels/…` path. No "pick a channel." No "New thread in the sidebar." `#create-thread` stays a ghost in `#new-task`.

**Could not load.** `boardState("error")` paints `.onboard.board-error` inside `#board`. Primary action: **Try again**. The message is one sentence (`humanError` / `unreachable`). No red filled panel: drop the `#fef2f2` background and the `#fecaca` border. A refresh that fails after the board is already showing stays a sentence in `#board-summary .board-note`.

**Row and toast.** `.ny-err` and `#toasts .toast` are the same sentence. `showError` already avoids `alert()`. Keep that.

## Change list

Already on main. Do not re-specify or re-implement: live board and names (#1082), Needs you with Approve / Request changes / Close task (#1084), team strip and card glide (#1085), command palette and Connect sheet (#1086), toasts instead of `alert()` (#1117), one board render (#1118), first-run card (#1123), token rotate (#1127), attachment names and images (#1135), Connect creates the member and the worker token (#1144), the demo board script (#1146), a refused close recorded and drawn (#1147), the only channel opens itself (#1148), Live as a status line, one filled button on a decision row, empty Needs you as a line (#1151), failures said in words and a DM that selects the person (#1157).

Not on main:

1. **`#tools` starts closed.** Remove the `open` attribute from `<details id="tools">`. `openTool` still opens a tab from the palette.
2. **No thread until a card is open.** `#collab-panel` is `hidden` while `selectedThreadId` is null. No "none selected", no composer, no Post on the first screen.
3. **Lanes are space.** `.board-col` background transparent, no padding box, no shadow. Remove `.board-col .dot` and the white `.count` pill. The lane heading is the name plus the number, in 12px muted type.
4. **State is a word on the card.** Stop calling `chromeBadge` inside `renderBoard`. Put `chrome.label` in `.card-foot`. Delete `<details class="legend-box">`. `#thread-badge` uses the same word. `.ny-kind` is text, not a tinted chip.
5. **`#team` is only people holding work.** In `renderTeam`, do not append a chip whose state is `idle`. The viewer may still show when `needsYou.length` is non-zero ("N waiting on you"). When someone holds two open tasks, the title is the `running` one, else the one with the latest `updated_at`. Not "the first non-closed task unless a later one is running."
6. **One channel, no sidebar.** In `loadChannels`, if `channels.length === 1`, hide `aside`. The name is already `#board-title`. At two or more, show `aside` on `--soft` (`#fbfaf7`), dimmer than `#board`. Do not add a second sidebar.
7. **No refusal strip.** Do not paint `#board-refusal`. Keep `.card-refusal` with the text "Close refused" on that `.card`. The server's sentence can be the card `title`. Do not drop the refusal itself.
8. **One sentence, one action, on an empty board.** Rewrite the static `#board-onboard` and `emptyChannelHelp()` so they do not say "pick a channel", "New thread in the sidebar", or a raw `POST` path. While `#first-run` is visible it is the only primary. `#create-thread` is `button.ghost`.
9. **A person never sees the raw error.** On `#toasts`, `.ny-err`, `.onboard.board-error`, `#message-list`, and `#board-summary .board-note`, show the `humanError` sentence only. Stop appending ` — ${detail}` and ` (HTTP ${status})` on those surfaces. `pre#out` and `#live-feed` may keep JSON. They are behind More tools and `#live-more`.
10. **Identity is type.** `#identity-pill` loses the pill border, the 999px radius, and the chip background. It is the name and the workspace name, muted, with `#conn-edit` as a ghost.

## Screenshot bar

A shot counts only if a person can see these. 1440px wide, light mode, real `/ui`.

**Board, work in flight.** One channel. At least one card. Needs you empty. The shot shows the lane words, card titles, a state word, and a holder name. It does not show a sidebar, More tools, a composer, a lane box, a badge, a legend, or a refusal strip. `#needs-you-quiet` reads "Nothing is waiting on you."

**Board, something waiting.** The same, plus one Needs you row and exactly one filled button, labeled Approve. Request changes is not filled.

**Refused close.** The card shows the line "Close refused". No banner above the lanes.

**Thread.** Opened from a card. One filled button. Messages are text. No JSON result blob (chips from `renderResult` are fine).

**Connect.** The dialog is open. One filled button: Create member and mint token.

**Empty channel.** One sentence and one filled Connect an agent. No four empty boxes.

**Error.** The board area is a sentence plus Try again. No `{`, no `HTTP`.

**Reduced motion.** The same board with `prefers-reduced-motion: reduce`. Cards are in their lanes. Nothing is mid-glide.

## Do / don't

Do:

- Edit `index.html` only for these ten items.
- Keep one `.primary` per screen, checked in the existing UI contract tests.
- Keep keyboard focus rings and the skip link.
- Keep names from `personEl`. A member id is a tooltip, not the label.
- Keep FLIP (`glideCards`) and the reduced-motion guard.

Don't:

- Add a second sidebar, a right-hand inspector, or a second board.
- Put a `.chrome-badge`, a count pill, or a status chip on every card.
- Paint a raw JSON error, a stack, or `HTTP 409` in the board, the row, the thread, or a toast.
- Open `#tools` or `#collab-panel` on first paint.
- Add a filled button next to Approve (Connect, Add task, Post, Decline).
- Restyle Connect's create-member flow, the single-channel auto-open, or the empty Needs you line. Those are done.
- Introduce a new font, a dark theme, or drag-and-drop between lanes.

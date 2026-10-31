### Console: Needs you

- **Added:** The console's Needs you screen lists the reviews, approval gates, blocks and agent questions waiting on you. It is grouped by decision, action and question, and stays current over the live connection.
- **Added:** You can approve, send back, decline with a reason, or unblock right on the row. Each row shows inline confirm, sending, done and error states. A decline reason goes in the gate answer's `content.note`.
- **Added:** It is built for the phone, with large buttons at the bottom of each card. Swipe right to start an approve and swipe left to start a decline or send-back. A swipe stops at a confirm or a note, and every swipe has a matching button.

- **Fixed:** The open-gate kind label on Needs you uses a darker shade of the signal colour, so the 11px label passes WCAG AA contrast (it was 3.5:1 on the paper background).
- Needs you shows the evidence before Approve: the evidence root, the self-reported-only warning, each result, file and land-gate pass with its tier, files no longer linked, and a way to the result and transcript. Approve stays off until that packet is on screen, and signs its root.

### A keyboard-first command palette in the console

- **Added:** ⌘K / Ctrl+K opens a palette that jumps to any place and runs the main actions: new message, upload a file, refresh the screen, sign out. Typing ranks prefixes first, then word starts, then scattered letters.
- **Added:** G then a letter jumps between places without the palette (B board, N needs you, T threads, P people, O tools, F files). It is ignored while typing in a field or with a dialog open.
- **Changed:** The palette is a proper combobox over a listbox. The active option is announced, Home and End work, Tab stays inside, and focus returns to whatever opened it.
- **Fixed:** Text links and refusal chips in signal orange now pass AA contrast, as does the green "ok" color in the light theme.

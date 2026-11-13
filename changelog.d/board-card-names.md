### A board card's name starts with the words it shows

- **Fixed:** A task card on the board no longer replaces its visible text with an `aria-label`. Its accessible name is the title and details it shows, followed by hidden words for its state: stalled, waiting on a person, close refused, and the votes. A screen reader and a voice-control user now name the card with the same words (WCAG 2.5.3) (#2273).
- **Changed:** The console and landing move to React 19.3, Vite 7.3.8 and motion 12.43, and the UI tests to Playwright 1.65 and axe 4.14. `eslint-plugin-react-hooks` stays at 7.0.1 until the console's React Compiler findings from 7.1 are fixed (#2273).

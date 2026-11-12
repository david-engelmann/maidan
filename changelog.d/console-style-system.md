### The console's stylesheet is modules on one token system

- **Changed:** the console's 1,524-line `styles.css` is now 20 modules under `web/console/src/styles/`, imported in cascade order; the split bundles to a byte-identical stylesheet.
- **Changed:** every padding, margin and gap in the console takes a `--space-*` token (a 4px scale with half steps), every font size a `--text-*` token, and media queries use four named breakpoints (narrow 380px, phone 600px, wide 900px, xwide 1440px) in place of nine widths. Values are unchanged; only layouts between 520px and 1100px wide move to the nearest named breakpoint.
- **Added:** `npm run lint` in `web/console` runs stylelint, which refuses a raw px spacing or font size and an unnamed breakpoint, and eslint now checks the markup with `eslint-plugin-jsx-a11y`.

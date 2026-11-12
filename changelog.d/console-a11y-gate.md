### An accessibility gate over every console place

- **Added:** `ui-tests/tests/console-axe.spec.ts` runs axe (WCAG 2.2 AA tags) over all eight console places, in the dark and light themes, at 1440 and 390 px, once each place's data has loaded. A serious or critical violation fails `ui tests (playwright)` and names its nodes. Before, axe ran in two specs and on none of the places.
- **Changed:** the review gallery also shoots 1024 x 768 and 768 x 1024, adds People, Activity, Catalog and Files, and fails on a console error as it already did on a page error.
- **Fixed:** Files scrolled sideways at tablet widths: the "Choose a task" select took its longest option's width and pushed past its card. Growing row-form fields now shrink with the card.

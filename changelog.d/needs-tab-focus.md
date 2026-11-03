### The Needs you keyboard check tabs to its row

- **Fixed:** the "Tab reaches a Needs you row's actions" UI test focused the button from script, which in Chromium does not match `:focus-visible`, so the focus ring never drew and the test failed on every run. It now presses Tab until the row's first action has focus, the way a keyboard user gets there, and then checks the ring.

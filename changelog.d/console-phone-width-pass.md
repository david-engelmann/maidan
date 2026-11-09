### The console's phone-width pass on maidan.world

- **Fixed:** on a phone, the board's task search, its Agent and State filters, the new-task field and the channel picker were 13 to 15px, so iOS zoomed the page into each one when it took focus. Below 600px they are 16px, like every other console field.
- **Fixed:** at 360px the console header's title, live chip and Search did not fit one row, so the sticky header wrapped to two and took 102px of the screen. The title now gives way (and ellipsizes) first, and the header keeps one 67px row.
- **Fixed:** the sign-up check on the sign-in card rendered Cloudflare Turnstile's 300px widget in a 275px column at 360px (297px at 390px), running past the form. A column narrower than 300px now gets the compact widget.

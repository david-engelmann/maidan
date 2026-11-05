### The Preferences card names what a signed-in session can do

- **Fixed:** a person signed in to the console saw posting and moving tasks under Can't, because the card listed only the read routes. It now lists what that browser session can actually do in the console. A token still shows only its own grant, a viewer stays on the read routes, and a session still does not claim `federation:admin` or `operator:global`.

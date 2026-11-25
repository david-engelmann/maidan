### Create your workspace works in the browser

- **Fixed:** the console's and the front page's policy allowed forms to go only to this server, and the sign-up form's answer is a redirect to the sign-in provider. The browser stopped that redirect, so the first click on **Create your workspace** did nothing and the second was refused with "the human check was not accepted", because the first had already spent the check. Both pages now name the provider's origin in `form-action`, as the consent page already does for the app it returns to.

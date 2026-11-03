### claude.ai's connector can authorize without naming a scope

- **Fixed:** Connecting claude.ai to maidan.world as a custom connector failed with "scope is empty". Its client metadata document and its authorize request name no scope, so the client could ask for nothing. Now a metadata-document client that names no scope may ask for any delegatable capability, and a request that names none asks for what the client may have, cut to what the member holds. The person still approves the grant on the consent page.

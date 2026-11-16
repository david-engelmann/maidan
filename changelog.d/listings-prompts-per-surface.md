### Each listing's example prompts fit the endpoint it lists

- **Fixed:** the ChatGPT listing's example prompts expected `get_review_packet` and `catch_up_events`, and the Cursor listing's expected `get_waiting_inbox` and `search_messages`, none of which the endpoint each listing connects to serves (`/mcp/coordinator` and `/mcp/worker`). Each prompt now asks for something its listing's tools can do.
- **Added:** `listings/prompts.json` names each listing's endpoint, and `scripts/listing-prompts.py --check` fails when a listing's prompt expects a tool that endpoint does not serve, or when a package's manifest connects somewhere else. Negative prompts, which pass when the model calls no Maidan tool, give the ChatGPT submission its three negative test cases.

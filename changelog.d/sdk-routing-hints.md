### SDK routing hints for self-hosted routers

- **Added:** every SDK gets `routing_hints` (`routingHints`, `RoutingHints`), which turns a thread and its parent into affinity headers for Dynamo (`X-Dynamo-Session-ID`, `X-Dynamo-Parent-Session-ID`), llm-d (`x-session-id`), AIBrix (`x-aibrix-session-key`) and the SGLang router (`X-SMG-Routing-Key`). A child thread is keyed by its parent so it lands where the parent's prefix is warm. Every SDK also gets `cache_salt`, a vLLM salt per workspace that is never shared across two. Both are hints, and the shared fixtures keep the four SDKs in step.

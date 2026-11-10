### Every Threat-Model row names the test that holds it

- **Added:** a Test column on the Primary threats table in `docs/Threat-Model.md`, each cell naming an existing Rust test function or a `*.rs` file stem. `threat_model_contract` fails if a name is missing or does not resolve in the repo. T6 (`sqlx` parameterized queries) has no dedicated test and is pinned as `practice`. The pull-request template asks for a threat-model delta when a change touches a trust boundary.

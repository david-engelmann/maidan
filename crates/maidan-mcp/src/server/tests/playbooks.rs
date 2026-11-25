//! Role playbooks over the wire: listed as prompts and resources, served by
//! `prompts/get` and `resources/read`, and named in the instructions.

use super::*;
use crate::profiles::Profile;

async fn caller() -> (McpServer, AuthContext) {
    let (server, _thread, member) = mk_server().await;
    let workspace = server.store.get_member(member).await.unwrap().workspace_id;
    (server, admin_auth(member, workspace))
}

async fn ok(server: &McpServer, auth: &AuthContext, method: &str, params: Value) -> Value {
    let response = server.handle(request(1, method, params), auth).await;
    assert!(response.error.is_none(), "{method}: {:?}", response.error);
    response.result.expect("a result")
}

#[tokio::test]
async fn prompts_list_includes_every_playbook_beside_thread_workflow() {
    let (server, auth) = caller().await;
    let listed = ok(&server, &auth, "prompts/list", json!({})).await;
    let prompts = listed["prompts"].as_array().unwrap();
    assert!(prompts.iter().any(|p| p["name"] == "thread_workflow"));
    for playbook in crate::playbooks::all() {
        let prompt = prompts
            .iter()
            .find(|p| p["name"] == playbook.prompt_name().as_str())
            .unwrap_or_else(|| panic!("{} is not listed", playbook.prompt_name()));
        assert_eq!(prompt["description"], playbook.description);
        assert_eq!(prompt["title"], playbook.title().as_str());
    }
    assert!(!crate::playbooks::all().is_empty());
}

#[tokio::test]
async fn prompts_get_returns_the_playbook_body() {
    let (server, auth) = caller().await;
    for playbook in crate::playbooks::all() {
        let got = ok(
            &server,
            &auth,
            "prompts/get",
            json!({ "name": playbook.prompt_name() }),
        )
        .await;
        assert_eq!(got["description"], playbook.description);
        assert_eq!(got["messages"][0]["role"], "user");
        assert_eq!(got["messages"][0]["content"]["type"], "text");
        assert_eq!(got["messages"][0]["content"]["text"], playbook.body);
    }
}

#[tokio::test]
async fn an_unknown_prompt_is_still_method_not_found() {
    let (server, auth) = caller().await;
    for name in ["playbook_nope", "no_such_prompt", "claimant"] {
        let response = server
            .handle(request(1, "prompts/get", json!({ "name": name })), &auth)
            .await;
        let error = response.error.expect("an error");
        assert_eq!(error.code, -32601, "{name}");
        assert!(error.message.contains(&format!("prompt:{name}")), "{name}");
    }
}

#[tokio::test]
async fn resources_list_names_the_workspace_first_then_every_playbook() {
    let (server, auth) = caller().await;
    let listed = ok(&server, &auth, "resources/list", json!({})).await;
    let resources = listed["resources"].as_array().unwrap();
    assert!(
        resources[0]["uri"]
            .as_str()
            .unwrap()
            .starts_with("maidan://workspaces/"),
        "a client reading the first entry must get the workspace"
    );
    for playbook in crate::playbooks::all() {
        let resource = resources
            .iter()
            .find(|r| r["uri"] == playbook.uri().as_str())
            .unwrap_or_else(|| panic!("{} is not listed", playbook.uri()));
        assert_eq!(resource["name"], playbook.prompt_name().as_str());
        assert_eq!(resource["mimeType"], "text/markdown");
        assert_eq!(resource["description"], playbook.description);
    }
    for resource in resources {
        let uri = resource["uri"].as_str().unwrap();
        assert!(
            !uri.contains('{'),
            "resources/list listed a template: {uri}"
        );
    }
}

#[tokio::test]
async fn resources_read_returns_the_playbook_body_as_markdown() {
    let (server, auth) = caller().await;
    for playbook in crate::playbooks::all() {
        let read = ok(
            &server,
            &auth,
            "resources/read",
            json!({ "uri": playbook.uri() }),
        )
        .await;
        let content = &read["contents"][0];
        assert_eq!(content["uri"], playbook.uri().as_str());
        assert_eq!(content["mimeType"], "text/markdown");
        assert_eq!(content["text"], playbook.body);
        assert_eq!(read["cacheScope"], "private");
        assert_eq!(read["ttlMs"], crate::caching::RELEASE_TTL_MS);
    }
}

#[tokio::test]
async fn an_unknown_playbook_resource_is_not_found() {
    let (server, auth) = caller().await;
    for uri in [
        "maidan://playbooks/nope",
        "maidan://playbooks/claimant/extra",
    ] {
        let response = server
            .handle(request(1, "resources/read", json!({ "uri": uri })), &auth)
            .await;
        assert_eq!(response.error.expect("an error").code, -32004, "{uri}");
    }
}

#[tokio::test]
async fn initialize_and_discover_instructions_name_the_playbooks() {
    let (server, auth) = caller().await;
    let init = ok(
        &server,
        &auth,
        "initialize",
        json!({ "protocolVersion": "2026-07-28" }),
    )
    .await;
    let instructions = init["instructions"].as_str().unwrap();
    assert!(instructions.contains("playbook_"), "{instructions}");
    assert!(
        instructions.contains("maidan://playbooks/"),
        "{instructions}"
    );
    let discover = ok(&server, &auth, "server/discover", json!({})).await;
    assert_eq!(discover["instructions"], instructions);
}

#[tokio::test]
async fn the_profile_endpoints_serve_the_playbooks_too() {
    let (server, auth) = caller().await;
    let playbook = crate::playbooks::find("claimant").expect("the claimant playbook");
    for profile in [Profile::Worker, Profile::Reviewer, Profile::Coordinator] {
        let name = profile.name();
        let prompts = server
            .handle_profile(request(1, "prompts/list", json!({})), &auth, profile)
            .await
            .result
            .expect("a result");
        assert!(
            prompts["prompts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p["name"] == playbook.prompt_name().as_str()),
            "{name}: prompts/list"
        );
        let read = server
            .handle_profile(
                request(1, "resources/read", json!({ "uri": playbook.uri() })),
                &auth,
                profile,
            )
            .await
            .result
            .expect("a result");
        assert_eq!(read["contents"][0]["text"], playbook.body, "{name}");
    }
}

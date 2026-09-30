//! SCIM 2.0 provisioning over HTTP: the full lifecycle — create, read,
//! list-with-filter, duplicate 409, PATCH deactivate (revokes the member's
//! tokens), delete — plus the token:admin gate, user renames, and Groups.
//! Auth-enabled with a minted token:admin bearer per workspace; the second
//! workspace is the other tenant every group and rename test probes from.
//!
//! The PATCH bodies are the shapes Okta and Entra ID send, from their SCIM
//! provisioning documentation: Okta renames a user with PUT, deactivates and
//! renames a group with a pathless `replace`, and removes a member with a
//! `members[value eq "…"]` path; Entra ID capitalizes `op`, renames a user with
//! `"path": "userName"`, sends `active` as the string `"False"`, removes a
//! member with `"path": "members"` and a value array, and lists groups with
//! `excludedAttributes=members`.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberId, MemberKind, NewApiToken, NewMember, NewWorkspace};
use reqwest::StatusCode;
use sqlx::sqlite::SqlitePoolOptions;

struct Harness {
    base: String,
    client: reqwest::Client,
    admin_auth: String,
    store: Arc<dyn Store>,
    ws: maidan_types::WorkspaceId,
    /// The other tenant's token:admin bearer.
    other_auth: String,
    _server: tokio::task::JoinHandle<()>,
}

async fn setup() -> Harness {
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let search: Arc<dyn maidan_search::Search> =
        Arc::new(maidan_search::SqliteSearch::new(pool.clone()));
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let state = AppState::new(
        store.clone(),
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();

    let (ws, admin_auth) = admin_workspace(&store, "w").await;
    let (_, other_auth) = admin_workspace(&store, "other").await;
    Harness {
        base: format!("http://{addr}"),
        client,
        admin_auth,
        store,
        ws,
        other_auth,
        _server: server,
    }
}

/// A workspace and a token:admin bearer for its IdP.
async fn admin_workspace(
    store: &Arc<dyn Store>,
    name: &str,
) -> (maidan_types::WorkspaceId, String) {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let admin = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "idp".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: admin.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: Some("idp".into()),
            capabilities: vec!["token:admin".into()],
            expires_at: None,
        })
        .await
        .unwrap();
    (ws.id, format!("Bearer {}", secret.as_str()))
}

impl Harness {
    /// A SCIM request with `auth`, answering status and body.
    async fn scim(
        &self,
        method: reqwest::Method,
        path: &str,
        auth: &str,
        body: Option<serde_json::Value>,
    ) -> (StatusCode, serde_json::Value) {
        let mut req = self
            .client
            .request(method, format!("{}{path}", self.base))
            .header("Authorization", auth);
        if let Some(body) = body {
            req = req
                .header("Content-Type", "application/scim+json")
                .body(body.to_string());
        }
        let resp = req.send().await.unwrap();
        let status = resp.status();
        let text = resp.text().await.unwrap();
        let value = if text.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_str(&text).unwrap()
        };
        (status, value)
    }

    async fn create_user(&self, auth: &str, user_name: &str) -> String {
        let (status, body) = self
            .scim(
                reqwest::Method::POST,
                "/scim/v2/Users",
                auth,
                Some(serde_json::json!({
                    "schemas": ["urn:ietf:params:scim:schemas:core:2.0:User"],
                    "userName": user_name,
                    "active": true
                })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        body["id"].as_str().unwrap().to_string()
    }
}

#[tokio::test]
async fn scim_user_lifecycle_over_http() {
    let h = setup().await;
    let users = format!("{}/scim/v2/Users", h.base);

    // ServiceProviderConfig advertises patch + filter support.
    let spc: serde_json::Value = h
        .client
        .get(format!("{}/scim/v2/ServiceProviderConfig", h.base))
        .header("Authorization", &h.admin_auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(spc["patch"]["supported"], true);
    assert_eq!(spc["filter"]["supported"], true);

    // Create.
    let created = h
        .client
        .post(&users)
        .header("Authorization", &h.admin_auth)
        .json(&serde_json::json!({
            "schemas": ["urn:ietf:params:scim:schemas:core:2.0:User"],
            "userName": "alice",
            "displayName": "Alice A",
            "externalId": "okta-1",
            "active": true,
            "emails": [{ "value": "alice@example.com", "primary": true }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let body: serde_json::Value = created.json().await.unwrap();
    assert_eq!(body["userName"], "alice");
    assert_eq!(body["externalId"], "okta-1");
    assert_eq!(body["active"], true);
    let user_id = body["id"].as_str().unwrap().to_string();
    let member_id = MemberId(user_id.parse().unwrap());

    // Duplicate userName → 409.
    assert_eq!(
        h.client
            .post(&users)
            .header("Authorization", &h.admin_auth)
            .json(&serde_json::json!({ "userName": "alice" }))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );

    // Get by id.
    assert_eq!(
        h.client
            .get(format!("{users}/{user_id}"))
            .header("Authorization", &h.admin_auth)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );

    // List with userName filter → exactly one.
    let list: serde_json::Value = h
        .client
        .get(format!("{users}?filter=userName%20eq%20%22alice%22"))
        .header("Authorization", &h.admin_auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list["totalResults"], 1);
    assert_eq!(list["Resources"][0]["userName"], "alice");

    // Give the provisioned member a token, then PATCH active=false → it's revoked.
    let member_secret = TokenSecret::generate();
    let token = h
        .store
        .create_api_token(NewApiToken {
            workspace_id: h.ws,
            member_id,
            app_installation_id: None,
            token_hash: hash_secret(member_secret.as_str()),
            label: Some("alice".into()),
            capabilities: vec!["workspace:read".into()],
            expires_at: None,
        })
        .await
        .unwrap();
    let patched: serde_json::Value = h
        .client
        .patch(format!("{users}/{user_id}"))
        .header("Authorization", &h.admin_auth)
        .json(&serde_json::json!({
            "schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"],
            "Operations": [{ "op": "replace", "path": "active", "value": false }]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(patched["active"], false);
    let toks = h
        .store
        .list_api_tokens_for_member(h.ws, member_id)
        .await
        .unwrap();
    assert!(
        toks.iter()
            .find(|t| t.id == token.id)
            .unwrap()
            .revoked_at
            .is_some(),
        "deactivation revokes the member's token"
    );

    // Delete → 204, then Get → 404.
    assert_eq!(
        h.client
            .delete(format!("{users}/{user_id}"))
            .header("Authorization", &h.admin_auth)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        h.client
            .get(format!("{users}/{user_id}"))
            .header("Authorization", &h.admin_auth)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn scim_requires_token_admin() {
    let h = setup().await;
    // A workspace:read-only bearer is forbidden from SCIM.
    let member = h
        .store
        .create_member(NewMember {
            workspace_id: h.ws,
            handle: "weak".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let secret = TokenSecret::generate();
    h.store
        .create_api_token(NewApiToken {
            workspace_id: h.ws,
            member_id: member.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec!["workspace:read".into()],
            expires_at: None,
        })
        .await
        .unwrap();
    let resp = h
        .client
        .get(format!("{}/scim/v2/Users", h.base))
        .header("Authorization", format!("Bearer {}", secret.as_str()))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

const PATCH_OP: &str = "urn:ietf:params:scim:api:messages:2.0:PatchOp";

fn patch_body(ops: serde_json::Value) -> Option<serde_json::Value> {
    Some(serde_json::json!({ "schemas": [PATCH_OP], "Operations": ops }))
}

#[tokio::test]
async fn scim_rename_keeps_the_member_and_refuses_a_taken_user_name() {
    use reqwest::Method;
    let h = setup().await;
    let alice = h.create_user(&h.admin_auth, "alice").await;
    h.create_user(&h.admin_auth, "bob").await;
    let alice_id = MemberId(alice.parse().unwrap());
    let user = format!("/scim/v2/Users/{alice}");

    // What is attributed to alice before the rename: a token and a message.
    let alice_secret = TokenSecret::generate();
    h.store
        .create_api_token(NewApiToken {
            workspace_id: h.ws,
            member_id: alice_id,
            app_installation_id: None,
            token_hash: hash_secret(alice_secret.as_str()),
            label: None,
            capabilities: vec!["workspace:read".into()],
            expires_at: None,
        })
        .await
        .unwrap();
    let channel = h
        .store
        .create_channel(maidan_types::NewChannel {
            workspace_id: h.ws,
            name: "general".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = h
        .store
        .create_thread(maidan_types::NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: None,
        })
        .await
        .unwrap();
    let message = h
        .store
        .post_message(maidan_types::NewMessage {
            thread_id: thread.id,
            author_id: alice_id,
            body: "before the rename".into(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .unwrap();

    // Okta renames with PUT of the whole user.
    let (status, body) = h
        .scim(
            Method::PUT,
            &user,
            &h.admin_auth,
            Some(serde_json::json!({
                "schemas": ["urn:ietf:params:scim:schemas:core:2.0:User"],
                "id": alice,
                "userName": "alice.okta",
                "name": { "givenName": "Alice", "familyName": "A" },
                "active": true
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["userName"], "alice.okta");
    assert_eq!(body["id"], alice.as_str());

    // Entra ID renames with a path.
    let (status, body) = h
        .scim(
            Method::PATCH,
            &user,
            &h.admin_auth,
            patch_body(serde_json::json!([
                { "op": "Replace", "path": "userName", "value": "alice@contoso.com" }
            ])),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["userName"], "alice@contoso.com");

    // A pathless replace names the attribute in its value.
    let (status, body) = h
        .scim(
            Method::PATCH,
            &user,
            &h.admin_auth,
            patch_body(serde_json::json!([
                { "op": "replace", "value": { "userName": "alice" } }
            ])),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["userName"], "alice");

    // Taken: 409 with scimType uniqueness, by PATCH and by PUT; nothing changes.
    for (method, body) in [
        (
            Method::PATCH,
            patch_body(serde_json::json!([
                { "op": "Replace", "path": "userName", "value": "bob" }
            ])),
        ),
        (
            Method::PUT,
            Some(serde_json::json!({ "userName": "bob", "active": false })),
        ),
    ] {
        let (status, body) = h.scim(method, &user, &h.admin_auth, body).await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(body["scimType"], "uniqueness");
    }
    let (_, body) = h.scim(Method::GET, &user, &h.admin_auth, None).await;
    assert_eq!(body["userName"], "alice");
    assert_eq!(body["active"], true, "the refused PUT deactivated nothing");

    // The final name, then: the member is the same, and shows it.
    let (status, _) = h
        .scim(
            Method::PATCH,
            &user,
            &h.admin_auth,
            patch_body(serde_json::json!([
                { "op": "Replace", "path": "userName", "value": "alice.renamed" }
            ])),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let member: serde_json::Value = h
        .client
        .get(format!("{}/members/{alice}", h.base))
        .header("Authorization", format!("Bearer {}", alice_secret.as_str()))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        member["handle"], "alice.renamed",
        "her own token still works"
    );
    assert_eq!(
        h.store.get_message(message.id).await.unwrap().author_id,
        alice_id
    );
    let (_, list) = h
        .scim(
            Method::GET,
            "/scim/v2/Users?filter=userName%20eq%20%22alice.renamed%22",
            &h.admin_auth,
            None,
        )
        .await;
    assert_eq!(list["totalResults"], 1);
    assert_eq!(list["Resources"][0]["id"], alice.as_str());
    let (_, list) = h
        .scim(
            Method::GET,
            "/scim/v2/Users?filter=userName%20eq%20%22alice%22",
            &h.admin_auth,
            None,
        )
        .await;
    assert_eq!(list["totalResults"], 0);
    let renames = h
        .store
        .list_audit(100)
        .await
        .unwrap()
        .into_iter()
        .filter(|row| {
            row.target_id == Some(alice_id.0) && row.metadata["userName"]["to"].is_string()
        })
        .count();
    assert_eq!(
        renames, 4,
        "each rename is recorded, the refused ones are not"
    );

    // The other tenant can neither see nor rename her, and its own "alice"
    // does not collide.
    let (status, _) = h
        .scim(
            Method::PATCH,
            &user,
            &h.other_auth,
            patch_body(serde_json::json!([
                { "op": "Replace", "path": "userName", "value": "taken-over" }
            ])),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = h
        .scim(
            Method::PUT,
            &user,
            &h.other_auth,
            Some(serde_json::json!({ "userName": "taken-over" })),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        h.store.get_member(alice_id).await.unwrap().handle,
        "alice.renamed"
    );
    h.create_user(&h.other_auth, "alice.renamed").await;

    // Entra ID sends `active` as the string "False".
    let (status, body) = h
        .scim(
            Method::PATCH,
            &user,
            &h.admin_auth,
            patch_body(serde_json::json!([
                { "op": "Replace", "path": "active", "value": "False" }
            ])),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["active"], false);
}

#[tokio::test]
async fn scim_groups_over_http_in_okta_and_entra_shapes() {
    use reqwest::Method;
    let h = setup().await;
    let ann = h.create_user(&h.admin_auth, "ann").await;
    let ben = h.create_user(&h.admin_auth, "ben").await;
    let outsider = h.create_user(&h.other_auth, "olga").await;

    // Okta creates the group empty, then looks it up by name.
    let (status, created) = h
        .scim(
            Method::POST,
            "/scim/v2/Groups",
            &h.admin_auth,
            Some(serde_json::json!({
                "schemas": ["urn:ietf:params:scim:schemas:core:2.0:Group"],
                "displayName": "Engineering",
                "members": []
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["displayName"], "Engineering");
    assert_eq!(created["meta"]["resourceType"], "Group");
    let gid = created["id"].as_str().unwrap().to_string();
    let group = format!("/scim/v2/Groups/{gid}");
    let (_, list) = h
        .scim(
            Method::GET,
            "/scim/v2/Groups?filter=displayName+eq+%22Engineering%22&startIndex=1&count=100",
            &h.admin_auth,
            None,
        )
        .await;
    assert_eq!(list["totalResults"], 1);
    assert_eq!(list["Resources"][0]["id"], gid.as_str());

    // Okta adds a member with a path and a value array.
    let (status, body) = h
        .scim(
            Method::PATCH,
            &group,
            &h.admin_auth,
            patch_body(serde_json::json!([{
                "op": "add",
                "path": "members",
                "value": [{ "value": ann, "display": "ann" }]
            }])),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["members"][0]["value"], ann.as_str());
    assert_eq!(body["members"][0]["display"], "ann");

    // Entra ID adds with a capitalized op and `$ref: null`.
    let (status, body) = h
        .scim(
            Method::PATCH,
            &group,
            &h.admin_auth,
            patch_body(serde_json::json!([{
                "op": "Add",
                "path": "members",
                "value": [{ "$ref": null, "value": ben }]
            }])),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["members"].as_array().unwrap().len(), 2);

    // Entra ID's lookup: displayName is not case-exact; members left out.
    let (_, list) = h
        .scim(
            Method::GET,
            "/scim/v2/Groups?excludedAttributes=members&filter=displayName%20eq%20%22engineering%22",
            &h.admin_auth,
            None,
        )
        .await;
    assert_eq!(list["totalResults"], 1);
    assert!(list["Resources"][0].get("members").is_none());

    // Another tenant's user, or an id that is no user at all, is the same
    // invalidValue, and the group is unchanged.
    for stranger in [outsider.clone(), uuid::Uuid::now_v7().to_string()] {
        let (status, body) = h
            .scim(
                Method::PATCH,
                &group,
                &h.admin_auth,
                patch_body(serde_json::json!([{
                    "op": "add", "path": "members", "value": [{ "value": stranger }]
                }])),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["scimType"], "invalidValue");
        assert_eq!(
            body["detail"],
            format!("{stranger} is not a user of this workspace")
        );
    }
    let (_, body) = h.scim(Method::GET, &group, &h.admin_auth, None).await;
    assert_eq!(body["members"].as_array().unwrap().len(), 2);

    // Okta removes with a filter path; Entra ID with a members value array.
    let (status, body) = h
        .scim(
            Method::PATCH,
            &group,
            &h.admin_auth,
            patch_body(serde_json::json!([{
                "op": "remove", "path": format!("members[value eq \"{ann}\"]")
            }])),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["members"][0]["value"], ben.as_str());
    let (status, body) = h
        .scim(
            Method::PATCH,
            &group,
            &h.admin_auth,
            patch_body(serde_json::json!([{
                "op": "Remove", "path": "members", "value": [{ "$ref": null, "value": ben }]
            }])),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["members"].as_array().unwrap().is_empty());

    // Okta renames with a pathless replace that repeats the id; Entra ID
    // with a path. Then a replace of the member set.
    let (_, body) = h
        .scim(
            Method::PATCH,
            &group,
            &h.admin_auth,
            patch_body(serde_json::json!([{
                "op": "replace", "value": { "id": gid, "displayName": "Eng" }
            }])),
        )
        .await;
    assert_eq!(body["displayName"], "Eng");
    let (_, body) = h
        .scim(
            Method::PATCH,
            &group,
            &h.admin_auth,
            patch_body(serde_json::json!([
                { "op": "Replace", "path": "displayName", "value": "Platform" },
                { "op": "Replace", "path": "externalId", "value": "entra-object-1" },
                { "op": "replace", "path": "members", "value": [{ "value": ann }, { "value": ben }] }
            ])),
        )
        .await;
    assert_eq!(body["displayName"], "Platform");
    assert_eq!(body["externalId"], "entra-object-1");
    assert_eq!(body["members"].as_array().unwrap().len(), 2);

    // PUT replaces the whole group.
    let (status, body) = h
        .scim(
            Method::PUT,
            &group,
            &h.admin_auth,
            Some(serde_json::json!({
                "schemas": ["urn:ietf:params:scim:schemas:core:2.0:Group"],
                "displayName": "Platform",
                "members": [{ "value": ann }]
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["members"].as_array().unwrap().len(), 1);
    assert!(body.get("externalId").is_none(), "PUT without it clears it");

    // A rename shows in the group at once.
    let (status, _) = h
        .scim(
            Method::PATCH,
            &format!("/scim/v2/Users/{ann}"),
            &h.admin_auth,
            patch_body(serde_json::json!([
                { "op": "Replace", "path": "userName", "value": "ann.b" }
            ])),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = h.scim(Method::GET, &group, &h.admin_auth, None).await;
    assert_eq!(body["members"][0]["display"], "ann.b");

    // The other tenant: nothing to see, change, or reach into.
    let (status, _) = h.scim(Method::GET, &group, &h.other_auth, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, list) = h
        .scim(Method::GET, "/scim/v2/Groups", &h.other_auth, None)
        .await;
    assert_eq!(list["totalResults"], 0);
    let (status, _) = h
        .scim(
            Method::PATCH,
            &group,
            &h.other_auth,
            patch_body(serde_json::json!([{ "op": "remove", "path": "members" }])),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = h
        .scim(
            Method::PUT,
            &group,
            &h.other_auth,
            Some(serde_json::json!({ "displayName": "Mine" })),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = h.scim(Method::DELETE, &group, &h.other_auth, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, body) = h
        .scim(
            Method::POST,
            "/scim/v2/Groups",
            &h.other_auth,
            Some(serde_json::json!({ "displayName": "Poach", "members": [{ "value": ann }] })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (_, body) = h.scim(Method::GET, &group, &h.admin_auth, None).await;
    assert_eq!(body["displayName"], "Platform");
    assert_eq!(body["members"].as_array().unwrap().len(), 1);

    // An unsupported filter is an error, not the whole list.
    let (status, body) = h
        .scim(
            Method::GET,
            "/scim/v2/Groups?filter=members%20eq%20%22x%22",
            &h.admin_auth,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["scimType"], "invalidFilter");

    // Deprovisioning a user takes it out of the group; deleting the group
    // leaves the users.
    let (status, _) = h
        .scim(
            Method::DELETE,
            &format!("/scim/v2/Users/{ann}"),
            &h.admin_auth,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, body) = h.scim(Method::GET, &group, &h.admin_auth, None).await;
    assert!(body["members"].as_array().unwrap().is_empty());
    let (status, _) = h.scim(Method::DELETE, &group, &h.admin_auth, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = h.scim(Method::GET, &group, &h.admin_auth, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = h
        .scim(
            Method::GET,
            &format!("/scim/v2/Users/{ben}"),
            &h.admin_auth,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // Every accepted write is on the record, against the group.
    let group_id: uuid::Uuid = gid.parse().unwrap();
    let actions: Vec<String> = h
        .store
        .list_audit(200)
        .await
        .unwrap()
        .into_iter()
        .filter(|row| row.target_id == Some(group_id))
        .map(|row| row.action)
        .collect();
    let count = |action: &str| actions.iter().filter(|a| *a == action).count();
    assert_eq!(count("scim.group.create"), 1);
    assert_eq!(count("scim.group.update"), 7);
    assert_eq!(count("scim.group.delete"), 1);
}

#[tokio::test]
async fn scim_groups_require_token_admin() {
    let h = setup().await;
    let member = h
        .store
        .create_member(NewMember {
            workspace_id: h.ws,
            handle: "weak".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let secret = TokenSecret::generate();
    h.store
        .create_api_token(NewApiToken {
            workspace_id: h.ws,
            member_id: member.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec!["workspace:read".into(), "workspace:write".into()],
            expires_at: None,
        })
        .await
        .unwrap();
    let weak = format!("Bearer {}", secret.as_str());
    let (status, _) = h
        .scim(reqwest::Method::GET, "/scim/v2/Groups", &weak, None)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = h
        .scim(
            reqwest::Method::POST,
            "/scim/v2/Groups",
            &weak,
            Some(serde_json::json!({ "displayName": "x" })),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn scim_user_filters_match_only_their_user_and_refuse_the_rest() {
    use reqwest::Method;
    let h = setup().await;
    let create = |auth: String, user_name: &'static str, external_id: &'static str| {
        let h = &h;
        async move {
            let (status, body) = h
                .scim(
                    Method::POST,
                    "/scim/v2/Users",
                    &auth,
                    Some(serde_json::json!({
                        "schemas": ["urn:ietf:params:scim:schemas:core:2.0:User"],
                        "userName": user_name,
                        "externalId": external_id,
                        "active": true
                    })),
                )
                .await;
            assert_eq!(status, StatusCode::CREATED, "{body}");
            body["id"].as_str().unwrap().to_string()
        }
    };
    let alice = create(h.admin_auth.clone(), "alice", "okta-a").await;
    let bob = create(h.admin_auth.clone(), "bob", "okta-b").await;
    // The other tenant reuses bob's externalId and alice's userName.
    let other_bob = create(h.other_auth.clone(), "alice", "okta-b").await;

    let ids = |list: &serde_json::Value| -> Vec<String> {
        list["Resources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_str().unwrap().to_string())
            .collect()
    };
    let list = |auth: String, query: String| {
        let h = &h;
        async move {
            h.scim(Method::GET, &format!("/scim/v2/Users?{query}"), &auth, None)
                .await
        }
    };

    // externalId eq finds exactly its user, in this workspace only.
    let (status, body) = list(
        h.admin_auth.clone(),
        "filter=externalId%20eq%20%22okta-b%22".into(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ids(&body), vec![bob.clone()], "{body}");
    assert_eq!(body["totalResults"], 1);
    let (_, body) = list(
        h.other_auth.clone(),
        "filter=externalId%20eq%20%22okta-b%22".into(),
    )
    .await;
    assert_eq!(ids(&body), vec![other_bob.clone()]);
    let (_, body) = list(
        h.admin_auth.clone(),
        "filter=externalId%20eq%20%22okta-missing%22".into(),
    )
    .await;
    assert_eq!(body["totalResults"], 0);

    // userName eq is not case-sensitive (RFC 7643 §4.1.1).
    let (_, body) = list(
        h.admin_auth.clone(),
        "filter=userName%20eq%20%22ALICE%22".into(),
    )
    .await;
    assert_eq!(ids(&body), vec![alice.clone()], "{body}");

    // id eq, and another workspace's id finds nothing.
    let (_, body) = list(
        h.admin_auth.clone(),
        format!("filter=id%20eq%20%22{bob}%22"),
    )
    .await;
    assert_eq!(ids(&body), vec![bob.clone()]);
    let (_, body) = list(
        h.other_auth.clone(),
        format!("filter=id%20eq%20%22{bob}%22"),
    )
    .await;
    assert_eq!(body["totalResults"], 0);

    // Anything else is invalidFilter, never the whole list.
    for query in [
        "filter=displayName%20eq%20%22Alice%22",
        "filter=active%20eq%20true",
        "filter=userName%20sw%20%22a%22",
        "filter=userName%20eq%20%22alice%22%20or%20userName%20eq%20%22bob%22",
        "filter=emails%5Btype%20eq%20%22work%22%5D",
    ] {
        let (status, body) = list(h.admin_auth.clone(), query.into()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{query}: {body}");
        assert_eq!(body["scimType"], "invalidFilter", "{query}");
        assert!(body.get("Resources").is_none());
    }

    // No filter lists this workspace's users only.
    let (_, body) = list(h.admin_auth.clone(), String::new()).await;
    let mut all = ids(&body);
    all.sort();
    let mut want = vec![alice, bob];
    want.sort();
    assert_eq!(all, want);
}

//! Prepaid credits end to end, with a fake chain: an open sign-up workspace
//! past its allowance on a priced meter is charged, cut off with a 402 when
//! its balance cannot cover the charge, asks for a top-up, and once the
//! watcher reads a confirmed transfer of exactly that amount it is credited
//! once and its cutoff lifts. Another workspace can neither read its credits
//! nor ask to be paid for it. CI never reaches a real chain.

mod seeded_workspace;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use maidan_auth::capability;
use maidan_server::credits::{ChainError, ChainReader, TransferLog, WatcherConfig};
use maidan_server::AppState;
use maidan_store::Store;
use maidan_types::*;
use reqwest::{Method, StatusCode};
use seeded_workspace::{member, spawn_with, token, Harness};
use serde_json::{json, Value};

const TO: &str = "0x00000000000000000000000000000000000000aa";

/// A chain that answers what the test set, whatever range is asked for.
struct FakeChain {
    head: Mutex<u64>,
    logs: Mutex<Vec<TransferLog>>,
}

#[async_trait::async_trait]
impl ChainReader for FakeChain {
    async fn block_number(&self) -> Result<u64, ChainError> {
        Ok(*self.head.lock().unwrap())
    }

    async fn transfer_logs(
        &self,
        token: &str,
        to: &str,
        from: u64,
        until: u64,
    ) -> Result<Vec<TransferLog>, ChainError> {
        assert_eq!(token, CreditChain::Base.usdc());
        assert_eq!(to, TO);
        assert!(
            until - from < maidan_server::credits::MAX_RANGE,
            "{from}..={until}"
        );
        Ok(self.logs.lock().unwrap().clone())
    }
}

fn transfer(tx: &str, amount: i64) -> TransferLog {
    TransferLog {
        tx_hash: format!("0x{tx:0>64}"),
        log_index: 0,
        block_number: 90,
        from: "0x00000000000000000000000000000000000000bb".into(),
        to: TO.into(),
        amount,
        removed: false,
    }
}

fn settings() -> CreditsSettings {
    CreditsSettings {
        chain: CreditChain::Base,
        address: TO.into(),
    }
}

fn watcher() -> WatcherConfig {
    WatcherConfig {
        settings: settings(),
        rpc_url: "http://chain.invalid".into(),
        confirmations: 5,
        interval: Duration::from_secs(1),
    }
}

fn terms(prices: &str) -> CreditTerms {
    CreditTerms {
        settings: Some(settings()),
        prices: OveragePrices::parse(prices).unwrap(),
    }
}

async fn signed_up(store: &dyn Store, who: &str) -> (WorkspaceId, MemberId) {
    let outcome = store
        .complete_oidc_signup(
            NewOidcSignup {
                workspace_name: format!("{who}'s workspace"),
                handle: who.into(),
                issuer: "https://idp.example".into(),
                subject: format!("sub-{who}"),
                email: format!("{who}@example.com"),
                invite_entry_id: None,
                session_expires_at: Utc::now() + Duration::from_secs(3600),
                open: true,
            },
            Box::new(|core| NewAuditEvent {
                scope: AuditScope::Workspace(core.workspace.id),
                actor_id: Some(core.member.id),
                action: "workspace.signup".into(),
                target_kind: Some("workspace".into()),
                target_id: Some(core.workspace.id.0),
                metadata: json!({}),
            }),
            Box::new(|session| NewAuditEvent {
                scope: AuditScope::Workspace(session.workspace_id),
                actor_id: Some(session.member_id),
                action: "session.create".into(),
                target_kind: Some("member".into()),
                target_id: Some(session.member_id.0),
                metadata: json!({}),
            }),
        )
        .await
        .unwrap();
    (outcome.workspace.id, outcome.member.id)
}

fn admin_caps() -> Vec<String> {
    vec![
        capability::WORKSPACE_READ.to_string(),
        capability::TOKEN_ADMIN.to_string(),
    ]
}

async fn call(
    h: &Harness,
    method: Method,
    path: &str,
    bearer: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let res = h.send(method, path, bearer, body).await;
    let status = res.status();
    (status, res.json().await.unwrap_or(Value::Null))
}

async fn mcp(h: &Harness, bearer: &str, name: &str, arguments: Value) -> Value {
    h.send(
        Method::POST,
        "/mcp",
        bearer,
        Some(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments }
        })),
    )
    .await
    .json()
    .await
    .unwrap()
}

/// The tool result's JSON text.
fn tool_json(answer: &Value) -> Value {
    serde_json::from_str(answer["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[tokio::test]
async fn a_confirmed_deposit_credits_its_workspace_once_and_lifts_its_cutoff() {
    let mut state: Option<AppState> = None;
    let h = spawn_with(|s| {
        s.usage_allowance = Arc::new(UsageAllowance::parse("requests=3").unwrap());
        // A dollar a request past the allowance.
        s.set_credit_terms(terms("requests=1000000/1"));
        state = Some(s.clone());
    })
    .await;
    let state = state.unwrap();
    let store = h.store.as_ref();
    let (a, a_admin) = signed_up(store, "ada").await;
    let a_token = token(store, a, a_admin, admin_caps()).await;
    let (b, b_admin) = signed_up(store, "bea").await;
    let b_token = token(store, b, b_admin, admin_caps()).await;
    let a_credits = format!("/workspaces/{}/credits", a.0);
    let a_top_ups = format!("/workspaces/{}/credits/top-ups", a.0);
    let a_home = format!("/workspaces/{}", a.0);

    // Workspace B can neither read A's credits nor ask to be paid for A.
    for (method, path, body) in [
        (Method::GET, a_credits.clone(), None),
        (Method::POST, a_top_ups.clone(), Some(json!({ "usd": 5 }))),
    ] {
        let (code, body) = call(&h, method, &path, &b_token, body).await;
        assert_eq!(code, StatusCode::FORBIDDEN, "{body}");
    }

    // A goes past its allowance with no credit: charged, short, cut off.
    for _ in 0..5 {
        assert_eq!(
            call(&h, Method::GET, &a_home, &a_token, None).await.0,
            StatusCode::OK
        );
    }
    maidan_server::usage_meters::tick(&state).await;
    let (code, body) = call(&h, Method::GET, &a_home, &a_token, None).await;
    assert_eq!(code, StatusCode::PAYMENT_REQUIRED, "{body}");
    assert_eq!(
        body["type"],
        "https://maidan.world/problems/payment-required"
    );
    let owed = store.credit_balance(a).await.unwrap();
    assert!(owed < 0, "{owed}");
    let charge = store.list_credit_ledger(a, 10).await.unwrap();
    assert_eq!(charge[0].kind, CreditEntryKind::Charge);
    assert!(charge[0].unpaid_micros > 0);
    // B is untouched by A's usage and A's debt.
    assert_eq!(
        call(
            &h,
            Method::GET,
            &format!("/workspaces/{}", b.0),
            &b_token,
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(store.credit_balance(b).await.unwrap(), 0);

    // Cut off, A can still read its credits and ask to be paid; a card
    // top-up is the next rail and is refused here, saying so.
    let (code, credits) = call(&h, Method::GET, &a_credits, &a_token, None).await;
    assert_eq!(code, StatusCode::OK, "{credits}");
    assert_eq!(credits["enabled"], true);
    assert_eq!(credits["methods"], json!(["usdc"]));
    assert_eq!(credits["balance_micros"], owed);
    let (code, card) = call(
        &h,
        Method::POST,
        &a_top_ups,
        &a_token,
        Some(json!({ "usd": 5, "method": "card" })),
    )
    .await;
    assert_eq!(code, StatusCode::CONFLICT, "{card}");
    assert!(
        card["detail"]
            .as_str()
            .unwrap()
            .contains("card top-ups are not configured"),
        "{card}"
    );
    let (code, top_up) = call(
        &h,
        Method::POST,
        &a_top_ups,
        &a_token,
        Some(json!({ "usd": 5 })),
    )
    .await;
    assert_eq!(code, StatusCode::CREATED, "{top_up}");
    assert_eq!(top_up["method"], "usdc");
    assert_eq!(top_up["chain_id"], 8453);
    let amount = top_up["amount_micros"].as_i64().unwrap();
    assert!((5_000_001..=5_009_999).contains(&amount), "{amount}");
    assert!(top_up["eip681"]
        .as_str()
        .unwrap()
        .ends_with(&format!("&uint256={amount}")));
    for usd in [0, 10_001] {
        let (code, _) = call(
            &h,
            Method::POST,
            &a_top_ups,
            &a_token,
            Some(json!({ "usd": usd })),
        )
        .await;
        assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY, "{usd}");
    }

    // The chain: A's exact amount, and a stray transfer of a round five
    // dollars, both confirmed; a log too recent to count is not read yet.
    let chain = FakeChain {
        head: Mutex::new(100),
        logs: Mutex::new(vec![transfer("a1", amount), transfer("b2", 5_000_000)]),
    };
    let report = maidan_server::credits::tick(&state, &chain, &watcher())
        .await
        .unwrap();
    assert_eq!(
        (report.credited, report.unmatched, report.scanned_to),
        (1, 1, Some(95))
    );
    assert_eq!(store.credit_balance(a).await.unwrap(), owed + amount);
    assert_eq!(
        call(&h, Method::GET, &a_home, &a_token, None).await.0,
        StatusCode::OK,
        "the cutoff lifted"
    );
    let told = store.list_notifications(a_admin, false, 10).await.unwrap();
    assert!(
        told.iter().any(|n| n.kind == EventKind::CreditsDeposited),
        "{told:?}"
    );

    // The same logs read again, from later blocks: nothing credited twice.
    *chain.head.lock().unwrap() = 300;
    let again = maidan_server::credits::tick(&state, &chain, &watcher())
        .await
        .unwrap();
    assert_eq!(
        (again.credited, again.unmatched, again.scanned_to),
        (0, 0, Some(295))
    );
    assert_eq!(store.credit_balance(a).await.unwrap(), owed + amount);
    let (_, credits) = call(&h, Method::GET, &a_credits, &a_token, None).await;
    assert_eq!(credits["open_top_ups"], json!([]));

    // The stray is the operator's to settle, and only the operator's.
    let ops = store
        .create_workspace(NewWorkspace { name: "ops".into() })
        .await
        .unwrap()
        .id;
    let operator = member(store, ops, "operator", MemberKind::Human).await;
    let operator_token = token(
        store,
        ops,
        operator,
        vec![capability::OPERATOR_GLOBAL.to_string()],
    )
    .await;
    let (code, unmatched) = call(
        &h,
        Method::GET,
        "/operator/credits/unmatched",
        &operator_token,
        None,
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{unmatched}");
    assert_eq!(unmatched.as_array().unwrap().len(), 1);
    assert_eq!(unmatched[0]["amount_micros"], 5_000_000);
    assert_eq!(
        call(
            &h,
            Method::GET,
            "/operator/credits/unmatched",
            &a_token,
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );

    // The operator settles it with a grant to the workspace that sent it;
    // a workspace's own admin cannot grant itself credit.
    let grant = json!({ "usd_micros": 5_000_000, "note": "unmatched deposit b2" });
    let grants = format!("/operator/workspaces/{}/credits/grants", b.0);
    let (code, entry) = call(
        &h,
        Method::POST,
        &grants,
        &operator_token,
        Some(grant.clone()),
    )
    .await;
    assert_eq!(code, StatusCode::CREATED, "{entry}");
    assert_eq!(
        (entry["kind"].clone(), entry["source"].clone()),
        (json!("grant"), json!("grant"))
    );
    assert_eq!(store.credit_balance(b).await.unwrap(), 5_000_000);
    let (code, _) = call(&h, Method::POST, &grants, &b_token, Some(grant)).await;
    assert_eq!(code, StatusCode::FORBIDDEN);
    assert_eq!(store.credit_balance(b).await.unwrap(), 5_000_000);
}

#[tokio::test]
async fn with_no_address_nothing_is_charged_and_no_top_up_opens() {
    let mut state: Option<AppState> = None;
    let h = spawn_with(|s| {
        s.usage_allowance = Arc::new(UsageAllowance::parse("requests=2").unwrap());
        // Prices without an address: behaviour is exactly the plain cutoff.
        s.set_credit_terms(CreditTerms {
            settings: None,
            prices: OveragePrices::parse("requests=1/1").unwrap(),
        });
        state = Some(s.clone());
    })
    .await;
    let state = state.unwrap();
    let store = h.store.as_ref();
    let (a, admin) = signed_up(store, "cy").await;
    let a_token = token(store, a, admin, admin_caps()).await;
    for _ in 0..4 {
        call(
            &h,
            Method::GET,
            &format!("/workspaces/{}", a.0),
            &a_token,
            None,
        )
        .await;
    }
    maidan_server::usage_meters::tick(&state).await;
    assert_eq!(
        call(
            &h,
            Method::GET,
            &format!("/workspaces/{}", a.0),
            &a_token,
            None
        )
        .await
        .0,
        StatusCode::PAYMENT_REQUIRED
    );
    assert!(
        store.list_credit_ledger(a, 10).await.unwrap().is_empty(),
        "nothing charged"
    );
    let (code, credits) = call(
        &h,
        Method::GET,
        &format!("/workspaces/{}/credits", a.0),
        &a_token,
        None,
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(
        (credits["enabled"].clone(), credits["methods"].clone()),
        (json!(false), json!([]))
    );
    let (code, refused) = call(
        &h,
        Method::POST,
        &format!("/workspaces/{}/credits/top-ups", a.0),
        &a_token,
        Some(json!({ "usd": 5 })),
    )
    .await;
    assert_eq!(code, StatusCode::CONFLICT, "{refused}");
}

#[tokio::test]
async fn an_agent_cut_off_on_mcp_calls_reads_its_credits_and_asks_to_be_paid_over_mcp() {
    let mut state: Option<AppState> = None;
    let h = spawn_with(|s| {
        s.usage_allowance = Arc::new(UsageAllowance::parse("mcp_calls=1").unwrap());
        s.set_credit_terms(terms("mcp_calls=1000000/1"));
        state = Some(s.clone());
    })
    .await;
    let state = state.unwrap();
    let store = h.store.as_ref();
    let (a, _) = signed_up(store, "dee").await;
    let agent = member(store, a, "agent", MemberKind::Agent).await;
    let agent_token = token(
        store,
        a,
        agent,
        vec![capability::WORKSPACE_READ.to_string()],
    )
    .await;
    let (b, b_admin) = signed_up(store, "eve").await;
    let b_token = token(store, b, b_admin, admin_caps()).await;
    for _ in 0..3 {
        mcp(&h, &agent_token, "whoami", json!({})).await;
    }
    maidan_server::usage_meters::tick(&state).await;

    // Past it, a tool call is a tool error the model reads, naming the meter
    // and the way to top up.
    let refused = mcp(&h, &agent_token, "whoami", json!({})).await;
    assert_eq!(refused["result"]["isError"], true, "{refused}");
    let text = refused["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("free allowance for mcp_calls") && text.contains("top_up_credits"),
        "{text}"
    );

    // The credit tools stay open.
    let credits = mcp(
        &h,
        &agent_token,
        "get_credits",
        json!({ "workspace_id": a.0 }),
    )
    .await;
    assert_eq!(credits["result"]["isError"], false, "{credits}");
    assert!(tool_json(&credits)["balance_micros"].as_i64().unwrap() < 0);
    let top_up = mcp(
        &h,
        &agent_token,
        "top_up_credits",
        json!({ "workspace_id": a.0, "usd": 3 }),
    )
    .await;
    assert_eq!(top_up["result"]["isError"], false, "{top_up}");
    let top_up = tool_json(&top_up);
    assert_eq!(top_up["method"], "usdc");
    assert_eq!(top_up["address"], TO);
    let card = mcp(
        &h,
        &agent_token,
        "top_up_credits",
        json!({ "workspace_id": a.0, "usd": 3, "method": "card" }),
    )
    .await;
    assert!(
        card.to_string().contains("card top-ups are not configured"),
        "{card}"
    );

    // Another workspace's agent cannot ask to be paid for this one.
    let other = mcp(
        &h,
        &b_token,
        "top_up_credits",
        json!({ "workspace_id": a.0, "usd": 3 }),
    )
    .await;
    assert!(
        other.get("error").is_some() || other["result"]["isError"] == true,
        "{other}"
    );
    assert_eq!(
        store
            .list_open_credit_top_ups(a, Utc::now())
            .await
            .unwrap()
            .len(),
        1
    );
}

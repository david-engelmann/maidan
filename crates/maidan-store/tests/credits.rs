//! Prepaid credits on both backends: an intent's amount is unique among the
//! instance's open intents, a confirmed transfer credits the workspace whose
//! intent asked for exactly it, once, and anything else is kept as unmatched;
//! charges cover each unit past the allowance once, and a charge the balance
//! cannot cover leaves it below zero; a deposit that brings the balance back
//! above zero lifts the month's crossings on the meters it is told to.

use std::time::Duration;

use chrono::{DateTime, Utc};
use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::*;
use sqlx::postgres::PgPoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

const TO: &str = "0x00000000000000000000000000000000000000aa";

async fn workspace(store: &dyn Store, name: &str) -> WorkspaceId {
    store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap()
        .id
}

fn intent(ws: WorkspaceId, usd: i64, offsets: &[i64], now: DateTime<Utc>) -> NewTopUp {
    NewTopUp {
        workspace_id: ws,
        chain: CreditChain::Base,
        address: TO.into(),
        usd,
        offsets: offsets.to_vec(),
        now,
    }
}

fn transfer(tx: &str, log_index: i64, amount_micros: i64) -> DepositLog {
    DepositLog {
        chain: CreditChain::Base,
        tx_hash: tx.into(),
        log_index,
        block_number: 100,
        from_address: "0x00000000000000000000000000000000000000bb".into(),
        to_address: TO.into(),
        amount_micros,
    }
}

fn credited(outcome: DepositOutcome) -> CreditedDeposit {
    match outcome {
        DepositOutcome::Credited(credited) => *credited,
        other => panic!("expected a credit, got {other:?}"),
    }
}

async fn run_suite(store: &dyn Store) {
    let now = Utc::now();
    let a = workspace(store, "credits-a").await;
    let b = workspace(store, "credits-b").await;

    // An intent asks for the dollars plus its offset; a second intent for the
    // same dollars, in another workspace, cannot take an offset already held.
    let first = store
        .create_credit_top_up(&intent(a, 5, &[7], now))
        .await
        .unwrap();
    assert_eq!(first.amount_micros, 5_000_007);
    assert_eq!(first.chain_id, Some(8453));
    assert_eq!(first.status, TopUpStatus::Open);
    assert_eq!(first.method, TopUpMethod::Usdc);
    assert!(
        first
            .eip681
            .as_deref()
            .unwrap()
            .ends_with("&uint256=5000007"),
        "{:?}",
        first.eip681
    );
    let second = store
        .create_credit_top_up(&intent(b, 5, &[7, 8], now))
        .await
        .unwrap();
    assert_eq!(second.amount_micros, 5_000_008);
    assert!(matches!(
        store
            .create_credit_top_up(&intent(b, 5, &[7, 8], now))
            .await,
        Err(StoreError::Conflict(_))
    ));
    // Other dollars, same offset: no clash.
    let other_dollars = store
        .create_credit_top_up(&intent(b, 6, &[7], now))
        .await
        .unwrap();
    assert_eq!(other_dollars.amount_micros, 6_000_007);

    // Twenty open intents per workspace, and another workspace is not held to
    // the first one's count.
    for n in 0..19 {
        store
            .create_credit_top_up(&intent(a, 100 + n, &[1], now))
            .await
            .unwrap();
    }
    assert!(matches!(
        store.create_credit_top_up(&intent(a, 200, &[1], now)).await,
        Err(StoreError::Conflict(_))
    ));
    assert_eq!(
        store.list_open_credit_top_ups(a, now).await.unwrap().len(),
        OPEN_TOP_UPS_PER_WORKSPACE as usize
    );
    assert_eq!(
        store.list_open_credit_top_ups(b, now).await.unwrap().len(),
        2
    );

    // A transfer of exactly the amount credits A and closes the intent; a
    // replay of the same log, or the log read again, changes nothing.
    let log = transfer("0xabc", 3, first.amount_micros);
    assert_eq!(
        store.find_credit_top_up_workspace(&log, now).await.unwrap(),
        Some(a)
    );
    let done = credited(
        store
            .record_credit_deposit(&log, &DepositEffects::default(), now)
            .await
            .unwrap(),
    );
    assert_eq!(
        (done.workspace_id, done.top_up_id, done.balance_micros),
        (a, first.id, 5_000_007)
    );
    assert!(done.event.is_none(), "no admins were named");
    assert_eq!(
        store
            .record_credit_deposit(&log, &DepositEffects::default(), now)
            .await
            .unwrap(),
        DepositOutcome::Seen
    );
    assert_eq!(store.credit_balance(a).await.unwrap(), 5_000_007);
    assert_eq!(store.credit_balance(b).await.unwrap(), 0);
    assert!(store
        .list_open_credit_top_ups(a, now)
        .await
        .unwrap()
        .iter()
        .all(|t| t.id != first.id));
    let ledger = store.list_credit_ledger(a, 10).await.unwrap();
    assert_eq!(ledger.len(), 1);
    assert_eq!(
        (
            ledger[0].kind,
            ledger[0].tx_hash.as_deref(),
            ledger[0].log_index,
            ledger[0].top_up_id
        ),
        (
            CreditEntryKind::Deposit,
            Some("0xabc"),
            Some(3),
            Some(first.id)
        )
    );
    assert_eq!(
        (ledger[0].source, ledger[0].external_ref.as_deref()),
        (Some(CreditSource::UsdcBase), Some("0xabc:3"))
    );
    assert!(store.list_credit_ledger(b, 10).await.unwrap().is_empty());

    // Its amount is free again: a new intent may take the offset.
    let reused = store
        .create_credit_top_up(&intent(b, 5, &[7], now))
        .await
        .unwrap();
    assert_eq!(reused.amount_micros, 5_000_007);

    // A transfer that matches nothing is kept for the operator, once; so is
    // the same amount sent again after its intent was paid.
    let stray = transfer("0xdef", 0, 42);
    assert_eq!(
        store
            .record_credit_deposit(&stray, &DepositEffects::default(), now)
            .await
            .unwrap(),
        DepositOutcome::Unmatched
    );
    assert_eq!(
        store
            .record_credit_deposit(&stray, &DepositEffects::default(), now)
            .await
            .unwrap(),
        DepositOutcome::Seen
    );
    let unmatched = store.list_unmatched_credit_deposits(10).await.unwrap();
    assert_eq!(unmatched.len(), 1);
    assert_eq!(
        (unmatched[0].amount_micros, unmatched[0].amount.as_str()),
        (42, "0.000042")
    );
    assert_eq!(store.credit_balance(b).await.unwrap(), 0);

    // An intent past its expiry still matches within the grace; one past the
    // grace does not, and its amount is given back.
    let late = store
        .create_credit_top_up(&intent(b, 9, &[9], now - chrono::Duration::hours(30)))
        .await
        .unwrap();
    assert!(store
        .list_open_credit_top_ups(b, now)
        .await
        .unwrap()
        .iter()
        .all(|t| t.id != late.id));
    let late_paid = credited(
        store
            .record_credit_deposit(
                &transfer("0x1a7e", 1, late.amount_micros),
                &DepositEffects::default(),
                now,
            )
            .await
            .unwrap(),
    );
    assert_eq!(late_paid.workspace_id, b);
    let stale = store
        .create_credit_top_up(&intent(b, 11, &[3], now - chrono::Duration::hours(72)))
        .await
        .unwrap();
    assert_eq!(
        store
            .record_credit_deposit(
                &transfer("0x57a1e", 0, stale.amount_micros),
                &DepositEffects::default(),
                now,
            )
            .await
            .unwrap(),
        DepositOutcome::Unmatched
    );

    // Charges: each unit past the allowance once, rounded up, never twice.
    let c = workspace(store, "credits-c").await;
    store
        .grant_credits(&CreditGrant {
            workspace_id: c,
            usd_micros: 1_000,
            note: "trial".into(),
            at: now,
        })
        .await
        .unwrap();
    let price = OveragePrice {
        micros: 100,
        per: 1,
    };
    let charge = |through_units, month: &str| CreditCharge {
        workspace_id: c,
        meter: UsageMeter::McpCalls,
        month: month.into(),
        through_units,
        price,
        at: now,
    };
    let paid = store.charge_credits(&charge(10, "2026-11")).await.unwrap();
    assert_eq!(
        (paid.charged_micros, paid.unpaid_micros, paid.balance_micros),
        (1_000, 0, 0)
    );
    let again = store.charge_credits(&charge(10, "2026-11")).await.unwrap();
    assert_eq!((again.charged_micros, again.balance_micros), (0, 0));
    let short = store.charge_credits(&charge(15, "2026-11")).await.unwrap();
    assert_eq!(
        (
            short.charged_micros,
            short.unpaid_micros,
            short.balance_micros
        ),
        (500, 500, -500)
    );
    // Another month starts from zero units.
    let next_month = store.charge_credits(&charge(1, "2026-12")).await.unwrap();
    assert_eq!(
        (next_month.charged_micros, next_month.unpaid_micros),
        (100, 100)
    );
    assert_eq!(store.credit_balance(c).await.unwrap(), -600);
    let charges: Vec<_> = store
        .list_credit_ledger(c, 10)
        .await
        .unwrap()
        .into_iter()
        .filter(|e| e.kind == CreditEntryKind::Charge)
        .collect();
    assert_eq!(charges.len(), 3);
    let november_short = charges
        .iter()
        .find(|e| e.month.as_deref() == Some("2026-11") && e.unpaid_micros > 0)
        .unwrap();
    assert_eq!(
        (november_short.units, november_short.through_units),
        (Some(5), Some(15))
    );

    // A deposit that leaves the balance at or below zero lifts nothing; one
    // that brings it above zero lifts the month's crossings on the meters
    // named, and tells the admins named for that workspace.
    let admin = store
        .create_member(NewMember {
            workspace_id: c,
            handle: "admin".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap()
        .id;
    for meter in [UsageMeter::McpCalls, UsageMeter::StoredBytes] {
        store
            .mark_over_allowance(
                &OverAllowance {
                    workspace_id: c,
                    meter,
                    month: "2026-11".into(),
                    quantity: 11,
                    allowance: 10,
                    marked_at: now,
                },
                &[],
            )
            .await
            .unwrap();
    }
    let effects = DepositEffects {
        month: "2026-11".into(),
        lift: vec![UsageMeter::McpCalls],
        admins_of: Some(c),
        admins: vec![admin],
    };
    let small = store
        .create_credit_top_up(&intent(c, 1, &[5], now))
        .await
        .unwrap();
    // Short of the intent by a micro: not its amount, so not matched.
    assert_eq!(
        store
            .record_credit_deposit(
                &transfer("0x5a11", 0, small.amount_micros - 1),
                &effects,
                now
            )
            .await
            .unwrap(),
        DepositOutcome::Unmatched
    );
    let debt_paid = credited(
        store
            .record_credit_deposit(&transfer("0x5a12", 0, small.amount_micros), &effects, now)
            .await
            .unwrap(),
    );
    assert_eq!(debt_paid.balance_micros, 1_000_005 - 600);
    assert_eq!(debt_paid.lifted, vec![UsageMeter::McpCalls]);
    let still: Vec<_> = store
        .list_over_allowance(c, "2026-11")
        .await
        .unwrap()
        .into_iter()
        .map(|m| m.meter)
        .collect();
    assert_eq!(
        still,
        vec![UsageMeter::StoredBytes],
        "a level is never lifted"
    );
    let event = debt_paid.event.expect("the admins were told");
    assert_eq!(event.kind, EventKind::CreditsDeposited);
    let told = store.list_notifications(admin, false, 10).await.unwrap();
    assert_eq!(
        (told.len(), told[0].kind, told[0].source_log_id),
        (1, EventKind::CreditsDeposited, event.id)
    );

    // The debt case: a deposit that does not cover what is owed lifts nothing.
    let d = workspace(store, "credits-d").await;
    store
        .charge_credits(&CreditCharge {
            workspace_id: d,
            ..charge(30_000, "2026-11")
        })
        .await
        .unwrap();
    store
        .mark_over_allowance(
            &OverAllowance {
                workspace_id: d,
                meter: UsageMeter::McpCalls,
                month: "2026-11".into(),
                quantity: 11,
                allowance: 10,
                marked_at: now,
            },
            &[],
        )
        .await
        .unwrap();
    let owed = store
        .create_credit_top_up(&intent(d, 1, &[2], now))
        .await
        .unwrap();
    let partial = credited(
        store
            .record_credit_deposit(
                &transfer("0xd0", 0, owed.amount_micros),
                &DepositEffects {
                    admins_of: Some(d),
                    admins: vec![],
                    ..effects.clone()
                },
                now,
            )
            .await
            .unwrap(),
    );
    assert!(partial.balance_micros < 0);
    assert!(partial.lifted.is_empty());
    assert_eq!(
        store.list_over_allowance(d, "2026-11").await.unwrap().len(),
        1
    );

    // The watcher's cursor only moves forward.
    assert_eq!(
        store.credit_cursor(CreditChain::Base, TO).await.unwrap(),
        None
    );
    store
        .set_credit_cursor(CreditChain::Base, TO, 120)
        .await
        .unwrap();
    store
        .set_credit_cursor(CreditChain::Base, TO, 90)
        .await
        .unwrap();
    assert_eq!(
        store.credit_cursor(CreditChain::Base, TO).await.unwrap(),
        Some(120)
    );
    assert_eq!(
        store
            .credit_cursor(CreditChain::BaseSepolia, TO)
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn credits_match_deposits_once_and_charge_each_unit_once_sqlite() {
    let pool = maidan_search::sqlite_pool_options()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    run_suite(&SqliteStore::for_tests(pool)).await;
}

#[tokio::test]
async fn credits_match_deposits_once_and_charge_each_unit_once_postgres() {
    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            maidan_store::test_support::docker::skip_start_failure(err).await;
            return;
        }
    };
    let host = container.get_host().await.expect("container host");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("container port");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&format!(
            "postgres://postgres:postgres@{host}:{port}/postgres"
        ))
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    run_suite(&PostgresStore::for_tests(pool)).await;
}

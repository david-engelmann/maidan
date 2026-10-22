//! The AWS KMS custody end to end: the production `AwsKmsCustody`, on the
//! real AWS SDK, against an in-memory fake of KMS's JSON API (CreateKey,
//! Encrypt, Decrypt, ScheduleKeyDeletion), under a store that seals messages.
//!
//! The fake binds each ciphertext to its key and encryption context and
//! refuses a key pending deletion, as KMS does, so the tests check what the
//! custody asks for, not only that it asks.

// The fake KMS is a plain axum server, not the API (see clippy.toml).
#![allow(clippy::disallowed_methods)]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::{
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing, Json, Router,
};
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use maidan_store::kms_custody::AwsKmsCustody;
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::*;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;
use uuid::Uuid;

#[derive(Default)]
struct FakeKms {
    /// Key id to (its workspace tag, the deletion window once scheduled).
    keys: HashMap<String, (String, Option<i64>)>,
    /// Ciphertext to (key id, encryption context, plaintext).
    ciphertexts: HashMap<String, (String, Value, String)>,
    /// Every call, by its `X-Amz-Target` operation.
    calls: Vec<String>,
}

type Shared = Arc<Mutex<FakeKms>>;

fn arn(id: &str) -> String {
    format!("arn:aws:kms:us-east-1:000000000000:key/{id}")
}

fn key_of(key_id: &str) -> String {
    key_id.rsplit('/').next().unwrap_or(key_id).to_string()
}

fn refuse(kind: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        [("content-type", "application/x-amz-json-1.1")],
        Json(json!({"__type": kind, "message": kind})),
    )
        .into_response()
}

fn answer(body: Value) -> Response {
    (
        StatusCode::OK,
        [("content-type", "application/x-amz-json-1.1")],
        Json(body),
    )
        .into_response()
}

async fn kms(
    axum::extract::State(fake): axum::extract::State<Shared>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let target = headers
        .get("x-amz-target")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .trim_start_matches("TrentService.")
        .to_string();
    let body: Value = serde_json::from_slice(&body).unwrap();
    let mut fake = fake.lock().unwrap();
    fake.calls.push(target.clone());
    match target.as_str() {
        "CreateKey" => {
            assert_eq!(body["KeyUsage"], "ENCRYPT_DECRYPT");
            assert_eq!(body["KeySpec"], "SYMMETRIC_DEFAULT");
            let workspace = body["Tags"][0]["TagValue"].as_str().unwrap().to_string();
            assert_eq!(body["Tags"][0]["TagKey"], "maidan:workspace");
            let id = Uuid::now_v7().to_string();
            fake.keys.insert(id.clone(), (workspace, None));
            answer(json!({"KeyMetadata": {
                "KeyId": id, "Arn": arn(&id), "AWSAccountId": "000000000000",
                "Enabled": true, "KeyState": "Enabled", "CreationDate": 1.8e9,
            }}))
        }
        "Encrypt" => {
            let id = key_of(body["KeyId"].as_str().unwrap());
            if !fake.keys.contains_key(&id) {
                return refuse("NotFoundException");
            }
            let blob = STANDARD.encode(Uuid::now_v7().as_bytes());
            fake.ciphertexts.insert(
                blob.clone(),
                (
                    id.clone(),
                    body["EncryptionContext"].clone(),
                    body["Plaintext"].as_str().unwrap().to_string(),
                ),
            );
            answer(
                json!({"CiphertextBlob": blob, "KeyId": arn(&id), "EncryptionAlgorithm": "SYMMETRIC_DEFAULT"}),
            )
        }
        "Decrypt" => {
            let id = key_of(body["KeyId"].as_str().unwrap());
            if fake
                .keys
                .get(&id)
                .is_some_and(|(_, window)| window.is_some())
            {
                return refuse("KMSInvalidStateException");
            }
            match fake
                .ciphertexts
                .get(body["CiphertextBlob"].as_str().unwrap())
            {
                Some((owner, context, plaintext))
                    if *owner == id && *context == body["EncryptionContext"] =>
                {
                    answer(
                        json!({"Plaintext": plaintext, "KeyId": arn(&id), "EncryptionAlgorithm": "SYMMETRIC_DEFAULT"}),
                    )
                }
                _ => refuse("InvalidCiphertextException"),
            }
        }
        "ScheduleKeyDeletion" => {
            let id = key_of(body["KeyId"].as_str().unwrap());
            let window = body["PendingWindowInDays"].as_i64().unwrap();
            match fake.keys.get_mut(&id) {
                None => refuse("NotFoundException"),
                Some((_, Some(_))) => refuse("KMSInvalidStateException"),
                Some((_, pending)) => {
                    *pending = Some(window);
                    answer(
                        json!({"KeyId": arn(&id), "DeletionDate": 1.9e9, "KeyState": "PendingDeletion", "PendingWindowInDays": window}),
                    )
                }
            }
        }
        other => panic!("the custody called {other}"),
    }
}

async fn spawn_kms(fake: Shared) -> String {
    let app = Router::new()
        .route("/", routing::post(kms))
        .with_state(fake);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

fn custody(endpoint: &str, deletion_days: i32) -> Arc<AwsKmsCustody> {
    let config = aws_sdk_kms::Config::builder()
        .behavior_version(aws_sdk_kms::config::BehaviorVersion::latest())
        .region(aws_sdk_kms::config::Region::new("us-east-1"))
        .credentials_provider(aws_sdk_kms::config::Credentials::new(
            "AKIDTEST", "secret", None, None, "test",
        ))
        .endpoint_url(endpoint)
        .build();
    Arc::new(AwsKmsCustody::new(
        aws_sdk_kms::Client::from_conf(config),
        deletion_days,
    ))
}

fn keyring(custody: &Arc<AwsKmsCustody>) -> Arc<ContentKeyring> {
    Arc::new(ContentKeyring::new([7; 32], Vec::new()).with_custody(custody.clone()))
}

struct Room {
    ws: Workspace,
    alice: Member,
    thread: Thread,
}

async fn room(store: &dyn Store, name: &str) -> Room {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let alice = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "alice".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "general".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("t".into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    Room { ws, alice, thread }
}

async fn post(store: &dyn Store, room: &Room, body: &str) -> StoredEvent {
    store
        .post_message_with_event(
            NewMessage {
                thread_id: room.thread.id,
                author_id: room.alice.id,
                body: body.into(),
                metadata: json!({}),
                content: None,
            },
            None,
        )
        .await
        .unwrap()
        .1
}

async fn body_of(store: &dyn Store, log_id: i64) -> Result<String, StoreError> {
    let read = store.get_stored_event(log_id).await?;
    match read.opened_event().unwrap() {
        Event::MessagePosted { message, .. } => Ok(message.body),
        other => panic!("not a message: {other:?}"),
    }
}

/// Each workspace's key is a KMS key of its own, tagged with the workspace,
/// and wrapped under an encryption context naming the workspace key. A
/// replica with nothing in memory decrypts through KMS. Erasing a workspace
/// schedules its KMS key's deletion, after which KMS refuses its ciphertext,
/// while the other workspace reads on.
#[tokio::test]
async fn each_workspace_key_is_a_kms_key_and_erasure_schedules_its_deletion() {
    let fake: Shared = Arc::default();
    let endpoint = spawn_kms(fake.clone()).await;
    let custody = custody(&endpoint, 7);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    maidan_store::configure_sqlite_pool(&pool).await.unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone(), keyring(&custody)));

    let a = room(store.as_ref(), "kms-a").await;
    let b = room(store.as_ref(), "kms-b").await;
    let in_a = post(store.as_ref(), &a, "words of a").await;
    let in_b = post(store.as_ref(), &b, "words of b").await;
    {
        let fake = fake.lock().unwrap();
        let mut tags: Vec<&String> = fake.keys.values().map(|(ws, _)| ws).collect();
        tags.sort();
        let mut expected = vec![a.ws.id.0.to_string(), b.ws.id.0.to_string()];
        expected.sort();
        assert_eq!(
            tags,
            expected.iter().collect::<Vec<_>>(),
            "a KMS key per workspace"
        );
        assert!(
            fake.ciphertexts
                .values()
                .all(|(_, context, _)| context["maidan:workspace_key"].is_string()),
            "every wrap names its workspace key"
        );
    }
    let (custody_name, wrapping): (String, String) = sqlx::query_as(
        "SELECT custody, wrapping_key_id FROM maidan_workspace_keys WHERE workspace_id = ?",
    )
    .bind(a.ws.id.0)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(custody_name, "aws-kms");
    assert!(wrapping.starts_with("aws-kms:arn:aws:kms:"), "{wrapping}");

    let replica: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone(), keyring(&custody)));
    assert_eq!(
        body_of(replica.as_ref(), in_a.id).await.unwrap(),
        "words of a"
    );
    assert_eq!(
        body_of(replica.as_ref(), in_b.id).await.unwrap(),
        "words of b"
    );
    let decrypts = |fake: &Shared| {
        fake.lock()
            .unwrap()
            .calls
            .iter()
            .filter(|c| *c == "Decrypt")
            .count()
    };
    assert_eq!(decrypts(&fake), 2, "the cold replica decrypted through KMS");

    store.erase_workspace(a.ws.id).await.unwrap();
    {
        let fake = fake.lock().unwrap();
        let windows: Vec<(String, Option<i64>)> = fake
            .keys
            .values()
            .map(|(ws, window)| (ws.clone(), *window))
            .collect();
        assert!(
            windows.contains(&(a.ws.id.0.to_string(), Some(7))),
            "{windows:?}"
        );
        assert!(
            windows.contains(&(b.ws.id.0.to_string(), None)),
            "{windows:?}"
        );
    }
    let cold: Arc<dyn Store> = Arc::new(SqliteStore::new(pool, keyring(&custody)));
    assert_eq!(body_of(cold.as_ref(), in_b.id).await.unwrap(), "words of b");
}

/// One workspace's wrapped key does not open as another's: KMS refuses a
/// ciphertext under another workspace key's encryption context, and under a
/// key it was not wrapped by. A destroyed key opens nothing, and destroying
/// it again succeeds.
#[tokio::test]
async fn one_workspaces_kms_key_opens_nothing_of_anothers() {
    let fake: Shared = Arc::default();
    let endpoint = spawn_kms(fake.clone()).await;
    let custody = custody(&endpoint, 30);
    let (a_key, b_key) = (Uuid::now_v7(), Uuid::now_v7());
    let secret = ContentKey::generate();
    let a = custody.wrap(a_key, Uuid::now_v7(), &secret).await.unwrap();
    let b = custody
        .wrap(b_key, Uuid::now_v7(), &ContentKey::generate())
        .await
        .unwrap();
    assert_eq!(custody.unwrap(a_key, &a).await.unwrap(), secret);

    let as_b = custody.unwrap(b_key, &a).await.unwrap_err();
    assert!(matches!(as_b, SealError::Custody(_)), "{as_b}");
    let under_b = WrappedKey {
        kek_id: b.kek_id.clone(),
        blob: a.blob.clone(),
    };
    assert!(custody.unwrap(a_key, &under_b).await.is_err());
    assert!(!as_b.to_string().contains("secret"), "{as_b}");

    custody.destroy(&a).await.unwrap();
    custody.destroy(&a).await.unwrap();
    assert!(custody.unwrap(a_key, &a).await.is_err());
    assert_eq!(custody.unwrap(b_key, &b).await.map(|_| ()), Ok(()));
    let window = fake
        .lock()
        .unwrap()
        .keys
        .values()
        .find_map(|(_, window)| *window);
    assert_eq!(window, Some(30));
}

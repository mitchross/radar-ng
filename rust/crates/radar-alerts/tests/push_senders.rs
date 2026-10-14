//! APNs and FCM senders against a local axum mock, the PUSH_DISABLED gate, and the storm-watch fan-out
//! (spec-temporal section 8, spec-temporal 4.11 and 8.1). Replays `testdata/push_apns.json` and
//! `testdata/push_fcm.json`, which were produced from the Python `temporal/shared/push.py`.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::extract::{Path as AxPath, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use jsonwebtoken::{Algorithm, DecodingKey, Header, Validation, decode, decode_header};
use radar_alerts::push::{
    PushConfig, PushError, PushPayload, PushSender, PushToken, send_push_notification,
};
use radar_alerts::storm_watch::{delete_push_token, fan_out_push_to_user, persist_push_token};
use radar_core::push_tokens::PushTokenStore;
use radar_types::payloads::push::{FanOutPushInput, PushTokenInput};
use serde_json::{Value, json};

fn testdata(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("testdata")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn key_file(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("testdata/keys")
        .join(name)
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect()
}

#[derive(Clone, Debug)]
struct Captured {
    path: String,
    headers: HashMap<String, String>,
    body: String,
}

struct Mock {
    apns: Vec<Captured>,
    apns_status: (u16, String),
    apns_overrides: HashMap<String, (u16, String)>,
    fcm: Vec<Captured>,
    fcm_status: (u16, String),
    oauth_bodies: Vec<String>,
}

type SharedMock = Arc<Mutex<Mock>>;

fn capture(path: String, headers: &HeaderMap, body: String) -> Captured {
    let headers = headers
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_owned(),
                value.to_str().unwrap_or("").to_owned(),
            )
        })
        .collect();
    Captured {
        path,
        headers,
        body,
    }
}

async fn apns_send(
    State(mock): State<SharedMock>,
    AxPath(token): AxPath<String>,
    headers: HeaderMap,
    body: String,
) -> (StatusCode, String) {
    let mut mock = mock.lock().unwrap();
    mock.apns
        .push(capture(format!("/3/device/{token}"), &headers, body));
    let (status, text) = mock
        .apns_overrides
        .get(&token)
        .cloned()
        .unwrap_or_else(|| mock.apns_status.clone());
    (StatusCode::from_u16(status).unwrap(), text)
}

async fn oauth_token(State(mock): State<SharedMock>, body: String) -> Json<Value> {
    mock.lock().unwrap().oauth_bodies.push(body);
    Json(json!({"access_token": "ya29.test", "expires_in": 3599, "token_type": "Bearer"}))
}

async fn fcm_send(
    State(mock): State<SharedMock>,
    AxPath(project): AxPath<String>,
    headers: HeaderMap,
    body: String,
) -> (StatusCode, String) {
    let mut mock = mock.lock().unwrap();
    mock.fcm.push(capture(
        format!("/v1/projects/{project}/messages:send"),
        &headers,
        body,
    ));
    let (status, text) = mock.fcm_status.clone();
    (StatusCode::from_u16(status).unwrap(), text)
}

async fn spawn_mock() -> (String, SharedMock) {
    let mock: SharedMock = Arc::new(Mutex::new(Mock {
        apns: Vec::new(),
        apns_status: (200, String::new()),
        apns_overrides: HashMap::new(),
        fcm: Vec::new(),
        fcm_status: (200, "{}".to_owned()),
        oauth_bodies: Vec::new(),
    }));
    let app = Router::new()
        .route("/3/device/{token}", post(apns_send))
        .route("/token", post(oauth_token))
        .route("/v1/projects/{project}/messages:send", post(fcm_send))
        .with_state(Arc::clone(&mock));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), mock)
}

/// A closed local port, for tests that must not reach the network.
async fn closed_port_url() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    format!("http://127.0.0.1:{port}")
}

fn config(base: &str) -> PushConfig {
    PushConfig {
        disabled: false,
        apns_host: base.to_owned(),
        apns_topic: "me.example.radar".to_owned(),
        apns_key_id: "ABC123DEFG".to_owned(),
        apns_team_id: "TEAM123456".to_owned(),
        apns_key_path: key_file("apns-test.p8"),
        fcm_project_id: "radar-test".to_owned(),
        fcm_credentials_path: key_file("fcm-test-service-account.json"),
        fcm_base_url: base.to_owned(),
        fcm_token_url: format!("{base}/token"),
    }
}

fn make_sender(cfg: PushConfig, dir: &Path) -> (PushSender, PushTokenStore) {
    let store = PushTokenStore::new(dir);
    (PushSender::new(cfg, store.clone()).unwrap(), store)
}

fn token(value: &str, platform: &str) -> PushToken {
    PushToken {
        user_id: "u1".to_owned(),
        token: value.to_owned(),
        platform: platform.to_owned(),
    }
}

fn payload(title: &str, body: &str, collapse_id: &str, extra: &[(&str, &str)]) -> PushPayload {
    PushPayload {
        title: title.to_owned(),
        body: body.to_owned(),
        collapse_id: collapse_id.to_owned(),
        extra: extra
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect::<BTreeMap<_, _>>(),
    }
}

fn remaining_tokens(store: &PushTokenStore, user: &str) -> Vec<String> {
    store
        .list_for_user(user)
        .unwrap()
        .into_iter()
        .map(|t| t.token)
        .collect()
}

fn verify(jwt: &str, algorithm: Algorithm, public_pem: &Path) -> (Header, Value) {
    let header = decode_header(jwt).unwrap();
    let pem = std::fs::read(public_pem).unwrap();
    let key = match algorithm {
        Algorithm::ES256 => DecodingKey::from_ec_pem(&pem).unwrap(),
        _ => DecodingKey::from_rsa_pem(&pem).unwrap(),
    };
    let mut validation = Validation::new(algorithm);
    validation.validate_exp = false;
    validation.validate_aud = false;
    validation.required_spec_claims.clear();
    let data = decode::<Value>(jwt, &key, &validation).expect("signature must verify");
    (header, data.claims)
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap();
                out.push(u8::from_str_radix(hex, 16).unwrap());
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8(out).unwrap()
}

#[tokio::test]
async fn apns_requests_match_python_golden_and_jwt_verifies() {
    let fixture = testdata("push_apns.json");
    let (base, mock) = spawn_mock().await;
    let dir = tempfile::tempdir().unwrap();
    let (sender, _store) = make_sender(config(&base), dir.path());
    let config_fixture = &fixture["config"];
    assert_eq!(config_fixture["key_id"], "ABC123DEFG");

    for case in fixture["cases"].as_array().unwrap() {
        let token_value = case["token"].as_str().unwrap();
        let extra: Vec<(String, String)> = case["extra"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_owned()))
            .collect();
        let extra_refs: Vec<(&str, &str)> = extra
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let sent = payload(
            case["title"].as_str().unwrap(),
            case["body"].as_str().unwrap(),
            case["collapse_id"].as_str().unwrap(),
            &extra_refs,
        );
        send_push_notification(&sender, &token(token_value, "ios"), &sent)
            .await
            .unwrap();

        let captured = mock
            .lock()
            .unwrap()
            .apns
            .last()
            .cloned()
            .expect("a request was sent");
        assert_eq!(captured.path, format!("/3/device/{token_value}"));
        for (name, expected) in case["headers"].as_object().unwrap() {
            assert_eq!(
                captured.headers.get(name).map(String::as_str),
                expected.as_str(),
                "header {name}"
            );
        }
        let body: Value = serde_json::from_str(&captured.body).unwrap();
        assert_eq!(body, case["body_json"], "body for {token_value}");

        let authorization = captured.headers.get("authorization").unwrap();
        let prefix = case["authorization_prefix"].as_str().unwrap();
        let jwt = authorization
            .strip_prefix(&format!("{prefix} "))
            .expect("bearer prefix");
        let (header, claims) = verify(jwt, Algorithm::ES256, &key_file("apns-test.pub.pem"));
        assert_eq!(header.alg, Algorithm::ES256);
        assert_eq!(header.kid.as_deref(), Some("ABC123DEFG"));
        assert_eq!(claims["iss"], case["jwt_iss"]);
        let mut claim_keys: Vec<String> = claims.as_object().unwrap().keys().cloned().collect();
        claim_keys.sort();
        assert_eq!(Value::from(claim_keys), case["jwt_claims_keys"]);
    }
}

#[tokio::test]
async fn apns_status_handling_matches_python_golden() {
    let fixture = testdata("push_apns.json");
    let (base, mock) = spawn_mock().await;
    let dir = tempfile::tempdir().unwrap();
    let (sender, store) = make_sender(config(&base), dir.path());
    let sent = payload("t", "b", "c", &[]);

    for case in fixture["status_cases"].as_array().unwrap() {
        let status = case["status"].as_u64().unwrap() as u16;
        let text = case["text"].as_str().unwrap().to_owned();
        mock.lock().unwrap().apns_status = (status, text);
        store.upsert("u1", "tok-x", "ios").unwrap();

        let result = send_push_notification(&sender, &token("tok-x", "ios"), &sent).await;

        let deleted = strings(&case["deleted"]);
        let remaining = remaining_tokens(&store, "u1");
        assert_eq!(
            remaining.contains(&"tok-x".to_owned()),
            !deleted.contains(&"tok-x".to_owned()),
            "status {status}"
        );

        let outcome = &case["outcome"];
        if outcome["raised"].as_bool().unwrap() {
            let err = result.expect_err("status should fail");
            assert_eq!(
                err.to_string(),
                outcome["message"].as_str().unwrap(),
                "status {status}"
            );
            assert_eq!(
                !err.is_retryable(),
                outcome["non_retryable"].as_bool().unwrap(),
                "status {status}"
            );
        } else {
            result.expect("status should succeed");
        }
    }
}

#[tokio::test]
async fn fcm_requests_match_python_golden_and_assertion_verifies() {
    let fixture = testdata("push_fcm.json");
    let (base, mock) = spawn_mock().await;
    let dir = tempfile::tempdir().unwrap();
    let (sender, _store) = make_sender(config(&base), dir.path());
    let service_account: Value = serde_json::from_str(
        &std::fs::read_to_string(key_file("fcm-test-service-account.json")).unwrap(),
    )
    .unwrap();

    let mut replayed = 0;
    for case in fixture["cases"].as_array().unwrap() {
        let Some(send_body) = case.get("send_body") else {
            continue;
        };
        replayed += 1;
        let message = &send_body["message"];
        let extra: Vec<(String, String)> = message["data"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_owned()))
            .collect();
        let extra_refs: Vec<(&str, &str)> = extra
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let sent = payload(
            message["notification"]["title"].as_str().unwrap(),
            message["notification"]["body"].as_str().unwrap(),
            message["android"]["collapse_key"].as_str().unwrap(),
            &extra_refs,
        );
        let token_value = message["token"].as_str().unwrap();
        send_push_notification(&sender, &token(token_value, "android"), &sent)
            .await
            .unwrap();

        // OAuth exchange: form-encoded JWT bearer grant.
        let form = mock
            .lock()
            .unwrap()
            .oauth_bodies
            .last()
            .cloned()
            .expect("token exchange happened");
        let pairs: Vec<(String, String)> = form
            .split('&')
            .map(|pair| {
                let (k, v) = pair.split_once('=').unwrap();
                (percent_decode(k), percent_decode(v))
            })
            .collect();
        let mut keys: Vec<&str> = pairs.iter().map(|(k, _)| k.as_str()).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            strings(&case["oauth_form_keys"])
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
        );
        let grant = pairs.iter().find(|(k, _)| k == "grant_type").unwrap();
        assert_eq!(grant.1, case["oauth_grant_type"].as_str().unwrap());
        let assertion = pairs
            .iter()
            .find(|(k, _)| k == "assertion")
            .unwrap()
            .1
            .clone();

        let (header, claims) = verify(&assertion, Algorithm::RS256, &key_file("fcm-test.pub.pem"));
        assert_eq!(header.alg, Algorithm::RS256);
        assert_eq!(claims["iss"], service_account["client_email"]);
        assert_eq!(claims["iss"], case["assertion_claims"]["iss"]);
        assert_eq!(claims["scope"], case["assertion_claims"]["scope"]);
        assert_eq!(claims["aud"], "https://oauth2.googleapis.com/token");
        let iat = claims["iat"].as_i64().unwrap();
        assert_eq!(claims["exp"].as_i64().unwrap() - iat, 3600);

        // The send itself: path, auth header and exact JSON body.
        let captured = mock
            .lock()
            .unwrap()
            .fcm
            .last()
            .cloned()
            .expect("a send happened");
        assert_eq!(captured.path, "/v1/projects/radar-test/messages:send");
        assert_eq!(
            captured.headers.get("authorization").map(String::as_str),
            Some("Bearer ya29.test")
        );
        assert_eq!(
            captured.headers.get("content-type").map(String::as_str),
            Some("application/json")
        );
        let body: Value = serde_json::from_str(&captured.body).unwrap();
        assert_eq!(body, *send_body);
    }
    assert!(replayed >= 1, "the fixture has at least one FCM send case");
}

#[tokio::test]
async fn fcm_access_token_is_cached_across_sends() {
    let (base, mock) = spawn_mock().await;
    let dir = tempfile::tempdir().unwrap();
    let (sender, _store) = make_sender(config(&base), dir.path());
    let sent = payload("t", "b", "c", &[]);
    for _ in 0..2 {
        send_push_notification(&sender, &token("fcm-1", "android"), &sent)
            .await
            .unwrap();
    }
    let guard = mock.lock().unwrap();
    assert_eq!(
        guard.oauth_bodies.len(),
        1,
        "one OAuth exchange for two sends"
    );
    assert_eq!(guard.fcm.len(), 2);
}

#[tokio::test]
async fn fcm_status_handling_matches_python_golden() {
    let fixture = testdata("push_fcm.json");
    let (base, mock) = spawn_mock().await;
    let dir = tempfile::tempdir().unwrap();
    let (sender, store) = make_sender(config(&base), dir.path());
    let sent = payload("t", "b", "c", &[]);

    for case in fixture["status_cases"].as_array().unwrap() {
        let status = case["status"].as_u64().unwrap() as u16;
        let text = case["text"].as_str().unwrap().to_owned();
        mock.lock().unwrap().fcm_status = (status, text);
        store.upsert("u1", "fcm-tok-x", "android").unwrap();

        let result = send_push_notification(&sender, &token("fcm-tok-x", "android"), &sent).await;

        let deleted = strings(&case["deleted"]);
        let remaining = remaining_tokens(&store, "u1");
        assert_eq!(
            remaining.contains(&"fcm-tok-x".to_owned()),
            !deleted.contains(&"fcm-tok-x".to_owned()),
            "status {status}"
        );
        let outcome = &case["outcome"];
        if outcome["raised"].as_bool().unwrap() {
            let err: PushError = result.expect_err("status should fail");
            assert_eq!(
                err.to_string(),
                outcome["message"].as_str().unwrap(),
                "status {status}"
            );
            assert_eq!(
                !err.is_retryable(),
                outcome["non_retryable"].as_bool().unwrap(),
                "status {status}"
            );
        } else {
            result.expect("status should succeed");
        }
    }
}

#[tokio::test]
async fn disabled_gate_counts_success_and_never_reaches_the_network() {
    let (base, mock) = spawn_mock().await;
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config(&base);
    cfg.disabled = true;
    let (sender, store) = make_sender(cfg, dir.path());
    store.upsert("u1", "ios-1", "ios").unwrap();
    store.upsert("u1", "android-1", "android").unwrap();

    send_push_notification(
        &sender,
        &token("ios-1", "ios"),
        &payload("t", "b", "c", &[]),
    )
    .await
    .unwrap();
    let result = fan_out_push_to_user(
        &sender,
        &FanOutPushInput {
            user_id: "u1".to_owned(),
            title: "t".to_owned(),
            body: "b".to_owned(),
            collapse_id: "c".to_owned(),
            extra: BTreeMap::new(),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        result.sent, 2,
        "disabled sends count as successes (spec D6)"
    );

    let guard = mock.lock().unwrap();
    assert!(guard.apns.is_empty() && guard.fcm.is_empty() && guard.oauth_bodies.is_empty());
}

#[tokio::test]
async fn unknown_platform_and_missing_apns_configuration_are_non_retryable() {
    let (base, _mock) = spawn_mock().await;
    let dir = tempfile::tempdir().unwrap();
    let (sender, _store) = make_sender(config(&base), dir.path());
    let sent = payload("t", "b", "c", &[]);

    let err = send_push_notification(&sender, &token("x", "web"), &sent)
        .await
        .unwrap_err();
    assert_eq!(
        err,
        PushError::NonRetryable("unknown platform: web".to_owned())
    );

    let mut cfg = config(&base);
    cfg.apns_topic.clear();
    let (sender, _store) = make_sender(cfg, dir.path());
    let err = send_push_notification(&sender, &token("x", "ios"), &sent)
        .await
        .unwrap_err();
    assert!(!err.is_retryable());
    assert!(err.to_string().starts_with("APNS not configured"));
}

#[tokio::test]
async fn persist_and_delete_push_token_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let store = PushTokenStore::new(dir.path());
    let input = PushTokenInput {
        user_id: "u1".to_owned(),
        token: "tok-1".to_owned(),
        platform: "ios".to_owned(),
    };
    persist_push_token(&store, &input).await.unwrap();
    // Re-registering the same token replaces the row rather than adding one.
    let moved = PushTokenInput {
        platform: "android".to_owned(),
        ..input.clone()
    };
    persist_push_token(&store, &moved).await.unwrap();
    let rows = store.list_for_user("u1").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].platform, "android");

    assert_eq!(delete_push_token(&store, "tok-1").await.unwrap(), 1);
    assert_eq!(delete_push_token(&store, "tok-1").await.unwrap(), 0);
    assert!(remaining_tokens(&store, "u1").is_empty());
}

#[tokio::test]
async fn fan_out_counts_successes_and_deletes_dead_tokens() {
    let (base, mock) = spawn_mock().await;
    let dir = tempfile::tempdir().unwrap();
    let (sender, store) = make_sender(config(&base), dir.path());
    {
        let mut guard = mock.lock().unwrap();
        guard
            .apns_overrides
            .insert("tok-bad".to_owned(), (503, "ServiceUnavailable".to_owned()));
        guard
            .apns_overrides
            .insert("tok-dead".to_owned(), (410, String::new()));
    }
    for t in ["tok-good", "tok-bad", "tok-dead"] {
        store.upsert("u1", t, "ios").unwrap();
    }
    let result = fan_out_push_to_user(
        &sender,
        &FanOutPushInput {
            user_id: "u1".to_owned(),
            title: "Storm intensifying".to_owned(),
            body: "+12 dBZ".to_owned(),
            collapse_id: "watch:x:intensifying".to_owned(),
            extra: BTreeMap::from([("kind".to_owned(), "intensifying".to_owned())]),
        },
    )
    .await
    .unwrap();

    // One send succeeded. The retryable failure is logged and skipped, so the call is still Ok.
    assert_eq!(result.sent, 1);
    assert_eq!(
        mock.lock().unwrap().apns.len(),
        3,
        "every token was attempted"
    );
    let mut remaining = remaining_tokens(&store, "u1");
    remaining.sort();
    assert_eq!(
        remaining,
        vec!["tok-bad".to_owned(), "tok-good".to_owned()],
        "the dead token was deleted"
    );
}

#[tokio::test]
async fn fan_out_fails_when_every_send_fails() {
    let (base, mock) = spawn_mock().await;
    let dir = tempfile::tempdir().unwrap();
    let (sender, store) = make_sender(config(&base), dir.path());
    mock.lock().unwrap().apns_status = (503, "ServiceUnavailable".to_owned());
    store.upsert("u1", "tok-a", "ios").unwrap();
    store.upsert("u1", "tok-b", "ios").unwrap();
    let err = fan_out_push_to_user(
        &sender,
        &FanOutPushInput {
            user_id: "u1".to_owned(),
            title: "t".to_owned(),
            body: "b".to_owned(),
            collapse_id: "c".to_owned(),
            extra: BTreeMap::new(),
        },
    )
    .await
    .unwrap_err();
    assert!(
        err.to_string().contains("all 2 push sends failed"),
        "got {err:#}"
    );
}

#[tokio::test]
async fn fan_out_with_no_tokens_is_a_zero_success() {
    let (base, mock) = spawn_mock().await;
    let dir = tempfile::tempdir().unwrap();
    let (sender, _store) = make_sender(config(&base), dir.path());
    let result = fan_out_push_to_user(
        &sender,
        &FanOutPushInput {
            user_id: "nobody".to_owned(),
            title: "t".to_owned(),
            body: "b".to_owned(),
            collapse_id: "c".to_owned(),
            extra: BTreeMap::new(),
        },
    )
    .await
    .unwrap();
    assert_eq!(result.sent, 0);
    assert!(mock.lock().unwrap().apns.is_empty());
}

#[tokio::test]
async fn disabled_gate_still_applies_with_an_unreachable_host() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config(&closed_port_url().await);
    cfg.disabled = true;
    let (sender, _store) = make_sender(cfg, dir.path());
    tokio::time::timeout(
        Duration::from_secs(5),
        send_push_notification(&sender, &token("x", "ios"), &payload("t", "b", "c", &[])),
    )
    .await
    .expect("no network call is made")
    .unwrap();
}
